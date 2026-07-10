//! Multi-span Z-dexel primitives — the material-model evolution of the
//! single-Z heightmap (see the `ivac-58nl.6` design note).
//!
//! This module holds the **pure 1-D interval algebra** that a per-column
//! span list needs, with no field, no sweep, and no rendering wired up yet.
//! It is the smallest reviewable slice of the multi-span dexel core: a
//! column of stock becomes a sorted, disjoint list of solid `Span`s along
//! Z, and carving removes a `[lo, hi]` interval from that list.
//!
//! Why this shape matters: the current [`super::heightmap::Heightmap`] is a
//! degenerate **1-span** dexel (one solid span `[floor, top]` per column).
//! [`subtract_interval`] applied to a single top-reaching span reduces
//! exactly to the monotone-`min()` the heightmap does today, so the eventual
//! `DexelField` can keep the 3-axis hot path byte-for-byte identical while
//! this algebra handles the undercut / two-sided cases the heightmap
//! structurally cannot.
//!
//! **Invariants** upheld by every function here:
//! - each `Span` has `lo < hi` (no empty or inverted spans),
//! - a span list is **sorted ascending by `lo`** and **pairwise disjoint**
//!   (no two spans touch or overlap after [`merge_adjacent`]).

// Z coordinates are `f32` to match the heightmap's cell storage; the
// interval math is exact on the endpoints we feed it (no accumulation).
#![allow(clippy::module_name_repetitions)]

/// A solid interval along one column, Z up. Invariant: `lo < hi`.
///
/// A column of stock is represented as a sorted, disjoint `Vec<Span>`.
/// An empty vec means "no solid material left in this column".
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Span {
    /// Lower Z bound (inclusive in intent; boundaries are exact floats).
    pub lo: f32,
    /// Upper Z bound. Always strictly greater than `lo`.
    pub hi: f32,
}

impl Span {
    /// Construct a span, returning `None` for an empty or inverted range
    /// (`lo >= hi`) so callers can't smuggle in a degenerate span.
    #[must_use]
    pub fn new(lo: f32, hi: f32) -> Option<Self> {
        (lo < hi).then_some(Self { lo, hi })
    }

    /// Height of solid material this span represents.
    #[must_use]
    pub fn height(&self) -> f32 {
        self.hi - self.lo
    }
}

/// Remove the material interval `[a, b]` from a **sorted, disjoint** span
/// list, splitting a span when the removal falls strictly inside it.
///
/// This is 1-D interval difference: `spans \ [a, b]`. The result stays
/// sorted and disjoint. An empty removal (`a >= b`) is a no-op — critically,
/// it does *not* spuriously split a span at a zero-width cut.
///
/// Modelling note: `[a, b]` is the **removed** (cut-away) region, so what
/// survives is the part of each span *below* `a` and *above* `b`.
pub fn subtract_interval(spans: &mut Vec<Span>, a: f32, b: f32) {
    // Empty / inverted removal removes nothing. Guard first so a
    // zero-width cut (a == b) can't split a span into two touching halves.
    if a >= b {
        return;
    }
    let mut out: Vec<Span> = Vec::with_capacity(spans.len() + 1);
    for s in spans.iter().copied() {
        // Disjoint from the removal ⇒ the whole span survives.
        if b <= s.lo || a >= s.hi {
            out.push(s);
            continue;
        }
        // Overlapping: keep the sub-span below the cut and/or above it.
        // Each guard also guarantees the new span's `lo < hi`.
        if a > s.lo {
            out.push(Span { lo: s.lo, hi: a });
        }
        if b < s.hi {
            out.push(Span { lo: b, hi: s.hi });
        }
        // else: the removal covers this span entirely — drop it.
    }
    *spans = out;
}

/// Coalesce touching or overlapping spans in a **sorted-by-`lo`** list into
/// a minimal set of disjoint spans. Idempotent; a no-op on an already
/// disjoint list.
///
/// Two spans are merged when `prev.hi >= cur.lo` (they touch or overlap).
/// After [`subtract_interval`] the list is already disjoint, but a union of
/// carves from two sides (Phase 2 flip machining) can leave touching spans;
/// this is the normaliser that keeps the invariant.
pub fn merge_adjacent(spans: &mut Vec<Span>) {
    if spans.len() < 2 {
        return;
    }
    let mut out: Vec<Span> = Vec::with_capacity(spans.len());
    let mut cur = spans[0];
    for &s in &spans[1..] {
        if s.lo <= cur.hi {
            // Touching or overlapping — extend the running span upward.
            // max() guards against a fully-nested span (cur already
            // covers s) shrinking the upper bound.
            cur.hi = cur.hi.max(s.hi);
        } else {
            out.push(cur);
            cur = s;
        }
    }
    out.push(cur);
    *spans = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(pairs: &[(f32, f32)]) -> Vec<Span> {
        pairs.iter().map(|&(lo, hi)| Span { lo, hi }).collect()
    }

    #[test]
    fn span_new_rejects_empty_and_inverted() {
        assert_eq!(Span::new(1.0, 2.0), Some(Span { lo: 1.0, hi: 2.0 }));
        assert_eq!(Span::new(2.0, 2.0), None);
        assert_eq!(Span::new(3.0, 1.0), None);
    }

    #[test]
    fn subtract_disjoint_keeps_span() {
        // Removal entirely below the span.
        let mut s = spans(&[(0.0, 5.0)]);
        subtract_interval(&mut s, -3.0, -1.0);
        assert_eq!(s, spans(&[(0.0, 5.0)]));
        // Removal entirely above the span.
        subtract_interval(&mut s, 6.0, 9.0);
        assert_eq!(s, spans(&[(0.0, 5.0)]));
        // Touching at the boundary (b == s.lo / a == s.hi) is still disjoint.
        subtract_interval(&mut s, -2.0, 0.0);
        assert_eq!(s, spans(&[(0.0, 5.0)]));
        subtract_interval(&mut s, 5.0, 7.0);
        assert_eq!(s, spans(&[(0.0, 5.0)]));
    }

    #[test]
    fn subtract_trims_bottom() {
        // Remove [-1, 2] from [0, 5] ⇒ [2, 5].
        let mut s = spans(&[(0.0, 5.0)]);
        subtract_interval(&mut s, -1.0, 2.0);
        assert_eq!(s, spans(&[(2.0, 5.0)]));
    }

    #[test]
    fn subtract_trims_top() {
        // Remove [3, 9] from [0, 5] ⇒ [0, 3]. This is the top-down carve —
        // exactly the heightmap's monotone-min on a single top span.
        let mut s = spans(&[(0.0, 5.0)]);
        subtract_interval(&mut s, 3.0, 9.0);
        assert_eq!(s, spans(&[(0.0, 3.0)]));
    }

    #[test]
    fn subtract_interior_splits() {
        // Remove [2, 3] from [0, 5] ⇒ [0, 2] + [3, 5]: an undercut void.
        let mut s = spans(&[(0.0, 5.0)]);
        subtract_interval(&mut s, 2.0, 3.0);
        assert_eq!(s, spans(&[(0.0, 2.0), (3.0, 5.0)]));
    }

    #[test]
    fn subtract_full_cover_empties() {
        // Removal swallows the span (and then some) ⇒ nothing left.
        let mut s = spans(&[(1.0, 4.0)]);
        subtract_interval(&mut s, 0.0, 9.0);
        assert!(s.is_empty());
        // Exact-boundary cover also empties.
        let mut s = spans(&[(1.0, 4.0)]);
        subtract_interval(&mut s, 1.0, 4.0);
        assert!(s.is_empty());
    }

    #[test]
    fn subtract_zero_width_is_noop() {
        // A zero-width removal (a == b) must NOT split the span — the guard
        // is what stops a 60fps partial-t driver from shredding a column
        // into touching slivers at chord joints.
        let mut s = spans(&[(0.0, 5.0)]);
        subtract_interval(&mut s, 2.5, 2.5);
        assert_eq!(s, spans(&[(0.0, 5.0)]));
        // Inverted removal is likewise a no-op.
        subtract_interval(&mut s, 3.0, 1.0);
        assert_eq!(s, spans(&[(0.0, 5.0)]));
    }

    #[test]
    fn subtract_spans_multiple() {
        // One removal crossing several spans trims each independently.
        let mut s = spans(&[(0.0, 2.0), (4.0, 6.0), (8.0, 10.0)]);
        // Remove [1, 9]: clips [0,2]→[0,1], swallows [4,6], clips [8,10]→[9,10].
        subtract_interval(&mut s, 1.0, 9.0);
        assert_eq!(s, spans(&[(0.0, 1.0), (9.0, 10.0)]));
    }

    #[test]
    fn merge_coalesces_touching_and_overlapping() {
        let mut s = spans(&[(0.0, 2.0), (2.0, 4.0)]); // touching
        merge_adjacent(&mut s);
        assert_eq!(s, spans(&[(0.0, 4.0)]));

        let mut s = spans(&[(0.0, 3.0), (2.0, 5.0)]); // overlapping
        merge_adjacent(&mut s);
        assert_eq!(s, spans(&[(0.0, 5.0)]));

        let mut s = spans(&[(0.0, 9.0), (2.0, 4.0)]); // nested (cur covers s)
        merge_adjacent(&mut s);
        assert_eq!(s, spans(&[(0.0, 9.0)]));
    }

    #[test]
    fn merge_keeps_disjoint_and_is_idempotent() {
        let disjoint = spans(&[(0.0, 2.0), (3.0, 5.0)]);
        let mut s = disjoint.clone();
        merge_adjacent(&mut s);
        assert_eq!(s, disjoint);
        // Running it again changes nothing.
        merge_adjacent(&mut s);
        assert_eq!(s, disjoint);
    }

    /// The load-bearing property: carving is **associative**, so splitting a
    /// cut into partial-t chords `[0,t]` then `[t,1]` yields the byte-identical
    /// span list as the whole cut `[0,1]`. This is the interval-algebra
    /// analogue of `sweep.rs`'s `partial_advance_non_flat_no_drift` guarantee
    /// (which today rests on `min()` being order-independent).
    #[test]
    fn partial_removal_is_associative() {
        // A generous span so the removals fall strictly inside it.
        let base = spans(&[(-5.0, 5.0)]);

        // Whole removal in one shot.
        let mut whole = base.clone();
        subtract_interval(&mut whole, 0.0, 1.0);
        merge_adjacent(&mut whole);

        // Same removal split at an arbitrary interior seam t = 0.5.
        let mut split = base.clone();
        subtract_interval(&mut split, 0.0, 0.5);
        subtract_interval(&mut split, 0.5, 1.0);
        merge_adjacent(&mut split);

        assert_eq!(whole, split, "partial-t carve must be bitwise-identical");

        // And splitting the OTHER way (top half first) is identical too.
        let mut split_rev = base.clone();
        subtract_interval(&mut split_rev, 0.5, 1.0);
        subtract_interval(&mut split_rev, 0.0, 0.5);
        merge_adjacent(&mut split_rev);
        assert_eq!(whole, split_rev, "carve order must not matter");
    }

    /// A single top-reaching span carved from above collapses to exactly the
    /// heightmap's monotone-min: repeatedly removing `[z, +big]` leaves the
    /// span `[floor, min_z_so_far]`, and a higher (weaker) cut is a no-op.
    #[test]
    fn top_down_carve_matches_monotone_min() {
        let floor = -10.0_f32;
        let mut s = spans(&[(floor, 0.0)]); // stock top at z = 0
                                            // Cut to z = -2.
        subtract_interval(&mut s, -2.0, 1000.0);
        assert_eq!(s, spans(&[(floor, -2.0)]));
        // A shallower cut to z = -1 must NOT raise the surface (min wins).
        subtract_interval(&mut s, -1.0, 1000.0);
        assert_eq!(s, spans(&[(floor, -2.0)]));
        // A deeper cut to z = -5 lowers it.
        subtract_interval(&mut s, -5.0, 1000.0);
        assert_eq!(s, spans(&[(floor, -5.0)]));
    }
}
