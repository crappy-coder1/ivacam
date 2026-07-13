//! Binary STL export of the carved simulated stock.
//!
//! Two entry points share one serializer:
//! * [`heightmap_to_stl_binary`] meshes a single-valued [`Heightmap`] — the
//!   3-axis top surface plus a straight perimeter skirt.
//! * [`dexel_to_stl_binary`] meshes a [`DexelField`]: the same dense top-
//!   surface shell **plus** the undercut void cavities carried in the sparse
//!   sidecar, so a form-tool's T-slot / dovetail voids survive the export
//!   instead of being silently flattened to the top surface.
//!
//! Both produce a watertight-*ish* triangle mesh so users can inspect / 3-D
//! print the post-cut geometry, and so visual regression tests (e.g. the
//! chamfer cone-below-floor bug) can diff against a reference STL instead of
//! relying on screenshots.
//!
//! ## Mesh shape
//!
//! Top surface — every 2×2 block of sample points becomes two triangles,
//! so the carved heightfield reads as a smooth (interpolated) topographic
//! mesh rather than Minecraft-style voxel boxes. Pairs with how the
//! heightmap's bilinear `sample()` already treats the data as a regular
//! grid of point samples, not unit cells.
//!
//! Perimeter walls — the four edge runs of samples each drop straight
//! down to `stock_bottom_z`. Plus one flat bottom quad. Together this
//! gives a watertight mesh suitable for STL viewers / mesh-compare tools.
//!
//! Undercut voids (dexel only) — each interior void (a gap between two
//! consecutive solid spans of a sidecar column) becomes a box-like cell:
//! a FLOOR (top of the material below, +Z), a CEILING (underside of the
//! overhang above, −Z), and vertical WALLS on each of the four faces over
//! exactly the Z sub-intervals where the neighbour column is solid. This
//! is the voxel-box geometry the 3-D preview's undercut renderer draws
//! (`frontend/src/lib/sim/undercut_mesh.ts`), so the STL matches what the
//! user sees. Openings back to the neck (where a neighbour is void over the
//! same Z) are intentionally left unwalled — they connect to the dense top
//! shell, exactly as in the preview. The interpolated top shell is *not*
//! voxelized, so the seam between it and a voxel void isn't a closed
//! manifold; a fully watertight two-surface merge is out of scope here
//! (`ivac-58nl.6.5.5` and beyond).
//!
//! ## Triangle budget
//!
//! For a `cols × rows` heightmap: `2·(cols-1)·(rows-1)` top triangles,
//! `4·(cols + rows - 2)` perimeter triangles, and 2 bottom triangles. At
//! 50 bytes/tri the binary STL fits in ~1 MB per 200×200 grid. The dexel
//! builder appends void triangles on top of that shell — a few per undercut
//! column, so negligible for a form-tool cavity of a few hundred columns.

use crate::sim::dexel::DexelField;
use crate::sim::heightmap::Heightmap;

/// Float slack for f32 span endpoints — collapses zero-height voids and
/// zero-area wall sub-intervals so no degenerate quad reaches the mesh.
/// Matches the sub-mm scale the sim carves at (and the frontend renderer's
/// `EPS`).
const VOID_EPS: f32 = 1e-5;

/// Serialize the heightmap as a binary STL with a flat bottom plane at
/// `stock_bottom_z`. Returns the raw bytes — callers wire it through the
/// active transport's "save bytes to file" facility.
///
/// `stock_bottom_z` is the absolute Z of the stock's underside (typically
/// `top_z - stock_thickness`). Edge walls of the mesh drop from the
/// height at each perimeter sample down to this plane.
#[must_use]
pub fn heightmap_to_stl_binary(hm: &Heightmap, stock_bottom_z: f32) -> Vec<u8> {
    let mut tris: Vec<[[f32; 3]; 3]> = Vec::new();
    push_shell_tris(
        &mut tris,
        hm.cols,
        hm.rows,
        hm.cell,
        hm.origin.x,
        hm.origin.y,
        &hm.data,
        stock_bottom_z,
    );
    serialize_binary_stl(&tris)
}

/// Serialize a [`DexelField`] as a binary STL: the dense top-surface shell
/// (identical to [`heightmap_to_stl_binary`] over `field.top()`) plus the
/// undercut void cavities from the sparse sidecar.
///
/// For a pure 3-axis job the sidecar is empty, so the output is **byte-for-
/// byte identical** to `heightmap_to_stl_binary` fed the same top surface,
/// grid, and `stock_bottom_z`. Undercut columns append the extra void
/// surfaces (see the module docs).
///
/// `stock_bottom_z` is the underside plane the perimeter skirt drops to —
/// the caller's stock floor. The void geometry uses the field's own span
/// floor (`DexelField::stock_bottom_z`), which the frontend passes as the
/// same value.
#[must_use]
pub fn dexel_to_stl_binary(field: &DexelField, stock_bottom_z: f32) -> Vec<u8> {
    let mut tris: Vec<[[f32; 3]; 3]> = Vec::new();
    push_shell_tris(
        &mut tris,
        field.cols,
        field.rows,
        field.cell,
        field.origin.x,
        field.origin.y,
        field.top(),
        stock_bottom_z,
    );
    push_void_tris(&mut tris, field);
    serialize_binary_stl(&tris)
}

/// Serialize a [`DexelField`] as a binary STL of the **whole solid**, meshed
/// as per-column voxel boxes straight off the sparse spans — the "Path A"
/// watertight export.
///
/// Unlike [`dexel_to_stl_binary`] (the smooth, preview-parity default), this
/// makes **no** interpolated top shell. Every column's solid spans become
/// axis-aligned faces: a `+Z` top and a `−Z` bottom per span, plus vertical
/// walls over exactly the Z where the column is solid and the lateral
/// neighbour is air (walls split at the neighbour's span boundaries so
/// coincident faces share edges). The result encloses the exact carved
/// volume — undercut voids included — with **no** interpenetrating seam. So
/// it is watertight (hole-free, consistently wound) and safe for the boolean
/// / slicer / generic-mesh-viewer consumers that choke on the smooth
/// export's non-manifold ramp-through-void.
///
/// Tradeoffs, by design (Path A vs the deferred smooth-stitched Path B):
/// * The top is **stair-stepped** (flat per cell), not smooth — so for a
///   pure 3-axis job this is **not** byte-identical to
///   [`heightmap_to_stl_binary`]; it is a separate, opt-in export.
/// * It is watertight but **not strictly 2-manifold**: up to four wall faces
///   can meet along a shared vertical voxel edge (a T-junction, not an area
///   gap). Slicers and most boolean engines accept this; a consumer needing
///   a true 2-manifold surface would need Path B.
///
/// The solid's floor is intrinsic — each column's lowest span already bottoms
/// at [`DexelField::stock_bottom_z`] — so there is no separate skirt-floor
/// parameter, and (unlike the smooth skirt) no material is invented under a
/// from-below floating slab.
#[must_use]
pub fn dexel_to_stl_solid_binary(field: &DexelField) -> Vec<u8> {
    let mut tris: Vec<[[f32; 3]; 3]> = Vec::new();
    push_solid_tris(&mut tris, field);
    serialize_binary_stl(&tris)
}

/// Build the dense top surface + perimeter skirt + flat bottom for a
/// single-valued heightfield `data` (`cols * rows`, row-major). Shared by
/// both entry points so the 3-axis shell is bit-identical between them.
// STL is an f32 mesh format; the cast site below downcasts world coords
// (f64) and cell indices (usize) to f32 by design — the truncation /
// precision loss is the format's contract, not a bug. The grid is bounded
// by `Heightmap::MAX_CELLS` (a few M cells), well below f32's 23-bit
// mantissa breakdown.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn push_shell_tris(
    tris: &mut Vec<[[f32; 3]; 3]>,
    cols: u32,
    rows: u32,
    cell: f64,
    origin_x: f64,
    origin_y: f64,
    data: &[f32],
    stock_bottom_z: f32,
) {
    let cols = cols as usize;
    let rows = rows as usize;
    let cell = cell as f32;
    let ox = origin_x as f32;
    let oy = origin_y as f32;

    // Sample point in world coordinates (cell-center-as-grid-vertex).
    let pos = |c: usize, r: usize, z: f32| -> [f32; 3] {
        [ox + c as f32 * cell, oy + r as f32 * cell, z]
    };
    let h = |c: usize, r: usize| -> f32 { data[r * cols + c] };

    // ── Top surface: 2 triangles per 2×2 sample block, CCW from +Z ─────
    for r in 0..rows.saturating_sub(1) {
        for c in 0..cols.saturating_sub(1) {
            let p00 = pos(c, r, h(c, r));
            let p10 = pos(c + 1, r, h(c + 1, r));
            let p01 = pos(c, r + 1, h(c, r + 1));
            let p11 = pos(c + 1, r + 1, h(c + 1, r + 1));
            tris.push([p00, p10, p11]);
            tris.push([p00, p11, p01]);
        }
    }

    // ── Perimeter walls: each edge run drops to stock_bottom_z ────────
    //
    // Winding convention: each tri's vertices listed CCW when viewed from
    // OUTSIDE the volume, so the right-hand-rule normal points outward.

    // South edge (r = 0), outward normal -Y.
    for c in 0..cols.saturating_sub(1) {
        let top0 = pos(c, 0, h(c, 0));
        let top1 = pos(c + 1, 0, h(c + 1, 0));
        let bot0 = pos(c, 0, stock_bottom_z);
        let bot1 = pos(c + 1, 0, stock_bottom_z);
        tris.push([top0, bot0, bot1]);
        tris.push([top0, bot1, top1]);
    }
    // North edge (r = rows-1), outward normal +Y.
    if rows >= 2 {
        let last_r = rows - 1;
        for c in 0..cols.saturating_sub(1) {
            let top0 = pos(c, last_r, h(c, last_r));
            let top1 = pos(c + 1, last_r, h(c + 1, last_r));
            let bot0 = pos(c, last_r, stock_bottom_z);
            let bot1 = pos(c + 1, last_r, stock_bottom_z);
            tris.push([top1, bot1, bot0]);
            tris.push([top1, bot0, top0]);
        }
    }
    // West edge (c = 0), outward normal -X.
    for r in 0..rows.saturating_sub(1) {
        let top0 = pos(0, r, h(0, r));
        let top1 = pos(0, r + 1, h(0, r + 1));
        let bot0 = pos(0, r, stock_bottom_z);
        let bot1 = pos(0, r + 1, stock_bottom_z);
        tris.push([top1, bot1, bot0]);
        tris.push([top1, bot0, top0]);
    }
    // East edge (c = cols-1), outward normal +X.
    if cols >= 2 {
        let last_c = cols - 1;
        for r in 0..rows.saturating_sub(1) {
            let top0 = pos(last_c, r, h(last_c, r));
            let top1 = pos(last_c, r + 1, h(last_c, r + 1));
            let bot0 = pos(last_c, r, stock_bottom_z);
            let bot1 = pos(last_c, r + 1, stock_bottom_z);
            tris.push([top0, bot0, bot1]);
            tris.push([top0, bot1, top1]);
        }
    }
    // Bottom face: one big quad at stock_bottom, outward normal -Z.
    if cols >= 2 && rows >= 2 {
        let bot00 = pos(0, 0, stock_bottom_z);
        let bot10 = pos(cols - 1, 0, stock_bottom_z);
        let bot01 = pos(0, rows - 1, stock_bottom_z);
        let bot11 = pos(cols - 1, rows - 1, stock_bottom_z);
        tris.push([bot00, bot01, bot11]);
        tris.push([bot00, bot11, bot10]);
    }
}

/// Which of a column's four horizontal faces a wall sub-quad sits on. The
/// name is the direction of the SOLID neighbour; the emitted normal points
/// the opposite way (out of the solid, into the void).
#[derive(Copy, Clone)]
enum Face {
    /// +X neighbour solid: wall at `x = xr`, normal −X.
    Px,
    /// −X neighbour solid: wall at `x = xl`, normal +X.
    Nx,
    /// +Y neighbour solid: wall at `y = yt`, normal −Y.
    Py,
    /// −Y neighbour solid: wall at `y = yb`, normal +Y.
    Ny,
}

/// Append the undercut void surfaces (floors, ceilings, walls) from the
/// field's sparse sidecar. Empty for a pure 3-axis job (no undercut
/// columns). Columns are visited in ascending flat-cell-index order so the
/// output is deterministic regardless of `HashMap` iteration order.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn push_void_tris(tris: &mut Vec<[[f32; 3]; 3]>, field: &DexelField) {
    let cols = field.cols as usize;
    let cell = field.cell as f32;
    let ox = field.origin.x as f32;
    let oy = field.origin.y as f32;

    for idx in field.undercut_cells_sorted() {
        let ix = (idx % cols) as u32;
        let iy = (idx / cols) as u32;
        let spans = field.spans_at(ix, iy);
        // Need at least two spans for an interior void; a single-span
        // sidecar column (a from-below floating slab) has no gap to draw.
        if spans.len() < 2 {
            continue;
        }
        let xl = ox + ix as f32 * cell;
        let xr = xl + cell;
        let yb = oy + iy as f32 * cell;
        let yt = yb + cell;

        // Neighbour span lists (resolved once per column, reused for every
        // void). An off-grid neighbour is "open to outside" ⇒ no walls.
        let px = if ix + 1 < field.cols {
            field.spans_at(ix + 1, iy)
        } else {
            Vec::new()
        };
        let nx = if ix > 0 {
            field.spans_at(ix - 1, iy)
        } else {
            Vec::new()
        };
        let py = if iy + 1 < field.rows {
            field.spans_at(ix, iy + 1)
        } else {
            Vec::new()
        };
        let ny = if iy > 0 {
            field.spans_at(ix, iy - 1)
        } else {
            Vec::new()
        };

        for w in spans.windows(2) {
            let vlo = w[0].hi; // top of the lower span = void floor
            let vhi = w[1].lo; // bottom of the upper span = void ceiling
            if vhi - vlo <= VOID_EPS {
                continue;
            }
            // Floor: top face of the material below the void (+Z).
            push_quad(
                tris,
                [xl, yb, vlo],
                [xr, yb, vlo],
                [xr, yt, vlo],
                [xl, yt, vlo],
            );
            // Ceiling: underside of the overhang above the void (−Z).
            push_quad(
                tris,
                [xl, yb, vhi],
                [xl, yt, vhi],
                [xr, yt, vhi],
                [xr, yb, vhi],
            );
            // Walls: only over the Z where the neighbour column is solid.
            emit_face_walls(tris, &px, vlo, vhi, Face::Px, xl, xr, yb, yt);
            emit_face_walls(tris, &nx, vlo, vhi, Face::Nx, xl, xr, yb, yt);
            emit_face_walls(tris, &py, vlo, vhi, Face::Py, xl, xr, yb, yt);
            emit_face_walls(tris, &ny, vlo, vhi, Face::Ny, xl, xr, yb, yt);
        }
    }
}

/// Emit wall quads on one face plane for the parts of the void `[vlo, vhi]`
/// where `neighbour` is solid. Winding is chosen per face so the recomputed
/// STL normal points out of the solid (into the void).
fn emit_face_walls(
    tris: &mut Vec<[[f32; 3]; 3]>,
    neighbour: &[crate::sim::dexel::Span],
    vlo: f32,
    vhi: f32,
    face: Face,
    xl: f32,
    xr: f32,
    yb: f32,
    yt: f32,
) {
    for s in neighbour {
        let wlo = vlo.max(s.lo);
        let whi = vhi.min(s.hi);
        if whi - wlo <= VOID_EPS {
            continue;
        }
        match face {
            Face::Px => push_quad(
                tris,
                [xr, yb, wlo],
                [xr, yb, whi],
                [xr, yt, whi],
                [xr, yt, wlo],
            ),
            Face::Nx => push_quad(
                tris,
                [xl, yb, wlo],
                [xl, yt, wlo],
                [xl, yt, whi],
                [xl, yb, whi],
            ),
            Face::Py => push_quad(
                tris,
                [xl, yt, wlo],
                [xr, yt, wlo],
                [xr, yt, whi],
                [xl, yt, whi],
            ),
            Face::Ny => push_quad(
                tris,
                [xl, yb, wlo],
                [xl, yb, whi],
                [xr, yb, whi],
                [xr, yb, wlo],
            ),
        }
    }
}

/// Which outward face of a solid column a wall sub-quad sits on — named by
/// the direction of the AIR it faces (the opposite of the void [`Face`],
/// whose name is the solid neighbour). Winding puts the recomputed normal
/// outward, out of the solid.
#[derive(Copy, Clone)]
enum SolidFace {
    /// +X neighbour air: wall at `x = xr`, normal +X.
    Px,
    /// −X neighbour air: wall at `x = xl`, normal −X.
    Nx,
    /// +Y neighbour air: wall at `y = yt`, normal +Y.
    Py,
    /// −Y neighbour air: wall at `y = yb`, normal −Y.
    Ny,
}

/// Mesh the whole carved solid as per-column voxel boxes (the Path A
/// watertight export). Each column's spans become a `+Z`/`−Z` face pair plus
/// outward walls over the Z where the column is solid and the lateral
/// neighbour is air. Columns are visited in row-major order so the output is
/// deterministic; fully carved-away columns (no span) contribute nothing.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn push_solid_tris(tris: &mut Vec<[[f32; 3]; 3]>, field: &DexelField) {
    let cols = field.cols;
    let rows = field.rows;
    let cell = field.cell as f32;
    let ox = field.origin.x as f32;
    let oy = field.origin.y as f32;

    for iy in 0..rows {
        for ix in 0..cols {
            let spans = field.spans_at(ix, iy);
            if spans.is_empty() {
                continue;
            }
            let xl = ox + ix as f32 * cell;
            let xr = xl + cell;
            let yb = oy + iy as f32 * cell;
            let yt = yb + cell;

            // Lateral neighbour spans (empty = off-grid ⇒ full outward wall).
            let px = if ix + 1 < cols {
                field.spans_at(ix + 1, iy)
            } else {
                Vec::new()
            };
            let nx = if ix > 0 {
                field.spans_at(ix - 1, iy)
            } else {
                Vec::new()
            };
            let py = if iy + 1 < rows {
                field.spans_at(ix, iy + 1)
            } else {
                Vec::new()
            };
            let ny = if iy > 0 {
                field.spans_at(ix, iy - 1)
            } else {
                Vec::new()
            };

            for s in &spans {
                // Top: +Z face at the span top.
                push_quad(
                    tris,
                    [xl, yb, s.hi],
                    [xr, yb, s.hi],
                    [xr, yt, s.hi],
                    [xl, yt, s.hi],
                );
                // Bottom: −Z face at the span floor.
                push_quad(
                    tris,
                    [xl, yb, s.lo],
                    [xl, yt, s.lo],
                    [xr, yt, s.lo],
                    [xr, yb, s.lo],
                );
                // Outward walls where the neighbour is air over [lo, hi].
                emit_solid_wall(tris, &px, s, SolidFace::Px, xl, xr, yb, yt);
                emit_solid_wall(tris, &nx, s, SolidFace::Nx, xl, xr, yb, yt);
                emit_solid_wall(tris, &py, s, SolidFace::Py, xl, xr, yb, yt);
                emit_solid_wall(tris, &ny, s, SolidFace::Ny, xl, xr, yb, yt);
            }
        }
    }
}

/// Emit outward wall quads on one face for the parts of the solid span
/// `[s.lo, s.hi]` where `neighbour` is **air** — i.e. `span − ⋃ neighbour`.
/// `neighbour` is the sorted, disjoint, ascending span list of the adjacent
/// column (empty for an off-grid neighbour ⇒ the whole span walls). Splitting
/// at the neighbour's span edges is what makes the wall share edges with the
/// neighbour's own faces (no area gap on the shared plane).
fn emit_solid_wall(
    tris: &mut Vec<[[f32; 3]; 3]>,
    neighbour: &[crate::sim::dexel::Span],
    s: &crate::sim::dexel::Span,
    face: SolidFace,
    xl: f32,
    xr: f32,
    yb: f32,
    yt: f32,
) {
    // Sweep [s.lo, s.hi], emitting each gap the neighbour does NOT fill.
    let mut z = s.lo;
    for n in neighbour {
        if n.hi <= z {
            continue; // neighbour span entirely below the cursor
        }
        if n.lo >= s.hi {
            break; // neighbour has passed the top of this span
        }
        let air_hi = n.lo.min(s.hi);
        if air_hi - z > VOID_EPS {
            push_solid_wall_quad(tris, face, z, air_hi, xl, xr, yb, yt);
        }
        z = z.max(n.hi);
        if z >= s.hi {
            break;
        }
    }
    if s.hi - z > VOID_EPS {
        push_solid_wall_quad(tris, face, z, s.hi, xl, xr, yb, yt);
    }
}

/// Emit one outward-facing wall quad spanning `[zlo, zhi]` on the given face.
/// Winding is chosen per face so [`triangle_normal`] recomputes an outward
/// normal (out of the solid, into the neighbouring air).
fn push_solid_wall_quad(
    tris: &mut Vec<[[f32; 3]; 3]>,
    face: SolidFace,
    zlo: f32,
    zhi: f32,
    xl: f32,
    xr: f32,
    yb: f32,
    yt: f32,
) {
    match face {
        // Wall at x = xr, normal +X.
        SolidFace::Px => push_quad(
            tris,
            [xr, yb, zlo],
            [xr, yt, zlo],
            [xr, yt, zhi],
            [xr, yb, zhi],
        ),
        // Wall at x = xl, normal −X.
        SolidFace::Nx => push_quad(
            tris,
            [xl, yb, zlo],
            [xl, yb, zhi],
            [xl, yt, zhi],
            [xl, yt, zlo],
        ),
        // Wall at y = yt, normal +Y.
        SolidFace::Py => push_quad(
            tris,
            [xr, yt, zlo],
            [xl, yt, zlo],
            [xl, yt, zhi],
            [xr, yt, zhi],
        ),
        // Wall at y = yb, normal −Y.
        SolidFace::Ny => push_quad(
            tris,
            [xl, yb, zlo],
            [xr, yb, zlo],
            [xr, yb, zhi],
            [xl, yb, zhi],
        ),
    }
}

/// Emit a planar quad `p0→p1→p2→p3` as two triangles. Vertex order is the
/// caller's responsibility — [`triangle_normal`] recomputes each face's
/// normal from the winding, so the order fixes the outward direction.
#[inline]
fn push_quad(
    tris: &mut Vec<[[f32; 3]; 3]>,
    p0: [f32; 3],
    p1: [f32; 3],
    p2: [f32; 3],
    p3: [f32; 3],
) {
    tris.push([p0, p1, p2]);
    tris.push([p0, p2, p3]);
}

/// Binary STL wire form: 80-byte banner header + u32 triangle count + 50
/// bytes/triangle (a recomputed normal, three vertices, 2 attr bytes).
// `tris.len() as u32` is bounded by the grid cell budget; the void append
// is a small constant per column. Truncation is unreachable in practice.
#[allow(clippy::cast_possible_truncation)]
fn serialize_binary_stl(tris: &[[[f32; 3]; 3]]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(84 + tris.len() * 50);
    let mut header = [0u8; 80];
    let banner = b"ivaCAM simulated stock STL (9c34)";
    header[..banner.len()].copy_from_slice(banner);
    out.extend_from_slice(&header);
    out.extend_from_slice(&(tris.len() as u32).to_le_bytes());
    for tri in tris {
        let n = triangle_normal(tri);
        for f in n {
            out.extend_from_slice(&f.to_le_bytes());
        }
        for v in tri {
            for f in v {
                out.extend_from_slice(&f.to_le_bytes());
            }
        }
        // Attribute byte count — always zero.
        out.extend_from_slice(&[0u8; 2]);
    }
    out
}

/// Right-hand-rule normal of a triangle. Degenerate (zero-area) triangles
/// fall back to +Z so an STL viewer doesn't paint them black.
// Standard vector-math letters (a, b, c vertices; u, v edges;
// n cross product). Renaming would obscure the textbook formula.
#[allow(clippy::many_single_char_names)]
fn triangle_normal(tri: &[[f32; 3]; 3]) -> [f32; 3] {
    let a = tri[0];
    let b = tri[1];
    let c = tri[2];
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let len2 = n[0] * n[0] + n[1] * n[1] + n[2] * n[2];
    if len2 > 1e-18 {
        let inv = 1.0 / len2.sqrt();
        [n[0] * inv, n[1] * inv, n[2] * inv]
    } else {
        [0.0, 0.0, 1.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Point2;
    use crate::sim::dexel::Span;

    /// Triangle count matches the closed-form formula: `2·(cols-1)·(rows-1)`
    /// top tris + `4·(cols + rows - 2)` perimeter tris + 2 bottom tris.
    #[test]
    fn triangle_count_matches_formula() {
        let hm = Heightmap::new(Point2::new(0.0, 0.0), 1.0, 5, 4, 0.0);
        let bytes = heightmap_to_stl_binary(&hm, -10.0);
        // u32 triangle count lives at offset 80 (after the 80-byte header).
        let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
        let expected = 2 * (5 - 1) * (4 - 1) + 4 * (5 + 4 - 2) + 2;
        assert_eq!(count, expected);
        // Header + u32 + 50 bytes per triangle.
        assert_eq!(bytes.len(), 84 + count * 50);
    }

    /// A flat (uncarved) heightmap produces a top surface whose triangles
    /// all face +Z, and a bottom whose triangles face -Z. Walls face
    /// outward (+/- X or Y). Sanity-checks the winding convention.
    #[test]
    fn flat_heightmap_has_axis_aligned_normals() {
        let hm = Heightmap::new(Point2::new(0.0, 0.0), 1.0, 3, 3, 2.5);
        let bytes = heightmap_to_stl_binary(&hm, -1.0);
        let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
        // For each triangle, the normal is at byte offset 84 + i*50.
        let mut up = 0usize;
        let mut down = 0usize;
        let mut sideways = 0usize;
        for i in 0..count {
            let off = 84 + i * 50;
            let nx = f32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
            let ny = f32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap());
            let nz = f32::from_le_bytes(bytes[off + 8..off + 12].try_into().unwrap());
            if (nz - 1.0).abs() < 1e-3 && nx.abs() < 1e-3 && ny.abs() < 1e-3 {
                up += 1;
            } else if (nz + 1.0).abs() < 1e-3 && nx.abs() < 1e-3 && ny.abs() < 1e-3 {
                down += 1;
            } else if nz.abs() < 1e-3 {
                sideways += 1;
            }
        }
        assert_eq!(up, 2 * (3 - 1) * (3 - 1), "top triangles all +Z");
        assert_eq!(down, 2, "exactly two bottom triangles, -Z");
        assert_eq!(sideways, 4 * (3 + 3 - 2), "perimeter walls, normal in XY");
    }

    /// Header begins with our banner and the STL is byte-stable across
    /// repeated calls (no nondeterministic ordering).
    #[test]
    fn header_banner_and_byte_stability() {
        let hm = Heightmap::new(Point2::new(2.0, -3.0), 0.5, 4, 4, 1.0);
        let a = heightmap_to_stl_binary(&hm, 0.0);
        let b = heightmap_to_stl_binary(&hm, 0.0);
        assert_eq!(a, b, "stl bytes must be deterministic");
        assert!(
            a.starts_with(b"ivaCAM simulated stock STL (9c34)"),
            "header banner missing",
        );
    }

    /// A carved cell produces a non-flat top surface — verify the affected
    /// region's Z values appear in the emitted vertex data.
    #[test]
    fn carved_cells_show_up_in_vertex_data() {
        let mut hm = Heightmap::new(Point2::new(0.0, 0.0), 1.0, 3, 3, 0.0);
        // Carve the centre cell down to -2.5 mm. Index is `row*cols+col`
        // with row=1, col=1 on a 3×3 grid.
        hm.data[3 + 1] = -2.5;
        let bytes = heightmap_to_stl_binary(&hm, -10.0);
        let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
        let mut saw_carved_z = false;
        for i in 0..count {
            let off = 84 + i * 50 + 12; // skip 3 f32 normal
            for v in 0..3 {
                let vz = f32::from_le_bytes(
                    bytes[off + v * 12 + 8..off + v * 12 + 12]
                        .try_into()
                        .unwrap(),
                );
                if (vz - (-2.5)).abs() < 1e-3 {
                    saw_carved_z = true;
                }
            }
        }
        assert!(
            saw_carved_z,
            "carved Z (-2.5) should appear in some triangle's vertex list"
        );
    }

    // ─────────────────────────── dexel builder ───────────────────────────

    /// Parse the u32 triangle count out of a binary STL.
    fn tri_count(bytes: &[u8]) -> usize {
        u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize
    }

    /// Read triangle `i`'s normal (nx, ny, nz).
    fn tri_normal(bytes: &[u8], i: usize) -> [f32; 3] {
        let off = 84 + i * 50;
        [
            f32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()),
            f32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap()),
            f32::from_le_bytes(bytes[off + 8..off + 12].try_into().unwrap()),
        ]
    }

    /// Read triangle `i`'s three vertices.
    fn tri_verts(bytes: &[u8], i: usize) -> [[f32; 3]; 3] {
        let base = 84 + i * 50 + 12; // skip 3-f32 normal
        let mut out = [[0.0f32; 3]; 3];
        for (v, slot) in out.iter_mut().enumerate() {
            for (k, coord) in slot.iter_mut().enumerate() {
                let o = base + v * 12 + k * 4;
                *coord = f32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
            }
        }
        out
    }

    /// The load-bearing regression guard: a pure top-down (3-axis) carve
    /// sequence drives `DexelField` and `Heightmap` to a bit-identical top
    /// surface with an empty sidecar, so `dexel_to_stl_binary` must be
    /// **byte-for-byte identical** to `heightmap_to_stl_binary` over the
    /// same grid + `stock_bottom_z`.
    #[test]
    fn dexel_top_down_is_byte_identical_to_heightmap() {
        let origin = Point2::new(-2.0, 1.0);
        let (cell, cols, rows, top_z) = (0.5_f64, 9_u32, 7_u32, 3.0_f32);
        let stock_bottom = -5.0_f32;
        let mut hm = Heightmap::new(origin, cell, cols, rows, top_z);
        let mut df = DexelField::new(origin, cell, cols, rows, top_z, stock_bottom);

        // Deterministic top-down carves into both.
        let mut state: u64 = 0x1234_5678_9abc_def0;
        for _ in 0..4000 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let r = (state >> 33) as u32;
            let ix = r % cols;
            let iy = (r / cols) % rows;
            let z = top_z - (r % 400) as f32 * 0.01;
            hm.lower_at(ix, iy, z);
            df.lower_at(ix, iy, z);
        }
        assert_eq!(df.undercut_columns(), 0, "top-down must not grow a sidecar");

        let from_hm = heightmap_to_stl_binary(&hm, stock_bottom);
        let from_df = dexel_to_stl_binary(&df, stock_bottom);
        assert_eq!(
            from_hm, from_df,
            "dexel STL must be byte-identical to the heightmap STL for 3-axis"
        );
    }

    /// An interior void grows the mesh past the bare shell and its floor /
    /// ceiling / wall surfaces show up with the expected axis-aligned
    /// normals at the void's Z bounds.
    #[test]
    fn undercut_void_meshes_floor_ceiling_and_walls() {
        // 3×3 grid, single interior void carved into the centre column so
        // all four neighbours are solid full-height columns.
        let mut df = DexelField::new(Point2::new(0.0, 0.0), 1.0, 3, 3, 5.0, 0.0);
        df.carve_cell(1, 1, 2.0, 3.0); // centre: [0,2] + [3,5], void [2,3]
        assert_eq!(df.undercut_columns(), 1);

        let shell_only = {
            // Same grid, no void: the shell triangle count to compare against.
            let df2 = DexelField::new(Point2::new(0.0, 0.0), 1.0, 3, 3, 5.0, 0.0);
            tri_count(&dexel_to_stl_binary(&df2, 0.0))
        };
        let bytes = dexel_to_stl_binary(&df, 0.0);
        let count = tri_count(&bytes);
        // Void adds: floor (2) + ceiling (2) + 4 walls (2 each) = 12 tris.
        assert_eq!(
            count,
            shell_only + 12,
            "one fully-enclosed void = floor + ceiling + four walls"
        );

        // Tally the void surfaces by normal, restricted to triangles whose
        // vertices sit inside the centre cell's XY footprint and Z ∈ [2,3].
        let mut floor = 0; // +Z at z=2
        let mut ceiling = 0; // -Z at z=3
        let mut walls = 0; // horizontal normal
        for i in 0..count {
            let n = tri_normal(&bytes, i);
            let vs = tri_verts(&bytes, i);
            let in_centre = vs.iter().all(|v| {
                (1.0..=2.0).contains(&v[0])
                    && (1.0..=2.0).contains(&v[1])
                    && (2.0 - 1e-4..=3.0 + 1e-4).contains(&v[2])
            });
            if !in_centre {
                continue;
            }
            if (n[2] - 1.0).abs() < 1e-3 {
                floor += 1;
            } else if (n[2] + 1.0).abs() < 1e-3 {
                ceiling += 1;
            } else if n[2].abs() < 1e-3 {
                walls += 1;
            }
        }
        assert_eq!(floor, 2, "void floor: two +Z triangles at z=2");
        assert_eq!(ceiling, 2, "void ceiling: two -Z triangles at z=3");
        assert_eq!(walls, 8, "four solid neighbours ⇒ four walls (8 tris)");
    }

    /// Wall emission keys off neighbour solidity: an edge column's outward
    /// face (off-grid neighbour) draws NO wall, while a wall toward a void
    /// neighbour over the same Z is omitted too. Here the centre void of a
    /// 1×3 strip has two off-grid Y faces open and one solid / one void X
    /// neighbour.
    #[test]
    fn void_walls_only_where_neighbour_is_solid() {
        // 3 cols × 1 row strip.
        let mut df = DexelField::new(Point2::new(0.0, 0.0), 1.0, 3, 1, 5.0, 0.0);
        // Column 0: solid full height (untouched).
        // Column 1: interior void [2,3]  → walls we care about.
        // Column 2: void over the SAME Z band [2,3] so its +X wall collapses.
        df.carve_cell(1, 0, 2.0, 3.0);
        df.carve_cell(2, 0, 2.0, 3.0);
        let bytes = dexel_to_stl_binary(&df, 0.0);
        let count = tri_count(&bytes);

        // Restrict to column 1's void walls (x ∈ [1,2], z ∈ [2,3], vertical).
        let mut nx_walls = 0; // toward col 0 (solid) → present
        let mut px_walls = 0; // toward col 2 (void)  → absent
        let mut y_walls = 0; // off-grid ±Y neighbours → absent
        for i in 0..count {
            let n = tri_normal(&bytes, i);
            if n[2].abs() >= 1e-3 {
                continue; // not a vertical wall
            }
            let vs = tri_verts(&bytes, i);
            let z_ok = vs.iter().all(|v| (2.0 - 1e-4..=3.0 + 1e-4).contains(&v[2]));
            let x_ok = vs.iter().all(|v| (1.0..=2.0).contains(&v[0]));
            if !(z_ok && x_ok) {
                continue;
            }
            if n[0].abs() > 0.5 {
                if vs.iter().all(|v| (v[0] - 1.0).abs() < 1e-4) {
                    nx_walls += 1; // plane x=1, wall toward col 0
                } else if vs.iter().all(|v| (v[0] - 2.0).abs() < 1e-4) {
                    px_walls += 1; // plane x=2, wall toward col 2
                }
            } else if n[1].abs() > 0.5 {
                y_walls += 1;
            }
        }
        assert_eq!(nx_walls, 2, "wall toward the solid −X neighbour is drawn");
        assert_eq!(px_walls, 0, "wall toward the void +X neighbour collapses");
        assert_eq!(y_walls, 0, "off-grid ±Y faces draw no wall");
    }

    /// A from-below floating slab (single sidecar span, no interior gap)
    /// adds no void geometry — only interior voids mesh.
    #[test]
    fn floating_slab_adds_no_void_geometry() {
        let mut df = DexelField::new(Point2::new(0.0, 0.0), 1.0, 3, 3, 5.0, 0.0);
        let shell = tri_count(&dexel_to_stl_binary(&df, 0.0));
        df.carve_cell(1, 1, 0.0, 2.0); // [0,5] → [2,5]: floating slab, 1 span
        assert_eq!(df.undercut_columns(), 1);
        let with_slab = tri_count(&dexel_to_stl_binary(&df, 0.0));
        assert_eq!(with_slab, shell, "a single-span sidecar column has no void");
    }

    /// Deterministic across repeated calls even with a populated sidecar
    /// (undercut columns visited in sorted flat-index order).
    #[test]
    fn dexel_stl_is_deterministic_with_undercuts() {
        let mut df = DexelField::new(Point2::new(0.5, -1.0), 0.75, 5, 5, 4.0, -2.0);
        df.carve_cell(3, 1, 1.0, 2.0);
        df.carve_cell(1, 3, 0.5, 1.5);
        df.carve_cell(2, 2, 0.0, 3.0);
        let a = dexel_to_stl_binary(&df, -2.0);
        let b = dexel_to_stl_binary(&df, -2.0);
        assert_eq!(a, b, "dexel STL with undercuts must be deterministic");
    }

    /// The ported wall windings really do point out of the solid: build a
    /// void bordered by a solid neighbour on each side and assert each face
    /// carries the outward normal (−X toward +X neighbour, etc.).
    #[test]
    fn wall_normals_point_out_of_solid() {
        let mut df = DexelField::new(Point2::new(0.0, 0.0), 1.0, 3, 3, 5.0, 0.0);
        df.carve_cell(1, 1, 2.0, 3.0);
        let bytes = dexel_to_stl_binary(&df, 0.0);
        let count = tri_count(&bytes);
        let (mut neg_x, mut pos_x, mut neg_y, mut pos_y) = (0, 0, 0, 0);
        for i in 0..count {
            let n = tri_normal(&bytes, i);
            let vs = tri_verts(&bytes, i);
            // Only vertical walls of the centre cell's void.
            if n[2].abs() >= 1e-3 {
                continue;
            }
            let void_z = vs.iter().all(|v| (2.0 - 1e-4..=3.0 + 1e-4).contains(&v[2]));
            if !void_z {
                continue;
            }
            // Plane x=2 with normal −X = the +X-neighbour wall.
            if (n[0] + 1.0).abs() < 1e-3 && vs.iter().all(|v| (v[0] - 2.0).abs() < 1e-4) {
                neg_x += 1;
            } else if (n[0] - 1.0).abs() < 1e-3 && vs.iter().all(|v| (v[0] - 1.0).abs() < 1e-4) {
                pos_x += 1;
            } else if (n[1] + 1.0).abs() < 1e-3 && vs.iter().all(|v| (v[1] - 2.0).abs() < 1e-4) {
                neg_y += 1;
            } else if (n[1] - 1.0).abs() < 1e-3 && vs.iter().all(|v| (v[1] - 1.0).abs() < 1e-4) {
                pos_y += 1;
            }
        }
        assert_eq!(neg_x, 2, "wall at x=2 faces −X (out of the +X solid)");
        assert_eq!(pos_x, 2, "wall at x=1 faces +X (out of the −X solid)");
        assert_eq!(neg_y, 2, "wall at y=2 faces −Y (out of the +Y solid)");
        assert_eq!(pos_y, 2, "wall at y=1 faces +Y (out of the −Y solid)");
    }

    /// `Span` is used only through the field here; touch it so the import
    /// stays honest if the void path ever stops resolving spans.
    #[test]
    fn spans_at_reports_void() {
        let mut df = DexelField::new(Point2::new(0.0, 0.0), 1.0, 3, 3, 5.0, 0.0);
        df.carve_cell(1, 1, 2.0, 3.0);
        assert_eq!(
            df.spans_at(1, 1),
            vec![Span::new(0.0, 2.0).unwrap(), Span::new(3.0, 5.0).unwrap()]
        );
    }

    // ───────────────────── solid (Path A) builder ─────────────────────

    /// True iff `(px, py)` lies inside triangle `t`'s XY projection (used to
    /// count the horizontal faces a vertical ray pierces). Robust for a point
    /// strictly interior to a cell and off the quad diagonal.
    fn point_in_tri_xy(p: [f32; 2], t: &[[f32; 3]; 3]) -> bool {
        let cross = |a: [f32; 3], b: [f32; 3]| -> f32 {
            (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
        };
        let d1 = cross(t[0], t[1]);
        let d2 = cross(t[1], t[2]);
        let d3 = cross(t[2], t[0]);
        let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
        let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
        !(has_neg && has_pos)
    }

    /// The sorted `(z, sign)` of every horizontal face a vertical ray at
    /// `(px, py)` pierces. `sign` = +1 for a +Z (top) face, −1 for a −Z
    /// (bottom) face. Walls are vertical ⇒ never pierced by a vertical ray, so
    /// this reconstructs the column's span boundaries — a per-column
    /// watertightness probe that tolerates the mesh's non-2-manifold edges.
    fn vertical_crossings(bytes: &[u8], px: f32, py: f32) -> Vec<(f32, i8)> {
        let count = tri_count(bytes);
        let mut out: Vec<(f32, i8)> = Vec::new();
        for i in 0..count {
            let n = tri_normal(bytes, i);
            if n[0].abs() > 1e-3 || n[1].abs() > 1e-3 {
                continue; // not a horizontal face
            }
            let vs = tri_verts(bytes, i);
            if point_in_tri_xy([px, py], &vs) {
                out.push((vs[0][2], if n[2] > 0.0 { 1 } else { -1 }));
            }
        }
        out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        out
    }

    /// Quantized vertex key for edge bookkeeping (mm at 1e-3 resolution).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn qkey(v: [f32; 3]) -> (i32, i32, i32) {
        (
            (v[0] * 1000.0).round() as i32,
            (v[1] * 1000.0).round() as i32,
            (v[2] * 1000.0).round() as i32,
        )
    }

    /// Assert a closed, consistently-wound 2-manifold: every directed edge
    /// a→b is cancelled by an opposite b→a. Only valid where the mesh has NO
    /// T-junctions (a single box, or a flat uniform field); stepped/void
    /// fields are watertight but not 2-manifold, so they use ray probes.
    fn assert_closed_manifold(bytes: &[u8]) {
        use std::collections::HashMap;
        let count = tri_count(bytes);
        let mut edges: HashMap<((i32, i32, i32), (i32, i32, i32)), i32> = HashMap::new();
        for i in 0..count {
            let vs = tri_verts(bytes, i);
            for e in 0..3 {
                let a = qkey(vs[e]);
                let b = qkey(vs[(e + 1) % 3]);
                *edges.entry((a, b)).or_insert(0) += 1;
            }
        }
        for (&(a, b), &c) in &edges {
            let opp = edges.get(&(b, a)).copied().unwrap_or(0);
            assert_eq!(
                c, opp,
                "directed edge {a:?}->{b:?} ({c}) != reverse ({opp})"
            );
        }
    }

    /// A lone uncut column (1×1 grid, all neighbours off-grid) meshes as a
    /// closed six-face box: 12 triangles, one outward normal per axis
    /// direction, and a clean 2-manifold.
    #[test]
    fn solid_single_column_is_closed_box() {
        let df = DexelField::new(Point2::new(0.0, 0.0), 2.0, 1, 1, 4.0, -1.0);
        let bytes = dexel_to_stl_solid_binary(&df);
        assert_eq!(tri_count(&bytes), 12, "top + bottom + 4 walls = 6 quads");
        assert_closed_manifold(&bytes);
        let dirs = [
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
        ];
        for d in dirs {
            let c = (0..tri_count(&bytes))
                .filter(|&i| {
                    let n = tri_normal(&bytes, i);
                    (n[0] - d[0]).abs() < 1e-3
                        && (n[1] - d[1]).abs() < 1e-3
                        && (n[2] - d[2]).abs() < 1e-3
                })
                .count();
            assert_eq!(c, 2, "face {d:?} should be exactly two triangles");
        }
    }

    /// A flat, uncut field has one equal-height span per column ⇒ no interior
    /// walls and coplanar tops/bottoms ⇒ a closed 2-manifold slab (the
    /// T-junction-free case, so the strict directed-edge check applies).
    #[test]
    fn solid_flat_field_is_two_manifold() {
        let df = DexelField::new(Point2::new(1.0, -2.0), 0.5, 4, 3, 2.0, -1.0);
        let bytes = dexel_to_stl_solid_binary(&df);
        assert_closed_manifold(&bytes);
    }

    /// The load-bearing watertightness proof: for every cell, a vertical ray
    /// pierces exactly the top/bottom faces of that column's spans, in order.
    /// Covers 3-axis lowers (single span) and undercut voids (two spans).
    #[test]
    fn solid_ray_parity_reconstructs_spans() {
        let origin = Point2::new(-1.0, 2.0);
        let (cell, cols, rows, top_z, bottom) = (0.5_f64, 4_u32, 4_u32, 6.0_f32, -1.0_f32);
        let mut df = DexelField::new(origin, cell, cols, rows, top_z, bottom);
        df.lower_at(0, 0, 3.0);
        df.lower_at(1, 0, 2.0);
        df.lower_at(3, 3, 0.5);
        df.carve_cell(2, 2, 2.0, 4.0); // spans [-1,2] + [4,6]
        df.carve_cell(1, 2, 1.0, 3.5); // spans [-1,1] + [3.5,6]
        let bytes = dexel_to_stl_solid_binary(&df);

        let cf = cell as f32;
        let (ox, oy) = (origin.x as f32, origin.y as f32);
        for iy in 0..rows {
            for ix in 0..cols {
                // Interior sample, off the quad diagonal (0.37 ≠ 0.63).
                let px = ox + (ix as f32 + 0.37) * cf;
                let py = oy + (iy as f32 + 0.63) * cf;
                let got = vertical_crossings(&bytes, px, py);
                let mut want: Vec<(f32, i8)> = Vec::new();
                for s in df.spans_at(ix, iy) {
                    want.push((s.lo, -1));
                    want.push((s.hi, 1));
                }
                want.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                assert_eq!(got.len(), want.len(), "cell ({ix},{iy}) crossing count");
                for (g, w) in got.iter().zip(want.iter()) {
                    assert!(
                        (g.0 - w.0).abs() < 1e-4 && g.1 == w.1,
                        "cell ({ix},{iy}) crossing {g:?} != {w:?}"
                    );
                }
            }
        }
    }

    /// An interior void is fully sealed: the centre column's ray shows both
    /// spans (so the void is capped floor + ceiling), and each of the four
    /// sides carries a wall over the void band (the neighbours seal it). This
    /// is exactly the watertightness the smooth exporter lacks.
    #[test]
    fn solid_undercut_void_is_sealed() {
        let mut df = DexelField::new(Point2::new(0.0, 0.0), 1.0, 3, 3, 6.0, 0.0);
        df.carve_cell(1, 1, 2.0, 4.0); // centre spans [0,2] + [4,6], void [2,4]
        let bytes = dexel_to_stl_solid_binary(&df);

        // Sample off the cell center (= quad diagonal) so each face counts once.
        let got = vertical_crossings(&bytes, 1.37, 1.63);
        let want = [(0.0f32, -1i8), (2.0, 1), (4.0, -1), (6.0, 1)];
        assert_eq!(got.len(), want.len(), "centre column crossing count");
        for (g, w) in got.iter().zip(want.iter()) {
            assert!(
                (g.0 - w.0).abs() < 1e-4 && g.1 == w.1,
                "centre ray {g:?} != {w:?}"
            );
        }

        let count = tri_count(&bytes);
        let (mut px, mut nx, mut py, mut ny) = (0, 0, 0, 0);
        for i in 0..count {
            let n = tri_normal(&bytes, i);
            if n[2].abs() > 1e-3 {
                continue; // vertical walls only
            }
            let vs = tri_verts(&bytes, i);
            if !vs.iter().all(|v| (2.0 - 1e-4..=4.0 + 1e-4).contains(&v[2])) {
                continue; // only the void band
            }
            if (n[0] + 1.0).abs() < 1e-3 && vs.iter().all(|v| (v[0] - 2.0).abs() < 1e-4) {
                px += 1; // plane x=2, −X (from the +X neighbour)
            } else if (n[0] - 1.0).abs() < 1e-3 && vs.iter().all(|v| (v[0] - 1.0).abs() < 1e-4) {
                nx += 1; // plane x=1, +X (from the −X neighbour)
            } else if (n[1] + 1.0).abs() < 1e-3 && vs.iter().all(|v| (v[1] - 2.0).abs() < 1e-4) {
                py += 1;
            } else if (n[1] - 1.0).abs() < 1e-3 && vs.iter().all(|v| (v[1] - 1.0).abs() < 1e-4) {
                ny += 1;
            }
        }
        assert_eq!(
            (px, nx, py, ny),
            (2, 2, 2, 2),
            "void sealed by a wall on each of the four sides"
        );
    }

    /// A height step emits a wall over the difference, facing the lower side,
    /// with nothing over the shared full-height part.
    #[test]
    fn solid_step_emits_outward_wall() {
        let mut df = DexelField::new(Point2::new(0.0, 0.0), 1.0, 2, 1, 5.0, 0.0);
        df.lower_at(1, 0, 2.0); // col0 [0,5], col1 [0,2]; step over [2,5]
        let bytes = dexel_to_stl_solid_binary(&df);
        let count = tri_count(&bytes);
        let mut wall = 0;
        for i in 0..count {
            let n = tri_normal(&bytes, i);
            let vs = tri_verts(&bytes, i);
            if (n[0] - 1.0).abs() < 1e-3
                && vs.iter().all(|v| (v[0] - 1.0).abs() < 1e-4)
                && vs.iter().all(|v| (2.0 - 1e-4..=5.0 + 1e-4).contains(&v[2]))
            {
                wall += 1;
            }
        }
        assert_eq!(wall, 2, "step wall over [2,5] at x=1 facing +X");
    }

    /// Every span contributes one +Z and one −Z face, so their triangle
    /// counts balance regardless of the carve pattern.
    #[test]
    fn solid_top_bottom_face_counts_balance() {
        let mut df = DexelField::new(Point2::new(0.0, 0.0), 1.0, 4, 4, 5.0, 0.0);
        df.carve_cell(1, 1, 2.0, 3.0);
        df.carve_cell(2, 2, 1.0, 4.0);
        df.lower_at(3, 0, 2.0);
        let bytes = dexel_to_stl_solid_binary(&df);
        let count = tri_count(&bytes);
        let (mut up, mut down) = (0, 0);
        for i in 0..count {
            let n = tri_normal(&bytes, i);
            if (n[2] - 1.0).abs() < 1e-3 {
                up += 1;
            } else if (n[2] + 1.0).abs() < 1e-3 {
                down += 1;
            }
        }
        assert_eq!(up, down, "one +Z top per −Z bottom across all spans");
        assert!(up > 0, "carved field must have top faces");
    }

    /// Deterministic across repeated calls with a populated sidecar.
    #[test]
    fn solid_is_deterministic_with_undercuts() {
        let mut df = DexelField::new(Point2::new(0.5, -1.0), 0.75, 5, 5, 4.0, -2.0);
        df.carve_cell(3, 1, 1.0, 2.0);
        df.carve_cell(1, 3, 0.5, 1.5);
        df.carve_cell(2, 2, 0.0, 3.0);
        let a = dexel_to_stl_solid_binary(&df);
        let b = dexel_to_stl_solid_binary(&df);
        assert_eq!(a, b, "solid STL must be deterministic");
    }

    /// The stair-stepped solid is deliberately NOT byte-identical to the
    /// smooth default, even for a pure 3-axis job — hence a separate fn.
    #[test]
    fn solid_differs_from_smooth_for_3axis() {
        let mut df = DexelField::new(Point2::new(0.0, 0.0), 1.0, 4, 4, 5.0, -2.0);
        df.lower_at(1, 1, 1.0);
        df.lower_at(2, 1, 2.0);
        assert_eq!(df.undercut_columns(), 0, "still a pure 3-axis field");
        let smooth = dexel_to_stl_binary(&df, -2.0);
        let solid = dexel_to_stl_solid_binary(&df);
        assert_ne!(smooth, solid, "stair-stepped solid must differ from smooth");
    }
}
