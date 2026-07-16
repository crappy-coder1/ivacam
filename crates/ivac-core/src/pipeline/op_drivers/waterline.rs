//! Waterline / constant-Z 3D roughing driver.
//!
//! Resolves the op's [`crate::project::ReliefSource`] (which must be an STL
//! height grid — [`crate::project::ReliefGrid::Heightgrid`]), rebuilds that
//! grid into a triangle-mesh skin, and hands it to the pure geometry core
//! [`crate::cam::waterline::waterline_rough`]: slice the mesh at descending Z
//! levels, area-clear the solid cross-section at each, and tag every clearing
//! chain with its level Z. The resulting level chains are emitted as XYZ
//! blocks via [`emit_vcarve_block`] — the same per-point-Z emitter the
//! `VCarve` / `Halfpipe` / relief drivers use — so this driver writes XYZ
//! blocks straight to the post (no offset cascade).
//!
//! This is the ROUGH half of the standard rough-then-finish 3D flow; a
//! following [`crate::project::OpKind::ReliefMill`] finish pass cleans the
//! stock staircase this leaves. Clears INSIDE each contour — the
//! cavity/relief convention (see `cam::waterline`).

// f64 world coords narrow to the f32 mesh the slicer consumes; grid indices
// widen to usize. Both are bounded (grid dims are validated non-empty).
#![allow(clippy::cast_possible_truncation)]

use crate::cam::setup::Setup;
use crate::cam::waterline::waterline_rough;
use crate::gcode::{emit_vcarve_block, PostProcessor};
use crate::geometry::Point2;
use crate::pipeline::warnings::push_tool_fit_kind_warnings;
use crate::pipeline::{CancelToken, PipelineError, PipelineWarning};
use crate::project::{Op, OpKind, Project, ReliefGrid, ReliefSource};

fn find_source(project: &Project, id: u32) -> Option<&ReliefSource> {
    project.relief_sources.iter().find(|s| s.id == id)
}

/// True when the waterline op references an existing, non-empty source — the
/// Level-1 emit gate (mirrors `relief_would_emit`). A grayscale source still
/// passes the gate so the driver runs and emits the "needs an STL" warning
/// rather than the op vanishing silently; only a truly wrong grid is unroughed.
pub(in crate::pipeline) fn waterline_would_emit(op: &Op, project: &Project) -> bool {
    let OpKind::WaterlineRough { source_id, .. } = &op.kind else {
        return false;
    };
    find_source(project, *source_id).is_some_and(|s| !s.grid.is_empty())
}

/// Reconstruct a triangle-mesh skin from a row-major height grid: two
/// triangles per cell quad over the `cols × rows` lattice, each vertex placed
/// at its world XY (`origin + index * cell`) and grid Z. Slicing this skin at a
/// level reproduces the marching-squares iso-contour of the height field — the
/// solid outline the waterline core then area-clears.
fn heightgrid_skin(source: &ReliefSource, z: &[f32]) -> Vec<[[f32; 3]; 3]> {
    let cols = source.cols;
    let rows = source.rows;
    let cell = source.cell;
    let ox = source.origin.x;
    let oy = source.origin.y;
    let vert = |ix: u32, iy: u32| -> [f32; 3] {
        [
            (ox + f64::from(ix) * cell) as f32,
            (oy + f64::from(iy) * cell) as f32,
            z[(iy as usize) * (cols as usize) + (ix as usize)],
        ]
    };
    let mut tris = Vec::with_capacity(
        2 * (cols.saturating_sub(1) as usize) * (rows.saturating_sub(1) as usize),
    );
    for iy in 0..rows.saturating_sub(1) {
        for ix in 0..cols.saturating_sub(1) {
            let a = vert(ix, iy);
            let b = vert(ix + 1, iy);
            let c = vert(ix + 1, iy + 1);
            let d = vert(ix, iy + 1);
            tris.push([a, b, c]);
            tris.push([a, c, d]);
        }
    }
    tris
}

/// Emit a waterline-roughing op. No-op (with a warning) when the source is
/// missing, grayscale, or malformed; the `would_emit` gate normally screens
/// those out before the M6 envelope.
#[allow(clippy::too_many_arguments)]
pub(in crate::pipeline) fn run_waterline_op<P: PostProcessor>(
    op: &Op,
    project: &Project,
    setup: &Setup,
    post: &mut P,
    last_pos: &mut Point2,
    warnings: &mut Vec<PipelineWarning>,
    _cancel: Option<&CancelToken>,
) -> Result<(), PipelineError> {
    let OpKind::WaterlineRough {
        source_id,
        z_step_mm,
        stepover_mm,
        floor_z_mm,
    } = &op.kind
    else {
        return Ok(());
    };

    // Tool-kind gate (non-milling cutters) lives in the shared helper so a
    // cached replay surfaces it too.
    push_tool_fit_kind_warnings(op, project, setup, warnings);

    let tool = project
        .tools
        .iter()
        .find(|t| t.id == op.tool_id)
        .ok_or(PipelineError::UnknownTool(op.id, op.tool_id))?;

    let Some(source) = find_source(project, *source_id) else {
        return Ok(());
    };

    // Waterline needs REAL geometry to slice; a brightness relief has none.
    let ReliefGrid::Heightgrid { z } = &source.grid else {
        warnings.push(
            PipelineWarning::for_op(
                op.id,
                "waterline_source_not_stl",
                format!(
                    "Waterline op '{}': source #{source_id} is a grayscale image, not an STL height grid. Waterline roughing needs a mesh to slice — import an STL source.",
                    op.name
                ),
            )
            .with_param("op_name", op.name.as_str())
            .with_param("source_id", source_id),
        );
        return Ok(());
    };

    let expected = u64::from(source.cols) * u64::from(source.rows);
    if source.cols < 2 || source.rows < 2 || source.cell <= 0.0 || z.len() as u64 != expected {
        warnings.push(
            PipelineWarning::for_op(
                op.id,
                "waterline_source_invalid",
                format!(
                    "Waterline op '{}': source #{source_id} has a malformed grid (cols × rows must equal the grid length, both ≥ 2, cell > 0).",
                    op.name
                ),
            )
            .with_param("op_name", op.name.as_str())
            .with_param("source_id", source_id),
        );
        return Ok(());
    }

    let tool_diameter = tool.diameter;
    if tool_diameter <= 0.0 {
        return Ok(());
    }
    // A non-positive per-level depth would slice nothing — surface it rather
    // than silently emitting an empty op.
    if *z_step_mm <= 0.0 {
        warnings.push(
            PipelineWarning::for_op(
                op.id,
                "waterline_z_step_invalid",
                format!(
                    "Waterline op '{}': per-level depth of cut must be > 0 (got {z_step_mm}). Nothing roughed.",
                    op.name
                ),
            )
            .with_param("op_name", op.name.as_str()),
        );
        return Ok(());
    }
    // Lateral stepover: an explicit positive value wins; otherwise derive a
    // 40 %-of-diameter default so a fresh op still cuts.
    let stride = if *stepover_mm > 0.0 {
        *stepover_mm
    } else {
        tool_diameter * 0.4
    };

    // Z window. The grid's max is the stock top (uncovered cells sit at 0);
    // the deepest cell is the model floor. z_levels steps strictly below the
    // top, so a grazing slice on the flat top is never taken.
    let (mut zmin, mut zmax) = (f32::INFINITY, f32::NEG_INFINITY);
    for &v in z {
        zmin = zmin.min(v);
        zmax = zmax.max(v);
    }
    let top_z = f64::from(zmax).min(0.0);
    // A user-set negative floor clamps shallower than the model; the default
    // (0) reaches the model's deepest point.
    let mut bottom_z = if *floor_z_mm < 0.0 {
        f64::from(zmin).max(*floor_z_mm)
    } else {
        f64::from(zmin)
    };
    // Clamp the floor to what the flutes can reach.
    if let Some(flute) = tool.flute_length_mm.filter(|v| *v > 0.0) {
        if bottom_z < -flute {
            bottom_z = -flute;
            warnings.push(
                PipelineWarning::for_op(
                    op.id,
                    "waterline_tool_reach_exceeded",
                    format!(
                        "Waterline op '{}': the requested depth is deeper than tool '{}' can reach (flute length {flute:.3} mm). Roughing clipped to that depth — use a longer-flute tool or a shallower floor.",
                        op.name, tool.name
                    ),
                )
                .with_param("op_name", op.name.as_str())
                .with_param("tool_name", tool.name.as_str())
                .with_param("flute_length", format!("{flute:.3}")),
            );
        }
    }

    if top_z <= bottom_z {
        return Ok(());
    }

    let tris = heightgrid_skin(source, z);
    let chains = waterline_rough(&tris, tool_diameter, stride, top_z, bottom_z, *z_step_mm);
    if chains.is_empty() {
        return Ok(());
    }

    // Each level chain is a constant-Z cut path; lift each into an XYZ
    // polyline the shared emitter walks (safe-Z lift + rapid + plunge between
    // chains). Sub-two-point chains can't cut, so drop them.
    let polylines: Vec<Vec<(f64, f64, f64)>> = chains
        .iter()
        .filter(|c| c.path.len() >= 2)
        .map(|c| c.path.iter().map(|p| (p.x, p.y, c.z)).collect())
        .collect();
    if polylines.is_empty() {
        return Ok(());
    }
    emit_vcarve_block(setup, &polylines, post, last_pos);
    Ok(())
}
