/// Integration test for the two-sided conflict seam inside `HeightfieldDriver`.
///
/// The detection MATH is unit-tested in `two_sided_conflict.test.ts`. What
/// nobody else exercises is the driver PLUMBING the live app runs: on a
/// two-sided build the driver carves the reflected back surface to
/// completion, then carves a THROWAWAY front sim to completion, reads both
/// finished heightfields back out of WASM memory, and pairs them through
/// `detectTwoSidedConflicts` — exposed via `getTwoSidedConflicts()`.
///
/// We drive the REAL driver (and real `HeightfieldMeshPyramid`, which builds
/// fine under node) with a Simulator stub whose `advance()` stamps a scripted
/// FINAL heightfield into that instance's own slab of a real
/// `WebAssembly.Memory` — so the driver's zero-copy `data_ptr` views read
/// genuine bytes. Front vs back programs are distinguished by toolpath-array
/// identity, mirroring how the driver feeds each sim its own program.

import { describe, expect, it, vi, beforeEach } from 'vitest';
import * as THREE from 'three';
import type { ImportResponse, GenerateResponse } from '../api/types';
import type { ToolEntry } from '../state/project-types';

const COLS = 4;
const ROWS = 4;
const CELLS = COLS * ROWS;

const h = vi.hoisted(() => {
  const memory = new WebAssembly.Memory({ initial: 1 }); // 64 KiB
  const cols = 4;
  const rows = 4;
  const cells = cols * rows;
  const SLAB_BYTES = cells * 4;
  // toolpath-array identity → the FINAL heightfield that program carves.
  // Set per-test; both the live front sim and the throwaway front sim get the
  // same front array, so they resolve to the same scripted heights.
  const scripts = new Map<object, Float32Array>();
  const instances: FakeSimulator[] = [];
  let nextSlab = 0;

  /// Minimal stand-in for the wasm `Simulator` covering the methods the
  /// two-sided build + conflict pass call. Each instance owns a private slab
  /// of `memory` so front / back / throwaway heightfields never collide.
  class FakeSimulator {
    private readonly offBytes: number;
    private readonly topZv: number;
    private readonly heights: Float32Array;
    private script: Float32Array | null = null;

    constructor(
      _minX: number,
      _minY: number,
      _maxX: number,
      _maxY: number,
      _cellSize: number,
      topZ: number,
      _floorZ: number,
    ) {
      this.offBytes = nextSlab++ * SLAB_BYTES;
      this.topZv = topZ;
      this.heights = new Float32Array(memory.buffer, this.offBytes, cells);
      this.heights.fill(topZ); // uncut stock
      instances.push(this);
    }

    cols() {
      return cols;
    }
    rows() {
      return rows;
    }
    cell_size() {
      return 1;
    }
    origin_x() {
      return 0;
    }
    origin_y() {
      return 0;
    }
    top_z() {
      return this.topZv;
    }
    data_ptr() {
      return this.offBytes;
    }

    reset() {
      this.heights.fill(this.topZv);
    }
    set_fixtures() {}
    set_toolpath(tp: unknown) {
      this.script = scripts.get(tp as object) ?? null;
      return 0;
    }
    clear_toolpath() {}
    clear_checkpoints() {}
    take_diagnostics() {
      return { warnings: [] };
    }
    // Carve = stamp the scripted final heights into this instance's slab.
    advance() {
      if (this.script) this.heights.set(this.script);
      return new Uint32Array([0, 0, cols, rows]);
    }
    partial_advance() {
      if (this.script) this.heights.set(this.script);
      return new Uint32Array([0, 0, cols, rows]);
    }
    undercut_column_count() {
      return 0;
    }
    free() {}
  }

  return {
    memory,
    cols,
    rows,
    cells,
    scripts,
    instances,
    FakeSimulator,
    resetSlabs() {
      nextSlab = 0;
    },
  };
});

vi.mock('ivac-wasm', () => ({
  default: async () => ({ memory: h.memory }),
  Simulator: h.FakeSimulator,
}));

// Import AFTER the mock is registered.
import { HeightfieldDriver } from './driver';

const IMPORTED = { bbox: { min_x: 0, min_y: 0, max_x: 4, max_y: 4 } } as unknown as ImportResponse;
const TOOL = {
  id: 1,
  name: 'T1',
  kind: 'endmill',
  diameter: 3,
  flutes: 2,
  speed: 18000,
  plungeRate: 200,
  feedRate: 1200,
  coolant: 'off',
} as unknown as ToolEntry;
// 10 mm thick stock, top at Z=0 → mid-plane −5, reflection offset (2·mid) −10.
const STOCK = { mode: 'manual' as const, margin: 0, thickness: 10, customX: 4, customY: 4 };
const SETTINGS = {
  cellResolutionMode: 'manual',
  cellResolutionMm: 1,
  maxSimulationCells: 1_000_000,
  maxRenderTriangles: 2_000_000,
  solidColor: '#c8b48a',
  solidOpacity: 1,
  edgeColor: '#1a1a1a',
  edgeOpacity: 1,
} as unknown as Parameters<HeightfieldDriver['build']>[0]['settings'];

/// A finished heightfield: every cell at topZ (0) except the listed cells,
/// each set to `topZ - depth`.
function carved(cuts: Record<number, number>): Float32Array {
  const a = new Float32Array(CELLS); // 0 = topZ
  for (const [idx, depth] of Object.entries(cuts)) a[Number(idx)] = -depth;
  return a;
}

/// Two 1-segment programs (single tool → one carve run) with distinct array
/// identities so the mock can hand each its own scripted heightfield.
function programs(front: Float32Array, back: Float32Array) {
  const frontTp = [{}];
  const backTp = [{}];
  h.scripts.set(frontTp, front);
  h.scripts.set(backTp, back);
  return {
    generated: { toolpath: frontTp } as unknown as GenerateResponse,
    generatedBack: { toolpath: backTp } as unknown as GenerateResponse,
  };
}

function buildInput(over: Record<string, unknown>) {
  return {
    imported: IMPORTED,
    tool: TOOL,
    toolForSeg: () => TOOL,
    toolForSegBack: () => TOOL,
    stock: STOCK,
    settings: SETTINGS,
    ...over,
  } as unknown as Parameters<HeightfieldDriver['build']>[0];
}

async function freshDriver(): Promise<HeightfieldDriver> {
  const driver = new HeightfieldDriver({ scene: new THREE.Scene(), requestRender: vi.fn() });
  await driver.init();
  return driver;
}

describe('HeightfieldDriver two-sided conflict pass', () => {
  beforeEach(() => {
    h.instances.length = 0;
    h.scripts.clear();
    h.resetSlabs();
  });

  it('flags a column where the finished front + back carves overlap', async () => {
    const driver = await freshDriver();
    // Cell 0: 6 mm front + 6 mm back on 10 mm stock → 2 mm overlap. Cell 5:
    // 3 mm + 3 mm → 4 mm material left (clear).
    const { generated, generatedBack } = programs(carved({ 0: 6, 5: 3 }), carved({ 0: 6, 5: 3 }));
    driver.build(buildInput({ generated, generatedBack }));

    const conflicts = driver.getTwoSidedConflicts();
    expect(conflicts).toHaveLength(1);
    expect(conflicts[0].x).toBeCloseTo(0.5, 6); // cell 0 center
    expect(conflicts[0].y).toBeCloseTo(0.5, 6);
    expect(conflicts[0].overlapMm).toBeCloseTo(2, 5);
    expect(conflicts[0].z).toBeCloseTo(-5, 5); // overlap midpoint = mid-plane
  });

  it('reports no conflict when the two carves stay clear of each other', async () => {
    const driver = await freshDriver();
    // 3 mm + 3 mm everywhere cut → 4 mm remains, no crossing.
    const { generated, generatedBack } = programs(carved({ 0: 3 }), carved({ 0: 3 }));
    driver.build(buildInput({ generated, generatedBack }));
    expect(driver.getTwoSidedConflicts()).toEqual([]);
  });

  it('is empty for a single-sided build (no back program)', async () => {
    const driver = await freshDriver();
    const frontTp = [{}];
    h.scripts.set(frontTp, carved({ 0: 9 }));
    driver.build(buildInput({ generated: { toolpath: frontTp } as unknown as GenerateResponse }));
    expect(driver.getTwoSidedConflicts()).toEqual([]);
  });

  it('clears conflicts when a two-sided build is replaced by a single-sided one', async () => {
    const driver = await freshDriver();
    const two = programs(carved({ 0: 6 }), carved({ 0: 6 }));
    driver.build(buildInput({ generated: two.generated, generatedBack: two.generatedBack }));
    expect(driver.getTwoSidedConflicts()).toHaveLength(1);

    // Rebuild single-sided → stale markers must drop.
    const frontTp = [{}];
    h.scripts.set(frontTp, carved({ 0: 2 }));
    driver.build(buildInput({ generated: { toolpath: frontTp } as unknown as GenerateResponse }));
    expect(driver.getTwoSidedConflicts()).toEqual([]);
  });
});
