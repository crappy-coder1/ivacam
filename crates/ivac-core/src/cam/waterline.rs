//! Constant-Z ("waterline") slicing of a triangle mesh into closed loops.
//!
//! The geometric core of Z-level 3D roughing (`ivac-58nl.3`): the one 3D
//! strategy ivaCAM lacked. Given the STL triangles (as
//! [`crate::sim::stl::parse_stl`] / [`crate::sim::stl::StlTriangle`] return
//! them) and a horizontal plane `Z = z`, intersect the mesh with the plane and
//! stitch the per-triangle crossing segments into closed XY contour loops. Each
//! loop is the outline of solid material at that height; [`clear_level`] then
//! area-clears inside those loops (nesting outer boundaries vs. holes and
//! raster-filling each via `cam::offsets`). What remains for a full op is the
//! `WaterlineRough` op kind + driver that walks [`z_levels`], slices, clears,
//! and stamps each level's Z into a toolpath.
//!
//! GrblGru's `DoJob3DWaterLine` is the reference flow (loop Z from the top down
//! by the depth step, mesh-plane slice each level, rough each contour as a
//! pocket); this module is the "mesh-plane slice each level" primitive.
//!
//! ## Algorithm
//!
//! Marching-triangles contour extraction. For each triangle we take the signed
//! plane distance of its three vertices and, on each of the three edges whose
//! endpoints straddle the plane, linearly interpolate the crossing point — so a
//! straddling triangle yields exactly one XY segment. A half-open sign
//! convention (a vertex exactly ON the plane counts as *below*) keeps the
//! crossing count even and drops mere vertex touches. Edge endpoints are put in
//! a canonical order before interpolation so the two triangles sharing a mesh
//! edge compute a **bit-identical** crossing point — the segments then stitch
//! into rings by exact quantized-key match, with no distance epsilon.
//!
//! ## Degeneracies
//!
//! Slicing a plane that lands exactly on a vertex, or that is coplanar with a
//! flat top/bottom face, is ambiguous (the plane grazes rather than cuts). Real
//! roughing never needs a level exactly at such a Z, so [`z_levels`] steps
//! *strictly between* the top and the floor; a caller slicing arbitrary Z can
//! nudge by a hair to avoid coincidence. Any chain that fails to close (a
//! non-manifold mesh, or such a grazing plane) is dropped rather than emitted as
//! a bogus open contour.

// f32 mesh coordinates widen to f64 for the slice math; the quantized stitch
// key rounds f64 mm to integer µm. Both are intentional and bounded.
#![allow(clippy::cast_possible_truncation)]

use std::collections::HashMap;

use crate::cam::offsets::{inflate_islands_by_tool_radius, pocket_zigzag};
use crate::geometry::{point_in_polygon, Point2, Segment};

/// A closed cross-section contour at a slice height: an ordered ring of XY
/// points, **implicitly closed** — the last vertex connects back to the first,
/// which is not repeated at the end. Wound in no guaranteed direction; use
/// [`loop_signed_area`] when orientation matters.
pub type Loop = Vec<Point2>;

/// Stitch-match resolution: 1 µm expressed as the reciprocal in mm. Crossing
/// points closer than this collapse to one vertex — far below any CNC
/// tolerance, far above the f64 noise the canonical-order interpolation leaves.
const MICRON: f64 = 1e3;

/// Quantized XY key for matching coincident crossing points. Two points map to
/// the same key iff they agree to the micron.
type QKey = (i64, i64);

fn qkey(p: Point2) -> QKey {
    ((p.x * MICRON).round() as i64, (p.y * MICRON).round() as i64)
}

/// Lexicographic order on a vertex by `(z, x, y)`. Used only to pick a
/// canonical endpoint order per edge so both adjacent triangles interpolate the
/// crossing identically. Mesh coordinates are finite, so the partial order is
/// total here.
fn lex_le(a: [f64; 3], b: [f64; 3]) -> bool {
    // Tuple `<=` is PartialOrd; mesh coords are finite so it's a total order.
    (a[2], a[0], a[1]) <= (b[2], b[0], b[1])
}

/// Interpolate the XY point where edge `a→b` crosses the plane, given the
/// vertices' signed plane distances `da`, `db` (opposite signs guaranteed by
/// the caller, so `da - db` is non-zero). The endpoints are reordered into a
/// canonical (lexicographically smaller-first) order first, so the crossing is
/// identical regardless of which triangle — or which edge direction — asks.
fn edge_crossing(a: [f64; 3], b: [f64; 3], da: f64, db: f64) -> Point2 {
    let (a, b, da, db) = if lex_le(a, b) {
        (a, b, da, db)
    } else {
        (b, a, db, da)
    };
    let t = da / (da - db);
    Point2::new(a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1]))
}

/// The XY segment where one triangle crosses the horizontal plane `z`, or
/// `None` when it doesn't straddle it (wholly above/below, or merely grazing a
/// vertex/edge). A vertex exactly on the plane counts as *below* (half-open), so
/// a triangle only ever yields 0 or 2 crossings; a zero-length touch is dropped.
fn triangle_crossing(tri: &[[f32; 3]; 3], z: f64) -> Option<(Point2, Point2)> {
    let v = [
        [
            f64::from(tri[0][0]),
            f64::from(tri[0][1]),
            f64::from(tri[0][2]),
        ],
        [
            f64::from(tri[1][0]),
            f64::from(tri[1][1]),
            f64::from(tri[1][2]),
        ],
        [
            f64::from(tri[2][0]),
            f64::from(tri[2][1]),
            f64::from(tri[2][2]),
        ],
    ];
    let d = [v[0][2] - z, v[1][2] - z, v[2][2] - z];
    // Half-open: strictly above the plane vs. on-or-below it.
    let above = |i: usize| d[i] > 0.0;

    let mut pts: [Option<Point2>; 2] = [None, None];
    let mut n = 0usize;
    for &(a, b) in &[(0usize, 1usize), (1, 2), (2, 0)] {
        if above(a) != above(b) {
            if n < 2 {
                pts[n] = Some(edge_crossing(v[a], v[b], d[a], d[b]));
            }
            n += 1;
        }
    }
    // A plane cuts a triangle in exactly two edges; any other count is a
    // grazing/degenerate case we skip.
    if n != 2 {
        return None;
    }
    let p0 = pts[0]?;
    let p1 = pts[1]?;
    if qkey(p0) == qkey(p1) {
        return None; // degenerate: the plane only touches a vertex
    }
    Some((p0, p1))
}

/// Stitch a bag of undirected crossing segments into closed rings by walking
/// endpoint adjacency. Each crossing point sits on a shared mesh edge and so is
/// an endpoint of exactly two segments (degree 2) in a clean manifold slice, so
/// the walk follows one chain at a time. Open chains (a dead end — non-manifold
/// input or a grazing plane) are discarded.
fn stitch_segments(segs: &[(Point2, Point2)]) -> Vec<Loop> {
    let mut adj: HashMap<QKey, Vec<usize>> = HashMap::new();
    for (i, &(a, b)) in segs.iter().enumerate() {
        adj.entry(qkey(a)).or_default().push(i);
        adj.entry(qkey(b)).or_default().push(i);
    }

    let mut used = vec![false; segs.len()];
    let mut loops: Vec<Loop> = Vec::new();

    for start in 0..segs.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let (a, b) = segs[start];
        let start_key = qkey(a);
        let mut ring: Loop = vec![a];
        let mut end = b;
        let mut closed = false;

        loop {
            let end_key = qkey(end);
            if end_key == start_key {
                closed = true;
                break;
            }
            ring.push(end);
            // Advance to the one unused segment sharing this endpoint.
            let Some(next) = adj
                .get(&end_key)
                .and_then(|cands| cands.iter().copied().find(|&j| !used[j]))
            else {
                break; // dead end — open chain
            };
            used[next] = true;
            let (na, nb) = segs[next];
            end = if qkey(na) == end_key { nb } else { na };
        }

        // A valid contour is closed and has real area (≥ 3 distinct vertices).
        if closed && ring.len() >= 3 {
            loops.push(ring);
        }
    }
    loops
}

/// Slice a triangle mesh with the horizontal plane `Z = z` into closed XY
/// contour loops — the material outline at that height.
///
/// `tris` are `[v0, v1, v2]` triangles of `[x, y, z]` in mm (Z up), exactly as
/// [`crate::sim::stl::parse_stl`] returns. The output is deterministic for a
/// given triangle order (crossing points are computed in a canonical,
/// direction-independent way). A plane that misses the mesh, or only grazes it,
/// yields an empty `Vec`. See the module docs for the degenerate cases.
#[must_use]
pub fn slice_mesh_at_z(tris: &[[[f32; 3]; 3]], z: f64) -> Vec<Loop> {
    let mut segs: Vec<(Point2, Point2)> = Vec::new();
    for tri in tris {
        if let Some(s) = triangle_crossing(tri, z) {
            segs.push(s);
        }
    }
    stitch_segments(&segs)
}

/// Signed area of a closed loop (shoelace). Positive is counter-clockwise
/// (an outer boundary), negative clockwise (a hole), in the standard
/// screen-Y-up convention. Zero for a degenerate ring of fewer than 3 vertices.
#[must_use]
pub fn loop_signed_area(ring: &[Point2]) -> f64 {
    if ring.len() < 3 {
        return 0.0;
    }
    let mut sum = 0.0;
    for i in 0..ring.len() {
        let a = ring[i];
        let b = ring[(i + 1) % ring.len()];
        sum += a.x * b.y - b.x * a.y;
    }
    sum * 0.5
}

/// The descending list of Z heights to rough at, stepping *strictly between*
/// the stock top and the floor so no plane lands exactly on the top/bottom face
/// (the grazing degeneracy the slicer documents).
///
/// Starting at `top_z - step` the levels descend by `step`; the final level is
/// pinned exactly to `bottom_z` so the deepest pass clears the floor even when
/// the span isn't an integer number of steps. Returns empty when `top_z <=
/// bottom_z` or `step <= 0`. `bottom_z` itself is included (a finishing pass at
/// the floor); `top_z` never is.
#[must_use]
pub fn z_levels(top_z: f64, bottom_z: f64, step: f64) -> Vec<f64> {
    if step <= 0.0 || step.is_nan() || top_z <= bottom_z {
        return Vec::new();
    }
    let mut levels = Vec::new();
    let mut z = top_z - step;
    // Stop a hair above the floor; the exact-floor pass is appended once.
    while z > bottom_z + step * 1e-6 {
        levels.push(z);
        z -= step;
    }
    levels.push(bottom_z);
    levels
}

/// One solid region of a Z-level cross-section: an outer boundary loop with the
/// hole loops cut out of it. Both are raw sliced [`Loop`]s (see [`nest_loops`]);
/// orientation is whatever [`slice_mesh_at_z`] produced.
#[derive(Debug, Clone)]
pub struct LevelRegion {
    /// The solid's outer boundary.
    pub boundary: Loop,
    /// Islands to leave uncut (immediate children of `boundary`). A solid
    /// nested inside one of these holes is a *separate* region, not listed here.
    pub holes: Vec<Loop>,
}

/// Whether `inner`'s representative vertex lies inside polygon `outer`. Slice
/// loops never share vertices, so a vertex of `inner` is a safe probe of
/// containment in another loop.
fn loop_inside(inner: &[Point2], outer: &[Point2]) -> bool {
    inner.len() >= 3 && outer.len() >= 3 && point_in_polygon(outer, inner[0].x, inner[0].y)
}

/// Sort the sliced loops at one Z level into solid regions by even-odd
/// containment. A loop nested inside an *even* number of others is a solid
/// outer boundary; inside an *odd* number, it's a hole belonging to its
/// immediate (innermost) container. A solid nested inside a hole (a boss in a
/// cavity) becomes its own region, so arbitrary nesting resolves correctly.
///
/// The containment family of a planar slice is laminar (loops never cross), so
/// a loop's innermost container is exactly its parent and every hole listed
/// under a region is one containment level deeper than that region's boundary.
#[must_use]
pub fn nest_loops(loops: &[Loop]) -> Vec<LevelRegion> {
    let n = loops.len();
    // depth[i] = how many other loops contain loop i.
    // parent[i] = the innermost such container (max depth), or None at top level.
    let mut depth = vec![0usize; n];
    let mut parent: Vec<Option<usize>> = vec![None; n];
    for i in 0..n {
        depth[i] = (0..n)
            .filter(|&j| j != i && loop_inside(&loops[i], &loops[j]))
            .count();
    }
    // Second pass: parent = the container with the greatest depth (innermost).
    for i in 0..n {
        let mut best: Option<usize> = None;
        let mut best_depth = 0usize;
        for j in 0..n {
            if i == j || !loop_inside(&loops[i], &loops[j]) {
                continue;
            }
            if best.is_none() || depth[j] >= best_depth {
                best = Some(j);
                best_depth = depth[j];
            }
        }
        parent[i] = best;
    }

    // Each even-depth loop is a solid region; its holes are the loops whose
    // immediate parent is it (necessarily one level deeper, i.e. odd depth).
    let mut regions = Vec::new();
    for s in 0..n {
        if depth[s] % 2 != 0 || loops[s].len() < 3 {
            continue;
        }
        let holes = (0..n)
            .filter(|&i| parent[i] == Some(s))
            .map(|i| loops[i].clone())
            .collect();
        regions.push(LevelRegion {
            boundary: loops[s].clone(),
            holes,
        });
    }
    regions
}

/// Area-clear the solid cross-section at one Z level: [`nest_loops`] the sliced
/// contours into regions, then raster-fill each region's boundary with its
/// holes left standing. This is the "rough each contour as a pocket" step of
/// waterline roughing, delegating to the same [`pocket_zigzag`] the 2.5D Pocket
/// op uses (the boundary is inset and the holes inflated by the tool radius, so
/// the cutter centerline keeps its clearance).
///
/// `tool_diameter` drives both the boundary inset and the island inflation;
/// `stride` is the scanline stepover. Returns the cut-move chains for the level
/// (the caller lifts to clearance between chains and stamps the level's Z). An
/// empty slice, sub-tool region, or degenerate stride yields no chains.
#[must_use]
pub fn clear_level(loops: &[Loop], tool_diameter: f64, stride: f64) -> Vec<Vec<Segment>> {
    let tool_r = tool_diameter * 0.5;
    let mut chains = Vec::new();
    for region in nest_loops(loops) {
        let islands = inflate_islands_by_tool_radius(&region.holes, tool_r);
        chains.extend(pocket_zigzag(
            &region.boundary,
            &islands,
            stride,
            tool_diameter,
        ));
    }
    chains
}

/// One clearing chain of a waterline pass: a connected XY cut path at a fixed
/// level height `z`. The tool stays plunged along `path`; the caller lifts to
/// clearance and rapids between successive chains.
#[derive(Debug, Clone)]
pub struct RoughChain {
    /// The absolute Z of this clearing pass (a [`z_levels`] height).
    pub z: f64,
    /// Connected XY cut polyline (successive points are cut moves).
    pub path: Vec<Point2>,
}

/// Flatten a connected chain of [`pocket_zigzag`] line segments into its vertex
/// polyline. Successive segments share endpoints (`seg[i].end == seg[i+1].start`),
/// so the polyline is the first start followed by every segment end.
fn chain_to_polyline(chain: &[Segment]) -> Vec<Point2> {
    let mut pts = Vec::with_capacity(chain.len() + 1);
    if let Some(first) = chain.first() {
        pts.push(first.start);
    }
    for s in chain {
        pts.push(s.end);
    }
    pts
}

/// Assemble the full waterline roughing toolpath over a mesh: for each Z level
/// from the top down ([`z_levels`]), slice the mesh, area-clear inside the
/// resulting contours ([`clear_level`] — "rough each contour as a pocket"), and
/// tag every clearing chain with its level Z.
///
/// The mesh is sliced **in its own coordinates**; `top_z` / `bottom_z` bound the
/// roughed span and `z_step` is the per-level depth (the caller maps the mesh
/// onto the stock datum, as the relief path does). `tool_diameter` and `stride`
/// drive the per-level pocket fill. Chains come back top level first, in cut
/// order within a level; the caller inserts rapids / plunges / leads between
/// them and emits the XYZ moves. Levels that slice to nothing contribute no
/// chains.
///
/// This clears **inside** each sliced contour — the cavity/relief convention
/// GrblGru's `DoJob3DWaterLine` uses (and ivaCAM's existing downward-relief
/// model). Boss roughing (clearing the stock *around* a raised part) would
/// invert the regions against the stock boundary; that's a future option.
#[must_use]
pub fn waterline_rough(
    tris: &[[[f32; 3]; 3]],
    tool_diameter: f64,
    stride: f64,
    top_z: f64,
    bottom_z: f64,
    z_step: f64,
) -> Vec<RoughChain> {
    let mut out = Vec::new();
    for z in z_levels(top_z, bottom_z, z_step) {
        let loops = slice_mesh_at_z(tris, z);
        for chain in clear_level(&loops, tool_diameter, stride) {
            out.push(RoughChain {
                z,
                path: chain_to_polyline(&chain),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Emit the vertical side walls (no caps) of a polygon extruded from
    /// `z_lo` to `z_hi`, as two triangles per edge — enough for slicing, which
    /// only reads the walls a horizontal plane actually crosses.
    #[allow(clippy::many_single_char_names)] // p/q/a/b/c/d are polygon-edge points
    fn prism_walls(poly: &[[f32; 2]], z_lo: f32, z_hi: f32) -> Vec<[[f32; 3]; 3]> {
        let mut tris = Vec::new();
        for i in 0..poly.len() {
            let p = poly[i];
            let q = poly[(i + 1) % poly.len()];
            let a = [p[0], p[1], z_lo];
            let b = [q[0], q[1], z_lo];
            let c = [q[0], q[1], z_hi];
            let d = [p[0], p[1], z_hi];
            tris.push([a, b, c]);
            tris.push([a, c, d]);
        }
        tris
    }

    fn bbox(ring: &[Point2]) -> (f64, f64, f64, f64) {
        let mut mnx = f64::INFINITY;
        let mut mny = f64::INFINITY;
        let mut mxx = f64::NEG_INFINITY;
        let mut mxy = f64::NEG_INFINITY;
        for p in ring {
            mnx = mnx.min(p.x);
            mny = mny.min(p.y);
            mxx = mxx.max(p.x);
            mxy = mxy.max(p.y);
        }
        (mnx, mny, mxx, mxy)
    }

    /// A ring is closed: consecutive vertices (wrapping last→first) are all
    /// within a small gap, and no two adjacent vertices coincide.
    fn assert_closed(ring: &[Point2]) {
        assert!(ring.len() >= 3, "ring too short: {}", ring.len());
        for i in 0..ring.len() {
            let a = ring[i];
            let b = ring[(i + 1) % ring.len()];
            assert!(
                a.distance(b) > 1e-9,
                "duplicate adjacent vertices at {i}: {a:?}"
            );
        }
    }

    /// Slicing a square prism mid-height yields one closed loop whose XY
    /// footprint is the square.
    #[test]
    fn slices_square_prism_to_one_loop() {
        let sq = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let tris = prism_walls(&sq, 0.0, 8.0);
        let loops = slice_mesh_at_z(&tris, 4.0);
        assert_eq!(loops.len(), 1, "one solid outline");
        assert_closed(&loops[0]);
        let (mnx, mny, mxx, mxy) = bbox(&loops[0]);
        assert!(
            (mnx).abs() < 1e-9 && (mny).abs() < 1e-9,
            "min corner at origin"
        );
        assert!((mxx - 10.0).abs() < 1e-9 && (mxy - 10.0).abs() < 1e-9);
        // Area of the 10×10 square, sign aside.
        assert!(
            (loop_signed_area(&loops[0]).abs() - 100.0).abs() < 1e-6,
            "area {}",
            loop_signed_area(&loops[0])
        );
    }

    /// Planes above the top and below the bottom of the prism cut nothing.
    #[test]
    fn planes_outside_the_span_are_empty() {
        let sq = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let tris = prism_walls(&sq, 0.0, 8.0);
        assert!(slice_mesh_at_z(&tris, 12.0).is_empty(), "above the top");
        assert!(slice_mesh_at_z(&tris, -3.0).is_empty(), "below the floor");
    }

    /// A square-tube (annulus) prism — outer wall + inner wall — slices into
    /// two nested loops with opposite winding, the outer enclosing the inner.
    #[test]
    fn slices_annulus_to_two_nested_loops() {
        let outer = [[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]];
        // Inner hole wound the OTHER way so its wall normals face inward; the
        // slicer doesn't care about winding, but this mirrors a real cavity.
        let inner = [[6.0, 6.0], [6.0, 14.0], [14.0, 14.0], [14.0, 6.0]];
        let mut tris = prism_walls(&outer, 0.0, 10.0);
        tris.extend(prism_walls(&inner, 0.0, 10.0));

        let loops = slice_mesh_at_z(&tris, 5.0);
        assert_eq!(loops.len(), 2, "outer + inner contour");
        for l in &loops {
            assert_closed(l);
        }
        // One loop has |area| 400 (outer 20²), the other 64 (inner 8²).
        let mut areas: Vec<f64> = loops.iter().map(|l| loop_signed_area(l).abs()).collect();
        areas.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert!((areas[0] - 64.0).abs() < 1e-6, "inner area {}", areas[0]);
        assert!((areas[1] - 400.0).abs() < 1e-6, "outer area {}", areas[1]);
    }

    /// The slicer output is deterministic across repeated calls (canonical
    /// crossing points + stable segment order).
    #[test]
    fn slicing_is_deterministic() {
        let sq = [[1.0, -2.0], [9.0, -2.0], [9.0, 6.0], [1.0, 6.0]];
        let tris = prism_walls(&sq, -1.0, 5.0);
        let a = slice_mesh_at_z(&tris, 2.0);
        let b = slice_mesh_at_z(&tris, 2.0);
        assert_eq!(a, b);
    }

    /// A triangulated cylinder slices to a loop that approximates its circle:
    /// facet corners sit exactly on radius R, while a wall quad's diagonal
    /// midpoint sags inside by the chord height. Every vertex must land in that
    /// `[R·cos(π/n), R]` band — never outside the circle — and the loop closes.
    #[test]
    fn slices_cylinder_to_a_ring_near_radius() {
        let (r, cx, cy) = (5.0_f32, 3.0_f32, 4.0_f32);
        let n = 48usize;
        let poly: Vec<[f32; 2]> = (0..n)
            .map(|i| {
                let a = std::f32::consts::TAU * (i as f32) / (n as f32);
                [cx + r * a.cos(), cy + r * a.sin()]
            })
            .collect();
        let tris = prism_walls(&poly, 0.0, 6.0);
        let loops = slice_mesh_at_z(&tris, 3.0);
        assert_eq!(loops.len(), 1);
        assert_closed(&loops[0]);
        // Deepest a chord midpoint sags inside: R(1 − cos(π/n)).
        let sag = f64::from(r) * (1.0 - (std::f64::consts::PI / n as f64).cos());
        for p in &loops[0] {
            let dist = (p.x - f64::from(cx)).hypot(p.y - f64::from(cy));
            assert!(
                dist <= f64::from(r) + 1e-6 && dist >= f64::from(r) - sag - 1e-6,
                "vertex off circle: {dist} (R={r}, sag={sag})"
            );
        }
    }

    /// `z_levels` descends strictly inside `(bottom, top)`, pins the last pass
    /// to the floor, and rejects degenerate spans.
    #[test]
    fn z_levels_descend_between_top_and_floor() {
        let levels = z_levels(10.0, 0.0, 3.0);
        // 7, 4, 1, then the pinned floor 0.
        assert_eq!(levels, vec![7.0, 4.0, 1.0, 0.0]);
        // Strictly descending, none at or above the top, last exactly the floor.
        assert!(levels.windows(2).all(|w| w[0] > w[1]));
        assert!(*levels.first().unwrap() < 10.0);
        assert!((*levels.last().unwrap() - 0.0).abs() < 1e-12);

        // An exact multiple still ends on the floor without a duplicate.
        assert_eq!(z_levels(9.0, 0.0, 3.0), vec![6.0, 3.0, 0.0]);

        // Degenerate spans / steps produce nothing.
        assert!(z_levels(0.0, 0.0, 1.0).is_empty());
        assert!(z_levels(5.0, 10.0, 1.0).is_empty());
        assert!(z_levels(10.0, 0.0, 0.0).is_empty());
        assert!(z_levels(10.0, 0.0, -2.0).is_empty());
    }

    // ── nesting + per-level clearing (stage 2) ───────────────────────────

    /// An axis-aligned rectangle loop, CCW.
    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Loop {
        vec![
            Point2::new(x0, y0),
            Point2::new(x1, y0),
            Point2::new(x1, y1),
            Point2::new(x0, y1),
        ]
    }

    /// A lone solid loop nests to one region with no holes.
    #[test]
    fn nest_single_solid_has_no_holes() {
        let regions = nest_loops(&[rect(0.0, 0.0, 10.0, 10.0)]);
        assert_eq!(regions.len(), 1);
        assert!(regions[0].holes.is_empty());
    }

    /// An annulus (outer + one contained loop) nests to one region whose single
    /// hole is the inner loop.
    #[test]
    fn nest_annulus_is_one_region_with_one_hole() {
        let outer = rect(0.0, 0.0, 20.0, 20.0);
        let inner = rect(6.0, 6.0, 14.0, 14.0);
        let regions = nest_loops(&[outer, inner.clone()]);
        assert_eq!(regions.len(), 1, "the outer solid is the only region");
        assert_eq!(regions[0].holes.len(), 1);
        assert_eq!(regions[0].holes[0], inner, "the inner loop is the hole");
    }

    /// Two separated solids nest to two independent hole-free regions,
    /// regardless of the order they're passed.
    #[test]
    fn nest_disjoint_solids_are_two_regions() {
        let a = rect(0.0, 0.0, 5.0, 5.0);
        let b = rect(20.0, 20.0, 25.0, 25.0);
        let regions = nest_loops(&[b, a]);
        assert_eq!(regions.len(), 2);
        assert!(regions.iter().all(|r| r.holes.is_empty()));
    }

    /// A boss inside a cavity (outer solid ⊃ hole ⊃ inner solid) splits into
    /// two regions: the outer keeps only the cavity as its hole, and the boss
    /// is its own hole-free region — even-odd depth, not blind containment.
    #[test]
    fn nest_boss_in_cavity_becomes_its_own_region() {
        let outer = rect(0.0, 0.0, 30.0, 30.0); // depth 0 → solid
        let cavity = rect(6.0, 6.0, 24.0, 24.0); // depth 1 → hole of outer
        let boss = rect(12.0, 12.0, 18.0, 18.0); // depth 2 → solid, own region
        let regions = nest_loops(&[outer, cavity.clone(), boss.clone()]);
        assert_eq!(regions.len(), 2);
        let outer_reg = regions
            .iter()
            .find(|r| r.boundary[0] == Point2::new(0.0, 0.0))
            .unwrap();
        assert_eq!(
            outer_reg.holes,
            vec![cavity],
            "outer's only hole is the cavity"
        );
        let boss_reg = regions.iter().find(|r| r.boundary == boss).unwrap();
        assert!(
            boss_reg.holes.is_empty(),
            "the boss is a solid with no holes"
        );
    }

    /// Clearing a solid square fills it with raster strokes whose every endpoint
    /// stays inside the tool-radius-inset boundary (no wall gouge).
    #[test]
    fn clear_level_fills_solid_square_inside_the_inset() {
        let sq = rect(0.0, 0.0, 20.0, 20.0);
        let (tool_d, stride) = (2.0, 1.0);
        let chains = clear_level(std::slice::from_ref(&sq), tool_d, stride);
        assert!(!chains.is_empty(), "a solid square must produce strokes");
        let r = tool_d * 0.5;
        for chain in &chains {
            for s in chain {
                for p in [s.start, s.end] {
                    assert!(
                        p.x >= r - 1e-6 && p.x <= 20.0 - r + 1e-6,
                        "endpoint x={} outside the inset [{r}, {}]",
                        p.x,
                        20.0 - r
                    );
                    assert!(
                        p.y >= r - 1e-6 && p.y <= 20.0 - r + 1e-6,
                        "endpoint y={} outside the inset",
                        p.y
                    );
                }
            }
        }
    }

    /// Clearing an annulus leaves the hole standing: with the island inflated by
    /// the tool radius, no cut endpoint lands inside the raw hole footprint.
    #[test]
    fn clear_level_leaves_the_hole_standing() {
        let outer = rect(0.0, 0.0, 20.0, 20.0);
        let hole = rect(6.0, 6.0, 14.0, 14.0);
        let chains = clear_level(&[outer, hole], 2.0, 1.0);
        assert!(!chains.is_empty());
        for chain in &chains {
            for s in chain {
                for p in [s.start, s.end] {
                    let inside_hole = p.x > 6.001 && p.x < 13.999 && p.y > 6.001 && p.y < 13.999;
                    assert!(
                        !inside_hole,
                        "cut endpoint gouges the standing island: {p:?}"
                    );
                }
            }
        }
    }

    /// Clearing is deterministic for a fixed input.
    #[test]
    fn clear_level_is_deterministic() {
        let outer = rect(0.0, 0.0, 15.0, 12.0);
        let hole = rect(4.0, 4.0, 9.0, 8.0);
        let a = clear_level(&[outer.clone(), hole.clone()], 2.0, 1.3);
        let b = clear_level(&[outer, hole], 2.0, 1.3);
        assert_eq!(a, b);
    }

    // ── multi-level assembly (waterline_rough) ───────────────────────────

    /// Roughing a box prism emits clearing chains at exactly the z_levels, each
    /// filling the box footprint and staying inside the tool-radius inset.
    #[test]
    fn waterline_rough_emits_chains_per_level() {
        let sq = [[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]];
        let tris = prism_walls(&sq, 0.0, 10.0);
        let (tool_d, stride) = (2.0, 2.0);
        let chains = waterline_rough(&tris, tool_d, stride, 10.0, 0.0, 3.0);
        assert!(!chains.is_empty(), "a box must rough into chains");

        // Every chain sits at one of the expected levels (7, 4, 1, 0).
        let want_levels = z_levels(10.0, 0.0, 3.0);
        for c in &chains {
            assert!(
                want_levels.iter().any(|&z| (z - c.z).abs() < 1e-9),
                "chain z {} is not a slice level",
                c.z
            );
            // And its path stays inside the box's tool-radius inset.
            let r = tool_d * 0.5;
            for p in &c.path {
                assert!(p.x >= r - 1e-6 && p.x <= 20.0 - r + 1e-6, "x {} out", p.x);
                assert!(p.y >= r - 1e-6 && p.y <= 20.0 - r + 1e-6, "y {} out", p.y);
            }
        }
        // Every level that the box spans produced at least one chain.
        for &z in &want_levels {
            assert!(
                chains.iter().any(|c| (c.z - z).abs() < 1e-9),
                "level {z} produced no clearing chain"
            );
        }
    }

    /// A mesh that no level intersects (roughing span entirely below it) yields
    /// no chains rather than panicking.
    #[test]
    fn waterline_rough_below_mesh_is_empty() {
        let sq = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let tris = prism_walls(&sq, 50.0, 60.0);
        // Rough a span well below the prism: no level slices it.
        let chains = waterline_rough(&tris, 2.0, 2.0, 10.0, 0.0, 3.0);
        assert!(chains.is_empty());
    }
}
