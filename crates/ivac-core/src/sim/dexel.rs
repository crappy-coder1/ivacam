//! Multi-span Z-dexel primitives — the material-model evolution of the
//! single-Z heightmap (see the `ivac-58nl.6` design note).
//!
//! This module holds the **pure 1-D interval algebra** that a per-column
//! span list needs, plus [`DexelField`] — the hybrid dense-top +
//! sparse-undercut container built on it — with no sweep and no rendering
//! wired up yet. A column of stock becomes a sorted, disjoint list of solid
//! `Span`s along Z, and carving removes a `[lo, hi]` interval from that list.
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
// `DexelField`'s grid plumbing does the same f64↔u32↔usize casts as
// `super::heightmap`, so it carries the same cast allows.
#![allow(
    clippy::module_name_repetitions,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

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

use std::collections::HashMap;

use crate::geometry::Point2;

/// Hybrid material field: a **dense top surface** (identical layout and
/// semantics to [`super::heightmap::Heightmap`]'s `data`, so the 3-axis hot
/// path and the zero-copy WASM/GL upload are unchanged) plus a **sparse
/// undercut sidecar** carrying the full span list only for the columns that
/// actually grew an interior void.
///
/// The invariant that keeps the fast path free: a column with a single
/// full-height solid span `[stock_bottom_z, top]` lives in the dense `top`
/// array *alone* and never touches the sidecar. `top[idx]` always mirrors
/// the highest solid surface, so every existing dense reader keeps working.
///
/// Carving a top-reaching interval from such a column is exactly the
/// heightmap's monotone-`min()` (see [`DexelField::carve_cell`]'s fast
/// path); only an interior removal (an undercut) or a from-below carve
/// populates the sidecar.
#[derive(Debug, Clone)]
pub struct DexelField {
    pub origin: Point2,
    pub cell: f64,
    pub cols: u32,
    pub rows: u32,
    /// Uncut stock surface (mirrors `Heightmap::top_z`).
    pub top_z: f32,
    /// Stock floor — the `lo` of a fresh, uncut column's single span.
    /// Implicit in `Heightmap` (which is unbounded below); made explicit
    /// here because a span needs a bottom.
    pub stock_bottom_z: f32,
    /// DENSE fast path: highest solid Z per column, row-major `cols * rows`.
    /// Byte-identical in layout to `Heightmap::data`; zero-copy to WASM/GL.
    top: Vec<f32>,
    /// SPARSE sidecar keyed by cell index. Present ONLY for columns with an
    /// interior void; an absent key means the implicit single span
    /// `[stock_bottom_z, top[idx]]`. When present, the span list is the
    /// authoritative sorted, disjoint, ascending column and `top[idx]`
    /// still mirrors its highest `hi` for the dense readers.
    undercut: HashMap<usize, Vec<Span>>,
    /// Half-open dirty rectangle in cell indices (same contract as
    /// `Heightmap`'s): `None` = no mutations since the last `clear_dirty()`.
    dirty: Option<(u32, u32, u32, u32)>,
}

/// An opaque, cloneable capture of a [`DexelField`]'s full carve state —
/// the dense top surface **and** the undercut sidecar — for the live sim's
/// backward-scrub checkpoints. Restoring one reproduces the field exactly
/// (see [`DexelField::snapshot`] / [`DexelField::restore`]). Because carving
/// is monotone, checkpoints stay orderable by the segment boundary they
/// represent, exactly as the single-Z `Vec<f32>` snapshots did.
#[derive(Debug, Clone)]
pub struct DexelSnapshot {
    top: Vec<f32>,
    undercut: HashMap<usize, Vec<Span>>,
}

impl DexelSnapshot {
    /// The captured dense top surface (same layout as
    /// [`DexelField::top`]). Primarily for tests / inspection.
    #[must_use]
    pub fn top(&self) -> &[f32] {
        &self.top
    }

    /// Number of columns carrying an undercut sidecar entry in this snapshot.
    #[must_use]
    pub fn undercut_columns(&self) -> usize {
        self.undercut.len()
    }
}

impl DexelField {
    /// # Panics
    ///
    /// Panics on a non-positive `cell`, zero `cols` / `rows`, a `cols * rows`
    /// product that overflows `usize`, or `stock_bottom_z >= top_z`.
    #[must_use]
    pub fn new(
        origin: Point2,
        cell: f64,
        cols: u32,
        rows: u32,
        top_z: f32,
        stock_bottom_z: f32,
    ) -> Self {
        assert!(cell > 0.0, "DexelField cell size must be > 0");
        assert!(cols > 0 && rows > 0, "DexelField dimensions must be > 0");
        assert!(
            stock_bottom_z < top_z,
            "DexelField stock_bottom_z must be below top_z"
        );
        // Same overflow guard as Heightmap::new — on wasm32 `usize` is u32,
        // so `cols * rows` can wrap and silently under-allocate.
        let len = (cols as usize)
            .checked_mul(rows as usize)
            .expect("dexel dim overflow");
        Self {
            origin,
            cell,
            cols,
            rows,
            top_z,
            stock_bottom_z,
            top: vec![top_z; len],
            undercut: HashMap::new(),
            dirty: None,
        }
    }

    /// Size a field to cover the world rectangle `[min_x, max_x] ×
    /// [min_y, max_y]` with `cell`-mm cells — the `DexelField` analogue of
    /// [`super::heightmap::Heightmap::from_bbox`], sharing its exact
    /// cols/rows sizing (a `ceil` plus a one-cell fencepost pad) so the
    /// dense grid is index-for-index identical to the heightmap the live sim
    /// used to build. `stock_bottom_z` is the span floor the heightmap left
    /// implicit.
    ///
    /// # Panics
    ///
    /// Panics on a non-positive `cell`, an empty bbox (`max_x <= min_x` or
    /// `max_y <= min_y`), or `stock_bottom_z >= top_z`.
    #[must_use]
    pub fn from_bbox(
        min_x: f64,
        min_y: f64,
        max_x: f64,
        max_y: f64,
        cell: f64,
        top_z: f32,
        stock_bottom_z: f32,
    ) -> Self {
        assert!(cell > 0.0, "DexelField cell size must be > 0");
        assert!(
            max_x > min_x && max_y > min_y,
            "DexelField bbox must be non-empty"
        );
        // Identical fencepost sizing to `Heightmap::from_bbox` — see the note
        // there for why the +1 pad keeps the bbox max-corner sampleable.
        let cols = (((max_x - min_x) / cell).ceil() as u32)
            .saturating_add(1)
            .max(1);
        let rows = (((max_y - min_y) / cell).ceil() as u32)
            .saturating_add(1)
            .max(1);
        Self::new(
            Point2::new(min_x, min_y),
            cell,
            cols,
            rows,
            top_z,
            stock_bottom_z,
        )
    }

    /// Build a `DexelField` from an existing [`super::heightmap::Heightmap`],
    /// adopting its geometry, current top surface, and dirty rectangle. The
    /// sidecar starts empty (every column is a single full-height span), so
    /// the two are carve-for-carve equivalent on the 3-axis path.
    /// `stock_bottom_z` supplies the span floor the heightmap leaves implicit.
    ///
    /// # Panics
    ///
    /// Panics if `stock_bottom_z >= hm.top_z`.
    #[must_use]
    pub fn from_heightmap(hm: &super::heightmap::Heightmap, stock_bottom_z: f32) -> Self {
        assert!(
            stock_bottom_z < hm.top_z,
            "DexelField stock_bottom_z must be below top_z"
        );
        Self {
            origin: hm.origin,
            cell: hm.cell,
            cols: hm.cols,
            rows: hm.rows,
            top_z: hm.top_z,
            stock_bottom_z,
            top: hm.data.clone(),
            undercut: HashMap::new(),
            dirty: hm.dirty_aabb(),
        }
    }

    #[inline]
    fn idx_of(&self, ix: u32, iy: u32) -> usize {
        (iy as usize) * (self.cols as usize) + (ix as usize)
    }

    /// Bounds-checked top-down carve: lower the column's top surface to `z`
    /// if `z` is below it. Exactly `Heightmap::lower_at`; a no-op for cells
    /// outside the grid.
    pub fn lower_at(&mut self, ix: u32, iy: u32, z: f32) {
        if ix >= self.cols || iy >= self.rows {
            return;
        }
        self.carve_cell(ix, iy, z, f32::INFINITY);
    }

    /// Unchecked top-down carve — the sweep loop pre-clamps to the cell
    /// rectangle. Equivalent to `carve_cell(ix, iy, z, +∞)`; kept as a named
    /// parity with `Heightmap::lower_at_unchecked`.
    #[inline]
    pub fn lower_at_unchecked(&mut self, ix: u32, iy: u32, z: f32) {
        self.carve_cell(ix, iy, z, f32::INFINITY);
    }

    /// Remove the solid material interval `[removed_lo, removed_hi]` from
    /// cell `(ix, iy)`. The general carve entry point.
    ///
    /// **Fast path** (the whole 3-axis workload): when the removal reaches
    /// the current top *and* the column is still a single full-height span
    /// (no sidecar entry), this is a plain top-down cut — literally the
    /// heightmap's monotone-`min()` on the dense array, with zero sidecar
    /// cost. `removed_hi == f32::INFINITY` always takes this branch on an
    /// undisturbed column, so top-down carving is byte-identical to
    /// `Heightmap::lower_at_unchecked`.
    ///
    /// **Slow path**: an interior removal (leaving material above the cut, an
    /// undercut) or an already multi-span column routes through
    /// [`subtract_interval`] and re-canonicalises via `store_column`.
    #[inline]
    pub fn carve_cell(&mut self, ix: u32, iy: u32, removed_lo: f32, removed_hi: f32) {
        let idx = self.idx_of(ix, iy);
        // `undercut.is_empty()` is an O(1), hash-free escape: for a pure
        // 3-axis job the sidecar is always empty, so this branch stays a
        // length check + the dense write below — matching Heightmap's cost.
        if removed_hi >= self.top[idx]
            && (self.undercut.is_empty() || !self.undercut.contains_key(&idx))
        {
            if removed_lo < self.top[idx] {
                self.top[idx] = removed_lo;
                self.mark_dirty(ix, iy);
            }
            return;
        }
        let mut spans = self.spans_for(idx);
        subtract_interval(&mut spans, removed_lo, removed_hi);
        self.store_column(idx, ix, iy, spans);
    }

    /// The authoritative span list for a column: the sidecar entry if
    /// present, else the implicit single span `[stock_bottom_z, top[idx]]`
    /// (or an empty list if the column has been cut below the floor, i.e. no
    /// solid material remains).
    fn spans_for(&self, idx: usize) -> Vec<Span> {
        match self.undercut.get(&idx) {
            Some(s) => s.clone(),
            None => {
                if self.top[idx] > self.stock_bottom_z {
                    vec![Span {
                        lo: self.stock_bottom_z,
                        hi: self.top[idx],
                    }]
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// Write a column back, keeping the dense/sidecar split canonical: a
    /// cleared or single full-height span lives in the dense array alone;
    /// anything with an interior void goes to the sidecar with `top[idx]`
    /// mirroring the highest surface. Always marks the cell dirty.
    fn store_column(&mut self, idx: usize, ix: u32, iy: u32, mut spans: Vec<Span>) {
        merge_adjacent(&mut spans);
        match spans.as_slice() {
            // Cut clean through — no solid left. Dense top drops to the floor
            // (the mesh collapses the underside there).
            [] => {
                self.top[idx] = self.stock_bottom_z;
                self.undercut.remove(&idx);
            }
            // A single floor-reaching span is a pure top-down column: dense
            // only, no sidecar. This is how an undercut heals back once a
            // deeper cut removes the overhang above it.
            [s] if s.lo <= self.stock_bottom_z => {
                self.top[idx] = s.hi;
                self.undercut.remove(&idx);
            }
            // Interior void (or a floating slab that no longer reaches the
            // floor) ⇒ sidecar; dense top mirrors the highest surface.
            _ => {
                let hi = spans.last().map_or(self.stock_bottom_z, |s| s.hi);
                self.top[idx] = hi;
                self.undercut.insert(idx, spans);
            }
        }
        self.mark_dirty(ix, iy);
    }

    #[inline]
    fn mark_dirty(&mut self, ix: u32, iy: u32) {
        self.dirty = Some(match self.dirty {
            None => (ix, iy, ix + 1, iy + 1),
            Some((x0, y0, x1, y1)) => (x0.min(ix), y0.min(iy), x1.max(ix + 1), y1.max(iy + 1)),
        });
    }

    /// Reset every column to uncut stock and clear the sidecar + dirty rect.
    pub fn reset(&mut self) {
        for c in &mut self.top {
            *c = self.top_z;
        }
        self.undercut.clear();
        self.dirty = None;
    }

    #[must_use]
    pub fn dirty_aabb(&self) -> Option<(u32, u32, u32, u32)> {
        self.dirty
    }

    pub fn clear_dirty(&mut self) {
        self.dirty = None;
    }

    /// Mark the whole grid dirty (e.g. after a checkpoint restore).
    pub fn mark_all_dirty(&mut self) {
        self.dirty = Some((0, 0, self.cols, self.rows));
    }

    /// The dense top surface — same layout / semantics as `Heightmap::data`.
    #[must_use]
    pub fn top(&self) -> &[f32] {
        &self.top
    }

    /// Highest solid Z at cell `(ix, iy)`.
    #[must_use]
    pub fn top_at(&self, ix: u32, iy: u32) -> f32 {
        self.top[self.idx_of(ix, iy)]
    }

    #[must_use]
    pub fn top_ptr(&self) -> *const f32 {
        self.top.as_ptr()
    }

    #[must_use]
    pub fn top_len(&self) -> usize {
        self.top.len()
    }

    /// Number of columns currently carrying an undercut sidecar entry. `0`
    /// for any pure 3-axis (top-down) job.
    #[must_use]
    pub fn undercut_columns(&self) -> usize {
        self.undercut.len()
    }

    /// The authoritative solid span list for a column (sidecar entry or the
    /// implicit single span). Primarily for inspection / tests and the
    /// eventual undercut mesh builder.
    #[must_use]
    pub fn spans_at(&self, ix: u32, iy: u32) -> Vec<Span> {
        self.spans_for(self.idx_of(ix, iy))
    }

    /// Capture the full carve state (dense top + undercut sidecar) for a
    /// backward-scrub checkpoint. Cheap for a pure 3-axis job — the sidecar
    /// clone is empty and only the dense `Vec<f32>` is copied, exactly the
    /// old `Heightmap::data.clone()` cost.
    #[must_use]
    pub fn snapshot(&self) -> DexelSnapshot {
        DexelSnapshot {
            top: self.top.clone(),
            undercut: self.undercut.clone(),
        }
    }

    /// Restore a previously captured [`DexelSnapshot`], overwriting both the
    /// dense top and the sidecar and marking the whole grid dirty so a
    /// renderer re-uploads everything.
    ///
    /// # Panics
    ///
    /// Panics if the snapshot's grid size differs from this field's (i.e. it
    /// came from a differently-sized field).
    pub fn restore(&mut self, snap: &DexelSnapshot) {
        assert_eq!(
            snap.top.len(),
            self.top.len(),
            "DexelSnapshot grid size mismatch on restore"
        );
        self.top.copy_from_slice(&snap.top);
        self.undercut.clone_from(&snap.undercut);
        self.mark_all_dirty();
    }

    /// Flatten the undercut sidecar into three parallel CSR buffers for a
    /// zero-copy upload to JS (the renderer builds undercut walls/floors from
    /// them). Columns are emitted **sorted by flat cell index** so the output
    /// is deterministic regardless of the sidecar `HashMap`'s iteration order.
    ///
    /// Layout — for the `i`-th emitted column:
    /// * `col_index[i]` is its flat cell index (`iy * cols + ix`),
    /// * its spans are `spans[2 * span_offsets[i] .. 2 * span_offsets[i + 1]]`
    ///   as consecutive `(lo, hi)` `f32` pairs.
    ///
    /// `span_offsets` is a CSR row-pointer with `col_index.len() + 1` entries:
    /// `span_offsets[0] == 0` and the final entry is the total span count. For
    /// a pure 3-axis job the sidecar is empty, so `col_index` and `spans` are
    /// empty and `span_offsets == [0]`.
    #[must_use]
    pub fn undercut_csr(&self) -> (Vec<u32>, Vec<u32>, Vec<f32>) {
        let mut keys: Vec<usize> = self.undercut.keys().copied().collect();
        keys.sort_unstable();
        let mut col_index = Vec::with_capacity(keys.len());
        let mut span_offsets = Vec::with_capacity(keys.len() + 1);
        let mut spans = Vec::new();
        span_offsets.push(0u32);
        let mut running = 0u32;
        for idx in keys {
            let column = &self.undercut[&idx];
            col_index.push(idx as u32);
            for s in column {
                spans.push(s.lo);
                spans.push(s.hi);
            }
            running += column.len() as u32;
            span_offsets.push(running);
        }
        (col_index, span_offsets, spans)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::heightmap::Heightmap;

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

    // --- DexelField ---

    /// Deterministic LCG (Knuth MMIX constants) so the differential corpus
    /// below is reproducible without an rng dependency.
    fn lcg(state: &mut u64) -> u32 {
        *state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (*state >> 33) as u32
    }

    /// The crown-jewel guarantee of landing #2: a pure top-down carve
    /// sequence drives `DexelField` and `Heightmap` to a **bit-for-bit
    /// identical** top surface, never touches the undercut sidecar, and
    /// tracks the same dirty AABB. The dense fast path *is* the heightmap's
    /// monotone-`min()`.
    #[test]
    fn dexel_top_down_matches_heightmap_bitwise() {
        let origin = Point2::new(-3.0, 2.0);
        let (cell, cols, rows, top_z) = (0.2_f64, 17_u32, 11_u32, 4.0_f32);
        let mut hm = Heightmap::new(origin, cell, cols, rows, top_z);
        // Floor far below so it never interferes; top-down carving must stay
        // on the dense fast path regardless of where it sits.
        let mut df = DexelField::new(origin, cell, cols, rows, top_z, top_z - 1000.0);

        let mut rng: u64 = 0x2545_f491_4f6c_dd1d;
        for _ in 0..20_000 {
            let r = lcg(&mut rng);
            let ix = r % cols;
            let iy = (r / cols) % rows;
            // z ranges from top_z down to top_z - 6: some ops are no-ops
            // (z above the current surface), some descend — hammering both
            // sides of the monotone-min branch.
            let z = top_z - (lcg(&mut rng) % 600) as f32 * 0.01;
            hm.lower_at_unchecked(ix, iy, z);
            df.carve_cell(ix, iy, z, f32::INFINITY);
        }

        // 1. The dense top surface is byte-for-byte identical.
        assert_eq!(df.top().len(), hm.data.len());
        for (a, b) in df.top().iter().zip(hm.data.iter()) {
            assert_eq!(
                a.to_bits(),
                b.to_bits(),
                "top surface diverged from heightmap"
            );
        }
        // 2. Pure top-down carving NEVER populates the undercut sidecar.
        assert_eq!(df.undercut_columns(), 0, "top-down carves must stay dense");
        // 3. The dirty AABB tracks identically.
        assert_eq!(df.dirty_aabb(), hm.dirty_aabb());
    }

    #[test]
    fn from_heightmap_adopts_surface_and_dirty() {
        let origin = Point2::new(0.0, 0.0);
        let mut hm = Heightmap::new(origin, 1.0, 4, 4, 5.0);
        hm.lower_at(1, 2, 2.0);
        let df = DexelField::from_heightmap(&hm, -10.0);
        assert_eq!(df.top(), hm.data.as_slice());
        assert_eq!(df.dirty_aabb(), hm.dirty_aabb());
        assert_eq!(df.undercut_columns(), 0);
    }

    #[test]
    fn interior_carve_populates_sidecar_and_mirrors_top() {
        let mut df = DexelField::new(Point2::new(0.0, 0.0), 1.0, 4, 4, 5.0, 0.0);
        // Remove the interior band [2,3] from column (1,1): [0,5] -> [0,2]+[3,5].
        df.carve_cell(1, 1, 2.0, 3.0);
        assert_eq!(df.undercut_columns(), 1);
        assert_eq!(
            df.spans_at(1, 1),
            spans(&[(0.0, 2.0), (3.0, 5.0)]),
            "interior cut leaves an undercut void"
        );
        // Dense top still mirrors the highest solid surface.
        assert_eq!(df.top_at(1, 1), 5.0);
        // Neighbouring columns are untouched and stay dense.
        assert_eq!(df.top_at(0, 0), 5.0);
    }

    #[test]
    fn undercut_heals_back_to_dense_when_overhang_removed() {
        let mut df = DexelField::new(Point2::new(0.0, 0.0), 1.0, 4, 4, 5.0, 0.0);
        df.carve_cell(1, 1, 2.0, 3.0); // [0,2]+[3,5] — sidecar
        assert_eq!(df.undercut_columns(), 1);
        // Remove everything above z = 1 ⇒ single floor-reaching span [0,1].
        df.carve_cell(1, 1, 1.0, f32::INFINITY);
        assert_eq!(
            df.undercut_columns(),
            0,
            "a floor-reaching single span heals back to dense"
        );
        assert_eq!(df.spans_at(1, 1), spans(&[(0.0, 1.0)]));
        assert_eq!(df.top_at(1, 1), 1.0);
    }

    #[test]
    fn floating_slab_is_treated_as_undercut() {
        let mut df = DexelField::new(Point2::new(0.0, 0.0), 1.0, 4, 4, 5.0, 0.0);
        // A from-below carve removes [0,2]: [0,5] -> [2,5], a floating slab
        // that no longer reaches the floor ⇒ sidecar.
        df.carve_cell(1, 1, 0.0, 2.0);
        assert_eq!(df.undercut_columns(), 1);
        assert_eq!(df.spans_at(1, 1), spans(&[(2.0, 5.0)]));
        assert_eq!(df.top_at(1, 1), 5.0);
    }

    #[test]
    fn from_bbox_matches_heightmap_sizing() {
        // The dense grid must be index-for-index identical to the heightmap
        // the live sim built before the flip, so the zero-copy top upload and
        // every cell index carry over unchanged.
        let hm = Heightmap::from_bbox(-3.0, 2.0, 17.0, 22.0, 0.75, 4.0);
        let df = DexelField::from_bbox(-3.0, 2.0, 17.0, 22.0, 0.75, 4.0, -6.0);
        assert_eq!(df.cols, hm.cols);
        assert_eq!(df.rows, hm.rows);
        assert_eq!(df.origin, hm.origin);
        assert!((df.cell - hm.cell).abs() < 1e-12);
        assert_eq!(df.top(), hm.data.as_slice());
        assert!((df.stock_bottom_z - -6.0).abs() < 1e-6);
    }

    #[test]
    fn snapshot_restore_round_trips_undercut_sidecar() {
        let mut df = DexelField::new(Point2::new(0.0, 0.0), 1.0, 4, 4, 5.0, 0.0);
        // Grow an undercut void in one column so the snapshot carries a
        // non-empty sidecar (the whole point of storing top + undercut).
        df.carve_cell(1, 1, 2.0, 3.0); // [0,2]+[3,5] — sidecar entry
        assert_eq!(df.undercut_columns(), 1);
        let snap = df.snapshot();
        assert_eq!(snap.undercut_columns(), 1);
        assert_eq!(snap.top(), df.top());

        // Carve further (deepen an unrelated column + heal the undercut) so
        // the live field diverges from the snapshot.
        df.carve_cell(2, 2, 1.0, f32::INFINITY);
        df.carve_cell(1, 1, 1.0, f32::INFINITY); // heals the void back to dense
        assert_eq!(df.undercut_columns(), 0);

        df.clear_dirty();
        df.restore(&snap);
        // The sidecar void and the dense top are both back, byte-identical.
        assert_eq!(df.undercut_columns(), 1);
        assert_eq!(df.spans_at(1, 1), spans(&[(0.0, 2.0), (3.0, 5.0)]));
        assert_eq!(df.top(), snap.top());
        // Restore marks the whole grid dirty for a full re-upload.
        assert_eq!(df.dirty_aabb(), Some((0, 0, df.cols, df.rows)));
    }

    #[test]
    fn undercut_csr_flattens_sidecar_sorted() {
        let mut df = DexelField::new(Point2::new(0.0, 0.0), 1.0, 4, 4, 5.0, 0.0);
        // Two undercut columns with a distinct span shape each, carved in
        // reverse index order to prove the CSR sorts by flat cell index.
        df.carve_cell(2, 2, 2.0, 3.0); // flat idx = 2*4 + 2 = 10 → [0,2]+[3,5]
        df.carve_cell(1, 0, 1.0, 2.0); // flat idx = 0*4 + 1 = 1  → [0,1]+[2,5]
        let (col_index, span_offsets, spans_flat) = df.undercut_csr();
        assert_eq!(col_index, vec![1, 10], "columns sorted by flat cell index");
        // Each column has 2 spans → offsets 0,2,4.
        assert_eq!(span_offsets, vec![0, 2, 4]);
        // Column 1 (idx 1): spans [0,1] and [2,5].
        assert_eq!(&spans_flat[0..4], &[0.0, 1.0, 2.0, 5.0]);
        // Column 10 (idx 10): spans [0,2] and [3,5].
        assert_eq!(&spans_flat[4..8], &[0.0, 2.0, 3.0, 5.0]);
    }

    #[test]
    fn undercut_csr_empty_for_top_down_only() {
        let mut df = DexelField::new(Point2::new(0.0, 0.0), 1.0, 4, 4, 5.0, -10.0);
        df.carve_cell(1, 1, 2.0, f32::INFINITY); // pure top-down
        let (col_index, span_offsets, spans_flat) = df.undercut_csr();
        assert!(col_index.is_empty());
        assert!(spans_flat.is_empty());
        assert_eq!(span_offsets, vec![0], "CSR row-pointer keeps its leading 0");
    }

    #[test]
    fn carve_through_below_floor_stays_dense() {
        let mut df = DexelField::new(Point2::new(0.0, 0.0), 1.0, 4, 4, 5.0, 0.0);
        // Fast path: a top-reaching removal whose floor dips below the stock
        // bottom (Heightmap allows z below anything) — dense, no sidecar.
        df.carve_cell(1, 1, -1.0, f32::INFINITY);
        assert_eq!(df.top_at(1, 1), -1.0);
        assert_eq!(df.undercut_columns(), 0);
        // spans_for now yields no solid material (top below the floor).
        assert!(df.spans_at(1, 1).is_empty());
    }
}
