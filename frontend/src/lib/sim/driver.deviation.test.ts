/// Integration test for the deviation-overlay glue inside `HeightfieldDriver`.
///
/// The three links in the overlay chain are each already covered in isolation:
///   * classification (carved field → 0/1/2 class codes) — Rust core
///     `SurfaceField::deviation_*` + the wasm `deviation_recompute` partial==full
///     test;
///   * class → vertex-hue painting — `heightfield_mesh.test.ts`
///     (`setDeviation` tints top faces gouge-red / rest-green / on-target-gray);
///   * target-surface building from ops+sources — `deviation_target.test.ts`.
///
/// What NObody exercised is the SEAM the live app runs: the driver caching the
/// target on the sim, recomputing the class buffer, re-taking the zero-copy
/// view, and forwarding it to the mesh — on enable, per-carve over the dirty
/// AABB, on disable, and re-armed across a `build()`. That is item 5 of
/// ivac-58nl.1.1 ("e2e check that the toggle paints vertex colors"), and the
/// gating manual item 1 hangs off the same data path. We drive the REAL driver
/// and REAL `HeightfieldMeshPyramid` (Three.js CPU objects construct fine under
/// node), injecting a Simulator stub backed by a real `WebAssembly.Memory` so
/// the driver's zero-copy `deviation_ptr` view reads genuine bytes.

import { describe, expect, it, vi, beforeEach } from 'vitest';
import * as THREE from 'three';
import type { ImportResponse, GenerateResponse, SurfaceField } from '../api/types';
import type { ToolEntry } from '../state/project-types';

// Deviation class codes + the per-class top-face RGB, kept identical to
// `heightfield_mesh.ts` (also hard-coded in `heightfield_mesh.test.ts`).
const ON_TARGET = 0;
const GOUGE = 1;
const REST = 2;
const RGB_GOUGE = [0.8, 0.12, 0.12] as const;
const RGB_REST = [0.16, 0.62, 0.24] as const;
const RGB_NEUTRAL = [0.75, 0.75, 0.75] as const;

// 4×4 sim grid — small, and comfortably under the LOD budget so level 0 stays
// active (its color buffer maps 1:1 to the class grid).
const COLS = 4;
const ROWS = 4;
const CELLS = COLS * ROWS;

/// Shared WASM-memory + fake-simulator registry. Hoisted so the `vi.mock`
/// factory (itself hoisted above imports) and the test bodies see the same
/// objects. The driver reads `new Uint8Array(wasm.memory.buffer, ptr, len)`,
/// so the class bytes must live in a real `WebAssembly.Memory`.
const h = vi.hoisted(() => {
  const memory = new WebAssembly.Memory({ initial: 1 }); // 64 KiB, never grown
  const HEIGHTS_OFF = 0; // 16 f32 = 64 bytes
  const CLASS_OFF = 8192; // 16 u8, well clear of the heights region
  const cols = 4;
  const rows = 4;
  const cells = cols * rows;

  /// Minimal stand-in for the wasm `Simulator` covering exactly the methods
  /// the driver calls on the build + advance + deviation paths. `truth` is the
  /// scripted class grid the "sim" would have classified; `deviation_recompute*`
  /// copy it (whole grid / sub-rect) into the real memory the driver views.
  class FakeSimulator {
    heights: Float32Array;
    buf: Uint8Array;
    truth: Uint8Array;
    sized = false;
    target: unknown = null;
    nextAabb: Uint32Array = new Uint32Array([0, 0, cols, rows]);

    constructor() {
      this.heights = new Float32Array(memory.buffer, HEIGHTS_OFF, cells);
      this.heights.fill(0); // all at top_z
      this.buf = new Uint8Array(memory.buffer, CLASS_OFF, cells);
      this.buf.fill(0);
      this.truth = new Uint8Array(cells); // default all on-target
      instances.push(this);
    }

    // — grid geometry (drives the mesh the driver builds) —
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
      return 0;
    }
    data_ptr() {
      return HEIGHTS_OFF;
    }

    // — lifecycle / carve (no-op carve; just reports a dirty AABB) —
    reset() {}
    set_fixtures() {}
    set_toolpath() {
      return 0;
    }
    clear_toolpath() {}
    clear_checkpoints() {}
    take_diagnostics() {
      return { warnings: [] };
    }
    advance() {
      return this.nextAabb;
    }
    partial_advance() {
      return this.nextAabb;
    }
    undercut_column_count() {
      return 0;
    }

    // — deviation overlay —
    set_deviation_target(surfaces: unknown) {
      this.target = surfaces;
      this.sized = false; // force a full rebuild next recompute (real contract)
    }
    clear_deviation_target() {
      this.target = null;
      this.sized = false;
    }
    has_deviation_target() {
      return this.target != null;
    }
    deviation_recompute() {
      this.buf.set(this.truth);
      this.sized = true;
    }
    deviation_recompute_in(ix0: number, iy0: number, ix1: number, iy1: number) {
      if (!this.sized) {
        this.deviation_recompute();
        return;
      }
      for (let iy = iy0; iy < iy1; iy++) {
        for (let ix = ix0; ix < ix1; ix++) {
          const i = iy * cols + ix;
          this.buf[i] = this.truth[i];
        }
      }
    }
    deviation_ptr() {
      return CLASS_OFF;
    }
    deviation_len() {
      return this.sized ? cells : 0;
    }
    free() {}
  }

  const instances: FakeSimulator[] = [];
  return { memory, HEIGHTS_OFF, CLASS_OFF, instances, FakeSimulator };
});

vi.mock('ivac-wasm', () => ({
  default: async () => ({ memory: h.memory }),
  Simulator: h.FakeSimulator,
}));

// Import AFTER the mock is registered.
import { HeightfieldDriver } from './driver';

/// Pull the active mesh's per-vertex 'color' buffer out of the driver group.
/// Only the active LOD level (0 here) is attached, and `EdgesGeometry` carries
/// no 'color' attribute, so the first hit is the terrain color buffer.
function colorArray(group: THREE.Object3D): Float32Array {
  let arr: Float32Array | undefined;
  group.traverse((o: THREE.Object3D) => {
    const g = (
      o as unknown as { geometry?: { getAttribute?: (n: string) => { array: Float32Array } } }
    ).geometry;
    const c = g?.getAttribute?.('color');
    if (c && !arr) arr = c.array;
  });
  if (!arr) throw new Error('no color attribute found');
  return arr;
}

/// First float of a cell's TOP face. Layout (shared with the mesh unit test):
/// top faces first (TOP_BASE = 0), 4 verts/cell, 3 floats/vert.
function topFloat(cellIdx: number): number {
  return cellIdx * 4 * 3;
}

function expectCellColor(colors: Float32Array, cellIdx: number, rgb: readonly number[]): void {
  const p = topFloat(cellIdx);
  expect(colors[p + 0]).toBeCloseTo(rgb[0]);
  expect(colors[p + 1]).toBeCloseTo(rgb[1]);
  expect(colors[p + 2]).toBeCloseTo(rgb[2]);
}

const IMPORTED = { bbox: { min_x: 0, min_y: 0, max_x: 4, max_y: 4 } } as unknown as ImportResponse;
// A 2-segment toolpath: below MIN_SEGMENTS_FOR_CHECKPOINTS, so checkpointing
// stays off (the fake needn't implement snapshot/restore).
const GENERATED = { toolpath: [{}, {}] } as unknown as GenerateResponse;
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
const STOCK = { mode: 'manual' as const, margin: 0, thickness: 10, customX: 4, customY: 4 };
// Only the fields build() actually reads; high budgets keep LOD level 0 active
// and avoid the coarsening path.
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

function buildInput() {
  return { imported: IMPORTED, generated: GENERATED, tool: TOOL, stock: STOCK, settings: SETTINGS };
}

// A stand-in target surface — the fake sim ignores its payload (classes come
// from the scripted `truth`), it only needs the SurfaceField shape.
const TARGET: SurfaceField = {
  origin: { x: 0, y: 0 },
  cell: 1,
  cols: COLS,
  rows: ROWS,
  z: new Array(CELLS).fill(-2),
} as unknown as SurfaceField;

function latestSim() {
  return h.instances[h.instances.length - 1];
}

async function freshDriver(): Promise<{ driver: HeightfieldDriver; scene: THREE.Scene }> {
  const scene = new THREE.Scene();
  const driver = new HeightfieldDriver({ scene, requestRender: vi.fn() });
  await driver.init();
  return { driver, scene };
}

describe('HeightfieldDriver deviation overlay (toggle → mesh vertex colors)', () => {
  beforeEach(() => {
    h.instances.length = 0;
  });

  it('paints the whole grid per class when the overlay is enabled', async () => {
    const { driver } = await freshDriver();
    driver.build(buildInput());

    // Script a distinct class per corner: gouge / rest / on-target.
    const sim = latestSim();
    sim.truth[0] = GOUGE; // cell (0,0)
    sim.truth[1] = REST; // cell (1,0)
    sim.truth[2] = ON_TARGET; // cell (2,0)

    driver.setDeviationTarget([TARGET], 0.05);

    const colors = colorArray(driver.group);
    expectCellColor(colors, 0, RGB_GOUGE);
    expectCellColor(colors, 1, RGB_REST);
    expectCellColor(colors, 2, RGB_NEUTRAL);

    driver.destroy();
  });

  it('repaints only the carve dirty-AABB on advance, leaving other cells intact', async () => {
    const { driver } = await freshDriver();
    driver.build(buildInput());

    // Enable with an all-on-target grid → full neutral paint.
    const sim = latestSim();
    driver.setDeviationTarget([TARGET], 0.05);
    let colors = colorArray(driver.group);
    expectCellColor(colors, 0, RGB_NEUTRAL);
    expectCellColor(colors, 15, RGB_NEUTRAL);

    // Now a carve gouges only the far corner cell (3,3) and the sim reports
    // exactly that cell as the dirty AABB.
    sim.truth[15] = GOUGE;
    sim.nextAabb = new Uint32Array([3, 3, COLS, ROWS]); // half-open [3,4)×[3,4)
    const changed = driver.advanceTo(1, GENERATED.toolpath, TOOL);
    expect(changed).toBe(true);

    colors = colorArray(driver.group);
    // The gouged cell turned red...
    expectCellColor(colors, 15, RGB_GOUGE);
    // ...and a cell outside the dirty AABB kept its neutral class (partial
    // recompute did not touch it).
    expectCellColor(colors, 0, RGB_NEUTRAL);

    driver.destroy();
  });

  it('clears the overlay back to the white identity multiplier when disabled', async () => {
    const { driver } = await freshDriver();
    driver.build(buildInput());
    const sim = latestSim();
    sim.truth[0] = GOUGE;
    driver.setDeviationTarget([TARGET], 0.05);
    expectCellColor(colorArray(driver.group), 0, RGB_GOUGE);

    // Empty target array turns the overlay off.
    driver.setDeviationTarget([], 0.05);
    const cleared = colorArray(driver.group);
    // setDeviation(null) restores every vertex to the white (1,1,1) identity.
    const p = topFloat(0);
    expect(cleared[p + 0]).toBe(1);
    expect(cleared[p + 1]).toBe(1);
    expect(cleared[p + 2]).toBe(1);

    driver.destroy();
  });

  it('re-arms the overlay across a rebuild (build() re-applies the active target)', async () => {
    const { driver } = await freshDriver();
    driver.build(buildInput());
    driver.setDeviationTarget([TARGET], 0.05);
    // Overlay is on. Rebuild the sim/mesh (e.g. a new Generate) — the driver
    // must re-arm the overlay on the fresh mesh rather than drop it.
    driver.build(buildInput());

    const colors = colorArray(driver.group);
    // The rebuilt sim starts all-on-target, so the re-armed overlay paints
    // neutral gray — crucially NOT the white identity, which would mean the
    // overlay was silently lost across the rebuild.
    expectCellColor(colors, 0, RGB_NEUTRAL);
    expect(colors[topFloat(0)]).not.toBe(1);

    driver.destroy();
  });
});
