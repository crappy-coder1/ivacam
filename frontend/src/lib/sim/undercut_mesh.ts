/// Undercut void renderer — the second surface the dense heightfield mesh
/// structurally cannot draw.
///
/// The live sim (landing #5.2) carries undercuts in a sparse CSR sidecar
/// alongside the dense top surface: a column that grew an interior void
/// (a T-slot neck overhang, a dovetail wall) is a sorted, disjoint list of
/// SOLID `[lo, hi]` spans; the gaps between consecutive spans are the voids.
/// `HeightfieldMesh` renders only the single top surface (`top[idx]` = the
/// highest span's `hi`), so those interior voids are invisible in it. This
/// builder consumes the sidecar and emits the extra surfaces that reveal
/// each void: its FLOOR (the top face of the span below), its CEILING (the
/// underside of the overhang above), and the vertical WALLS where the void
/// borders solid neighbour material.
///
/// Decomposed like the scene3d builders (see ../scene3d/builder.ts) and
/// `HeightfieldDriver`: it owns a `THREE.Group`, rebuilds it from a plain
/// typed [`UndercutSnapshot`] via `build()`, and never reads the Svelte rune
/// store — so the geometry math ([`emitUndercutMesh`]) is unit-testable
/// headlessly with hand-authored snapshots. The `HeightfieldDriver` reads
/// the WASM CSR buffers into a snapshot after each carve and drives this.

import * as THREE from 'three';

/// Float slack for f32 span endpoints — collapses zero-height voids and
/// zero-area wall sub-intervals so the rasteriser never sees a degenerate
/// quad. Matches the sub-mm scale the sim carves at.
const EPS = 1e-5;

/// A plain, THREE-free capture of the dexel field's undercut state — the
/// dense top surface plus the CSR sidecar — everything [`emitUndercutMesh`]
/// needs. The `col*`/`span*` arrays mirror the WASM `undercut_*` accessors
/// (see ivac-wasm::sim); the driver may hand in live views into WASM memory
/// (read synchronously) or copies (tests).
export interface UndercutSnapshot {
  cols: number;
  rows: number;
  cellSize: number;
  originX: number;
  originY: number;
  /// Uncut stock surface Z (mirrors the sim's `top_z`). Kept for context;
  /// void bounds come from the span lists, so it isn't read directly.
  topZ: number;
  /// Physical stock floor — the `lo` of a fresh, uncut column's single
  /// implicit span. Used to resolve a NON-undercut neighbour's solidity.
  stockBottomZ: number;
  /// Dense top surface, row-major `cols * rows` (`top[iy * cols + ix]`).
  /// A neighbour absent from the sidecar is solid over
  /// `[stockBottomZ, top[idx]]`; this array supplies that upper bound.
  top: Float32Array;
  /// CSR: flat cell index (`iy * cols + ix`) per undercut column, sorted
  /// ascending. Length == undercut column count.
  colIndex: Uint32Array;
  /// CSR row-pointer, `colIndex.length + 1` entries: column `i`'s spans are
  /// `spans[2*spanOffsets[i] .. 2*spanOffsets[i+1]]`. Leading entry is 0.
  spanOffsets: Uint32Array;
  /// Flat solid spans as consecutive `(lo, hi)` `f32` pairs, sliced per
  /// column by `spanOffsets`.
  spans: Float32Array;
}

/// Non-indexed triangle soup for the undercut voids: flat `xyz` `positions`
/// with matching flat per-vertex `normals` (both `9 * triangles` long).
export interface UndercutMeshData {
  positions: Float32Array;
  normals: Float32Array;
  triangles: number;
}

/// Growable, non-indexed triangle-soup accumulator that writes straight into
/// `Float32Array`s. Replaces the old `number[]`-push-then-convert path: for a
/// large cavity the per-neighbour array allocation and the final `number[] →
/// Float32Array` copy were the emit's dominant GC cost (ivac-58nl.6.5.5), and
/// both are gone here — the only allocations are the doubling grows.
class TriBuf {
  positions: Float32Array;
  normals: Float32Array;
  /// Floats written so far (== `9 · triangles`); both arrays advance together.
  len = 0;

  constructor(capacityFloats = 512) {
    this.positions = new Float32Array(capacityFloats);
    this.normals = new Float32Array(capacityFloats);
  }

  private grow(need: number): void {
    let cap = this.positions.length;
    while (cap < this.len + need) cap *= 2;
    const p = new Float32Array(cap);
    p.set(this.positions);
    this.positions = p;
    const n = new Float32Array(cap);
    n.set(this.normals);
    this.normals = n;
  }

  /// Emit a planar quad `p0→p1→p2→p3` as two triangles (`0,1,2` + `0,2,3`)
  /// sharing the flat `normal`. Winding is chosen per call site to face
  /// `normal`; the mesh renders `DoubleSide` (interior surfaces are viewed
  /// from the void), so the flat normal is what matters — for lighting.
  quad(
    x0: number,
    y0: number,
    z0: number,
    x1: number,
    y1: number,
    z1: number,
    x2: number,
    y2: number,
    z2: number,
    x3: number,
    y3: number,
    z3: number,
    nx: number,
    ny: number,
    nz: number,
  ): void {
    if (this.len + 18 > this.positions.length) this.grow(18);
    const p = this.positions;
    const m = this.normals;
    let o = this.len;
    // Triangle A: p0, p1, p2.
    p[o] = x0;
    p[o + 1] = y0;
    p[o + 2] = z0;
    p[o + 3] = x1;
    p[o + 4] = y1;
    p[o + 5] = z1;
    p[o + 6] = x2;
    p[o + 7] = y2;
    p[o + 8] = z2;
    // Triangle B: p0, p2, p3.
    p[o + 9] = x0;
    p[o + 10] = y0;
    p[o + 11] = z0;
    p[o + 12] = x2;
    p[o + 13] = y2;
    p[o + 14] = z2;
    p[o + 15] = x3;
    p[o + 16] = y3;
    p[o + 17] = z3;
    for (o = this.len; o < this.len + 18; o += 3) {
      m[o] = nx;
      m[o + 1] = ny;
      m[o + 2] = nz;
    }
    this.len += 18;
  }
}

/// Generate the undercut void surfaces from a snapshot. Pure and
/// THREE-free so the span→triangle math is unit-testable without a WebGL
/// context. Empty (no triangles) for a pure 3-axis job — its sidecar is
/// empty, so `colIndex.length == 0`.
///
/// For each undercut column and each void (the gap between a consecutive
/// pair of its solid spans) it emits:
///   * a FLOOR quad at the void's lower Z (top of the material below, +Z),
///   * a CEILING quad at the void's upper Z (underside of the overhang, −Z),
///   * WALL quads on each of the four horizontal faces, over exactly the
///     Z sub-intervals where the neighbouring column is SOLID.
///
/// The wall rule keys off neighbour solidity so the cavity's OUTER walls
/// (against surrounding solid stock) are drawn while the opening back to the
/// neck (where the neighbour is void over the same Z) is left open, and
/// walls interior to a multi-column cavity collapse to nothing. Because a
/// void lies strictly below its column's top surface while the dense mesh's
/// walls live at the top-value step between neighbours, void walls never
/// overlap dense walls in Z — no z-fighting with the untouched heightfield.
export function emitUndercutMesh(snap: UndercutSnapshot): UndercutMeshData {
  const {
    cols,
    rows,
    cellSize,
    originX,
    originY,
    stockBottomZ,
    top,
    colIndex,
    spanOffsets,
    spans,
  } = snap;
  const n = colIndex.length;
  if (n === 0) {
    return { positions: new Float32Array(0), normals: new Float32Array(0), triangles: 0 };
  }

  // cellIdx → CSR column index, so a neighbour lookup resolves an undercut
  // column's authoritative span list in O(1).
  const csrByCell = new Map<number, number>();
  for (let i = 0; i < n; i++) csrByCell.set(colIndex[i], i);

  const buf = new TriBuf(Math.max(512, n * 36));

  // Neighbour solid-span lists resolved WITHOUT per-neighbour allocation:
  // for each of the four faces we hold a source `Float32Array`, a start
  // index into it (in floats), and a pair count. An undercut neighbour points
  // straight at the shared `spans` buffer; a plain in-bounds neighbour uses a
  // reused 2-slot scratch holding its implicit `[stockBottomZ, top]` span; an
  // out-of-bounds / carved-through neighbour is zero pairs. Order is
  // [px, nx, py, ny] — matching the original emit order.
  const nbrArr: Float32Array[] = [spans, spans, spans, spans];
  const nbrBase = [0, 0, 0, 0];
  const nbrPairs = [0, 0, 0, 0];
  const scratch = [
    new Float32Array(2),
    new Float32Array(2),
    new Float32Array(2),
    new Float32Array(2),
  ];

  /// Resolve neighbour cell `cellIdx` (or `-1` at a grid edge) into face slot
  /// `k`'s `(nbrArr, nbrBase, nbrPairs)`. Mirrors the old `spansOf`: sidecar
  /// list for an undercut column, implicit `[stockBottomZ, top]` for a plain
  /// column, empty for out-of-bounds or fully carved-through.
  const resolveNbr = (k: number, cellIdx: number): void => {
    if (cellIdx < 0) {
      nbrPairs[k] = 0;
      return;
    }
    const ci = csrByCell.get(cellIdx);
    if (ci !== undefined) {
      nbrArr[k] = spans;
      nbrBase[k] = spanOffsets[ci] * 2;
      nbrPairs[k] = spanOffsets[ci + 1] - spanOffsets[ci];
      return;
    }
    if (cellIdx >= top.length) {
      nbrPairs[k] = 0;
      return;
    }
    const t = top[cellIdx];
    if (t > stockBottomZ + EPS) {
      scratch[k][0] = stockBottomZ;
      scratch[k][1] = t;
      nbrArr[k] = scratch[k];
      nbrBase[k] = 0;
      nbrPairs[k] = 1;
    } else {
      nbrPairs[k] = 0;
    }
  };

  /// Emit wall quads on one face plane for the parts of `[vlo, vhi]` where
  /// face slot `k`'s neighbour is solid. `orient` picks the plane + normal.
  const emitFaceWalls = (
    k: number,
    vlo: number,
    vhi: number,
    orient: 'px' | 'nx' | 'py' | 'ny',
    xL: number,
    xR: number,
    yB: number,
    yT: number,
  ): void => {
    const arr = nbrArr[k];
    const base = nbrBase[k];
    const pairs = nbrPairs[k];
    for (let s = 0; s < pairs; s++) {
      const wlo = Math.max(vlo, arr[base + 2 * s]);
      const whi = Math.min(vhi, arr[base + 2 * s + 1]);
      if (whi - wlo <= EPS) continue;
      switch (orient) {
        case 'px': // +X neighbour: plane x = xR, normal −X (into this cell).
          buf.quad(xR, yB, wlo, xR, yT, wlo, xR, yT, whi, xR, yB, whi, -1, 0, 0);
          break;
        case 'nx': // −X neighbour: plane x = xL, normal +X.
          buf.quad(xL, yB, wlo, xL, yB, whi, xL, yT, whi, xL, yT, wlo, 1, 0, 0);
          break;
        case 'py': // +Y neighbour: plane y = yT, normal −Y.
          buf.quad(xL, yT, wlo, xL, yT, whi, xR, yT, whi, xR, yT, wlo, 0, -1, 0);
          break;
        case 'ny': // −Y neighbour: plane y = yB, normal +Y.
          buf.quad(xL, yB, wlo, xR, yB, wlo, xR, yB, whi, xL, yB, whi, 0, 1, 0);
          break;
      }
    }
  };

  for (let i = 0; i < n; i++) {
    const idx = colIndex[i];
    const start = spanOffsets[i];
    const end = spanOffsets[i + 1];
    // Need at least two spans for an interior void; a single-span column in
    // the sidecar (e.g. a from-below carve, Phase 2) has no gap to draw.
    if (end - start < 2) continue;
    const ix = idx % cols;
    const iy = (idx - ix) / cols;
    const xL = originX + ix * cellSize;
    const xR = xL + cellSize;
    const yB = originY + iy * cellSize;
    const yT = yB + cellSize;
    // Resolve the four neighbours once per column, reused for every void.
    resolveNbr(0, ix + 1 < cols ? iy * cols + (ix + 1) : -1); // px
    resolveNbr(1, ix > 0 ? iy * cols + (ix - 1) : -1); // nx
    resolveNbr(2, iy + 1 < rows ? (iy + 1) * cols + ix : -1); // py
    resolveNbr(3, iy > 0 ? (iy - 1) * cols + ix : -1); // ny

    for (let s = start; s < end - 1; s++) {
      const vlo = spans[2 * s + 1]; // hi of the lower span = void floor
      const vhi = spans[2 * (s + 1)]; // lo of the upper span = void ceiling
      if (vhi - vlo <= EPS) continue;
      // Floor: top face of the material below the void (+Z).
      buf.quad(xL, yB, vlo, xR, yB, vlo, xR, yT, vlo, xL, yT, vlo, 0, 0, 1);
      // Ceiling: underside of the overhang above the void (−Z).
      buf.quad(xL, yB, vhi, xL, yT, vhi, xR, yT, vhi, xR, yB, vhi, 0, 0, -1);
      // Walls: only where the neighbour is solid across the void's Z range.
      emitFaceWalls(0, vlo, vhi, 'px', xL, xR, yB, yT);
      emitFaceWalls(1, vlo, vhi, 'nx', xL, xR, yB, yT);
      emitFaceWalls(2, vlo, vhi, 'py', xL, xR, yB, yT);
      emitFaceWalls(3, vlo, vhi, 'ny', xL, xR, yB, yT);
    }
  }

  return {
    positions: buf.positions.slice(0, buf.len),
    normals: buf.normals.slice(0, buf.len),
    triangles: buf.len / 9,
  };
}

/// Style knobs shared with the dense heightfield mesh so the void surfaces
/// read as the same carved stock material.
export interface UndercutStyle {
  solidColor: string;
  solidOpacity: number;
}

const DEFAULT_STYLE: UndercutStyle = { solidColor: '#b8b8b8', solidOpacity: 1 };

/// Scene builder for the undercut voids. Owns a `THREE.Group` (attached to
/// the parent handed in at construction — the driver passes its own sim
/// group so visibility/teardown follow the dense mesh) and swaps a single
/// `Mesh`'s geometry on each `build()`.
export class UndercutMeshBuilder {
  readonly group: THREE.Group;
  private readonly parent: THREE.Object3D;
  private readonly requestRender: () => void;
  private readonly material: THREE.MeshStandardMaterial;
  private geometry: THREE.BufferGeometry | null = null;
  private mesh: THREE.Mesh | null = null;

  constructor(opts: {
    /// Parent to attach the group to. The driver passes its own sim
    /// `THREE.Group` (an `Object3D`) so preview-mode visibility toggling
    /// and disposal cascade from the dense mesh.
    scene: THREE.Object3D;
    requestRender: () => void;
    style?: UndercutStyle;
  }) {
    this.group = new THREE.Group();
    this.parent = opts.scene;
    this.requestRender = opts.requestRender;
    const style = opts.style ?? DEFAULT_STYLE;
    const transparent = style.solidOpacity < 1;
    this.material = new THREE.MeshStandardMaterial({
      color: new THREE.Color(style.solidColor),
      opacity: style.solidOpacity,
      transparent,
      // Same rule as the dense mesh: translucent stock must NOT write depth
      // or nearer void faces occlude farther ones in emit order.
      depthWrite: !transparent,
      roughness: 0.85,
      metalness: 0.0,
      side: THREE.DoubleSide,
      flatShading: true,
    });
    this.parent.add(this.group);
  }

  /// Rebuild the void geometry from a snapshot. Cheap no-op path for a pure
  /// 3-axis carve (no undercut columns → empty mesh cleared). Recreates the
  /// geometry each call; the undercut set is small so this stays light.
  build(snap: UndercutSnapshot): void {
    const data = emitUndercutMesh(snap);
    if (data.triangles === 0) {
      this.clear();
      return;
    }
    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.BufferAttribute(data.positions, 3));
    geo.setAttribute('normal', new THREE.BufferAttribute(data.normals, 3));
    if (this.mesh) {
      this.group.remove(this.mesh);
      this.geometry?.dispose();
      this.mesh.geometry = geo;
      this.group.add(this.mesh);
    } else {
      this.mesh = new THREE.Mesh(geo, this.material);
      this.group.add(this.mesh);
    }
    this.geometry = geo;
    this.requestRender();
  }

  /// Drop the current void geometry (but keep the group + material so the
  /// next `build()` reuses them). Called on a driver rebuild and whenever
  /// the sidecar goes empty.
  clear(): void {
    if (this.mesh) {
      this.group.remove(this.mesh);
      this.mesh = null;
    }
    if (this.geometry) {
      this.geometry.dispose();
      this.geometry = null;
    }
    this.requestRender();
  }

  /// Show / hide the void surfaces. Driven by the driver's solid-visibility
  /// toggle so they appear only in solid / both preview modes.
  setVisible(visible: boolean): void {
    this.group.visible = visible;
  }

  /// Live-apply color / opacity (shared with the dense mesh). No geometry
  /// rebuild — just material fields.
  setStyle(style: UndercutStyle): void {
    this.material.color.set(style.solidColor);
    const transparent = style.solidOpacity < 1;
    this.material.opacity = style.solidOpacity;
    this.material.transparent = transparent;
    this.material.depthWrite = !transparent;
    this.material.needsUpdate = true;
    this.requestRender();
  }

  /// Full teardown: free geometry + material and detach the group.
  dispose(): void {
    this.clear();
    this.material.dispose();
    this.parent.remove(this.group);
  }
}
