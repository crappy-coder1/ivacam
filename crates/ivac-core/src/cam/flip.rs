//! Flip transform for two-sided (flip-stock) machining.
//!
//! When one job cuts both faces of the stock, the back-side operations are
//! authored in the same design frame as the front but physically machined
//! after the stock is turned over about a single axis (see [`FlipAxis`]).
//! This module is the PURE geometry of that turn:
//!
//!   * **XY** — mirror the 2D toolpath about the stock's centre-line for
//!     the chosen axis. Flipping about the *centre* keeps the flipped
//!     footprint coincident with the original, so the machine origin /
//!     fixture doesn't move. Reuses [`mirror_segments_x`] /
//!     [`mirror_segments_y`], which already negate segment bulge so
//!     mirrored arcs stay valid.
//!   * **Z** — re-anchor each height by reflecting it about the stock's
//!     mid-plane. The old top maps to the old bottom and vice-versa, so a
//!     depth measured down from the front top becomes the matching depth
//!     measured down from the now-up back face.
//!
//! This is Phase 1 (rt1.11.2): the transform only, kept pure and fully
//! unit-tested. Applying it to `WorkpieceSide::Back` ops inside the
//! pipeline — plus front/back conflict detection and the dual-surface
//! preview — are later phases.

use super::{mirror_segments_x, mirror_segments_y};
use crate::geometry::{Point2, Segment};
use crate::project::{FlipAxis, StockConfig};

/// Z of the stock's mid-plane (mm): halfway between the top plane
/// (`top_z_mm`) and the bottom plane (`top_z_mm − thickness_mm`). The
/// physical flip mirrors every Z about this plane.
#[must_use]
pub fn stock_mid_z(stock: &StockConfig) -> f64 {
    stock.top_z_mm - stock.thickness_mm / 2.0
}

/// Re-anchor a single Z coordinate for the flipped stock by reflecting it
/// about the stock mid-plane (see [`stock_mid_z`]).
///
/// The old top (`top_z_mm`) maps to the old bottom
/// (`top_z_mm − thickness_mm`) and vice-versa; the mid-plane is fixed.
/// `flip_z` is its own inverse: `flip_z(flip_z(z)) == z`.
#[must_use]
pub fn flip_z(z: f64, stock: &StockConfig) -> f64 {
    2.0 * stock_mid_z(stock) - z
}

/// The XY centre of the stock box — the point the 2D mirror pivots about.
/// Only one coordinate is consumed per axis (`X` uses `y`, `Y` uses `x`),
/// but both are returned so callers needn't special-case.
#[must_use]
fn flip_center(stock: &StockConfig) -> Point2 {
    Point2::new(
        stock.origin[0] + stock.width_mm / 2.0,
        stock.origin[1] + stock.height_mm / 2.0,
    )
}

/// Mirror a single XY point about the stock centre-line for `axis`.
///
/// `FlipAxis::X` preserves X and mirrors Y (the flip line runs parallel to
/// X); `FlipAxis::Y` preserves Y and mirrors X. Used for op geometry and,
/// later, for mirroring dowel-hole positions between the front and back
/// programs. Self-inverse for a fixed `axis`/`stock`.
#[must_use]
pub fn flip_point_xy(p: Point2, axis: FlipAxis, stock: &StockConfig) -> Point2 {
    let c = flip_center(stock);
    match axis {
        FlipAxis::X => Point2::new(p.x, 2.0 * c.y - p.y),
        FlipAxis::Y => Point2::new(2.0 * c.x - p.x, p.y),
    }
}

/// Mirror the 2D geometry of a back-side op in place about the stock
/// centre-line for `axis`. `FlipAxis::X` mirrors Y; `FlipAxis::Y` mirrors
/// X. Bulge is negated by the underlying `mirror_segments_*` so arcs
/// survive the mirror. Applying the same flip twice restores the input.
pub fn flip_segments_xy(segments: &mut [Segment], axis: FlipAxis, stock: &StockConfig) {
    let pivot = flip_center(stock);
    match axis {
        FlipAxis::X => mirror_segments_x(segments, pivot),
        FlipAxis::Y => mirror_segments_y(segments, pivot),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::SegmentKind;
    use std::sync::Arc;

    /// 100 × 50 stock at the origin, 10 mm thick, top at z = 0.
    /// Centre (50, 25); mid-plane z = −5.
    fn stock() -> StockConfig {
        StockConfig {
            origin: [0.0, 0.0],
            width_mm: 100.0,
            height_mm: 50.0,
            thickness_mm: 10.0,
            top_z_mm: 0.0,
            flip: None,
        }
    }

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "expected {b}, got {a}");
    }

    #[test]
    fn x_axis_mirrors_y_preserves_x() {
        // The canonical issue case: (10, 5) → (10, 45) on a 50 mm-tall stock.
        let p = flip_point_xy(Point2::new(10.0, 5.0), FlipAxis::X, &stock());
        approx(p.x, 10.0);
        approx(p.y, 45.0);
    }

    #[test]
    fn y_axis_mirrors_x_preserves_y() {
        let p = flip_point_xy(Point2::new(10.0, 5.0), FlipAxis::Y, &stock());
        approx(p.x, 90.0);
        approx(p.y, 5.0);
    }

    #[test]
    fn centre_line_points_are_fixed() {
        // A point on the Y centre-line (y = 25) is unmoved by an X flip.
        let p = flip_point_xy(Point2::new(37.0, 25.0), FlipAxis::X, &stock());
        approx(p.x, 37.0);
        approx(p.y, 25.0);
        // …and on the X centre-line (x = 50) by a Y flip.
        let q = flip_point_xy(Point2::new(50.0, 12.0), FlipAxis::Y, &stock());
        approx(q.x, 50.0);
        approx(q.y, 12.0);
    }

    #[test]
    fn point_flip_is_self_inverse() {
        let s = stock();
        for axis in [FlipAxis::X, FlipAxis::Y] {
            let p = Point2::new(13.0, 7.0);
            let back = flip_point_xy(flip_point_xy(p, axis, &s), axis, &s);
            approx(back.x, p.x);
            approx(back.y, p.y);
        }
    }

    #[test]
    fn z_reanchor_swaps_top_and_bottom() {
        let s = stock(); // top 0, bottom −10, mid −5
        approx(flip_z(0.0, &s), -10.0); // top → bottom
        approx(flip_z(-10.0, &s), 0.0); // bottom → top
        approx(flip_z(-5.0, &s), -5.0); // mid-plane fixed
        approx(flip_z(-2.0, &s), -8.0); // 2 below top → 2 above bottom
    }

    #[test]
    fn z_reanchor_honours_nonzero_top_z() {
        // Zeroed on the bed: top at +3, bottom at −7, mid at −2.
        let s = StockConfig {
            top_z_mm: 3.0,
            ..stock()
        };
        approx(stock_mid_z(&s), -2.0);
        approx(flip_z(3.0, &s), -7.0); // top → bottom
        approx(flip_z(-7.0, &s), 3.0); // bottom → top
    }

    #[test]
    fn z_flip_is_self_inverse() {
        let s = stock();
        for z in [0.0, -1.5, -5.0, -9.9, 2.0] {
            approx(flip_z(flip_z(z, &s), &s), z);
        }
    }

    fn arc(start: Point2, end: Point2, bulge: f64, center: Point2) -> Segment {
        Segment {
            kind: SegmentKind::Arc,
            start,
            end,
            bulge,
            center: Some(center),
            layer: Arc::from("0"),
            color: 7,
        }
    }

    #[test]
    fn segment_flip_x_mirrors_y_negates_bulge() {
        let mut segs = vec![arc(
            Point2::new(10.0, 5.0),
            Point2::new(20.0, 15.0),
            0.5,
            Point2::new(15.0, 8.0),
        )];
        flip_segments_xy(&mut segs, FlipAxis::X, &stock());
        let s = &segs[0];
        // X preserved, Y mirrored about y = 25.
        approx(s.start.x, 10.0);
        approx(s.start.y, 45.0);
        approx(s.end.x, 20.0);
        approx(s.end.y, 35.0);
        let c = s.center.unwrap();
        approx(c.x, 15.0);
        approx(c.y, 42.0);
        // Bulge negated so the arc bows the correct way in the mirror.
        approx(s.bulge, -0.5);
    }

    #[test]
    fn segment_flip_twice_is_identity() {
        let original = vec![arc(
            Point2::new(10.0, 5.0),
            Point2::new(20.0, 15.0),
            0.5,
            Point2::new(15.0, 8.0),
        )];
        for axis in [FlipAxis::X, FlipAxis::Y] {
            let mut segs = original.clone();
            flip_segments_xy(&mut segs, axis, &stock());
            flip_segments_xy(&mut segs, axis, &stock());
            let (a, b) = (&segs[0], &original[0]);
            approx(a.start.x, b.start.x);
            approx(a.start.y, b.start.y);
            approx(a.end.x, b.end.x);
            approx(a.end.y, b.end.y);
            approx(a.bulge, b.bulge);
        }
    }
}
