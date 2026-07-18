//! Two-sided (flip-stock) conflict guard — the correctness gate for jobs
//! that cut both faces of the stock.
//!
//! Phase 2 (rt1.11.3). This is the PURE analysis half: given each cutting
//! op reduced to which face it cuts, its XY footprint, and how deep it
//! removes material, decide whether the front and back programs can be
//! safely emitted. The pipeline wiring (resolving footprints, translating
//! the verdict into warnings / a refuse) lives in
//! [`crate::pipeline`].
//!
//! Two failure modes matter (see the parent epic rt1.11 — "front_depth +
//! back_depth vs thickness. Warn on overlap; refuse to emit if both sides
//! cut clean through"):
//!
//!   * **Front sever (refuse).** A single op removes material through the
//!     FULL stock thickness from the *front* face, with no holding tabs.
//!     That cuts the workpiece free *before* the flip, so it can't be
//!     turned over and re-registered for the second side. Fatal — the
//!     caller refuses to emit. A *back*-side through-cut is the normal way
//!     to release a finished part (it's the last thing that runs), so it
//!     is NOT a sever; and a *tabbed* front through-cut keeps the part
//!     bridged, so it's allowed too.
//!   * **Opposing overlap (warn).** A front op and a back op machine
//!     overlapping XY, and their removal depths sum to MORE than the stock
//!     thickness — so their cuts meet in the middle. This is usually an
//!     intended through-feature but can be a depth mistake, so it's a
//!     warning, not a refuse. Footprints are conservative axis-aligned
//!     boxes (an over-approximation), so overlap is *possible* rather than
//!     certain — another reason to warn rather than refuse.
//!
//! ## Known limitations (documented, deliberate for v1)
//!
//!   * Footprints are per-op bounding boxes, not the true machined region,
//!     so `OpposingOverlap` can fire on boxes that overlap while the actual
//!     geometry is disjoint. Erring toward a spurious warning (never a
//!     missed one) is the safe direction for a guard.
//!   * `removal_mm` is derived from the universal depth schedule
//!     (`start_depth`/`depth`/`through_depth`). Op kinds that carry their
//!     cut depth elsewhere (V-Carve, Halfpipe, Thread) report their common
//!     depth, which may understate the true reach. True per-cell
//!     rasterization against the opposing surface is reserved for the
//!     voxel-sim work (ivac-58nl.6).

use crate::geometry::BBox;
use crate::project::WorkpieceSide;

/// Floating-point slop for depth / box comparisons (mm). A cut that
/// reaches *exactly* the far face, or two boxes that merely touch along an
/// edge, sit on the boundary and shouldn't trip the guard.
const EPS: f64 = 1e-6;

/// One cutting op reduced to what the two-sided conflict guard needs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SideExtent {
    /// The op this extent came from — carried through into diagnostics.
    pub op_id: u32,
    /// Which face of the stock the op cuts.
    pub side: WorkpieceSide,
    /// Conservative XY footprint of the machined area — an axis-aligned
    /// bounding box of the op's source geometry.
    pub footprint: BBox,
    /// How far the op removes material below its own face (mm, positive).
    pub removal_mm: f64,
    /// Whether the op leaves holding tabs (bridges). A front through-cut
    /// with tabs still holds the workpiece across the flip, so it is not
    /// treated as a sever.
    pub tabbed: bool,
}

/// A conflict the guard found on a two-sided job.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Conflict {
    /// A single op cuts clean through the full stock thickness from the
    /// FRONT face with no tabs, severing the workpiece before it can be
    /// flipped and re-registered. Fatal — refuse to emit.
    FrontSever {
        op_id: u32,
        /// The offending op's removal depth (mm) — always `>= thickness`.
        removal_mm: f64,
    },
    /// A front op and a back op machine overlapping XY and their removal
    /// depths sum past the stock thickness, so their cuts meet in the
    /// middle. Non-fatal — warn and let the user verify.
    OpposingOverlap {
        front_op: u32,
        back_op: u32,
        /// How far past the stock thickness the two removals reach (mm).
        overlap_mm: f64,
    },
}

/// Do two axis-aligned boxes share positive-area overlap? Boxes that only
/// touch along an edge (zero-width overlap) don't count — that's not a
/// machining collision.
fn footprints_overlap(a: &BBox, b: &BBox) -> bool {
    if !a.is_finite() || !b.is_finite() {
        return false;
    }
    a.min_x < b.max_x - EPS
        && a.max_x > b.min_x + EPS
        && a.min_y < b.max_y - EPS
        && a.max_y > b.min_y + EPS
}

/// Analyse a two-sided job's cutting ops against the stock thickness.
///
/// Returns every conflict found: `FrontSever` entries are fatal (the
/// caller must refuse to emit); `OpposingOverlap` entries are warnings.
/// An empty result means the job is safe to split into front / back
/// programs. A non-positive `thickness_mm` (unknown stock) yields no
/// conflicts — there's nothing meaningful to compare against.
#[must_use]
pub fn analyze(extents: &[SideExtent], thickness_mm: f64) -> Vec<Conflict> {
    let mut conflicts = Vec::new();
    if thickness_mm <= EPS {
        return conflicts;
    }

    // (1) Front clean-through without tabs — severs the stock before the
    //     flip. Certain (a single op cuts through its own footprint), so
    //     it's the hard-refuse case.
    for e in extents {
        if e.side == WorkpieceSide::Front && !e.tabbed && e.removal_mm >= thickness_mm - EPS {
            conflicts.push(Conflict::FrontSever {
                op_id: e.op_id,
                removal_mm: e.removal_mm,
            });
        }
    }

    // (2) Opposing overlap — a front op and a back op whose footprints
    //     overlap and whose removals sum past the thickness. Coarse
    //     footprints make this "possible", not certain, so it's a warning.
    for f in extents.iter().filter(|e| e.side == WorkpieceSide::Front) {
        for b in extents.iter().filter(|e| e.side == WorkpieceSide::Back) {
            if !footprints_overlap(&f.footprint, &b.footprint) {
                continue;
            }
            let sum = f.removal_mm + b.removal_mm;
            if sum > thickness_mm + EPS {
                conflicts.push(Conflict::OpposingOverlap {
                    front_op: f.op_id,
                    back_op: b.op_id,
                    overlap_mm: sum - thickness_mm,
                });
            }
        }
    }

    conflicts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bbox(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> BBox {
        BBox {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    fn front(op_id: u32, footprint: BBox, removal_mm: f64) -> SideExtent {
        SideExtent {
            op_id,
            side: WorkpieceSide::Front,
            footprint,
            removal_mm,
            tabbed: false,
        }
    }

    fn back(op_id: u32, footprint: BBox, removal_mm: f64) -> SideExtent {
        SideExtent {
            op_id,
            side: WorkpieceSide::Back,
            footprint,
            removal_mm,
            tabbed: false,
        }
    }

    /// A 100 × 100 box centred on the origin quadrant, for footprints.
    fn big() -> BBox {
        bbox(0.0, 0.0, 100.0, 100.0)
    }

    #[test]
    fn empty_input_is_clean() {
        assert!(analyze(&[], 10.0).is_empty());
    }

    #[test]
    fn unknown_thickness_skips_everything() {
        // A front op that would otherwise sever, but thickness unknown.
        let ext = [front(1, big(), 20.0)];
        assert!(analyze(&ext, 0.0).is_empty());
    }

    #[test]
    fn front_through_no_tabs_severs() {
        // 12 mm removal through 10 mm stock, front, no tabs → refuse.
        let ext = [front(7, big(), 12.0)];
        assert_eq!(
            analyze(&ext, 10.0),
            vec![Conflict::FrontSever {
                op_id: 7,
                removal_mm: 12.0
            }]
        );
    }

    #[test]
    fn front_through_exactly_at_thickness_severs() {
        // Reaching exactly the far face still frees the part.
        let ext = [front(3, big(), 10.0)];
        assert_eq!(
            analyze(&ext, 10.0),
            vec![Conflict::FrontSever {
                op_id: 3,
                removal_mm: 10.0
            }]
        );
    }

    #[test]
    fn front_through_with_tabs_is_allowed() {
        // Tabbed front through-cut keeps the part bridged for the flip.
        let ext = [SideExtent {
            tabbed: true,
            ..front(7, big(), 12.0)
        }];
        assert!(analyze(&ext, 10.0).is_empty());
    }

    #[test]
    fn back_through_alone_is_the_normal_release_cut() {
        // A back-side through-cut is the last op and releases the finished
        // part — not a sever, no conflict.
        let ext = [back(9, big(), 15.0)];
        assert!(analyze(&ext, 10.0).is_empty());
    }

    #[test]
    fn shallow_front_and_back_leave_a_web() {
        // 4 + 4 = 8 < 10 → a 2 mm web remains, no conflict even though the
        // footprints overlap.
        let ext = [front(1, big(), 4.0), back(2, big(), 4.0)];
        assert!(analyze(&ext, 10.0).is_empty());
    }

    #[test]
    fn opposing_cuts_that_exactly_meet_are_clean() {
        // 5 + 5 == 10: the cuts just kiss at the mid-plane, no overlap.
        let ext = [front(1, big(), 5.0), back(2, big(), 5.0)];
        assert!(analyze(&ext, 10.0).is_empty());
    }

    #[test]
    fn opposing_cuts_that_overlap_warn() {
        // 6 + 5 = 11 > 10 → overlap by 1 mm. Neither alone goes through.
        let ext = [front(1, big(), 6.0), back(2, big(), 5.0)];
        assert_eq!(
            analyze(&ext, 10.0),
            vec![Conflict::OpposingOverlap {
                front_op: 1,
                back_op: 2,
                overlap_mm: 1.0
            }]
        );
    }

    #[test]
    fn disjoint_footprints_never_overlap() {
        // Deep front + deep back cuts, but in different XY regions — the
        // whole point of two-sided work. No conflict.
        let f = front(1, bbox(0.0, 0.0, 40.0, 40.0), 8.0);
        let b = back(2, bbox(60.0, 60.0, 100.0, 100.0), 8.0);
        assert!(analyze(&[f, b], 10.0).is_empty());
    }

    #[test]
    fn edge_touching_footprints_do_not_overlap() {
        // Boxes share the x = 50 edge only — zero-area contact, no collision.
        let f = front(1, bbox(0.0, 0.0, 50.0, 100.0), 8.0);
        let b = back(2, bbox(50.0, 0.0, 100.0, 100.0), 8.0);
        assert!(analyze(&[f, b], 10.0).is_empty());
    }

    #[test]
    fn partial_footprint_overlap_warns() {
        // Boxes overlap on [40,60] × [0,100]; removals 7 + 7 = 14 > 10.
        let f = front(1, bbox(0.0, 0.0, 60.0, 100.0), 7.0);
        let b = back(2, bbox(40.0, 0.0, 100.0, 100.0), 7.0);
        assert_eq!(
            analyze(&[f, b], 10.0),
            vec![Conflict::OpposingOverlap {
                front_op: 1,
                back_op: 2,
                overlap_mm: 4.0
            }]
        );
    }

    #[test]
    fn sever_and_overlap_reported_together() {
        // A front op severs (12 > 10) AND overlaps a back op (12 + 3 > 10).
        let f = front(1, big(), 12.0);
        let b = back(2, big(), 3.0);
        let got = analyze(&[f, b], 10.0);
        assert!(got.contains(&Conflict::FrontSever {
            op_id: 1,
            removal_mm: 12.0
        }));
        assert!(got.contains(&Conflict::OpposingOverlap {
            front_op: 1,
            back_op: 2,
            overlap_mm: 5.0
        }));
    }

    #[test]
    fn multiple_back_ops_against_one_front() {
        // One front op overlaps two back ops; only the deep one conflicts.
        let f = front(1, big(), 6.0);
        let shallow = back(2, big(), 2.0); // 6 + 2 = 8 ≤ 10 → clean
        let deep = back(3, big(), 6.0); // 6 + 6 = 12 > 10 → warn
        let got = analyze(&[f, shallow, deep], 10.0);
        assert_eq!(
            got,
            vec![Conflict::OpposingOverlap {
                front_op: 1,
                back_op: 3,
                overlap_mm: 2.0
            }]
        );
    }
}
