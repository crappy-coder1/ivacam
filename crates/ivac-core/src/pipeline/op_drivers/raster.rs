//! Laser raster-engrave driver (phase 3).
//!
//! Resolves the op's [`crate::project::ReliefSource`] brightness grid,
//! maps it through [`crate::cam::raster::PowerCurve`] to a per-pixel
//! laser-power (`S`) grid, and emits it row-by-row. Each row is
//! **run-length grouped by power**: a span of equal-power pixels becomes a
//! single `M3 S<power>` + `G1` (the post's `laser_on` is modal-deduped),
//! so smooth gradients and binary dithers both stay compact. `M3 S0` arms
//! the beam cold and `M5` drops it; the laser is dropped for every
//! inter-row reposition and re-armed for the scan.
//!
//! Plot-mode XY only — no Z modulation (laser focus is fixed). Honors the
//! scan direction, the link mode (unidirectional lift-between vs
//! boustrophedon), and an overscan lead-in/-out so the head reaches feed
//! before it crosses the first burning pixel. Laser-only.

use crate::cam::raster::{stream_power_cols, stream_power_rows, RasterLink};
use crate::cam::setup::Setup;
use crate::cam::surface_mill::ScanDirection;
use crate::gcode::PostProcessor;
use crate::geometry::Point2;
use crate::pipeline::{cancelled, CancelToken, PipelineError, PipelineWarning};
use crate::project::MachineMode;
use crate::project::{Op, OpKind, Project, ReliefSource};

/// Pixel-count ceiling for the one remaining non-streaming path: a
/// **vertical (AlongY) scan with a Floyd–Steinberg curve**. F–S diffuses
/// error row-major, so a column walk can't produce it lazily — that case
/// materializes the whole `cols × rows` power grid and warns + skips past
/// this cap rather than risk a huge transient allocation. Every other
/// combination streams unbounded at `O(cols)`/`O(rows)`: AlongX for any
/// curve via [`stream_power_rows`] (z9zh), and AlongY for the
/// position-independent curves via [`stream_power_cols`] (jyum).
const MAX_RASTER_PIXELS: usize = 16_000_000;

fn find_source(project: &Project, id: u32) -> Option<&ReliefSource> {
    project.relief_sources.iter().find(|s| s.id == id)
}

/// True when the op references an existing, non-empty source AND the
/// machine is in laser mode — the Level-1 emit gate (mirrors
/// `relief_would_emit`). Raster engraving is meaningless off a laser, so a
/// non-laser machine gates the op out (the op×machine-mode warning already
/// tells the user why).
pub(in crate::pipeline) fn raster_would_emit(op: &Op, project: &Project) -> bool {
    let OpKind::RasterEngrave { source_id, .. } = &op.kind else {
        return false;
    };
    matches!(project.machine.mode, MachineMode::Laser)
        && find_source(project, *source_id)
            .is_some_and(|s| s.brightness().is_some_and(|b| !b.is_empty()))
}

/// Post-resample grid dimensions for a `target_pitch`, computed WITHOUT
/// allocating. `resample` returns exactly these dims, so the emit cap can
/// be enforced against them before the (potentially huge) grid is built —
/// otherwise a tiny `target_pitch` blows up the allocation before the
/// pixel-count guard even runs. Mirrors `resample`'s identity short-circuit
/// (untouched dims when pitch is ≤0, within 1 µm of `cell`, or empty).
fn resampled_dims(cols: usize, rows: usize, cell: f64, target_pitch: f64) -> (usize, usize) {
    if target_pitch <= 0.0 || (target_pitch - cell).abs() < 1e-6 || cols == 0 || rows == 0 {
        return (cols, rows);
    }
    let width = cols as f64 * cell;
    let height = rows as f64 * cell;
    let new_cols = ((width / target_pitch).round() as usize).max(1);
    let new_rows = ((height / target_pitch).round() as usize).max(1);
    (new_cols, new_rows)
}

/// Nearest-neighbour resample of a brightness grid to a new square pitch.
/// Returns `(brightness, cols, rows)` at `target_pitch`. A `target_pitch`
/// at/below 0 or within 1 µm of `cell` returns the grid untouched.
fn resample(
    brightness: &[f32],
    cols: usize,
    rows: usize,
    cell: f64,
    target_pitch: f64,
) -> (Vec<f32>, usize, usize) {
    if target_pitch <= 0.0 || (target_pitch - cell).abs() < 1e-6 || cols == 0 || rows == 0 {
        return (brightness.to_vec(), cols, rows);
    }
    let (new_cols, new_rows) = resampled_dims(cols, rows, cell, target_pitch);
    let mut out = vec![0.0f32; new_cols * new_rows];
    for ny in 0..new_rows {
        // Sample at the centre of each target cell, mapped back to source.
        let sy = (((ny as f64 + 0.5) * target_pitch / cell) as usize).min(rows - 1);
        for nx in 0..new_cols {
            let sx = (((nx as f64 + 0.5) * target_pitch / cell) as usize).min(cols - 1);
            out[ny * new_cols + nx] = brightness[sy * cols + sx];
        }
    }
    (out, new_cols, new_rows)
}

/// Fill `out` with the `new_cols` raw (un-clamped) brightness values of
/// resampled row `ny` — the streaming counterpart of one row of
/// [`resample`]'s output. Uses the identical nearest-neighbour index math
/// (and the same `identity` short-circuit for `pitch ≤ 0` / within 1 µm of
/// `cell`) so the streamed AlongX path is byte-identical to the whole-grid
/// path. `PowerCurve`'s clamping happens downstream in [`stream_power_rows`],
/// exactly where `power_grid` clamps, so this must NOT clamp.
fn resample_row(
    src: &[f32],
    in_cols: usize,
    in_rows: usize,
    cell: f64,
    target_pitch: f64,
    new_cols: usize,
    ny: usize,
    out: &mut Vec<f32>,
) {
    out.clear();
    let identity =
        target_pitch <= 0.0 || (target_pitch - cell).abs() < 1e-6 || in_cols == 0 || in_rows == 0;
    let sy = if identity {
        ny
    } else {
        (((ny as f64 + 0.5) * target_pitch / cell) as usize).min(in_rows - 1)
    };
    for nx in 0..new_cols {
        let sx = if identity {
            nx
        } else {
            (((nx as f64 + 0.5) * target_pitch / cell) as usize).min(in_cols - 1)
        };
        out.push(src[sy * in_cols + sx]);
    }
}

/// Column counterpart of [`resample_row`]: fill `out` with the `new_rows`
/// raw brightness values of resampled column `nx`, feeding the AlongY
/// column-streaming path (jyum). Same nearest-neighbour math and identity
/// short-circuit as [`resample`], so a streamed column is byte-identical to
/// that column of the whole grid.
fn resample_col(
    src: &[f32],
    in_cols: usize,
    in_rows: usize,
    cell: f64,
    target_pitch: f64,
    new_rows: usize,
    nx: usize,
    out: &mut Vec<f32>,
) {
    out.clear();
    let identity =
        target_pitch <= 0.0 || (target_pitch - cell).abs() < 1e-6 || in_cols == 0 || in_rows == 0;
    let sx = if identity {
        nx
    } else {
        (((nx as f64 + 0.5) * target_pitch / cell) as usize).min(in_cols - 1)
    };
    for ny in 0..new_rows {
        let sy = if identity {
            ny
        } else {
            (((ny as f64 + 0.5) * target_pitch / cell) as usize).min(in_rows - 1)
        };
        out.push(src[sy * in_cols + sx]);
    }
}

/// Per-op scanline geometry shared by both emit paths. `over` is the
/// overscan lead-in/-out distance (constant across scanlines).
struct ScanGeom {
    /// True for AlongY (vertical scanlines, sweeping Y); false for AlongX.
    scan_y: bool,
    pitch: f64,
    /// Cross-axis origin: X for AlongY scanlines, Y for AlongX.
    fixed_origin: f64,
    /// Sweep-axis origin: Y for AlongY, X for AlongX.
    sweep_origin: f64,
    over: f64,
    bidirectional: bool,
}

/// Emit one scanline: reposition (beam off), optional overscan lead-in,
/// the run-length-grouped burning spans, and optional lead-out. `line` is
/// the scanline index; `line_powers[k]` is the `S`-power of the k-th pixel
/// along the sweep. Updates `final_pt` to the head's resting point.
///
/// This is the single emit implementation both the streaming (AlongX) and
/// whole-grid (AlongY) paths call, so their output is byte-identical
/// modulo the pixel-to-scanline mapping each feeds in.
fn emit_scanline<P: PostProcessor>(
    post: &mut P,
    g: &ScanGeom,
    line: usize,
    line_powers: &[u32],
    final_pt: &mut Point2,
) {
    let line_len = line_powers.len();
    let fixed = g.fixed_origin + line as f64 * g.pitch;
    let reverse = g.bidirectional && (line % 2 == 1);
    let order: Vec<usize> = if reverse {
        (0..line_len).rev().collect()
    } else {
        (0..line_len).collect()
    };
    // Run-length group into (power, far-boundary-index) spans.
    let mut spans: Vec<(u32, usize)> = Vec::new();
    let mut i = 0;
    while i < order.len() {
        let p = line_powers[order[i]];
        let mut j = i;
        while j + 1 < order.len() && line_powers[order[j + 1]] == p {
            j += 1;
        }
        let last = order[j];
        // forward: pixel `k` spans boundaries [k, k+1] ⇒ far edge k+1;
        // reverse: the far edge is the lower boundary `k`.
        let end_b = if reverse { last } else { last + 1 };
        spans.push((p, end_b));
        i = j + 1;
    }

    let boundary = |b: usize| g.sweep_origin + b as f64 * g.pitch;
    let world = |fixed: f64, sweep: f64| -> (f64, f64) {
        if g.scan_y {
            (fixed, sweep)
        } else {
            (sweep, fixed)
        }
    };
    let dir = if reverse { -1.0 } else { 1.0 };
    let lead = boundary(if reverse { line_len } else { 0 });
    let start = lead + dir * g.over;

    // Reposition with the beam off (M5 over the rapid), then re-arm.
    post.laser_off();
    let (sx, sy) = world(fixed, start);
    post.move_to(Some(sx), Some(sy), None);
    // Overscan lead-in at S0 so the head is at feed before burning.
    if g.over > 0.0 {
        post.laser_on(0);
        let (lx, ly) = world(fixed, lead);
        post.linear(Some(lx), Some(ly), None);
    }
    for (p, end_b) in spans {
        post.laser_on(p);
        let (ex, ey) = world(fixed, boundary(end_b));
        post.linear(Some(ex), Some(ey), None);
        *final_pt = Point2::new(ex, ey);
    }
    // Overscan lead-out at S0.
    if g.over > 0.0 {
        post.laser_on(0);
        let (tx, ty) = world(
            fixed,
            boundary(if reverse { 0 } else { line_len }) + dir * g.over,
        );
        post.linear(Some(tx), Some(ty), None);
        *final_pt = Point2::new(tx, ty);
    }
}

/// Emit a laser raster-engrave op. No-op when the source is missing /
/// empty or the machine isn't a laser (the `would_emit` gate normally
/// screens those out).
///
/// Two emit paths (z9zh): **AlongX** (the default horizontal scan) streams
/// image rows one at a time through [`stream_power_rows`], so peak memory
/// is `O(cols)` and there is no pixel cap — a genuinely huge engrave rides
/// the streaming gcode sink straight to the output. **AlongY** (vertical)
/// walks image columns, which needs the whole power grid resident, so it
/// materializes it and stays behind [`MAX_RASTER_PIXELS`] (warn + skip).
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(in crate::pipeline) fn run_raster_op<P: PostProcessor>(
    op: &Op,
    project: &Project,
    setup: &Setup,
    post: &mut P,
    last_pos: &mut Point2,
    warnings: &mut Vec<PipelineWarning>,
    cancel: Option<&CancelToken>,
) -> Result<(), PipelineError> {
    let OpKind::RasterEngrave {
        source_id,
        resolution_mm,
        power_curve,
        scan_direction,
        link,
        overscan_factor,
    } = &op.kind
    else {
        return Ok(());
    };
    // Laser-only; the op×machine-mode warning already flagged the misuse.
    if !matches!(project.machine.mode, MachineMode::Laser) {
        return Ok(());
    }
    let Some(source) = find_source(project, *source_id) else {
        return Ok(());
    };
    // Raster engrave needs a brightness grid; a height-grid (STL) source has
    // none, so it's a no-op here (the op×source-kind mismatch is screened by
    // `raster_would_emit`).
    let Some(src_brightness) = source.brightness() else {
        return Ok(());
    };
    if src_brightness.is_empty() || source.cols == 0 || source.rows == 0 {
        return Ok(());
    }

    let cell = if source.cell > 0.0 { source.cell } else { 1.0 };
    let in_cols = source.cols as usize;
    let in_rows = source.rows as usize;
    // Projected resample dims, computed WITHOUT allocating (a tiny
    // resolution_mm would otherwise balloon the grid before any guard runs).
    let (cols, rows) = resampled_dims(in_cols, in_rows, cell, *resolution_mm);
    if cols == 0 || rows == 0 {
        return Ok(());
    }

    // The image occupies a FIXED rectangle: pixel (col c, row r) lands at
    // world (origin.x + c*pitch, origin.y + r*pitch). Scan direction only
    // changes the laser's travel orientation, not where pixels land:
    //   AlongX → horizontal scanlines (one per grid row),  sweep X.
    //   AlongY → vertical   scanlines (one per grid column), sweep Y.
    let pitch = if *resolution_mm > 0.0 {
        *resolution_mm
    } else {
        cell
    };
    let ox = source.origin.x;
    let oy = source.origin.y;
    let feed = setup.tool.rate_h.max(1);
    let scan_y = matches!(scan_direction, ScanDirection::AlongY);
    // Only ONE combination can't stream: a vertical (AlongY) scan with a
    // Floyd–Steinberg curve. A column walk needs random row access, and F–S
    // diffuses row-major, so that case materializes the whole grid. AlongX
    // (any curve) and AlongY with a position-independent curve both stream.
    let along_y_whole_grid = scan_y && !power_curve.streams_column_major();
    // AlongX: one scanline per image row, sweeping across cols.
    // AlongY: one scanline per image column, sweeping down rows.
    let line_len = if scan_y { rows } else { cols };
    let geom = ScanGeom {
        scan_y,
        pitch,
        fixed_origin: if scan_y { ox } else { oy },
        sweep_origin: if scan_y { oy } else { ox },
        over: overscan_factor.max(0.0) * line_len as f64 * pitch,
        bidirectional: matches!(link, RasterLink::Bidirectional),
    };

    // The pixel cap now guards ONLY the non-streaming path (AlongY + F–S);
    // every streaming path is unbounded. Check BEFORE the op header so a
    // skipped op emits nothing.
    if along_y_whole_grid
        && cols
            .checked_mul(rows)
            .map_or(true, |n| n > MAX_RASTER_PIXELS)
    {
        warnings.push(PipelineWarning::for_op(
            op.id,
            "raster_too_large",
            format!(
                "raster op '{}' resamples to {cols}×{rows} pixels, over the {MAX_RASTER_PIXELS}-pixel cap for vertical (AlongY) scanning with a Floyd–Steinberg curve — the one combination that can't stream. Lower the resolution (larger resolution_mm), crop the image, switch to a Bayer / Threshold / Linear curve (all stream vertically, unbounded), or use horizontal (AlongX) scanning.",
                op.name
            ),
        )
        .with_param("op_name", op.name.as_str())
        .with_param("cols", cols)
        .with_param("rows", rows)
        .with_param("max_pixels", MAX_RASTER_PIXELS));
        return Ok(());
    }

    post.comment(&format!("OP {} raster engrave", op.id));
    post.laser_arm(); // M3 S0 — armed cold
    post.feedrate(feed);
    let mut final_pt = Point2::new(ox, oy);
    let mut aborted = false;
    {
        // One emit closure for all three routes: reposition + burn a single
        // scanline, honoring the cancel token (a huge engrave stays
        // interruptible). `line_powers[k]` is the k-th pixel along the sweep.
        let mut emit = |line: usize, line_powers: &[u32]| -> bool {
            if cancelled(cancel) {
                aborted = true;
                return false;
            }
            emit_scanline(post, &geom, line, line_powers, &mut final_pt);
            true
        };

        if along_y_whole_grid {
            // AlongY + Floyd–Steinberg: materialize the grid (cap-guarded
            // above), then walk columns — F–S must see the finished grid.
            let (brightness, _, _) =
                resample(src_brightness, in_cols, in_rows, cell, *resolution_mm);
            let powers = power_curve.power_grid(&brightness, cols, rows);
            drop(brightness); // free before emit; don't hold both grids live
            let mut col: Vec<u32> = vec![0u32; rows];
            for line in 0..cols {
                for (k, slot) in col.iter_mut().enumerate() {
                    *slot = powers[k * cols + line];
                }
                if !emit(line, &col) {
                    break;
                }
            }
        } else if scan_y {
            // AlongY, position-independent curve: stream image columns at
            // O(rows), no pixel cap.
            let handled = stream_power_cols(
                power_curve,
                cols,
                rows,
                |nx, buf| {
                    resample_col(
                        src_brightness,
                        in_cols,
                        in_rows,
                        cell,
                        *resolution_mm,
                        rows,
                        nx,
                        buf,
                    );
                },
                &mut emit,
            );
            // `streams_column_major` already excluded F–S, so this holds;
            // the assert documents the invariant without a release cost.
            debug_assert!(
                handled,
                "position-independent curve must stream column-major"
            );
        } else {
            // AlongX (any curve): stream image rows at O(cols), no pixel cap.
            stream_power_rows(
                power_curve,
                cols,
                rows,
                |ny, buf| {
                    resample_row(
                        src_brightness,
                        in_cols,
                        in_rows,
                        cell,
                        *resolution_mm,
                        cols,
                        ny,
                        buf,
                    );
                },
                &mut emit,
            );
        }
    }
    if aborted {
        return Err(PipelineError::Cancelled);
    }

    post.laser_off(); // M5 — beam down at op end
    *last_pos = final_pt;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_identity_when_pitch_matches_cell() {
        let b = vec![0.0, 0.5, 1.0, 0.25];
        let (out, c, r) = resample(&b, 2, 2, 0.1, 0.1);
        assert_eq!((c, r), (2, 2));
        assert_eq!(out, b);
        // pitch 0 ⇒ untouched too.
        let (out0, _, _) = resample(&b, 2, 2, 0.1, 0.0);
        assert_eq!(out0, b);
    }

    #[test]
    fn resample_halves_resolution() {
        // 4×4 at 0.1 mm cell → 0.2 mm pitch ⇒ 2×2.
        let b: Vec<f32> = (0..16).map(|i| i as f32 / 16.0).collect();
        let (out, c, r) = resample(&b, 4, 4, 0.1, 0.2);
        assert_eq!((c, r), (2, 2));
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn resampled_dims_matches_resample_output() {
        // The cap is enforced against resampled_dims, so it must predict
        // exactly the dims resample returns — for both the identity
        // short-circuit and a genuine resample.
        let b: Vec<f32> = (0..16).map(|i| i as f32 / 16.0).collect();
        for (cell, pitch) in [(0.1, 0.1), (0.1, 0.0), (0.1, 0.2), (0.1, 0.05)] {
            let (_, c, r) = resample(&b, 4, 4, cell, pitch);
            assert_eq!(
                resampled_dims(4, 4, cell, pitch),
                (c, r),
                "dims mismatch at cell={cell}, pitch={pitch}"
            );
        }
    }

    #[test]
    fn resampled_dims_explodes_for_tiny_pitch() {
        // A 2 mm-wide grid at a 0.0001 mm pitch projects to 20_000 px per
        // axis (400 Mpx) — well over MAX_RASTER_PIXELS. The guard reads
        // these dims BEFORE resample allocates, so the cap trips without
        // first materializing a 400 M-element grid.
        let (c, r) = resampled_dims(2, 2, 1.0, 0.0001);
        assert_eq!((c, r), (20_000, 20_000));
        assert!(c.checked_mul(r).is_some_and(|n| n > MAX_RASTER_PIXELS));
    }

    #[test]
    fn resample_row_assembles_to_resample() {
        // The streaming AlongX path pulls one resampled row at a time via
        // `resample_row`; concatenated, those rows must be byte-identical to
        // the whole-grid `resample` (else streamed gcode would drift from
        // the buffered path). Cover the identity short-circuit, an upsample,
        // and a downsample.
        let b: Vec<f32> = (0..24).map(|i| i as f32 / 24.0).collect();
        for (in_cols, in_rows, cell, pitch) in [
            (6usize, 4usize, 0.1, 0.1),
            (6, 4, 0.1, 0.2),
            (6, 4, 0.1, 0.05),
        ] {
            let (whole, nc, nr) = resample(&b, in_cols, in_rows, cell, pitch);
            let mut assembled = Vec::new();
            let mut row = Vec::new();
            for ny in 0..nr {
                resample_row(&b, in_cols, in_rows, cell, pitch, nc, ny, &mut row);
                assembled.extend_from_slice(&row);
            }
            assert_eq!(
                assembled, whole,
                "resample_row != resample at cell={cell}, pitch={pitch}"
            );
        }
    }

    #[test]
    fn resample_col_assembles_to_resample() {
        // The streaming AlongY path pulls one resampled column at a time via
        // `resample_col`; placed column-major, they must reconstruct exactly
        // the whole-grid `resample`. Same cases as the row test.
        let b: Vec<f32> = (0..24).map(|i| i as f32 / 24.0).collect();
        for (in_cols, in_rows, cell, pitch) in [
            (6usize, 4usize, 0.1, 0.1),
            (6, 4, 0.1, 0.2),
            (6, 4, 0.1, 0.05),
        ] {
            let (whole, nc, nr) = resample(&b, in_cols, in_rows, cell, pitch);
            let mut assembled = vec![0.0f32; nc * nr];
            let mut colbuf = Vec::new();
            for nx in 0..nc {
                resample_col(&b, in_cols, in_rows, cell, pitch, nr, nx, &mut colbuf);
                for (ny, &v) in colbuf.iter().enumerate() {
                    assembled[ny * nc + nx] = v;
                }
            }
            assert_eq!(
                assembled, whole,
                "resample_col != resample at cell={cell}, pitch={pitch}"
            );
        }
    }
}
