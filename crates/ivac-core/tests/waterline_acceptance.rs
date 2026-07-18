//! End-to-end acceptance test for the waterline rough → relief finish 3D
//! flow (bd ivac-58nl.8, follow-up to 58nl.3 stage 3).
//!
//! The stage-3 emit tests in `pipeline/tests.rs` prove per-level chain
//! emission — that a `WaterlineRough` op writes cut moves on the descending
//! slice levels and a `ReliefMill` op cuts the real STL Z. They do NOT prove
//! the combined rough-then-finish program is *correct against the target
//! surface*: no gouge below it, and a finish pass that actually cleans the
//! roughing staircase down to a bounded envelope.
//!
//! This test closes that gap. It:
//!   1. builds a pyramid-basin STL height grid (the target surface),
//!   2. roughs it with `OpKind::WaterlineRough` (flat endmill, constant-Z),
//!   3. finishes it with `OpKind::ReliefMill` (ball-nose drop-cutter),
//!   4. simulates the combined program into a sim heightmap — each op swept
//!      with its own tool profile — and
//!   5. classifies the carved material against the target with
//!      [`SurfaceField::deviation_of`], asserting:
//!        * NO gouge below the target surface (rough alone AND rough+finish),
//!        * the rough pass leaves real rest-stock (a staircase to clean),
//!        * the finish pass tightens that residual to a bounded, uniform
//!          envelope the rough pass does not achieve.

// The harness crosses f64 (geometry) / f32 (heightmap + surface) / u32 (grid
// dims) boundaries the underlying APIs require; each cast is deliberate.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::naive_bytecount,
    clippy::too_many_lines
)]

mod common;

use ivac_core::cam::surface::{Deviation, SurfaceField};
use ivac_core::cam::surface_mill::ScanDirection;
use ivac_core::gcode::preview::ToolpathSegment;
use ivac_core::geometry::Point2;
use ivac_core::project::{
    MachineConfig, Op, OpKind, OpParams, OpSource, Project, ReliefGrid, ReliefSource, StockConfig,
    ToolEntry, ToolKind, WorkOffset,
};
use ivac_core::schema::PostProcessorKind;
use ivac_core::sim::dexel::DexelField;
use ivac_core::sim::diagnostics::SimDiagnostics;
use ivac_core::sim::heightmap::ToolProfile;
use ivac_core::sim::sweep::sweep_range;

/// Row-major `n × n` pyramid-basin height grid at `cell` spacing: Z = 0 on
/// the border, sloping linearly down to `-depth` at the center — four planar
/// walls meeting in a pit. A horizontal slice at any intermediate level is a
/// closed contour, so waterline roughing produces a real staircase and the
/// ball-nose finish has sloped walls to clean.
fn pyramid_basin(n: u32, depth: f32) -> Vec<f32> {
    let center = (n - 1) as f32 * 0.5;
    let mut z = Vec::with_capacity((n * n) as usize);
    for iy in 0..n {
        for ix in 0..n {
            let edge = ix.min(iy).min(n - 1 - ix).min(n - 1 - iy) as f32;
            z.push(-depth * (edge / center));
        }
    }
    z
}

/// Simulate a subset of the program's ops into a fresh sim heightmap, each op
/// swept with its OWN tool profile (rough endmill vs finish ball-nose), then
/// return the carved material as a [`DexelField`] ready for deviation
/// classification. Ops are carved in the given order so the finish pass sees
/// the stock the rough pass left.
fn carve(
    stock: &StockConfig,
    cell_mm: f64,
    toolpath: &[ToolpathSegment],
    ops: &[(u32, &ToolEntry)],
) -> DexelField {
    let mut hm = common::build_heightmap(stock, cell_mm);
    for (op_id, tool) in ops {
        let segs: Vec<ToolpathSegment> = toolpath
            .iter()
            .filter(|s| s.op_id == *op_id)
            .cloned()
            .collect();
        let profile = ToolProfile::from_tool(tool);
        let mut diag = SimDiagnostics::default();
        sweep_range(
            &mut hm,
            &segs,
            0,
            segs.len(),
            &profile,
            &[],
            None,
            &mut diag,
        );
    }
    DexelField::from_heightmap(&hm, -(stock.thickness_mm as f32))
}

/// Count cells classified as `want` in a `deviation_of` result.
fn count(classes: &[u8], want: Deviation) -> usize {
    classes.iter().filter(|&&c| c == want as u8).count()
}

#[test]
fn waterline_rough_then_relief_finish_no_gouge_bounded_envelope() {
    // ── Target surface: 20 mm square pyramid basin, 6 mm deep ──────────────
    let n = 21u32;
    let src_cell = 1.0;
    let depth = 6.0f32;
    let z = pyramid_basin(n, depth);
    let source = ReliefSource {
        id: 1,
        name: "basin.stl".into(),
        origin: Point2::new(0.0, 0.0),
        cell: src_cell,
        cols: n,
        rows: n,
        grid: ReliefGrid::Heightgrid { z: z.clone() },
    };
    // Stock spans the source footprint [0, 20] with room below the -6 floor.
    let stock = StockConfig {
        origin: [0.0, 0.0],
        width_mm: (n - 1) as f64 * src_cell,
        height_mm: (n - 1) as f64 * src_cell,
        thickness_mm: 10.0,
        ..Default::default()
    };

    // ── Tools: 3 mm flat endmill (rough) + 2 mm ball-nose (finish) ─────────
    let mut rough_tool = common::endmill_tool(1, 3.0);
    rough_tool.flute_length_mm = Some(20.0);
    let mut finish_tool = common::endmill_tool(2, 2.0);
    finish_tool.kind = ToolKind::BallNose;
    finish_tool.flute_length_mm = Some(20.0);

    // ── Ops: waterline rough (op 1) then relief finish (op 2) ──────────────
    let rough = Op {
        id: 1,
        name: "Waterline rough".into(),
        enabled: true,
        kind: OpKind::WaterlineRough {
            source_id: 1,
            z_step_mm: 2.0,
            stepover_mm: 1.5,
            floor_z_mm: 0.0, // full model depth
        },
        tool_id: 1,
        finish_tool_id: None,
        source: OpSource::All,
        params: OpParams::mill_default(),
        group: None,
        pin_order: false,
        side: ivac_core::project::WorkpieceSide::Front,
    };
    let finish = Op {
        id: 2,
        name: "Relief finish".into(),
        enabled: true,
        kind: OpKind::ReliefMill {
            source_id: 1,
            z_min_mm: 0.0, // default range ⇒ cut the real model depth
            z_max_mm: 0.0,
            invert: false,
            scallop_height_mm: 0.0,
            stepover_mm: Some(0.5),
            scan_direction: ScanDirection::AlongX,
            along_step_mm: 0.5,
        },
        tool_id: 2,
        finish_tool_id: None,
        source: OpSource::All,
        params: OpParams::mill_default(),
        group: None,
        pin_order: false,
        side: ivac_core::project::WorkpieceSide::Front,
    };

    let project = Project {
        segments: Vec::new(),
        machine: MachineConfig::default(),
        tools: vec![rough_tool.clone(), finish_tool.clone()],
        operations: vec![rough, finish],
        fixtures: Vec::default(),
        text_layers: Vec::new(),
        work_offset: WorkOffset::default(),
        stock: Some(stock.clone()),
        relief_sources: vec![source.clone()],
        group_ops_by_tool: false,
    };

    let resp = common::run(project, PostProcessorKind::Linuxcnc);
    assert!(
        resp.gcode.contains("; OP 1") && resp.gcode.contains("; OP 2"),
        "both ops should emit: {}",
        resp.gcode
    );
    // Neither op should draw a tool-fit warning with these matched tools.
    assert!(
        !resp.warnings.iter().any(|w| w.kind == "tool_kind_mismatch"),
        "unexpected tool-fit warning: {:?}",
        resp.warnings
    );

    // ── Simulate rough-only and rough+finish into the sim heightmap ────────
    let sim_cell = 0.5;
    let rough_field = carve(&stock, sim_cell, &resp.toolpath, &[(1, &rough_tool)]);
    let finish_field = carve(
        &stock,
        sim_cell,
        &resp.toolpath,
        &[(1, &rough_tool), (2, &finish_tool)],
    );

    // ── Classify both against the target surface ───────────────────────────
    // The target's z = 0 datum is the stock top, which the sim heightmap also
    // initialises to 0, so surface_z0 = 0. The grids need not align — the API
    // samples the target at each sim cell's world center.
    let target = SurfaceField::new(source.origin, source.cell, source.cols, source.rows, z);
    let surface_z0 = 0.0f32;

    // Gouge band = one sim cell. Sampling the target at cell centers against a
    // heightmap carved at `sim_cell` resolution leaves a half-cell (0.25 mm)
    // dip on the slopes for BOTH passes — pure discretization, not a toolpath
    // gouge (waterline stays on/above its slice levels; the drop-cutter never
    // dips below the surface). A full-cell band clears that noise with margin.
    let tol_gouge = sim_cell as f32;
    // Envelope band: the acceptance ceiling for leftover stock after finish.
    // The rough staircase peaks ~2.85 mm above target; the finish drop-cutter
    // clears it to ~0 mm residual, so a 1 mm envelope separates them cleanly.
    let tol_env = 1.0f32;

    let rough_gouge = target.deviation_of(&rough_field, surface_z0, tol_gouge);
    let finish_gouge = target.deviation_of(&finish_field, surface_z0, tol_gouge);
    let rough_env = target.deviation_of(&rough_field, surface_z0, tol_env);
    let finish_env = target.deviation_of(&finish_field, surface_z0, tol_env);

    let rough_gouge_n = count(&rough_gouge, Deviation::Gouge);
    let finish_gouge_n = count(&finish_gouge, Deviation::Gouge);
    let rough_rest_tol = count(&rough_gouge, Deviation::RestStock); // rest-stock at the tight band
    let rough_rest_env = count(&rough_env, Deviation::RestStock);
    let finish_rest_env = count(&finish_env, Deviation::RestStock);

    // (1) NO gouge below the target surface — the core acceptance criterion.
    // A gouge-free rough (stays on/above slice levels) and a gouge-free
    // drop-cutter finish must never cut past the model.
    assert_eq!(
        rough_gouge_n, 0,
        "waterline rough gouged below the target surface"
    );
    assert_eq!(
        finish_gouge_n, 0,
        "relief finish gouged below the target surface"
    );

    // (2) The rough pass genuinely leaves stock to clean (a staircase), so
    // the finish assertion below is meaningful rather than vacuous.
    assert!(
        rough_rest_tol > 0,
        "rough pass left no rest-stock — nothing for the finish to clean"
    );

    // (3) Bounded, uniform finish envelope: after the finish pass NO cell is
    // more than tol_env above the target, while the rough pass alone has many
    // such cells. The finish clears the staircase to within the envelope.
    assert!(
        rough_rest_env > 0,
        "rough staircase should exceed the {tol_env} mm envelope somewhere"
    );
    assert_eq!(
        finish_rest_env, 0,
        "finish left stock beyond the {tol_env} mm envelope (staircase not cleaned)"
    );
}
