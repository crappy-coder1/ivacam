//! Relief / 3-axis ball-nose surfacing driver.
//!
//! Resolves the op's [`crate::project::ReliefSource`] into a target
//! [`SurfaceField`] — remapping brightness → Z across the op's depth range
//! for an image source, or cutting the STL's real Z directly (clamped to
//! tool reach) for a height-grid source — then
//! runs the drop-cutter raster engine ([`surface_mill`]) to get gouge-free
//! XYZ scanlines, and emits them with [`emit_vcarve_block`] — the same
//! per-point-Z emitter the `VCarve` / `Halfpipe` / `Thread` drivers use.
//! Like those it writes XYZ blocks straight to the post (no offset cascade).

use crate::cam::setup::Setup;
use crate::cam::surface::SurfaceField;
use crate::cam::surface_mill::{surface_mill, SurfaceMillParams};
use crate::gcode::{emit_vcarve_block, PostProcessor};
use crate::geometry::Point2;
use crate::pipeline::warnings::push_tool_fit_kind_warnings;
use crate::pipeline::{CancelToken, PipelineError, PipelineWarning};
use crate::project::{Op, OpKind, Project, ReliefGrid, ReliefSource};

fn find_source(project: &Project, id: u32) -> Option<&ReliefSource> {
    project.relief_sources.iter().find(|s| s.id == id)
}

/// True when the relief op references an existing, non-empty source — the
/// Level-1 emit gate (mirrors `halfpipe_would_emit` / `vcarve_would_emit`).
pub(in crate::pipeline) fn relief_would_emit(op: &Op, project: &Project) -> bool {
    let OpKind::ReliefMill { source_id, .. } = &op.kind else {
        return false;
    };
    find_source(project, *source_id).is_some_and(|s| !s.grid.is_empty())
}

/// Emit a relief-surfacing op. No-op (with a warning) when the source is
/// missing or malformed; the `would_emit` gate normally screens those out.
#[allow(clippy::too_many_arguments)]
pub(in crate::pipeline) fn run_relief_op<P: PostProcessor>(
    op: &Op,
    project: &Project,
    setup: &Setup,
    post: &mut P,
    last_pos: &mut Point2,
    warnings: &mut Vec<PipelineWarning>,
    _cancel: Option<&CancelToken>,
) -> Result<(), PipelineError> {
    let OpKind::ReliefMill {
        source_id,
        z_min_mm,
        z_max_mm,
        invert,
        scallop_height_mm,
        stepover_mm,
        scan_direction,
        along_step_mm,
    } = &op.kind
    else {
        return Ok(());
    };

    // Tool-kind gate (BallNose) + impossible-geometry checks live in the
    // shared helper so a cached replay surfaces them too.
    push_tool_fit_kind_warnings(op, project, setup, warnings);

    let tool = project
        .tools
        .iter()
        .find(|t| t.id == op.tool_id)
        .ok_or(PipelineError::UnknownTool(op.id, op.tool_id))?;

    let Some(source) = find_source(project, *source_id) else {
        return Ok(());
    };
    let expected = u64::from(source.cols) * u64::from(source.rows);
    if source.cols == 0
        || source.rows == 0
        || source.cell <= 0.0
        || source.grid.len() as u64 != expected
    {
        warnings.push(PipelineWarning::for_op(
            op.id,
            "relief_source_invalid",
            format!(
                "Relief op '{}': source #{source_id} has a malformed grid (cols × rows must equal the grid length and be non-empty).",
                op.name
            ),
        )
        .with_param("op_name", op.name.as_str())
        .with_param("source_id", source_id));
        return Ok(());
    }

    let radius = tool.diameter * 0.5;
    if radius <= 0.0 {
        return Ok(());
    }
    // A bull-nose surfaces with its corner-fillet tip (flat centre +
    // rounded corner); a ball-nose is the full-hemisphere case (corner =
    // radius). The drop-cutter + scallop use this corner radius.
    let corner_radius = match tool.kind {
        crate::project::ToolKind::BullNose => {
            tool.corner_radius_mm.unwrap_or(0.0).clamp(0.0, radius)
        }
        _ => radius,
    };

    // Resolve the source into a target field plus the tip Z window
    // [z_floor, z_top] the drop-cutter clamps to. The two grid kinds map
    // depth differently:
    //   * Grayscale: brightness is remapped to Z across [z_min, z_max]; the
    //     window is exactly that user range (ceiling never above stock top).
    //   * Heightgrid: the Z is REAL geometry (STL top already at 0), so the
    //     op's range degrades to a clamp — cut the full model depth by
    //     default (z_min == z_max == 0), or a shallower user-set floor.
    let (field, z_top, mut z_floor) = match &source.grid {
        ReliefGrid::Grayscale { brightness } => {
            let z_top = z_min_mm.max(*z_max_mm).min(0.0);
            let z_floor = z_min_mm.min(*z_max_mm);
            let field = SurfaceField::from_grayscale(
                source.origin,
                source.cell,
                source.cols,
                source.rows,
                brightness,
                *z_min_mm,
                *z_max_mm,
                *invert,
            );
            (field, z_top, z_floor)
        }
        ReliefGrid::Heightgrid { z } => {
            let field = SurfaceField::new(
                source.origin,
                source.cell,
                source.cols,
                source.rows,
                z.clone(),
            );
            let (field_min, field_max) = field.z_range();
            let z_top = f64::from(field_max).min(0.0);
            // A user-set negative limit clamps the floor shallower than the
            // model; the default (0) means "reach the model's deepest point".
            let user_floor = z_min_mm.min(*z_max_mm);
            let z_floor = if user_floor < 0.0 {
                f64::from(field_min).max(user_floor)
            } else {
                f64::from(field_min)
            };
            (field, z_top, z_floor)
        }
    };
    // Floor is clamped to what the flutes can reach (shared across kinds).
    if let Some(flute) = tool.flute_length_mm.filter(|v| *v > 0.0) {
        if z_floor < -flute {
            z_floor = -flute;
            warnings.push(PipelineWarning::for_op(
                op.id,
                "relief_tool_reach_exceeded",
                format!(
                    "Relief op '{}': the requested depth is deeper than ball-nose '{}' can reach (flute length {flute:.3} mm). Cut clipped to that depth — use a longer-flute tool or a shallower relief.",
                    op.name, tool.name
                ),
            )
            .with_param("op_name", op.name.as_str())
            .with_param("tool_name", tool.name.as_str())
            .with_param("flute_length", format!("{flute:.3}")));
        }
    }

    let params = SurfaceMillParams {
        tool_radius_mm: radius,
        corner_radius_mm: corner_radius,
        scallop_height_mm: *scallop_height_mm,
        stepover_mm: *stepover_mm,
        along_step_mm: *along_step_mm,
        direction: *scan_direction,
        z_floor_mm: z_floor,
        z_top_mm: z_top,
    };

    let polylines = surface_mill(&field, &params);
    if polylines.is_empty() {
        return Ok(());
    }
    emit_vcarve_block(setup, &polylines, post, last_pos);
    Ok(())
}
