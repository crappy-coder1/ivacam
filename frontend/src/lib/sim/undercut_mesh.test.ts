/// Pure-geometry tests for the undercut void mesher. Exercises
/// `emitUndercutMesh` (span lists → triangle soup) with hand-authored
/// snapshots — no WebGL / THREE geometry needed. The `UndercutMeshBuilder`
/// THREE wrapper is covered by the e2e / integration runs.

import { describe, expect, it } from 'vitest';
import { emitUndercutMesh, type UndercutSnapshot } from './undercut_mesh';

interface GridOpts {
  cellSize: number;
  originX: number;
  originY: number;
  topZ: number;
  stockBottomZ: number;
}

const DEFAULT_OPTS: GridOpts = {
  cellSize: 1,
  originX: 0,
  originY: 0,
  topZ: 0,
  stockBottomZ: -10,
};

/// Build a snapshot from a per-cell solid-span map. The dense `top` array is
/// derived from the map (highest `hi`) for undercut cells and taken from
/// `topByCell` for the rest.
function makeSnapshot(
  cols: number,
  rows: number,
  spansByCell: Map<number, Array<[number, number]>>,
  topByCell: Map<number, number>,
  opts: GridOpts = DEFAULT_OPTS,
): UndercutSnapshot {
  const colIndex = [...spansByCell.keys()].sort((a, b) => a - b);
  const spanOffsets: number[] = [0];
  const spans: number[] = [];
  for (const idx of colIndex) {
    const list = spansByCell.get(idx)!;
    for (const [lo, hi] of list) spans.push(lo, hi);
    spanOffsets.push(spanOffsets[spanOffsets.length - 1] + list.length);
  }
  const top = new Float32Array(cols * rows);
  top.fill(opts.stockBottomZ);
  for (const [idx, z] of topByCell) top[idx] = z;
  for (const idx of colIndex) {
    const list = spansByCell.get(idx)!;
    top[idx] = list.reduce((m, [, hi]) => Math.max(m, hi), opts.stockBottomZ);
  }
  return {
    cols,
    rows,
    cellSize: opts.cellSize,
    originX: opts.originX,
    originY: opts.originY,
    topZ: opts.topZ,
    stockBottomZ: opts.stockBottomZ,
    top,
    colIndex: new Uint32Array(colIndex),
    spanOffsets: new Uint32Array(spanOffsets),
    spans: new Float32Array(spans),
  };
}

/// Count vertices whose flat normal matches `[nx, ny, nz]` (within slack).
function countByNormal(normals: Float32Array, nx: number, ny: number, nz: number): number {
  let c = 0;
  for (let i = 0; i < normals.length; i += 3) {
    if (
      Math.abs(normals[i] - nx) < 1e-4 &&
      Math.abs(normals[i + 1] - ny) < 1e-4 &&
      Math.abs(normals[i + 2] - nz) < 1e-4
    ) {
      c++;
    }
  }
  return c;
}

describe('emitUndercutMesh', () => {
  it('emits nothing for an empty sidecar (pure 3-axis job)', () => {
    const snap = makeSnapshot(3, 3, new Map(), new Map());
    const data = emitUndercutMesh(snap);
    expect(data.triangles).toBe(0);
    expect(data.positions.length).toBe(0);
    expect(data.normals.length).toBe(0);
  });

  it('emits nothing for a single-span column (no interior void)', () => {
    // One column in the sidecar but with a single span — no gap to draw.
    const spansByCell = new Map<number, Array<[number, number]>>([[1, [[-10, -3]]]]);
    const snap = makeSnapshot(3, 1, spansByCell, new Map());
    expect(emitUndercutMesh(snap).triangles).toBe(0);
  });

  it('meshes a T-slot wing: floor + ceiling + one outer wall', () => {
    // 3×1 grid. Column 1 is the undercut wing:
    //   solid [-10,-5] (base) and [-2,0] (roof) → void [-5,-2].
    // Column 0 (left) is full-height solid [-10,0] → walls the void's −X face.
    // Column 2 (right) is solid only to -5 → NOT solid over the void, so the
    // void opens toward it (the neck) — no +X wall.
    const spansByCell = new Map<number, Array<[number, number]>>([
      [
        1,
        [
          [-10, -5],
          [-2, 0],
        ],
      ],
    ]);
    const topByCell = new Map<number, number>([
      [0, 0],
      [2, -5],
    ]);
    const snap = makeSnapshot(3, 1, spansByCell, topByCell);
    const data = emitUndercutMesh(snap);

    // Floor (2) + ceiling (2) + one −X wall (2) = 6 triangles.
    expect(data.triangles).toBe(6);
    // 2 tris each = 6 verts per face group.
    expect(countByNormal(data.normals, 0, 0, 1)).toBe(6); // floor, +Z
    expect(countByNormal(data.normals, 0, 0, -1)).toBe(6); // ceiling, −Z
    expect(countByNormal(data.normals, 1, 0, 0)).toBe(6); // −X wall faces +X
    // No +X wall (neighbour not solid over the void).
    expect(countByNormal(data.normals, -1, 0, 0)).toBe(0);

    // Floor sits at z = -5, ceiling at z = -2.
    const floorZs = new Set<number>();
    const ceilZs = new Set<number>();
    for (let i = 0; i < data.positions.length; i += 3) {
      const nz = data.normals[i + 2];
      if (Math.abs(nz - 1) < 1e-4) floorZs.add(Math.round(data.positions[i + 2]));
      if (Math.abs(nz + 1) < 1e-4) ceilZs.add(Math.round(data.positions[i + 2]));
    }
    expect([...floorZs]).toEqual([-5]);
    expect([...ceilZs]).toEqual([-2]);

    // The −X wall lives on the shared face x = 1 (column 1's left edge).
    for (let i = 0; i < data.positions.length; i += 3) {
      if (Math.abs(data.normals[i] - 1) < 1e-4) {
        expect(data.positions[i]).toBeCloseTo(1, 5);
      }
    }
  });

  it('suppresses the wall between two adjacent undercut columns sharing a void', () => {
    // 4×1 grid. Columns 1 and 2 are both undercut wings with the same void
    // [-5,-2]; columns 0 and 3 are full-height solid. The internal 1↔2 face
    // is void-on-both-sides → no wall; only the two OUTER walls (1's −X vs
    // col0, 2's +X vs col3) are emitted.
    const wing: Array<[number, number]> = [
      [-10, -5],
      [-2, 0],
    ];
    const spansByCell = new Map<number, Array<[number, number]>>([
      [1, wing],
      [2, wing],
    ]);
    const topByCell = new Map<number, number>([
      [0, 0],
      [3, 0],
    ]);
    const snap = makeSnapshot(4, 1, spansByCell, topByCell);
    const data = emitUndercutMesh(snap);

    // Per column: floor(2) + ceiling(2) + one outer wall(2) = 6 → 12 total.
    expect(data.triangles).toBe(12);
    // Exactly two walls total: col1's −X (+X normal) and col2's +X (−X normal).
    expect(countByNormal(data.normals, 1, 0, 0)).toBe(6);
    expect(countByNormal(data.normals, -1, 0, 0)).toBe(6);
  });

  it('walls a void against solid neighbours on all four sides', () => {
    // 3×3 grid, centre cell (idx 4) is the sole undercut wing; all eight
    // neighbours are full-height solid. Expect walls on all four faces.
    const spansByCell = new Map<number, Array<[number, number]>>([
      [
        4,
        [
          [-10, -5],
          [-2, 0],
        ],
      ],
    ]);
    const topByCell = new Map<number, number>();
    for (let i = 0; i < 9; i++) if (i !== 4) topByCell.set(i, 0);
    const snap = makeSnapshot(3, 3, spansByCell, topByCell);
    const data = emitUndercutMesh(snap);

    // Floor(2) + ceiling(2) + 4 walls(8) = 12 triangles.
    expect(data.triangles).toBe(12);
    expect(countByNormal(data.normals, 1, 0, 0)).toBe(6); // −X wall
    expect(countByNormal(data.normals, -1, 0, 0)).toBe(6); // +X wall
    expect(countByNormal(data.normals, 0, 1, 0)).toBe(6); // −Y wall
    expect(countByNormal(data.normals, 0, -1, 0)).toBe(6); // +Y wall
  });

  it('partially walls a void where the neighbour is solid over only part of its Z range', () => {
    // Column 1 void [-6,-1]. Left neighbour (col0) solid only to -3, so it
    // walls just [-6,-3] of the void; the rest opens. Right neighbour full.
    const spansByCell = new Map<number, Array<[number, number]>>([
      [
        1,
        [
          [-10, -6],
          [-1, 0],
        ],
      ],
    ]);
    const topByCell = new Map<number, number>([
      [0, -3],
      [2, 0],
    ]);
    const snap = makeSnapshot(3, 1, spansByCell, topByCell);
    const data = emitUndercutMesh(snap);

    // Both faces walled but the −X wall spans only [-6,-3]; +X spans [-6,-1].
    const nxZs: number[] = [];
    const pxZs: number[] = [];
    for (let i = 0; i < data.positions.length; i += 3) {
      if (Math.abs(data.normals[i] - 1) < 1e-4) nxZs.push(data.positions[i + 2]); // −X face (+X normal)
      if (Math.abs(data.normals[i] + 1) < 1e-4) pxZs.push(data.positions[i + 2]); // +X face (−X normal)
    }
    expect(Math.min(...nxZs)).toBeCloseTo(-6, 5);
    expect(Math.max(...nxZs)).toBeCloseTo(-3, 5); // clipped to neighbour top
    expect(Math.min(...pxZs)).toBeCloseTo(-6, 5);
    expect(Math.max(...pxZs)).toBeCloseTo(-1, 5); // full void height
  });
});
