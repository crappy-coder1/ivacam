//! Target 3D surface for relief / ball-nose surfacing.
//!
//! A [`SurfaceField`] is the INPUT counterpart to the simulator's
//! [`crate::sim::heightmap::Heightmap`]: same row-major grid + bilinear
//! sampling, but it describes the surface we WANT to cut, not the carved
//! result. Cell `z[iy * cols + ix]` is the target Z at that grid point,
//! with the stock top at `z = 0` and relief carved downward (negative Z).
//!
//! The surface SOURCE is pluggable — the first one is a grayscale
//! image mapped through [`SurfaceField::from_grayscale`]; a future STL
//! rasterizer feeds the very same type. The drop-cutter surfacing engine
//! reads it through [`SurfaceField::sample`].
//!
//! Outside the field footprint `sample` returns `0.0` (the stock top) —
//! there is no relief beyond the image, so a ball-nose probing past the
//! edge sees uncut stock and won't gouge below it.

// f64 ↔ u32 grid-coordinate plumbing means a lot of intentional casts,
// mirroring the sibling heightmap module.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_lossless
)]

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::geometry::Point2;
use crate::sim::dexel::DexelField;

/// Z (mm) returned when sampling outside the field footprint: the stock
/// top, i.e. "no relief here, don't cut below the surface".
pub const SURFACE_TOP_Z: f32 = 0.0;

/// How a carved simulation cell deviates from the target surface — the
/// per-cell class behind the red/green deviation overlay
/// ([`SurfaceField::deviation_of`]). The discriminants are the stable wire
/// values the WASM bridge hands JS as a `Uint8Array`, so JS can map them to
/// vertex colors without a second lookup table.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deviation {
    /// Carved surface within ±tol of the target — neutral (no tint).
    OnTarget = 0,
    /// Carved more than `tol` mm BELOW the target: material removed that
    /// should have remained. Rendered RED.
    Gouge = 1,
    /// Solid left more than `tol` mm ABOVE the target: rest stock still to
    /// remove. Rendered GREEN.
    RestStock = 2,
}

/// A target Z(x,y) surface over a rectangular footprint. Row-major
/// `cols * rows` cells; cell `(ix, iy)`'s center sits at
/// `origin + ((ix + 0.5) * cell, (iy + 0.5) * cell)`, matching the
/// simulator heightmap's cell-center convention so the two grids align.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SurfaceField {
    /// World XY of the field's min corner (the (0,0) cell's lower-left).
    pub origin: Point2,
    /// Cell size in mm (square cells).
    pub cell: f64,
    pub cols: u32,
    pub rows: u32,
    /// Row-major target Z per cell (mm). Length must be `cols * rows`.
    /// Convention: stock top at 0, relief carved downward (Z <= 0).
    pub z: Vec<f32>,
}

impl SurfaceField {
    /// Build a field from an explicit Z grid.
    ///
    /// # Panics
    ///
    /// Panics if `cell <= 0`, either dimension is 0, the `cols * rows`
    /// product overflows `usize`, or `z.len() != cols * rows`.
    #[must_use]
    pub fn new(origin: Point2, cell: f64, cols: u32, rows: u32, z: Vec<f32>) -> Self {
        assert!(cell > 0.0, "SurfaceField cell size must be > 0");
        assert!(cols > 0 && rows > 0, "SurfaceField dimensions must be > 0");
        let len = (cols as usize)
            .checked_mul(rows as usize)
            .expect("surface dim overflow");
        assert_eq!(z.len(), len, "SurfaceField z length must equal cols * rows");
        Self {
            origin,
            cell,
            cols,
            rows,
            z,
        }
    }

    /// Map a normalized-brightness grid (each value in `[0, 1]`, row-major
    /// `cols * rows`) into a target surface, the relief-milling source.
    /// Brightness is linearly mapped to Z in `[z_min_mm, z_max_mm]`:
    /// by default bright = high (toward `z_max_mm`, the shallow/top end),
    /// dark = low (toward `z_min_mm`, the deepest cut) — the standard
    /// white-is-high relief convention. `invert` flips that (useful for
    /// negatives / engrave-the-light-areas reliefs).
    ///
    /// `z_min_mm` is the deepest (most negative) Z and `z_max_mm` the
    /// shallowest; they're sorted internally so callers can't invert the
    /// span by accident. Brightness values are clamped to `[0, 1]`.
    ///
    /// # Panics
    ///
    /// Panics under the same dimension rules as [`SurfaceField::new`]:
    /// `cell` must be > 0, both dimensions must be > 0, and
    /// `brightness.len()` must equal `cols * rows`.
    #[must_use]
    pub fn from_grayscale(
        origin: Point2,
        cell: f64,
        cols: u32,
        rows: u32,
        brightness: &[f32],
        z_min_mm: f64,
        z_max_mm: f64,
        invert: bool,
    ) -> Self {
        let len = (cols as usize)
            .checked_mul(rows as usize)
            .expect("surface dim overflow");
        assert_eq!(
            brightness.len(),
            len,
            "brightness length must equal cols * rows"
        );
        // Tolerate a flipped span: lo is always the deepest cut.
        let lo = z_min_mm.min(z_max_mm) as f32;
        let hi = z_min_mm.max(z_max_mm) as f32;
        let z = brightness
            .iter()
            .map(|&b| {
                let mut t = b.clamp(0.0, 1.0);
                if invert {
                    t = 1.0 - t;
                }
                // t = 1 (bright) → hi (shallow/top); t = 0 (dark) → lo (deep).
                lo + t * (hi - lo)
            })
            .collect();
        Self::new(origin, cell, cols, rows, z)
    }

    /// Rasterize a triangle mesh into a target surface: a **Z-max height
    /// buffer** over the mesh's XY footprint, then shifted so the mesh's
    /// highest point sits at the stock top (`z = 0`) with everything below
    /// carved downward — the same "drop the model onto the stock top"
    /// convention [`SurfaceField::from_grayscale`] bakes in. Taking the max
    /// Z per column yields the visible TOP surface, which is exactly what
    /// the vertical drop-cutter in [`crate::cam::surface_mill`] should
    /// follow; undercuts a 3-axis tool cannot reach are correctly ignored.
    ///
    /// `tris` are `[v0, v1, v2]` triangles of `[x, y, z]` in mm (Z up), as
    /// [`crate::sim::stl::parse_stl`] returns. `cell` is the grid resolution
    /// in mm; the grid is sized to the mesh's XY bounding box. Cells that no
    /// triangle covers (holes, or the gap between a non-rectangular outline
    /// and its bounding box) stay at [`SURFACE_TOP_Z`] — no relief there, so
    /// a ball-nose probing them sees uncut stock and won't gouge below it.
    ///
    /// Returns `None` when the mesh has no positive-area XY footprint (empty,
    /// or every triangle is a vertical sliver seen from above): there is no
    /// surface to sample.
    ///
    /// # Panics
    ///
    /// Panics if `cell <= 0`.
    #[must_use]
    pub fn from_mesh(tris: &[[[f32; 3]; 3]], cell: f64) -> Option<Self> {
        assert!(cell > 0.0, "SurfaceField cell size must be > 0");

        // XY bounding box over every vertex.
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for tri in tris {
            for v in tri {
                min_x = min_x.min(v[0] as f64);
                min_y = min_y.min(v[1] as f64);
                max_x = max_x.max(v[0] as f64);
                max_y = max_y.max(v[1] as f64);
            }
        }
        // Empty, or the whole mesh collapses to a line/point in XY.
        if !min_x.is_finite() || max_x <= min_x || max_y <= min_y {
            return None;
        }

        let cols = (((max_x - min_x) / cell).ceil() as u32).max(1);
        let rows = (((max_y - min_y) / cell).ceil() as u32).max(1);
        let cols_us = cols as usize;
        let rows_us = rows as usize;
        // Max Z per cell; NEG_INFINITY marks "no triangle covered this cell".
        let mut buf = vec![f32::NEG_INFINITY; cols_us * rows_us];

        for tri in tris {
            let (a, b, c) = (tri[0], tri[1], tri[2]);
            let (ax, ay) = (a[0] as f64, a[1] as f64);
            let (bx, by) = (b[0] as f64, b[1] as f64);
            let (cx, cy) = (c[0] as f64, c[1] as f64);
            // Signed double-area in XY (also the barycentric denominator).
            // Skip vertical / degenerate triangles that project to ~zero
            // area — their top edge Z is picked up by the horizontal
            // triangles that share it.
            let denom = (bx - ax) * (cy - ay) - (cx - ax) * (by - ay);
            if denom.abs() < 1e-12 {
                continue;
            }

            // Triangle XY bbox → the block of cell centers it can cover. A
            // cell `(ix, iy)` centers at `min + (i + 0.5) * cell`, so invert
            // that to bound the index range.
            let tmin_x = ax.min(bx).min(cx);
            let tmax_x = ax.max(bx).max(cx);
            let tmin_y = ay.min(by).min(cy);
            let tmax_y = ay.max(by).max(cy);
            let ix_lo = (((tmin_x - min_x) / cell) - 0.5).ceil().max(0.0);
            let iy_lo = (((tmin_y - min_y) / cell) - 0.5).ceil().max(0.0);
            let ix_hi = (((tmax_x - min_x) / cell) - 0.5)
                .floor()
                .min((cols - 1) as f64);
            let iy_hi = (((tmax_y - min_y) / cell) - 0.5)
                .floor()
                .min((rows - 1) as f64);
            if ix_hi < ix_lo || iy_hi < iy_lo {
                continue;
            }
            let (ix_lo, ix_hi) = (ix_lo as usize, ix_hi as usize);
            let (iy_lo, iy_hi) = (iy_lo as usize, iy_hi as usize);

            for iy in iy_lo..=iy_hi {
                let py = min_y + (iy as f64 + 0.5) * cell;
                for ix in ix_lo..=ix_hi {
                    let px = min_x + (ix as f64 + 0.5) * cell;
                    // Barycentric weights relative to vertex `c`.
                    let l1 = ((by - cy) * (px - cx) + (cx - bx) * (py - cy)) / denom;
                    let l2 = ((cy - ay) * (px - cx) + (ax - cx) * (py - cy)) / denom;
                    let l3 = 1.0 - l1 - l2;
                    // A small negative tolerance keeps cell centers that land
                    // exactly on a shared edge claimed by at least one
                    // triangle, so there are no seam holes. Double-covering an
                    // edge cell is harmless under the Z-max reduction.
                    #[allow(clippy::items_after_statements)] // defined at use, with its comment
                    const BARY_EPS: f64 = 1e-9;
                    if l1 < -BARY_EPS || l2 < -BARY_EPS || l3 < -BARY_EPS {
                        continue;
                    }
                    let z = (l1 * a[2] as f64 + l2 * b[2] as f64 + l3 * c[2] as f64) as f32;
                    let idx = iy * cols_us + ix;
                    if z > buf[idx] {
                        buf[idx] = z;
                    }
                }
            }
        }

        // The mesh's highest sampled point becomes the stock top.
        let global_max = buf
            .iter()
            .copied()
            .filter(|z| z.is_finite())
            .fold(f32::NEG_INFINITY, f32::max);
        if !global_max.is_finite() {
            // Every triangle was degenerate in XY — nothing got covered.
            return None;
        }
        let z: Vec<f32> = buf
            .iter()
            .map(|&v| {
                if v.is_finite() {
                    v - global_max
                } else {
                    SURFACE_TOP_Z
                }
            })
            .collect();
        Some(Self::new(Point2::new(min_x, min_y), cell, cols, rows, z))
    }

    /// Parse an STL byte stream (binary or ASCII) and rasterize it into a
    /// target surface via [`SurfaceField::from_mesh`]. `cell` is the grid
    /// resolution in mm. Returns `Ok(None)` when the mesh has no XY footprint
    /// to sample (see [`SurfaceField::from_mesh`]).
    ///
    /// # Errors
    ///
    /// Returns [`crate::sim::stl::StlError`] if the bytes are not a valid STL.
    pub fn from_stl(bytes: &[u8], cell: f64) -> Result<Option<Self>, crate::sim::stl::StlError> {
        let tris = crate::sim::stl::parse_stl(bytes)?;
        Ok(Self::from_mesh(&tris, cell))
    }

    /// Like [`SurfaceField::from_stl`], but sizes the grid from a cell-count
    /// budget instead of an explicit `cell`: the longer XY side of the mesh
    /// spans at most `max_dim` cells (`cell = longer_extent / max_dim`).
    /// This mirrors the image relief's `maxDim` downsample budget so a
    /// physically large STL doesn't rasterize to an enormous grid, and it
    /// frees the caller from having to know the model's size up front.
    /// `max_dim` is clamped to at least 1. Returns `Ok(None)` when the mesh
    /// has no XY footprint (see [`SurfaceField::from_mesh`]).
    ///
    /// # Errors
    ///
    /// Returns [`crate::sim::stl::StlError`] if the bytes are not a valid STL.
    pub fn from_stl_capped(
        bytes: &[u8],
        max_dim: u32,
    ) -> Result<Option<Self>, crate::sim::stl::StlError> {
        let tris = crate::sim::stl::parse_stl(bytes)?;
        let max_dim = f64::from(max_dim.max(1));
        // Lightweight XY-extent pre-pass to pick the cell size; `from_mesh`
        // recomputes the full bbox (same triangles → consistent origin/dims).
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for tri in &tris {
            for v in tri {
                min_x = min_x.min(v[0] as f64);
                min_y = min_y.min(v[1] as f64);
                max_x = max_x.max(v[0] as f64);
                max_y = max_y.max(v[1] as f64);
            }
        }
        // No positive XY footprint → no surface (matches `from_mesh`).
        if !min_x.is_finite() || max_x <= min_x || max_y <= min_y {
            return Ok(None);
        }
        let cell = (max_x - min_x).max(max_y - min_y) / max_dim;
        Ok(Self::from_mesh(&tris, cell))
    }

    /// Target Z at cell `(ix, iy)`. Returns [`SURFACE_TOP_Z`] for indices
    /// outside the grid (no relief there).
    #[must_use]
    pub fn at(&self, ix: u32, iy: u32) -> f32 {
        if ix >= self.cols || iy >= self.rows {
            return SURFACE_TOP_Z;
        }
        self.z[(iy as usize) * (self.cols as usize) + (ix as usize)]
    }

    /// Bilinear sample of the target Z at world XY. Cell `(i, j)`'s center
    /// is `origin + (i + 0.5) * cell`; positions outside the sampleable
    /// region return [`SURFACE_TOP_Z`] (stock top — no relief beyond the
    /// footprint). Mirrors [`crate::sim::heightmap::Heightmap::sample`].
    #[must_use]
    pub fn sample(&self, x: f64, y: f64) -> f32 {
        let fx = (x - self.origin.x) / self.cell - 0.5;
        let fy = (y - self.origin.y) / self.cell - 0.5;
        if !fx.is_finite() || !fy.is_finite() {
            return SURFACE_TOP_Z;
        }
        let cols_max = self.cols as f64 - 1.0;
        let rows_max = self.rows as f64 - 1.0;
        if fx < 0.0 || fy < 0.0 || fx > cols_max || fy > rows_max {
            return SURFACE_TOP_Z;
        }
        let i0 = fx.floor();
        let j0 = fy.floor();
        let tx = (fx - i0) as f32;
        let ty = (fy - j0) as f32;
        let i0 = i0 as usize;
        let j0 = j0 as usize;
        let cols = self.cols as usize;
        let i1 = (i0 + 1).min(cols - 1);
        let j1 = (j0 + 1).min(self.rows as usize - 1);
        let v00 = self.z[j0 * cols + i0];
        let v10 = self.z[j0 * cols + i1];
        let v01 = self.z[j1 * cols + i0];
        let v11 = self.z[j1 * cols + i1];
        let a = v00 * (1.0 - tx) + v10 * tx;
        let b = v01 * (1.0 - tx) + v11 * tx;
        a * (1.0 - ty) + b * ty
    }

    /// World-space max corner (`origin + (cols, rows) * cell`).
    #[must_use]
    pub fn max_x(&self) -> f64 {
        self.origin.x + self.cols as f64 * self.cell
    }
    /// World-space max corner Y.
    #[must_use]
    pub fn max_y(&self) -> f64 {
        self.origin.y + self.rows as f64 * self.cell
    }

    /// `(min, max)` of the stored target Z, or `(0, 0)` for an empty grid.
    #[must_use]
    pub fn z_range(&self) -> (f32, f32) {
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for &v in &self.z {
            lo = lo.min(v);
            hi = hi.max(v);
        }
        if lo.is_finite() {
            (lo, hi)
        } else {
            (0.0, 0.0)
        }
    }

    /// Classify how a carved simulation field deviates from this target
    /// surface, cell-for-cell — the data behind the red/green deviation
    /// overlay (GrblGru structurally can't offer this: it never carves). For
    /// every cell of `field` the carved top surface is compared against the
    /// target Z sampled at that cell's world center:
    ///
    /// * [`Deviation::Gouge`] — carved more than `tol` mm BELOW target
    ///   (material removed that should have stayed). Rendered red.
    /// * [`Deviation::RestStock`] — solid left more than `tol` mm ABOVE target
    ///   (uncut stock still to remove). Rendered green.
    /// * [`Deviation::OnTarget`] — within ±`tol` mm of the target. Neutral.
    ///
    /// `surface_z0` is the world Z that this field's `z = 0` datum maps to.
    /// The target's z=0 is its highest point, dropped onto the stock top by
    /// [`SurfaceField::from_mesh`] / [`SurfaceField::from_grayscale`], so a
    /// relief job passes the simulator's stock-top Z (`field.top_z`). `tol` is
    /// the on-target band half-width in mm; a negative `tol` is clamped to 0.
    ///
    /// The result is a row-major `field.cols * field.rows` byte grid aligned
    /// index-for-index with [`DexelField::top`], each byte a [`Deviation`]
    /// `as u8`, so the WASM bridge can hand JS a `Uint8Array` for per-cell
    /// vertex coloring off the same dirty-AABB the carve already reports.
    ///
    /// The two grids need not share dimensions or origin — the comparison
    /// samples the target at each carved cell's world center, so any relative
    /// placement works. Cells the target footprint doesn't cover sample as the
    /// stock top (0), so uncut stock beyond the relief reads as on-target. For
    /// a form-tool undercut, `top` holds only the highest surface, so the
    /// overlay compares that (the deepest void is not represented — undercuts
    /// are out of scope for a 3-axis relief verify).
    #[must_use]
    pub fn deviation_of(&self, field: &DexelField, surface_z0: f32, tol: f32) -> Vec<u8> {
        deviation_union_of(std::slice::from_ref(self), field, surface_z0, tol)
    }

    /// Reclassify only the half-open cell rectangle `[ix0, ix1) × [iy0, iy1)`
    /// (in `field`'s grid coordinates) in place, leaving every cell outside it
    /// untouched — the dirty-AABB counterpart of [`SurfaceField::deviation_of`].
    ///
    /// A carve frame only changes the small rectangle the tool swept, so the
    /// per-cell class of every other cell is already correct in `out`. Passing
    /// that carve's dirty AABB here reclassifies just those cells instead of
    /// re-sampling the whole target every frame, mirroring the mesh's
    /// partial-AABB re-upload. Over many frames the caller keeps `out` globally
    /// valid by only ever passing rectangles that cover the cells whose height
    /// changed.
    ///
    /// `out` must be exactly `field.cols * field.rows` long and index-aligned
    /// with [`DexelField::top`]; the rectangle is clamped to the grid, and an
    /// empty or inverted rectangle is a no-op. See [`SurfaceField::deviation_of`]
    /// for the classification and the meaning of `surface_z0` / `tol`.
    ///
    /// # Panics
    ///
    /// Panics if `out.len() != field.cols * field.rows`.
    pub fn deviation_into(
        &self,
        field: &DexelField,
        surface_z0: f32,
        tol: f32,
        out: &mut [u8],
        ix0: u32,
        iy0: u32,
        ix1: u32,
        iy1: u32,
    ) {
        deviation_union_into(
            std::slice::from_ref(self),
            field,
            surface_z0,
            tol,
            out,
            ix0,
            iy0,
            ix1,
            iy1,
        );
    }
}

/// The full-grid convenience over [`deviation_union_into`]: classify `field`
/// against the deepest-cut union of `surfaces` and return a fresh row-major
/// `cols * rows` class buffer. See [`deviation_union_into`] for how multiple
/// targets combine.
#[must_use]
pub fn deviation_union_of(
    surfaces: &[SurfaceField],
    field: &DexelField,
    surface_z0: f32,
    tol: f32,
) -> Vec<u8> {
    let cols = field.cols as usize;
    let rows = field.rows as usize;
    let mut out = vec![Deviation::OnTarget as u8; cols * rows];
    deviation_union_into(
        surfaces, field, surface_z0, tol, &mut out, 0, 0, field.cols, field.rows,
    );
    out
}

/// Classify a carved field against the union of several target surfaces —
/// the multi-relief overlay ([`SurfaceField::deviation_of`] is the single
/// surface case, implemented as a one-element union).
///
/// Relief ops carve CUMULATIVELY: after they all run, the intended surface at
/// a cell is the DEEPEST (most-negative Z) target of every relief covering it,
/// because material any op removes stays removed. So the combined target Z at
/// each cell is the `min` of `surface.sample(cx, cy)` across `surfaces` — a
/// cell outside a given surface's footprint samples as the stock top (0), so a
/// relief only deepens the target where it actually reaches. The carved top is
/// then classified once against that combined Z (gouge / rest-stock / on-target
/// per [`SurfaceField::deviation_of`]); merging per-surface *classes* instead
/// would misread a cell as gouged against a shallow target that a deeper one
/// meant to cut anyway.
///
/// Only cells in the clamped half-open rectangle `[ix0, ix1) × [iy0, iy1)` are
/// rewritten (the dirty-AABB path); the rest of `out` is left untouched. With
/// no surfaces every sampled cell reads as the bare stock top. `out` must be
/// `field.cols * field.rows` long and index-aligned with [`DexelField::top`].
///
/// # Panics
///
/// Panics if `out.len() != field.cols * field.rows`.
pub fn deviation_union_into(
    surfaces: &[SurfaceField],
    field: &DexelField,
    surface_z0: f32,
    tol: f32,
    out: &mut [u8],
    ix0: u32,
    iy0: u32,
    ix1: u32,
    iy1: u32,
) {
    let cols = field.cols as usize;
    let rows = field.rows as usize;
    assert_eq!(
        out.len(),
        cols * rows,
        "deviation buffer must be field.cols * field.rows"
    );
    let tol = tol.max(0.0);
    // Clamp the requested rectangle to the grid; bail on an empty/inverted one.
    let ix0 = (ix0 as usize).min(cols);
    let iy0 = (iy0 as usize).min(rows);
    let ix1 = (ix1 as usize).min(cols);
    let iy1 = (iy1 as usize).min(rows);
    if ix0 >= ix1 || iy0 >= iy1 {
        return;
    }
    let top = field.top();
    for iy in iy0..iy1 {
        let cy = field.origin.y + (iy as f64 + 0.5) * field.cell;
        let row = iy * cols;
        for ix in ix0..ix1 {
            let cx = field.origin.x + (ix as f64 + 0.5) * field.cell;
            // Deepest (min-Z) target across every relief covering this cell;
            // the stock top when no surface reaches it.
            let target = surfaces
                .iter()
                .map(|s| s.sample(cx, cy))
                .reduce(f32::min)
                .unwrap_or(SURFACE_TOP_Z);
            // target_world lifts the target's stock-top-relative Z into the
            // simulator's world frame so both sides share a datum.
            let target_world = surface_z0 + target;
            let delta = top[row + ix] - target_world;
            out[row + ix] = if delta > tol {
                Deviation::RestStock as u8
            } else if delta < -tol {
                Deviation::Gouge as u8
            } else {
                Deviation::OnTarget as u8
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-5, "expected {b}, got {a}");
    }

    fn approx_tol(a: f32, b: f32, tol: f32) {
        assert!((a - b).abs() <= tol, "expected {b} ± {tol}, got {a}");
    }

    #[test]
    fn at_reads_cells_row_major_and_clamps_out_of_bounds() {
        // 3x2 grid: z = ix + 10*iy.
        let z = vec![0.0, 1.0, 2.0, 10.0, 11.0, 12.0];
        let f = SurfaceField::new(Point2::new(0.0, 0.0), 1.0, 3, 2, z);
        approx(f.at(0, 0), 0.0);
        approx(f.at(2, 0), 2.0);
        approx(f.at(0, 1), 10.0);
        approx(f.at(2, 1), 12.0);
        // Out of bounds → stock top.
        approx(f.at(3, 0), SURFACE_TOP_Z);
        approx(f.at(0, 2), SURFACE_TOP_Z);
    }

    #[test]
    fn sample_hits_cell_centers_and_interpolates_midpoints() {
        // 2x2 grid, cell 2mm, origin (0,0). Cell centers at 1mm and 3mm.
        let z = vec![0.0, -2.0, -4.0, -6.0];
        let f = SurfaceField::new(Point2::new(0.0, 0.0), 2.0, 2, 2, z);
        // Cell centers reproduce stored values exactly.
        approx(f.sample(1.0, 1.0), 0.0);
        approx(f.sample(3.0, 1.0), -2.0);
        approx(f.sample(1.0, 3.0), -4.0);
        approx(f.sample(3.0, 3.0), -6.0);
        // Midpoint between the 4 centers = mean.
        approx(f.sample(2.0, 2.0), -3.0);
        // Horizontal midpoint of the bottom row.
        approx(f.sample(2.0, 1.0), -1.0);
    }

    #[test]
    fn sample_outside_footprint_returns_stock_top() {
        let z = vec![-5.0; 4];
        let f = SurfaceField::new(Point2::new(0.0, 0.0), 2.0, 2, 2, z);
        // Left/below the first cell center, and past the last.
        approx(f.sample(-1.0, -1.0), SURFACE_TOP_Z);
        approx(f.sample(100.0, 100.0), SURFACE_TOP_Z);
        // NaN guard.
        approx(f.sample(f64::NAN, 1.0), SURFACE_TOP_Z);
    }

    #[test]
    fn from_grayscale_maps_bright_high_dark_low_by_default() {
        // 1x2 column: dark (0.0) then bright (1.0). z in [-5, 0].
        let f = SurfaceField::from_grayscale(
            Point2::new(0.0, 0.0),
            1.0,
            1,
            2,
            &[0.0, 1.0],
            -5.0,
            0.0,
            false,
        );
        approx(f.at(0, 0), -5.0); // dark → deepest
        approx(f.at(0, 1), 0.0); // bright → top
                                 // Mid-grey lands halfway.
        let g = SurfaceField::from_grayscale(
            Point2::new(0.0, 0.0),
            1.0,
            1,
            1,
            &[0.5],
            -5.0,
            0.0,
            false,
        );
        approx(g.at(0, 0), -2.5);
    }

    #[test]
    fn from_grayscale_invert_flips_and_span_order_is_tolerated() {
        // invert: bright → deep.
        let f = SurfaceField::from_grayscale(
            Point2::new(0.0, 0.0),
            1.0,
            2,
            1,
            &[0.0, 1.0],
            -4.0,
            0.0,
            true,
        );
        approx(f.at(0, 0), 0.0); // dark → top (inverted)
        approx(f.at(1, 0), -4.0); // bright → deep (inverted)

        // Passing the span flipped (max first) yields the same mapping as
        // the sorted form: lo is always the deepest.
        let g = SurfaceField::from_grayscale(
            Point2::new(0.0, 0.0),
            1.0,
            2,
            1,
            &[0.0, 1.0],
            0.0,
            -4.0,
            false,
        );
        approx(g.at(0, 0), -4.0);
        approx(g.at(1, 0), 0.0);
    }

    #[test]
    fn from_grayscale_clamps_out_of_range_brightness() {
        let f = SurfaceField::from_grayscale(
            Point2::new(0.0, 0.0),
            1.0,
            2,
            1,
            &[-0.5, 1.5],
            -2.0,
            0.0,
            false,
        );
        approx(f.at(0, 0), -2.0); // clamped to 0 brightness → deep
        approx(f.at(1, 0), 0.0); // clamped to 1 brightness → top
    }

    #[test]
    fn z_range_reports_min_max() {
        let f = SurfaceField::new(
            Point2::new(0.0, 0.0),
            1.0,
            2,
            2,
            vec![-3.0, -1.0, -5.0, 0.0],
        );
        let (lo, hi) = f.z_range();
        approx(lo, -5.0);
        approx(hi, 0.0);
    }

    #[test]
    fn max_corner_helpers() {
        let f = SurfaceField::new(Point2::new(1.0, 2.0), 2.0, 3, 4, vec![0.0; 12]);
        approx((f.max_x() - 7.0) as f32, 0.0); // 1 + 3*2
        approx((f.max_y() - 10.0) as f32, 0.0); // 2 + 4*2
    }

    #[test]
    #[should_panic(expected = "z length must equal")]
    fn new_rejects_mismatched_z_length() {
        let _ = SurfaceField::new(Point2::new(0.0, 0.0), 1.0, 2, 2, vec![0.0; 3]);
    }

    // ---- from_mesh / from_stl (STL rasterizer) ------------------------

    /// Two triangles tiling a `[0,10]²` square at a constant height rasterize
    /// to a uniform field pinned at the stock top (0): a flat top means no
    /// relief to cut.
    #[test]
    fn from_mesh_flat_quad_is_uniform_stock_top() {
        let tris = vec![
            [[0.0, 0.0, 4.0], [10.0, 0.0, 4.0], [0.0, 10.0, 4.0]],
            [[10.0, 0.0, 4.0], [10.0, 10.0, 4.0], [0.0, 10.0, 4.0]],
        ];
        let f = SurfaceField::from_mesh(&tris, 1.0).expect("has footprint");
        assert_eq!((f.cols, f.rows), (10, 10));
        for &v in &f.z {
            approx(v, SURFACE_TOP_Z);
        }
    }

    /// A tilted plane `z = y` reproduces its shape: every sampled cell center
    /// matches the plane, shifted so the highest point sits at stock top.
    #[test]
    fn from_mesh_ramp_reproduces_plane_shifted_to_top() {
        // z == y over [0,10]².
        let tris = vec![
            [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 10.0]],
            [[10.0, 0.0, 0.0], [10.0, 10.0, 10.0], [0.0, 10.0, 10.0]],
        ];
        let f = SurfaceField::from_mesh(&tris, 1.0).expect("has footprint");
        // Highest cell center is at y = 9.5, so the field shifts by -9.5.
        for iy in 0..f.rows {
            for ix in 0..f.cols {
                let px = 0.5 + f64::from(ix);
                let py = 0.5 + f64::from(iy);
                let expected = (py - 9.5) as f32;
                approx(f.sample(px, py), expected);
            }
        }
        // The top row lands exactly on the stock top.
        approx(f.sample(5.5, 9.5), SURFACE_TOP_Z);
    }

    /// A gap between two disjoint triangles leaves interior cells uncovered,
    /// which read as stock top (no relief) rather than garbage.
    #[test]
    fn from_mesh_uncovered_cells_are_stock_top() {
        // Two flat patches at z = 0, separated by a bare strip in x ∈ (3, 7).
        let tris = vec![
            [[0.0, 0.0, 0.0], [3.0, 0.0, 0.0], [0.0, 4.0, 0.0]],
            [[3.0, 0.0, 0.0], [3.0, 4.0, 0.0], [0.0, 4.0, 0.0]],
            [[7.0, 0.0, 0.0], [10.0, 0.0, 0.0], [7.0, 4.0, 0.0]],
            [[10.0, 0.0, 0.0], [10.0, 4.0, 0.0], [7.0, 4.0, 0.0]],
        ];
        let f = SurfaceField::from_mesh(&tris, 1.0).expect("has footprint");
        // A cell center in the bare strip (x = 5.5) is uncovered → stock top.
        approx(f.sample(5.5, 2.0), SURFACE_TOP_Z);
        // A covered cell is also at the top here (flat mesh).
        approx(f.sample(1.5, 2.0), SURFACE_TOP_Z);
    }

    /// Empty and edge-on (zero XY footprint) meshes have no surface.
    #[test]
    fn from_mesh_degenerate_meshes_return_none() {
        assert!(SurfaceField::from_mesh(&[], 1.0).is_none());
        // A vertical triangle in the x-z plane: zero extent in y.
        let vertical = vec![[[0.0, 5.0, 0.0], [10.0, 5.0, 0.0], [5.0, 5.0, 8.0]]];
        assert!(SurfaceField::from_mesh(&vertical, 1.0).is_none());
    }

    /// End-to-end through the ASCII path: bytes → parse → rasterize.
    #[test]
    fn from_stl_ascii_end_to_end() {
        let ascii = "solid s\n\
             facet normal 0 0 1 outer loop \
               vertex 0 0 2 vertex 4 0 2 vertex 0 4 2 endloop endfacet\n\
             facet normal 0 0 1 outer loop \
               vertex 4 0 2 vertex 4 4 2 vertex 0 4 2 endloop endfacet\n\
             endsolid s";
        let f = SurfaceField::from_stl(ascii.as_bytes(), 1.0)
            .expect("valid STL")
            .expect("has footprint");
        assert_eq!((f.cols, f.rows), (4, 4));
        // Flat mesh → whole field at stock top.
        for &v in &f.z {
            approx(v, SURFACE_TOP_Z);
        }
    }

    /// `from_stl_capped` derives the cell from a max-dimension budget: a
    /// 12×4 mm mesh at `max_dim = 6` picks `cell = 12 / 6 = 2 mm`, giving a
    /// 6×2 grid. Degenerate meshes still yield `None`.
    #[test]
    fn from_stl_capped_sizes_grid_from_max_dim() {
        // A flat 12 (x) by 4 (y) quad at z = 3.
        let ascii = "solid s\n\
             facet normal 0 0 1 outer loop \
               vertex 0 0 3 vertex 12 0 3 vertex 0 4 3 endloop endfacet\n\
             facet normal 0 0 1 outer loop \
               vertex 12 0 3 vertex 12 4 3 vertex 0 4 3 endloop endfacet\n\
             endsolid s";
        let f = SurfaceField::from_stl_capped(ascii.as_bytes(), 6)
            .expect("valid STL")
            .expect("has footprint");
        // Longer side (12) / 6 = 2 mm cell ⇒ 6 cols, 2 rows.
        assert!((f.cell - 2.0).abs() < 1e-9, "cell {} != 2", f.cell);
        assert_eq!((f.cols, f.rows), (6, 2));
        // max_dim is floored at 1 (no divide-by-zero panic). The resulting
        // cell equals the longer extent, so a thin mesh's lone cell center
        // may miss it and yield None — we only assert it returns cleanly and,
        // when a grid comes back, that its dims are valid.
        if let Some(g) = SurfaceField::from_stl_capped(ascii.as_bytes(), 0).expect("valid STL") {
            assert!(g.cols >= 1 && g.rows >= 1);
        }
        // A vertical (zero-XY-footprint) mesh has no surface.
        let vertical = "solid v facet normal 1 0 0 outer loop \
               vertex 5 0 0 vertex 5 0 8 vertex 5 4 0 endloop endfacet endsolid v";
        assert!(SurfaceField::from_stl_capped(vertical.as_bytes(), 8)
            .expect("valid STL")
            .is_none());
    }

    /// The acceptance case: a real binary STL emitted by the heightmap
    /// exporter parses and rasterizes back to the ORIGINAL top shape (within
    /// cell tolerance). Sampled on interior cells, the reconstructed relief
    /// matches the source heightmap's relative depths.
    #[test]
    fn from_stl_binary_roundtrips_a_heightmap_shape() {
        use crate::sim::heightmap::Heightmap;
        use crate::sim::stl::heightmap_to_stl_binary;

        // 12×12 heightmap, cell 1mm, a linear ramp top = -x (so a plane the
        // interpolated export reproduces exactly). Stock bottom well below.
        let cols = 12u32;
        let rows = 12u32;
        let mut hm = Heightmap::new(Point2::new(0.0, 0.0), 1.0, cols, rows, 0.0);
        for iy in 0..rows {
            for ix in 0..cols {
                let idx = (iy * cols + ix) as usize;
                hm.data[idx] = -(f64::from(ix) as f32); // top ramps 0 → -11 in x
            }
        }
        let bytes = heightmap_to_stl_binary(&hm, -20.0);

        let f = SurfaceField::from_stl(&bytes, 1.0)
            .expect("valid STL")
            .expect("has footprint");

        // Compare RELATIVE depth between two interior columns (avoid the
        // outer half-cell wall ring, and the export's global shift-to-top).
        // Heightmap top drops by 1mm per +1 in x, so between x≈3.5 and x≈8.5
        // the surface should drop ~5mm.
        let z_lo = f.sample(3.5, 5.5);
        let z_hi = f.sample(8.5, 5.5);
        approx_tol(z_lo - z_hi, 5.0, 0.25);
        // Constant along y at fixed x (a ramp in x only).
        approx_tol(f.sample(5.5, 3.5) - f.sample(5.5, 8.5), 0.0, 0.25);
    }

    /// A drop-cutter finishing pass over a `from_mesh` field never gouges:
    /// every emitted tip Z stays at or above the target surface it samples.
    #[test]
    #[allow(clippy::many_single_char_names)] // a/b/c/d are triangle vertices
    fn surface_mill_over_from_mesh_is_gouge_free() {
        use crate::cam::surface_mill::{surface_mill, ScanDirection, SurfaceMillParams};

        // A shallow dome: z peaks at the center, falls toward the edges.
        // Build it as a fan of triangles over a 20×20 grid.
        let n = 21usize;
        let span = 20.0f64;
        let vert = |i: usize, j: usize| -> [f32; 3] {
            let x = (i as f64) / (n as f64 - 1.0) * span;
            let y = (j as f64) / (n as f64 - 1.0) * span;
            // Dome: 0 at rim, ~ -0 at center → use downward relief.
            let r2 = ((x - 10.0).powi(2) + (y - 10.0).powi(2)) / 100.0;
            let z = -(r2 * 5.0); // center 0, rim about -10
            [x as f32, y as f32, z as f32]
        };
        let mut tris = Vec::new();
        for j in 0..n - 1 {
            for i in 0..n - 1 {
                let (a, b, c, d) = (
                    vert(i, j),
                    vert(i + 1, j),
                    vert(i + 1, j + 1),
                    vert(i, j + 1),
                );
                tris.push([a, b, c]);
                tris.push([a, c, d]);
            }
        }
        let field = SurfaceField::from_mesh(&tris, 0.5).expect("has footprint");

        let params = SurfaceMillParams {
            tool_radius_mm: 1.5,
            corner_radius_mm: 1.5, // ball-nose
            scallop_height_mm: 0.05,
            stepover_mm: None,
            along_step_mm: 0.5,
            direction: ScanDirection::AlongX,
            z_floor_mm: -20.0,
            z_top_mm: 0.0,
        };
        let paths = surface_mill(&field, &params);
        assert!(!paths.is_empty(), "expected finishing scanlines");

        // No point of the ball may dip below the target anywhere in its
        // footprint. Checking the tip against the sampled target at its own
        // XY is the necessary local condition the drop-cutter guarantees.
        for line in &paths {
            for &(x, y, z_tip) in line {
                let target = f64::from(field.sample(x, y));
                assert!(
                    z_tip >= target - 1e-3,
                    "gouge at ({x:.2},{y:.2}): tip {z_tip:.4} < target {target:.4}"
                );
            }
        }
    }

    // ---- deviation_of (red/green verify overlay) ----------------------
    // `Deviation`, `DexelField`, `SurfaceField` and `Point2` all arrive via
    // the module's `use super::*;` above.

    /// The hand-checked acceptance case: a 3-cell field where one column is
    /// carved exactly to target (on-target), one is over-cut (gouge), and one
    /// is left uncut (rest stock). All three classes appear at their expected
    /// indices.
    #[test]
    fn deviation_of_classifies_gouge_ontarget_reststock() {
        // 3×1 field, 1mm cells, stock top at z=0, floor well below.
        let mut field = DexelField::new(Point2::new(0.0, 0.0), 1.0, 3, 1, 0.0, -10.0);
        // Target: cut 2mm down in every column. z=0 datum == stock top.
        let target = SurfaceField::new(Point2::new(0.0, 0.0), 1.0, 3, 1, vec![-2.0, -2.0, -2.0]);

        field.lower_at(0, 0, -2.0); // exactly on target
        field.lower_at(1, 0, -3.0); // 1mm below target → gouge
                                    // column 2 stays at 0.0 → 2mm above target → rest stock

        let dev = target.deviation_of(&field, field.top_z, 0.1);
        assert_eq!(
            dev,
            vec![
                Deviation::OnTarget as u8,
                Deviation::Gouge as u8,
                Deviation::RestStock as u8,
            ],
        );
    }

    /// The tolerance band widens what reads as on-target: with `tol = 1.5` the
    /// 1mm over-cut falls inside the band (on-target) while the 2mm of rest
    /// stock still exceeds it.
    #[test]
    fn deviation_of_tolerance_band_absorbs_small_deltas() {
        let mut field = DexelField::new(Point2::new(0.0, 0.0), 1.0, 3, 1, 0.0, -10.0);
        let target = SurfaceField::new(Point2::new(0.0, 0.0), 1.0, 3, 1, vec![-2.0, -2.0, -2.0]);
        field.lower_at(0, 0, -2.0);
        field.lower_at(1, 0, -3.0); // 1mm below → within a 1.5mm band
        let dev = target.deviation_of(&field, field.top_z, 1.5);
        assert_eq!(
            dev,
            vec![
                Deviation::OnTarget as u8,
                Deviation::OnTarget as u8,
                Deviation::RestStock as u8, // 2mm above still exceeds 1.5
            ],
        );
    }

    /// `surface_z0` re-datums the target into the simulator's Z frame: a stock
    /// top at world Z=5 with a target that wants a 2mm cut is on-target when
    /// the carved surface sits at world Z=3.
    #[test]
    fn deviation_of_honors_surface_z0_offset() {
        let mut field = DexelField::new(Point2::new(0.0, 0.0), 1.0, 1, 1, 5.0, -10.0);
        let target = SurfaceField::new(Point2::new(0.0, 0.0), 1.0, 1, 1, vec![-2.0]);
        field.lower_at(0, 0, 3.0); // 2mm below the world-Z=5 stock top
        let dev = target.deviation_of(&field, field.top_z, 0.05);
        assert_eq!(dev, vec![Deviation::OnTarget as u8]);
        // Mis-datuming (surface_z0 = 0) drops the target to world Z=-2 while
        // the carve sits at world Z=3, so the same cut now reads as 5mm of
        // rest stock — proving the offset actually shifts the comparison.
        let dev_wrong = target.deviation_of(&field, 0.0, 0.05);
        assert_eq!(dev_wrong, vec![Deviation::RestStock as u8]);
    }

    /// A negative tolerance is clamped to 0 (exact-match band) rather than
    /// producing an inverted comparison.
    #[test]
    fn deviation_of_clamps_negative_tolerance() {
        let mut field = DexelField::new(Point2::new(0.0, 0.0), 1.0, 1, 1, 0.0, -10.0);
        let target = SurfaceField::new(Point2::new(0.0, 0.0), 1.0, 1, 1, vec![-1.0]);
        field.lower_at(0, 0, -1.0); // exactly on target
        let dev = target.deviation_of(&field, field.top_z, -5.0);
        assert_eq!(dev, vec![Deviation::OnTarget as u8]);
    }

    /// Cells whose centers fall outside the target footprint sample as the
    /// stock top (0): uncut stock beyond the relief reads as on-target, but
    /// carving there (below the implied stock top) shows as a gouge.
    #[test]
    fn deviation_of_outside_target_footprint_is_stock_top() {
        // Field spans x∈[0,3]; target only covers the first cell.
        let mut field = DexelField::new(Point2::new(0.0, 0.0), 1.0, 3, 1, 0.0, -10.0);
        let target = SurfaceField::new(Point2::new(0.0, 0.0), 1.0, 1, 1, vec![-2.0]);
        field.lower_at(2, 0, -1.0); // carve a column with no target coverage
        let dev = target.deviation_of(&field, field.top_z, 0.1);
        // col 0: uncut (0) vs target -2 → 2mm above → rest stock.
        // col 1: uncut, outside footprint → target 0, carved 0 → on target.
        // col 2: carved -1, outside footprint target 0 → gouge.
        assert_eq!(
            dev,
            vec![
                Deviation::RestStock as u8,
                Deviation::OnTarget as u8,
                Deviation::Gouge as u8,
            ],
        );
    }

    // ---- deviation_into (dirty-AABB partial reclassify) ----------------

    /// The partial path only rewrites cells inside the requested rectangle and
    /// leaves the rest of the buffer exactly as it found it — the invariant the
    /// per-frame carve overlay relies on (untouched cells keep their prior,
    /// still-correct class instead of being re-sampled every frame).
    #[test]
    fn deviation_into_touches_only_the_requested_rect() {
        let mut field = DexelField::new(Point2::new(0.0, 0.0), 1.0, 3, 1, 0.0, -10.0);
        let target = SurfaceField::new(Point2::new(0.0, 0.0), 1.0, 3, 1, vec![-2.0, -2.0, -2.0]);
        field.lower_at(0, 0, -2.0); // on target
        field.lower_at(1, 0, -3.0); // gouge
                                    // col 2 uncut → rest stock

        // Seed the buffer with a sentinel that is NOT a real class so any cell
        // the call leaves alone is obvious.
        let mut buf = vec![0xFFu8; 3];
        // Reclassify only the middle column.
        target.deviation_into(&field, field.top_z, 0.1, &mut buf, 1, 0, 2, 1);
        assert_eq!(
            buf,
            vec![0xFF, Deviation::Gouge as u8, 0xFF],
            "only col 1 should have been rewritten"
        );
    }

    /// Reclassifying the whole grid rectangle produces the identical buffer the
    /// full [`SurfaceField::deviation_of`] does — the partial path is a strict
    /// refinement, not a different classifier.
    #[test]
    fn deviation_into_full_rect_matches_deviation_of() {
        let mut field = DexelField::new(Point2::new(-1.0, 2.0), 0.75, 4, 3, 1.0, -8.0);
        let target = SurfaceField::new(
            Point2::new(-1.0, 2.0),
            0.75,
            4,
            3,
            vec![
                -1.5, -0.5, 0.0, -2.0, -1.0, -0.25, -3.0, -0.75, -1.25, -2.5, 0.0, -0.5,
            ],
        );
        // Carve an irregular pattern so all three classes show up.
        field.lower_at(0, 0, -0.5);
        field.lower_at(2, 1, -4.0);
        field.lower_at(3, 2, 0.5);
        field.lower_at(1, 2, -1.0);

        let full = target.deviation_of(&field, field.top_z, 0.1);
        let mut piecewise = vec![Deviation::OnTarget as u8; 4 * 3];
        // Cover the grid as a union of two disjoint rectangles that together
        // tile it — the same way accumulated carve AABBs eventually do.
        target.deviation_into(&field, field.top_z, 0.1, &mut piecewise, 0, 0, 4, 2);
        target.deviation_into(&field, field.top_z, 0.1, &mut piecewise, 0, 2, 4, 3);
        assert_eq!(piecewise, full);
    }

    /// The rectangle is clamped to the grid and an empty/inverted one is a
    /// no-op, so an over-wide or degenerate carve AABB can never panic or write
    /// out of bounds.
    #[test]
    fn deviation_into_clamps_and_ignores_empty_rects() {
        let mut field = DexelField::new(Point2::new(0.0, 0.0), 1.0, 2, 2, 0.0, -10.0);
        let target = SurfaceField::new(Point2::new(0.0, 0.0), 1.0, 2, 2, vec![-1.0; 4]);
        field.lower_at(0, 0, -1.0);
        field.lower_at(1, 1, -1.0);
        let mut buf = vec![0xFFu8; 4];

        // Over-wide rect is clamped to the 2×2 grid (no panic, all reclassified).
        target.deviation_into(&field, field.top_z, 0.1, &mut buf, 0, 0, 99, 99);
        assert!(
            buf.iter().all(|&c| c != 0xFF),
            "clamped rect covered the grid"
        );

        // Empty (ix0 == ix1) and inverted (iy0 > iy1) rects change nothing.
        let snapshot = buf.clone();
        target.deviation_into(&field, field.top_z, 0.1, &mut buf, 1, 0, 1, 2);
        target.deviation_into(&field, field.top_z, 0.1, &mut buf, 0, 2, 2, 1);
        assert_eq!(buf, snapshot, "degenerate rects are no-ops");
    }

    // ---- deviation_union (multi-relief, deepest-cut-wins) --------------

    /// A single-surface union is identical to `deviation_of` — the multi-relief
    /// path is a strict generalization, not a different classifier.
    #[test]
    fn deviation_union_of_single_surface_matches_deviation_of() {
        let mut field = DexelField::new(Point2::new(0.0, 0.0), 1.0, 3, 1, 0.0, -10.0);
        let target = SurfaceField::new(Point2::new(0.0, 0.0), 1.0, 3, 1, vec![-2.0, -2.0, -2.0]);
        field.lower_at(0, 0, -2.0);
        field.lower_at(1, 0, -3.0);
        assert_eq!(
            deviation_union_of(std::slice::from_ref(&target), &field, field.top_z, 0.1),
            target.deviation_of(&field, field.top_z, 0.1),
        );
    }

    /// Two reliefs cutting the same footprint to different depths union to the
    /// DEEPER target per cell (cumulative carving): a cut that gouges past the
    /// shallow relief but stops above the deep one reads as rest stock, because
    /// the deeper relief still wants more material gone there.
    #[test]
    fn deviation_union_takes_the_deepest_target_per_cell() {
        // 2 columns, both covered by both reliefs.
        let mut field = DexelField::new(Point2::new(0.0, 0.0), 1.0, 2, 1, 0.0, -10.0);
        let shallow = SurfaceField::new(Point2::new(0.0, 0.0), 1.0, 2, 1, vec![-1.0, -1.0]);
        let deep = SurfaceField::new(Point2::new(0.0, 0.0), 1.0, 2, 1, vec![-5.0, -5.0]);
        // Col 0 carved to -3: past the shallow (-1) target but 2mm above the
        // deep (-5) union target → rest stock (NOT a gouge).
        field.lower_at(0, 0, -3.0);
        // Col 1 carved to -6: 1mm below the deep union target → gouge.
        field.lower_at(1, 0, -6.0);

        let surfaces = [shallow.clone(), deep.clone()];
        let dev = deviation_union_of(&surfaces, &field, field.top_z, 0.1);
        assert_eq!(
            dev,
            vec![Deviation::RestStock as u8, Deviation::Gouge as u8],
            "union classifies against the deepest (-5) target, not the shallow one"
        );
        // Order-independent: swapping the surfaces yields the same union.
        let swapped = [deep, shallow];
        assert_eq!(dev, deviation_union_of(&swapped, &field, field.top_z, 0.1));
    }

    /// Reliefs over disjoint footprints each govern only the cells they cover;
    /// a cell outside a surface samples as the stock top (0), so it never drags
    /// the union shallower than a relief that does reach it.
    #[test]
    fn deviation_union_respects_disjoint_footprints() {
        // 3 columns; relief A covers col 0, relief B covers col 2.
        let mut field = DexelField::new(Point2::new(0.0, 0.0), 1.0, 3, 1, 0.0, -10.0);
        let a = SurfaceField::new(Point2::new(0.0, 0.0), 1.0, 1, 1, vec![-2.0]);
        let b = SurfaceField::new(Point2::new(2.0, 0.0), 1.0, 1, 1, vec![-4.0]);
        field.lower_at(0, 0, -2.0); // on A's target
        field.lower_at(2, 0, -4.0); // on B's target
                                    // col 1 uncut, covered by neither → stock top → on target
        let dev = deviation_union_of(&[a, b], &field, field.top_z, 0.1);
        assert_eq!(
            dev,
            vec![
                Deviation::OnTarget as u8,
                Deviation::OnTarget as u8,
                Deviation::OnTarget as u8,
            ],
        );
    }

    /// No surfaces → everything reads against the bare stock top: uncut cells
    /// are on-target, any carve is a gouge. (The overlay never calls this — it
    /// clears the target instead — but the degenerate must not panic.)
    #[test]
    fn deviation_union_with_no_surfaces_is_stock_top() {
        let mut field = DexelField::new(Point2::new(0.0, 0.0), 1.0, 2, 1, 0.0, -10.0);
        field.lower_at(1, 0, -1.0);
        let dev = deviation_union_of(&[], &field, field.top_z, 0.1);
        assert_eq!(dev, vec![Deviation::OnTarget as u8, Deviation::Gouge as u8]);
    }
}
