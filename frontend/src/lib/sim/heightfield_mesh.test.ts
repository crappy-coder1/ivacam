/// Pure-logic tests for the heightfield-mesh helpers that don't need
/// a WebGL context. The HeightfieldMesh / HeightfieldMeshPyramid
/// classes themselves are exercised by the e2e + integration runs;
/// here we cover the budget-driven LOD selection knob so the
/// state-machine math is locked down independently of Three.js.

import { describe, expect, it } from 'vitest';
import {
  DEVIATION_GOUGE,
  DEVIATION_ON_TARGET,
  DEVIATION_REST_STOCK,
  HeightfieldMesh,
  HeightfieldMeshPyramid,
  type HeightfieldOptions,
  pickMinLodLevelForBudget,
} from './heightfield_mesh';

describe('pickMinLodLevelForBudget', () => {
  it('returns 0 when the source grid already fits the budget', () => {
    // 200 × 200 × 6 = 240_000 triangles ≤ 2M → L0 is affordable.
    expect(pickMinLodLevelForBudget(200, 200, 2_000_000)).toBe(0);
  });

  it('returns 1 when L0 exceeds but L1 fits', () => {
    // L0 = 1000 * 1000 * 6 = 6M tri > 2M.
    // L1 = 500 * 500 * 6 = 1.5M tri ≤ 2M → minLevel = 1.
    expect(pickMinLodLevelForBudget(1000, 1000, 2_000_000)).toBe(1);
  });

  it('returns 2 when L0 + L1 both exceed', () => {
    // L0 = 2000 * 2000 * 6 = 24M > 2M.
    // L1 = 1000 * 1000 * 6 = 6M > 2M.
    // L2 = 500 * 500 * 6 = 1.5M ≤ 2M → minLevel = 2.
    expect(pickMinLodLevelForBudget(2000, 2000, 2_000_000)).toBe(2);
  });

  it('returns maxLevel when even the coarsest level exceeds the budget', () => {
    // Tiny budget forces all the way to L3 (the default cap).
    expect(pickMinLodLevelForBudget(2000, 2000, 1000)).toBe(3);
  });

  it('respects a custom maxLevel cap', () => {
    expect(pickMinLodLevelForBudget(2000, 2000, 1000, 5)).toBe(5);
  });

  it('defaults to 0 when the budget is zero or negative', () => {
    expect(pickMinLodLevelForBudget(1000, 1000, 0)).toBe(0);
    expect(pickMinLodLevelForBudget(1000, 1000, -1)).toBe(0);
  });

  it('handles rectangular grids with non-power-of-two dimensions', () => {
    // 1023 × 513 ≈ 525k cells × 6 = 3.15M tri > 2M.
    // L1 ≈ 512 × 257 × 6 ≈ 790k tri ≤ 2M.
    expect(pickMinLodLevelForBudget(1023, 513, 2_000_000)).toBe(1);
  });

  it('clamps to minLevel 0 for trivially small grids', () => {
    expect(pickMinLodLevelForBudget(1, 1, 2_000_000)).toBe(0);
    expect(pickMinLodLevelForBudget(10, 10, 2_000_000)).toBe(0);
  });
});

/// The deviation overlay writes a per-vertex color buffer. Three.js CPU
/// objects (BufferGeometry / materials) construct fine in the node test env
/// (no WebGL context needed), so we can drive `setDeviation` and read the
/// color attribute straight back to lock the class → hue mapping and the
/// dirty-AABB re-paint contract.
describe('HeightfieldMesh deviation overlay', () => {
  const baseOpts: HeightfieldOptions = {
    cols: 2,
    rows: 1,
    cellSize: 1,
    originX: 0,
    originY: 0,
    topZ: 0,
    floorZ: -10,
    solidColor: '#808080',
    solidOpacity: 1,
    edgeColor: '#000000',
    edgeOpacity: 1,
  };

  // Pull the (mesh + depthMesh shared) 'color' BufferAttribute out of a
  // group. EdgesGeometry carries only 'position', so it's skipped.
  function colorArray(group: { traverse: (cb: (o: unknown) => void) => void }): Float32Array {
    let arr: Float32Array | undefined;
    group.traverse((o: unknown) => {
      const g = (o as { geometry?: { getAttribute?: (n: string) => { array: Float32Array } } })
        .geometry;
      const c = g?.getAttribute?.('color');
      if (c) arr = c.array;
    });
    if (!arr) throw new Error('no color attribute found');
    return arr;
  }

  it('tints top faces per class and grays walls, then clears to white', () => {
    const mesh = new HeightfieldMesh(baseOpts);
    mesh.setDeviation(new Uint8Array([DEVIATION_GOUGE, DEVIATION_REST_STOCK]));
    expect(mesh.isDeviationActive()).toBe(true);

    const colors = colorArray(mesh.group);
    // Cell 0 top face (TOP_BASE=0, vertex 0) → gouge red.
    expect(colors[0]).toBeCloseTo(0.8);
    expect(colors[1]).toBeCloseTo(0.12);
    expect(colors[2]).toBeCloseTo(0.12);
    // Cell 1 top face (vertex TOP_BASE + 1*4 = 4 → floats 12..14) → rest green.
    expect(colors[12]).toBeCloseTo(0.16);
    expect(colors[13]).toBeCloseTo(0.62);
    expect(colors[14]).toBeCloseTo(0.24);
    // A wall vertex (RIGHT_BASE = 4*n = 8 → floats 24..26) stays neutral gray.
    expect(colors[24]).toBeCloseTo(0.75);

    // Clearing restores the white identity multiplier everywhere.
    mesh.setDeviation(null);
    expect(mesh.isDeviationActive()).toBe(false);
    const cleared = colorArray(mesh.group);
    expect(cleared[0]).toBe(1);
    expect(cleared[12]).toBe(1);
    expect(cleared[24]).toBe(1);
  });

  it('repaints only the dirty AABB on an incremental update', () => {
    const mesh = new HeightfieldMesh({ ...baseOpts, cols: 3 });
    // Activate with all-on-target (every top face neutral gray).
    mesh.setDeviation(
      new Uint8Array([DEVIATION_ON_TARGET, DEVIATION_ON_TARGET, DEVIATION_ON_TARGET]),
    );
    // Now only cell 1 gouges; restrict the repaint to its AABB.
    mesh.setDeviation(new Uint8Array([DEVIATION_ON_TARGET, DEVIATION_GOUGE, DEVIATION_ON_TARGET]), {
      ix0: 1,
      iy0: 0,
      ix1: 2,
      iy1: 1,
    });
    const colors = colorArray(mesh.group);
    // Cell 1 (vertex 4 → float 12) is now red.
    expect(colors[12]).toBeCloseTo(0.8);
    // Cell 0 (float 0) untouched → still neutral gray.
    expect(colors[0]).toBeCloseTo(0.75);
  });
});

describe('HeightfieldMeshPyramid deviation pooling', () => {
  const baseOpts: HeightfieldOptions = {
    cols: 4,
    rows: 1,
    cellSize: 1,
    originX: 0,
    originY: 0,
    topZ: 0,
    floorZ: -10,
    solidColor: '#808080',
    solidOpacity: 1,
    edgeColor: '#000000',
    edgeOpacity: 1,
  };

  function colorArray(group: { traverse: (cb: (o: unknown) => void) => void }): Float32Array {
    let arr: Float32Array | undefined;
    group.traverse((o: unknown) => {
      const g = (o as { geometry?: { getAttribute?: (n: string) => { array: Float32Array } } })
        .geometry;
      const c = g?.getAttribute?.('color');
      if (c) arr = c.array;
    });
    if (!arr) throw new Error('no color attribute found');
    return arr;
  }

  it('worst-wins pools L0 classes when a coarse level activates', () => {
    // 4×1 grid, 2 LOD levels. L1 pools 2×1 blocks.
    const pyr = new HeightfieldMeshPyramid({ ...baseOpts }, 1, 0);
    // The driver always feeds a height view before a level swap re-pools;
    // without it setActiveLevel short-circuits (no L0 data to pool from).
    pyr.updateHeights(new Float32Array([0, 0, 0, 0]));
    pyr.setDeviation(
      new Uint8Array([
        DEVIATION_GOUGE, // block 0: [gouge, on-target] → gouge
        DEVIATION_ON_TARGET,
        DEVIATION_ON_TARGET, // block 1: [on-target, rest] → rest
        DEVIATION_REST_STOCK,
      ]),
    );
    pyr.setActiveLevel(1);
    expect(pyr.getActiveLevel()).toBe(1);

    const colors = colorArray(pyr.group);
    // L1 cell 0 (float 0) → gouge red (a defect in the block survives pooling).
    expect(colors[0]).toBeCloseTo(0.8);
    // L1 cell 1 (vertex 4 → float 12) → rest green.
    expect(colors[12]).toBeCloseTo(0.16);
    expect(colors[13]).toBeCloseTo(0.62);
  });
});
