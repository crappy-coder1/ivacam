//! Acceptance test for the analytic swept-arc footprint (bd ivac-58nl.4).
//!
//! The sim historically saw arcs only as pre-tessellated chord segments (the
//! gcode preview walks G2/G3 at ~2° per chord). That imposes a chord-error
//! floor `R·(1−cos(step/2))`: below it, a finishing scallop the sim draws is a
//! tessellation artifact, not a real machining outcome.
//!
//! [`sweep_arc_segment`] carves the *analytic* swept-arc footprint instead.
//! This test proves it against a dense-chord baseline at a `cell` size well
//! below a deliberately COARSE tessellation's chord error:
//!
//!   * PARITY — the analytic arc matches a dense-chord (0.5°) carve of the
//!     same arc to within a fraction of a cell. Same true arc, same surface.
//!   * THE STEP IS REAL — a coarse (20°) tessellation visibly deviates from
//!     that dense baseline: the staircase the analytic path removes.
//!   * THE ANALYTIC PATH REMOVES IT — the analytic carve is several times
//!     closer to the truth than the coarse tessellation is.

// The harness crosses f64 (geometry) / f32 (heightmap) / u32 (grid dims)
// boundaries the sim APIs require; each cast is deliberate.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use ivac_core::gcode::preview::{MoveKind, Pose3, ToolpathSegment};
use ivac_core::geometry::Point2;
use ivac_core::sim::diagnostics::SimDiagnostics;
use ivac_core::sim::heightmap::{Heightmap, ToolProfile};
use ivac_core::sim::sweep::{sweep_arc_segment, sweep_range, ArcXY};

/// A fresh sim heightmap over `[origin, origin + span]²` at `cell` spacing,
/// stock top at `top_z`.
fn fresh(origin: f64, span: f64, cell: f64, top_z: f32) -> Heightmap {
    let n = (span / cell).round() as u32;
    Heightmap::new(Point2::new(origin, origin), cell, n, n, top_z)
}

/// Tessellate the arc `from → to` about center `(cx, cy)` (CCW) into chord
/// `ToolpathSegment`s at `step_deg` per chord — the same construction the
/// gcode preview uses, reproduced here so the baseline is independent of the
/// interpreter. Z is constant across this arc, so no Z interpolation is
/// needed. Chords are tagged `Cut` (the sweep carves `Cut` and `Arc`
/// identically today).
fn tessellate(from: Pose3, to: Pose3, cx: f64, cy: f64, step_deg: f64) -> Vec<ToolpathSegment> {
    let r = (from.x - cx).hypot(from.y - cy);
    let theta_start = (from.y - cy).atan2(from.x - cx);
    let theta_end = (to.y - cy).atan2(to.x - cx);
    let mut sweep = theta_end - theta_start;
    if sweep <= 1e-9 {
        sweep += std::f64::consts::TAU; // CCW quarter/whatever, positive span
    }
    let n = (sweep.abs() / step_deg.to_radians()).ceil().max(1.0) as usize;
    let dtheta = sweep / n as f64;
    let mut segs = Vec::with_capacity(n);
    let mut prev = from;
    for k in 1..=n {
        let theta = theta_start + dtheta * k as f64;
        let next = if k == n {
            to
        } else {
            Pose3 {
                x: cx + r * theta.cos(),
                y: cy + r * theta.sin(),
                z: from.z,
            }
        };
        segs.push(ToolpathSegment {
            from: prev,
            to: next,
            kind: MoveKind::Cut,
            gcode_line: 0,
            op_id: 0,
            arc: None,
        });
        prev = next;
    }
    segs
}

/// Carve a chord stream into a fresh heightmap and return it.
fn carve_chords(
    origin: f64,
    span: f64,
    cell: f64,
    top_z: f32,
    profile: &ToolProfile,
    segs: &[ToolpathSegment],
) -> Heightmap {
    let mut hm = fresh(origin, span, cell, top_z);
    let mut diag = SimDiagnostics::default();
    sweep_range(&mut hm, segs, 0, segs.len(), profile, &[], None, &mut diag);
    hm
}

/// Max absolute per-cell difference between two identically-shaped heightmaps.
fn max_abs_diff(a: &Heightmap, b: &Heightmap) -> f32 {
    assert_eq!(a.data.len(), b.data.len(), "grids must match");
    a.data
        .iter()
        .zip(&b.data)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max)
}

#[test]
fn analytic_arc_matches_dense_chords_and_removes_the_tessellation_step() {
    // ── Arc + tool ─────────────────────────────────────────────────────────
    // Quarter circle, R = 8 mm, center at the origin, from (8,0) CCW to
    // (0,8). A 2.5 mm ball-nose sweeps it at constant tip Z = 0; the stock
    // top sits at the tool radius so the groove rounds from z = 0 on the arc
    // up to the uncut top 2.5 mm away.
    let r_arc = 8.0f64;
    let from = Pose3 {
        x: r_arc,
        y: 0.0,
        z: 0.0,
    };
    let to = Pose3 {
        x: 0.0,
        y: r_arc,
        z: 0.0,
    };
    let arc = ArcXY {
        cx: 0.0,
        cy: 0.0,
        ccw: true,
    };
    let r_tool = 2.5f32;
    let profile = ToolProfile::BallNose { r: r_tool };

    // Grid spans the arc tube [-r_tool, R+r_tool] with margin; a fine cell
    // well below the coarse 20° chord error (≈ R·(1−cos 10°) ≈ 0.12 mm).
    let origin = -3.5;
    let span = 14.0;
    let cell = 0.05;
    let top_z = r_tool;

    // ── Three carves of the SAME arc ───────────────────────────────────────
    let mut analytic = fresh(origin, span, cell, top_z);
    let touched = sweep_arc_segment(&mut analytic, &from, &to, arc, &profile);
    assert!(touched > 0, "analytic arc carved nothing");

    let dense = carve_chords(
        origin,
        span,
        cell,
        top_z,
        &profile,
        &tessellate(from, to, 0.0, 0.0, 0.5), // 0.5° — the truth baseline
    );
    let coarse = carve_chords(
        origin,
        span,
        cell,
        top_z,
        &profile,
        &tessellate(from, to, 0.0, 0.0, 20.0), // 20° — the tessellation step
    );

    let max_ad = max_abs_diff(&analytic, &dense); // analytic vs truth
    let max_cd = max_abs_diff(&coarse, &dense); // coarse vs truth

    // (1) PARITY: analytic matches the dense-chord truth to within a fraction
    // of a cell — same arc, same surface.
    assert!(
        max_ad < (0.5 * cell) as f32,
        "analytic arc deviates from the dense-chord baseline by {max_ad:.4} mm \
         (> half a {cell} mm cell) — not parity"
    );

    // (2) THE STEP IS REAL: the coarse tessellation visibly deviates from the
    // truth, so assertion (3) is meaningful rather than vacuous.
    assert!(
        max_cd > 0.05,
        "coarse 20° tessellation deviates from truth by only {max_cd:.4} mm — \
         no visible step to remove (pick a coarser step or finer cell)"
    );

    // (3) THE ANALYTIC PATH REMOVES IT: the analytic carve is several times
    // closer to the truth than the coarse tessellation. This is the scallop
    // "matching the analytic arc, not the tessellation step".
    assert!(
        max_ad < 0.25 * max_cd,
        "analytic ({max_ad:.4} mm) is not decisively closer to truth than the \
         coarse tessellation ({max_cd:.4} mm)"
    );
}

#[test]
fn live_interpret_then_sweep_carves_the_analytic_arc() {
    // The full live wiring: real G3 gcode → `interpret` (tessellates + tags
    // each chord with its parent arc) → `sweep_range` (dispatches each arc
    // chord to the analytic sub-arc carve). The result must match the direct
    // analytic arc carve — i.e. the live sim shows the arc, not the chords.
    let g = "G21\nG0 X8 Y0\nG3 X0 Y8 I-8 J0 F500\n"; // quarter circle, R=8, CCW
    let toolpath = ivac_core::gcode::preview::interpret(g);
    // Guard: the arc must actually reach the sim as arc-tagged chords, else
    // this would silently exercise the straight-chord path.
    let n_arc = toolpath.iter().filter(|s| s.arc.is_some()).count();
    assert!(n_arc >= 40, "expected many arc-tagged chords, got {n_arc}");

    let r_tool = 2.5f32;
    let profile = ToolProfile::BallNose { r: r_tool };
    let origin = -3.5;
    let span = 14.0;
    let cell = 0.05;
    let top_z = r_tool;

    // Live path.
    let mut live = fresh(origin, span, cell, top_z);
    let mut diag = SimDiagnostics::default();
    sweep_range(
        &mut live,
        &toolpath,
        0,
        toolpath.len(),
        &profile,
        &[],
        None,
        &mut diag,
    );

    // Direct analytic reference over the same arc geometry.
    let from = Pose3 {
        x: 8.0,
        y: 0.0,
        z: 0.0,
    };
    let to = Pose3 {
        x: 0.0,
        y: 8.0,
        z: 0.0,
    };
    let arc = ArcXY {
        cx: 0.0,
        cy: 0.0,
        ccw: true,
    };
    let mut reference = fresh(origin, span, cell, top_z);
    let ref_touched = sweep_arc_segment(&mut reference, &from, &to, arc, &profile);
    assert!(ref_touched > 0, "reference arc carved nothing");

    let d = max_abs_diff(&live, &reference);
    assert!(
        d < (0.5 * cell) as f32,
        "live interpret→sweep deviates from the analytic arc by {d:.4} mm \
         (> half a {cell} mm cell)"
    );
}
