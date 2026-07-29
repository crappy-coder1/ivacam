//! Two-sided (flip-stock) two-program emission (rt1.11.3.1, Phase 2b).
//!
//! A two-sided job cuts the front face, the operator flips the stock about a
//! single axis, then cuts the back face. This orchestrator turns ONE project
//! into TWO programs without touching the single-program pipeline: it splits
//! the op list by [`WorkpieceSide`], runs the existing [`run_pipeline`] once
//! per side, and composes the results.
//!
//! ## Coordinate model (see bd rt1.11 part B + the guard convention)
//!
//! * **XY** — back-side geometry is mirrored about the stock centre-line for
//!   the flip axis ([`crate::cam::flip::flip_segments_xy`]) so that, once the
//!   operator physically turns the stock over, the mirrored toolpath lands on
//!   the correct back-side features.
//! * **Z** — a `Back` op's depth schedule is authored *own-face-relative*
//!   (0 = the back face, negative = into the stock — the same numeric form a
//!   `Front` op uses for the top face; this is exactly what the conflict
//!   guard's `op_removal_mm` assumes). The back program therefore emits those
//!   depths *verbatim*, and the header instructs the operator to **re-zero Z
//!   to the new top (back) face**. Real stock thickness varies ±0.5 mm, so a
//!   physical re-zero — not a computed datum shift — is the reliable anchor.
//!   `flip_z` is deliberately NOT applied to emitted cut depths: with the
//!   physical re-zero it would double-transform. (It remains the sim/preview
//!   frame relationship, wired in the Phase-3 dual-surface preview.)
//!
//! ## Dowel registration
//!
//! The front program drills auto-placed dowel holes on the flip-axis
//! centre-line (the mirror-invariant line, so the same holes register the
//! stock in both setups); the back program references those positions in its
//! header rather than re-drilling them. Placement is stock-relative
//! ([`DowelPinConfig`]: `count`, `margin_mm`), spread toward the two ends and
//! inset from the edges so they sit outside a centred part.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{
    run_pipeline, warnings, PipelineError, PipelineRequest, PipelineResponse, PostProcessorKind,
};
use crate::cam::chaining::{classify_containment, segments_to_objects};
use crate::cam::flip::{flip_point_xy, flip_segments_xy};
use crate::geometry::{Point2, Segment, SegmentKind};
use crate::project::{
    DowelPinConfig, DrillCycle, FlipAxis, Op, OpKind, OpSource, Project, SourceCombine,
    StockConfig, ToolKind, WorkpieceSide,
};

/// Synthetic layer the injected dowel-drill op targets. Underscore-prefixed
/// like the text-layer render pool (`__text_<id>`) so it can't collide with an
/// imported layer name.
const DOWEL_LAYER: &str = "__dowels";

/// Result of a two-sided emission: the front program plus, for a genuine
/// two-sided job, the back program. `back` is `None` for a single-sided
/// project — the orchestrator then returns the front field unchanged from the
/// ordinary single-program path, so single-sided callers see byte-identical
/// output.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TwoSidedResponse {
    /// The front program (and, in a two-sided job, the one that drills the
    /// dowel registration holes).
    pub front: PipelineResponse,
    /// The back program — present only when the job is two-sided (stock flip
    /// registration set and at least one enabled `Back` op). Its geometry is
    /// mirrored and its header carries the flip + Z-re-zero instructions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back: Option<PipelineResponse>,
}

/// Whether `project` is a genuine two-sided job: it declares a stock flip
/// registration AND has at least one enabled `Back` op. A project with `Back`
/// ops but no flip registration (or vice-versa) is treated as single-sided —
/// the orchestrator can't mirror without an axis, and the front-only path
/// reproduces today's behaviour.
fn two_sided_registration(project: &Project) -> Option<(&StockConfig, FlipAxis)> {
    let stock = project.stock.as_ref()?;
    let flip = stock.flip.as_ref()?;
    let has_back = project
        .operations
        .iter()
        .any(|o| o.enabled && o.side == WorkpieceSide::Back);
    has_back.then_some((stock, flip.axis))
}

/// Run the two-sided pipeline: one program per stock face.
///
/// For a single-sided project (no flip registration, or no enabled `Back` op)
/// this is exactly [`run_pipeline`] wrapped as `TwoSidedResponse { front, back:
/// None }` — output is byte-identical to the single-program path. HPGL (a
/// pen-plotter dialect with no Z / milling) is always treated as single-sided.
///
/// For a two-sided project it:
/// 1. runs the conflict guard on the WHOLE project (opposing-overlap detection
///    needs both sides' extents, so it must precede the per-side split);
/// 2. emits the front program from the `Front` ops plus an injected dowel-drill
///    op;
/// 3. emits the back program from the `Back` ops with their XY mirrored about
///    the stock centre-line;
/// 4. prepends flip / re-zero / dowel instructions to each program header.
///
/// # Errors
///
/// Propagates any [`PipelineError`] from either per-side run, and
/// [`PipelineError::TwoSidedThrough`] from the whole-project conflict guard
/// when a front op would sever the stock before the flip.
pub fn run_pipeline_two_sided<F: Fn(&str, f64, &str)>(
    req: PipelineRequest,
    progress: F,
) -> Result<TwoSidedResponse, PipelineError> {
    let post_kind = req.post_processor.unwrap_or_default();

    // HPGL or non-two-sided → the ordinary single program, unchanged.
    let Some((stock, axis)) = two_sided_registration(&req.project) else {
        return Ok(TwoSidedResponse {
            front: run_pipeline(req, &progress)?,
            back: None,
        });
    };
    if post_kind == PostProcessorKind::Hpgl {
        return Ok(TwoSidedResponse {
            front: run_pipeline(req, &progress)?,
            back: None,
        });
    }

    let stock = stock.clone();
    let dowels = stock.flip.as_ref().and_then(|f| f.dowels.clone());
    let dowel_centres = dowels
        .as_ref()
        .map(|d| dowel_centres(&stock, axis, d))
        .unwrap_or_default();

    // (1) Correctness gate on the UNSPLIT project: opposing-overlap detection
    // compares a Front and a Back footprint, so both sides must be present.
    // A FrontSever refuses the whole job here, before either program emits.
    // The per-side sub-runs' own `two_sided_guard` then no-ops (front: no Back
    // op; back: no Front op). Runs before dowel injection so the registration
    // through-holes never count as a "front sever".
    let mut overlap_warnings = Vec::new();
    {
        let mut objects = segments_to_objects(&req.project.segments);
        classify_containment(&mut objects);
        warnings::two_sided_guard(&req.project, &objects, &mut overlap_warnings)?;
    }

    // (2) Front program: keep Front ops, prepend the dowel-drill op.
    let mut front_project = req.project.clone();
    front_project
        .operations
        .retain(|o| o.side == WorkpieceSide::Front);
    if !dowel_centres.is_empty() {
        if let Some(d) = dowels.as_ref() {
            inject_dowel_op(&mut front_project, &stock, &dowel_centres, d);
        }
    }
    let mut front = run_pipeline(
        PipelineRequest {
            project: front_project,
            post_processor: Some(post_kind),
            cps_post: req.cps_post.clone(),
        },
        &progress,
    )?;
    // Attach the whole-project overlap warnings to the front program (the one
    // the operator reads first).
    front.warnings.extend(overlap_warnings);
    front.gcode = prepend_lines(&front.gcode, &front_header(&dowel_centres));

    // (3) Back program: keep Back ops, mirror their XY about the stock centre.
    let mut back_project = req.project.clone();
    back_project
        .operations
        .retain(|o| o.side == WorkpieceSide::Back);
    flip_segments_xy(&mut back_project.segments, axis, &stock);
    let mut back = run_pipeline(
        PipelineRequest {
            project: back_project,
            post_processor: Some(post_kind),
            cps_post: req.cps_post.clone(),
        },
        &progress,
    )?;
    back.gcode = prepend_lines(&back.gcode, &back_header(axis, &stock, &dowel_centres));

    Ok(TwoSidedResponse {
        front,
        back: Some(back),
    })
}

/// Auto-place dowel-hole centres on the flip-axis centre-line — the line the
/// mirror leaves invariant, so a hole drilled from the front registers the
/// same point on the back after the flip. Holes are spread evenly toward the
/// two ends of that line, inset `margin_mm` from the edges so they clear a
/// centred part.
fn dowel_centres(stock: &StockConfig, axis: FlipAxis, cfg: &DowelPinConfig) -> Vec<Point2> {
    let count = cfg.count.max(1);
    let margin = cfg.margin_mm.max(0.0);
    let (ox, oy) = (stock.origin[0], stock.origin[1]);
    let cx = ox + stock.width_mm / 2.0;
    let cy = oy + stock.height_mm / 2.0;

    // `span` runs ALONG the flip line: X for an X-axis flip (mirror-Y line
    // y = cy), Y for a Y-axis flip (mirror-X line x = cx).
    let (lo, hi) = match axis {
        FlipAxis::X => (ox + margin, ox + stock.width_mm - margin),
        FlipAxis::Y => (oy + margin, oy + stock.height_mm - margin),
    };
    (0..count)
        .map(|i| {
            let t = if count == 1 {
                0.5
            } else {
                f64::from(i) / f64::from(count - 1)
            };
            let along = lo + t * (hi - lo);
            match axis {
                FlipAxis::X => Point2::new(along, cy),
                FlipAxis::Y => Point2::new(cx, along),
            }
        })
        .collect()
}

/// Inject a dowel-drill op (targeting a synthetic point-geometry layer) as the
/// FIRST op of the front program, so the registration holes are drilled before
/// anything else moves. The holes go through the stock (+0.5 mm) so a pin can
/// pass through into the fixture. The drill tool is cloned from an existing
/// project tool (inheriting sensible feeds/speeds) with its kind/diameter
/// overridden.
fn inject_dowel_op(
    project: &mut Project,
    stock: &StockConfig,
    centres: &[Point2],
    cfg: &DowelPinConfig,
) {
    // Point geometry for each hole, on the synthetic dowel layer.
    let layer: Arc<str> = Arc::from(DOWEL_LAYER);
    for c in centres {
        project.segments.push(Segment {
            kind: SegmentKind::Point,
            start: *c,
            end: *c,
            bulge: 0.0,
            center: None,
            layer: Arc::clone(&layer),
            color: 7,
        });
    }

    // A drill tool: clone any existing tool for its feeds, override the bit.
    let tool_id = project.tools.iter().map(|t| t.id).max().unwrap_or(0) + 1;
    if let Some(template) = project.tools.first() {
        let mut tool = template.clone();
        tool.id = tool_id;
        tool.name = format!("{:.1}mm dowel drill", cfg.diameter_mm);
        tool.kind = ToolKind::Drill;
        tool.diameter = cfg.diameter_mm;
        tool.tip_diameter = None;
        project.tools.push(tool);
    }

    let op_id = project.operations.iter().map(|o| o.id).max().unwrap_or(0) + 1;
    let mut params = crate::project::OpParams::mill_default();
    params.start_depth = 0.0;
    // Through the stock into the fixture so the pin passes fully.
    params.depth = -(stock.thickness_mm + 0.5);
    params.fast_move_z = 5.0;

    let peck = cfg.diameter_mm.max(1.0);
    let dowel_op = Op {
        id: op_id,
        name: "Dowel registration holes".to_string(),
        enabled: true,
        kind: OpKind::Drill {
            cycle: DrillCycle::Peck {
                peck_step_mm: peck,
                dwell_sec: 0.0,
            },
            chamfer_after_width_mm: None,
            pattern: None,
            spot_first: None,
        },
        tool_id,
        finish_tool_id: None,
        source: OpSource::Layers {
            layers: vec![DOWEL_LAYER.to_string()],
            combine: SourceCombine::default(),
        },
        params,
        group: None,
        pin_order: true,
        side: WorkpieceSide::Front,
    };
    project.operations.insert(0, dowel_op);
}

/// Front-program header comment block.
fn front_header(dowel_centres: &[Point2]) -> Vec<String> {
    let mut lines = vec![
        "===== TWO-SIDED JOB: FRONT PROGRAM =====".to_string(),
        "Run this program FIRST.".to_string(),
    ];
    if dowel_centres.is_empty() {
        lines.push("No dowel registration configured — align the flip by hand.".to_string());
    } else {
        lines.push(format!(
            "Drills {} dowel registration hole(s) — leave the pins in for the back program.",
            dowel_centres.len()
        ));
    }
    lines.push("Do NOT unclamp the stock until the back program is set up.".to_string());
    lines
}

/// Back-program header comment block: flip axis, re-zero, dowel references.
fn back_header(axis: FlipAxis, stock: &StockConfig, dowel_centres: &[Point2]) -> Vec<String> {
    let axis_name = match axis {
        FlipAxis::X => "X",
        FlipAxis::Y => "Y",
    };
    let mut lines = vec![
        "===== TWO-SIDED JOB: BACK PROGRAM =====".to_string(),
        format!("1. FLIP the stock about the {axis_name} axis."),
    ];
    if dowel_centres.is_empty() {
        lines.push("2. Re-register the stock against your fixture.".to_string());
    } else {
        // Mirror the centres for reference (identity for on-axis holes, but
        // computed explicitly to document the front↔back mapping).
        let refs: Vec<String> = dowel_centres
            .iter()
            .map(|c| {
                let m = flip_point_xy(*c, axis, stock);
                format!("({:.2}, {:.2})", m.x, m.y)
            })
            .collect();
        lines.push(format!(
            "2. Seat the stock on the dowel pins at: {}.",
            refs.join(", ")
        ));
    }
    lines.push(
        "3. RE-ZERO Z to the new top (back) face before running — stock thickness varies."
            .to_string(),
    );
    lines
}

/// Prepend `;`-comment lines to a program. LinuxCNC / GRBL both treat a
/// leading `;` line as a full-line comment anywhere in the file, so a header
/// block at the very top is safe.
fn prepend_lines(gcode: &str, lines: &[String]) -> String {
    let mut out = String::with_capacity(gcode.len() + lines.len() * 48);
    for l in lines {
        out.push_str("; ");
        out.push_str(l);
        out.push('\n');
    }
    out.push_str(gcode);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::test_helpers::{endmill, pocket_op};
    use crate::pipeline::{run_pipeline, PipelineError};
    use crate::project::{FlipRegistration, Op, OpSource, Project, StockConfig, WorkpieceSide};

    /// `w × h` stock, 10 mm thick, top z = 0, X-axis flip (mirrors Y), with a
    /// 2-pin dowel registration.
    fn flip_stock(w: f64, h: f64) -> StockConfig {
        StockConfig {
            origin: [0.0, 0.0],
            width_mm: w,
            height_mm: h,
            thickness_mm: 10.0,
            top_z_mm: 0.0,
            flip: Some(FlipRegistration {
                axis: FlipAxis::X,
                dowels: Some(DowelPinConfig {
                    diameter_mm: 4.0,
                    count: 2,
                    margin_mm: 2.0,
                }),
            }),
        }
    }

    fn pocket(id: u32, side: WorkpieceSide, depth: f64) -> Op {
        let mut op = pocket_op(id, 1, OpSource::All);
        op.params.depth = depth;
        op.side = side;
        op
    }

    /// Largest Y coordinate emitted in `gcode` (naive `Y<num>` scan — enough
    /// to tell which half of the stock a program cuts in).
    fn max_y(gcode: &str) -> f64 {
        let mut m = f64::MIN;
        for tok in gcode.split_whitespace() {
            if let Some(rest) = tok.strip_prefix('Y') {
                if let Ok(v) = rest.parse::<f64>() {
                    m = m.max(v);
                }
            }
        }
        m
    }

    fn two_sided(w: f64, h: f64, ops: Vec<Op>) -> Project {
        let mut project = crate::pipeline::test_helpers::project_with(ops, vec![endmill(1, 3.0)]);
        project.stock = Some(flip_stock(w, h));
        project
    }

    fn run2(project: Project) -> Result<TwoSidedResponse, PipelineError> {
        run_pipeline_two_sided(
            PipelineRequest {
                project,
                post_processor: Some(PostProcessorKind::Linuxcnc),
                cps_post: None,
            },
            |_, _, _| {},
        )
    }

    /// A single-sided project (no Back op) emits exactly ONE program, and the
    /// front is byte-identical to the ordinary single-program pipeline.
    #[test]
    fn single_sided_emits_one_program_byte_identical() {
        let ops = vec![pocket(1, WorkpieceSide::Front, -3.0)];
        let mut project = crate::pipeline::test_helpers::project_with(ops, vec![endmill(1, 3.0)]);
        // Stock present but NO flip registration → single-sided.
        project.stock = Some(StockConfig {
            origin: [0.0, 0.0],
            width_mm: 20.0,
            height_mm: 20.0,
            thickness_mm: 10.0,
            top_z_mm: 0.0,
            flip: None,
        });
        let single = run_pipeline(
            PipelineRequest {
                project: project.clone(),
                post_processor: Some(PostProcessorKind::Linuxcnc),
                cps_post: None,
            },
            |_, _, _| {},
        )
        .expect("single-program run");
        let two = run2(project).expect("two-sided run on a single-sided project");
        assert!(
            two.back.is_none(),
            "single-sided job must not emit a back program"
        );
        assert_eq!(
            two.front.gcode, single.gcode,
            "single-sided front must be byte-identical to the single-program path"
        );
    }

    /// A two-sided project emits two programs, each with its own header; the
    /// back header names the flip axis and the Z re-zero.
    #[test]
    fn two_sided_emits_two_programs_with_headers() {
        let project = two_sided(
            20.0,
            40.0,
            vec![
                pocket(1, WorkpieceSide::Front, -3.0),
                pocket(2, WorkpieceSide::Back, -3.0),
            ],
        );
        let two = run2(project).expect("two-sided job emits");
        let back = two.back.expect("two-sided job has a back program");
        assert!(two.front.gcode.contains("FRONT PROGRAM"));
        assert!(back.gcode.contains("BACK PROGRAM"));
        assert!(
            back.gcode.contains("FLIP the stock about the X axis"),
            "back header must state the flip axis"
        );
        assert!(
            back.gcode.contains("RE-ZERO Z"),
            "back header must instruct the Z re-zero"
        );
    }

    /// The back program's geometry is mirrored about the stock centre-line: a
    /// feature in the bottom half of the stock (Y 0..20) machines in the top
    /// half (Y 20..40) of the back program.
    #[test]
    fn back_geometry_is_mirrored() {
        // 20×40 stock; closed_square(20) fills the bottom half (Y 0..20).
        let project = two_sided(
            20.0,
            40.0,
            vec![
                pocket(1, WorkpieceSide::Front, -3.0),
                pocket(2, WorkpieceSide::Back, -3.0),
            ],
        );
        let two = run2(project).expect("two-sided job emits");
        let back = two.back.expect("back program");
        let front_max = max_y(&two.front.gcode);
        let back_max = max_y(&back.gcode);
        assert!(
            front_max <= 20.5,
            "front cuts the bottom half only (max Y {front_max})"
        );
        assert!(
            back_max >= 35.0,
            "back is mirrored into the top half (max Y {back_max})"
        );
    }

    /// Dowel holes are drilled in the front program and only referenced in the
    /// back header (the back must not re-drill them).
    #[test]
    fn dowels_drilled_front_referenced_back() {
        let project = two_sided(
            20.0,
            40.0,
            vec![
                pocket(1, WorkpieceSide::Front, -3.0),
                pocket(2, WorkpieceSide::Back, -3.0),
            ],
        );
        let two = run2(project).expect("two-sided job emits");
        let back = two.back.expect("back program");
        // The dowel op is the only drill op → a peck cycle (G83) at each
        // auto-placed centre proves the front program drilled the registration
        // holes. Stock 20×40, margin 2, X-axis flip ⇒ centres on the y=20
        // centre-line at x=2 and x=18.
        let drilled = |x: &str| {
            two.front
                .gcode
                .lines()
                .any(|l| l.contains("G83") && l.contains(x))
        };
        assert!(drilled("X2 Y20"), "front drills the first dowel at (2, 20)");
        assert!(drilled("X18"), "front drills the second dowel at (18, 20)");
        assert!(
            back.gcode.contains("Seat the stock on the dowel pins"),
            "back header references the dowel positions"
        );
        assert!(
            !back.gcode.contains("G83"),
            "back program must not re-drill the dowels"
        );
    }

    /// The conflict guard still fires on the WHOLE project before the split: a
    /// front op cutting clean through the stock refuses both programs.
    #[test]
    fn front_through_cut_still_refuses() {
        let project = two_sided(
            20.0,
            20.0,
            vec![
                pocket(1, WorkpieceSide::Front, -12.0),
                pocket(2, WorkpieceSide::Back, -3.0),
            ],
        );
        match run2(project) {
            Err(PipelineError::TwoSidedThrough { op_id, .. }) => assert_eq!(op_id, 1),
            other => panic!("expected TwoSidedThrough refuse, got {other:?}"),
        }
    }

    /// Opposing overlapping cuts warn (non-fatal) and the warning rides on the
    /// front program the operator reads first.
    #[test]
    fn opposing_overlap_warns_on_front() {
        let project = two_sided(
            20.0,
            20.0,
            vec![
                pocket(1, WorkpieceSide::Front, -6.0),
                pocket(2, WorkpieceSide::Back, -6.0),
            ],
        );
        let two = run2(project).expect("overlapping job still emits");
        assert!(
            two.front
                .warnings
                .iter()
                .any(|w| w.kind == "two_sided_overlap"),
            "the whole-project overlap warning must ride on the front program"
        );
    }
}
