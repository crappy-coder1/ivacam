/// Glue between the WASM Simulator and the HeightfieldMesh:
/// owns one Simulator + one HeightfieldMesh per active project.gen.generated,
/// drives advance() on playhead change, and refreshes the affected mesh
/// region using the Float32Array view re-taken after every advance.
///
/// The lifecycle is:
///   * mount: wait for the WASM module to load, then create Simulator
///     (needs stock bbox + cell size) and HeightfieldMesh (matching grid).
///   * playhead change: call sim.advance(prev, next), get the dirty AABB,
///     re-take the Float32Array view (memory growth can detach the old
///     one), and call mesh.updateHeights(view, aabb).
///   * scrub backward: reset the simulator and replay 0..head.
///   * project.gen.generated changes: rebuild both Simulator + mesh.

import * as THREE from 'three';
import { HeightfieldMeshPyramid, pickMinLodLevelForBudget } from './heightfield_mesh';
import { UndercutMeshBuilder } from './undercut_mesh';
import { backReflectionOffsetZ } from './dual_surface';
import { detectTwoSidedConflicts, type ConflictMarker } from './two_sided_conflict';
import { planAdvance, playheadToSegment } from './playhead';
import { computeFootprint } from './footprint';
import { isWasmTransport } from '../api/transport-mode';
import { simWarningKey } from './warnings';
import type {
  GenerateResponse,
  ImportResponse,
  SimDiagnostics,
  SurfaceField,
  ToolpathSegment,
} from '../api/types';
import type { AppSettings, Fixture, ToolEntry } from '../state/project.svelte';
import { toWireToolKind } from '../api/build-project';

interface SimulatorWasm {
  // wasm-bindgen produces a constructor on the generated class; this
  // interface stands in for both the constructor signature and the
  // instance shape so we can `new SimulatorWasm(...)` without importing
  // the generated d.ts type. The no-misused-new lint would prefer a
  // class — wasm-bindgen owns the actual class definition.
  // eslint-disable-next-line @typescript-eslint/no-misused-new
  new (
    minX: number,
    minY: number,
    maxX: number,
    maxY: number,
    cellSize: number,
    topZ: number,
    /// Explicit span floor (physical stock bottom = topZ − thickness). The
    /// dexel field carves undercuts relative to it; 3-axis jobs never reach it.
    stockBottomZ: number,
  ): SimulatorWasm;
  reset(): void;
  /// Record that the JS driver coarsened cell_size to fit the
  /// user's maxSimulationCells budget. The warning rides out via
  /// take_diagnostics() so the UI surfaces it like any other sim
  /// warning instead of silently smoothing out small features.
  push_cell_size_coarsened(
    original_cell_size_mm: number,
    coarsened_cell_size_mm: number,
    reason: string,
  ): void;
  advance(tool: unknown, from_idx: number, to_idx: number): Uint32Array;
  /// Carve only chunk `[t_start, t_end]` of segment `seg_idx`. Used per
  /// render frame so the heightfield destruction follows the cutter
  /// inside long segments (drill plunges) instead of popping at segment
  /// boundaries.
  partial_advance(tool: unknown, seg_idx: number, t_start: number, t_end: number): Uint32Array;
  set_fixtures(fixtures: unknown): void;
  set_toolpath(segments: unknown): number;
  clear_toolpath(): void;
  toolpath_len(): number;
  /// Snapshot the current heightfield under `seg_idx` (a clean segment
  /// boundary) for fast backward scrubbing.
  checkpoint(seg_idx: number): void;
  /// Restore the heightfield from the snapshot at exactly `seg_idx`
  /// (marks the whole grid dirty). Returns true on a hit.
  restore_checkpoint(seg_idx: number): boolean;
  /// Largest checkpoint `seg_idx` ≤ `target`, or -1 if none.
  nearest_checkpoint(target: number): number;
  /// Drop all heightmap checkpoints (call when the toolpath changes).
  clear_checkpoints(): void;
  checkpoint_count(): number;
  take_diagnostics(): SimDiagnostics;
  cols(): number;
  rows(): number;
  cell_size(): number;
  origin_x(): number;
  origin_y(): number;
  top_z(): number;
  data_ptr(): number;
  /// Number of columns carrying an undercut sidecar entry (0 for a pure
  /// 3-axis job). Check this to skip the undercut upload entirely when
  /// there's nothing to draw.
  undercut_column_count(): number;
  /// Undercut sidecar as flat CSR buffers (zero-copy, like `data_ptr`).
  /// Column `i` lives at flat cell `col_index[i]`; its spans are
  /// `spans[2*span_offsets[i] .. 2*span_offsets[i+1]]` as `(lo, hi)` pairs.
  /// `span_offsets` has `undercut_column_count() + 1` entries. Re-take every
  /// view after each `advance()` — a growing WASM heap detaches them.
  undercut_col_index_ptr(): number;
  undercut_col_index_len(): number;
  undercut_span_offsets_ptr(): number;
  undercut_span_offsets_len(): number;
  undercut_spans_ptr(): number;
  undercut_spans_len(): number;
  /// Serialize the carved stock's dense top surface as a binary STL. The
  /// mesh drops to `stock_bottom_z` at every perimeter sample so the result
  /// is watertight. (Undercut voids below the top aren't meshed here yet.)
  export_stl(stock_bottom_z: number): Uint8Array;
  /// Serialize the carved stock as a watertight voxel-solid binary STL (Path
  /// A) — stair-stepped top, but hole-free through undercut voids, for
  /// slicer / boolean consumers. No `stock_bottom_z`: the solid's floor is
  /// intrinsic to the field's spans.
  export_stl_solid(): Uint8Array;
  /// Cache the target relief surface(s) for the red/green deviation overlay:
  /// an array of serialized SurfaceField (one per enabled relief op, unioned
  /// deepest-cut-wins), the world Z their `z = 0` datum maps to (the stock
  /// top), and the on-target tolerance band (mm). An empty array clears the
  /// overlay. Cached once so the per-frame recompute doesn't re-cross the WASM
  /// boundary with the (potentially large) target grids.
  set_deviation_target(surfaces: unknown, surface_z0: number, tol: number): void;
  /// Drop the cached deviation target (overlay turned off).
  clear_deviation_target(): void;
  /// Whether a deviation target is cached.
  has_deviation_target(): boolean;
  /// Reclassify the WHOLE carved field into the persistent class buffer
  /// (`deviation_ptr`), resizing it to cols*rows. Full-repaint path (overlay
  /// on, reset/backstep replay, rebuild).
  deviation_recompute(): void;
  /// Reclassify ONLY the half-open cell rectangle `[ix0,ix1) × [iy0,iy1)` in
  /// the persistent buffer, leaving other cells' classes intact — the carve's
  /// dirty AABB, mirroring the mesh partial re-upload. Falls back to a full
  /// recompute if the buffer isn't yet sized to the grid.
  deviation_recompute_in(ix0: number, iy0: number, ix1: number, iy1: number): void;
  /// Pointer to the persistent row-major cols*rows Uint8Array of deviation
  /// codes (0 on-target, 1 gouge, 2 rest stock), aligned with `data_ptr()`.
  /// Re-take the view after every advance/recompute — a growing heap detaches
  /// it. Length is `deviation_len()` (0 until the first recompute / overlay off).
  deviation_ptr(): number;
  deviation_len(): number;
  free(): void;
}

interface WasmModule {
  default?: (
    module_or_path?: unknown,
  ) => Promise<{ memory: WebAssembly.Memory } & Record<string, unknown>>;
  Simulator: new (
    minX: number,
    minY: number,
    maxX: number,
    maxY: number,
    cellSize: number,
    topZ: number,
    stockBottomZ: number,
  ) => SimulatorWasm;
}

/// The bundle of things the driver actually uses: the constructor
/// reference (so we can `new wasm.Simulator(...)`) plus the
/// WebAssembly.Memory grabbed from the InitOutput. Captured separately
/// because the imported module's namespace is read-only — assigning
/// `module.memory = ...` silently fails in strict mode (and ESM is
/// always strict). Stuffing memory back on the module object was a
/// real bug pre-fix; the Float32Array view never had valid memory and
/// the heightfield mesh stayed at top_z.
interface WasmHandle {
  Simulator: WasmModule['Simulator'];
  memory: WebAssembly.Memory;
}

/// Upper bound on heightmap checkpoints kept for fast backward
/// scrubbing. Each snapshot is cols·rows·4 bytes, so this caps the
/// checkpoint memory (e.g. a 1M-cell heightfield → ≤ ~48 MB across all
/// snapshots). Backward scrubs replay at most `ceil(total/MAX)` segments.
const MAX_CHECKPOINTS = 12;
/// Below this many toolpath segments a full replay is cheap enough that
/// checkpointing isn't worth the memory.
const MIN_SEGMENTS_FOR_CHECKPOINTS = 2 * MAX_CHECKPOINTS;

let wasmPromise: Promise<WasmHandle> | null = null;

async function loadWasm(): Promise<WasmHandle> {
  if (!wasmPromise) {
    wasmPromise = (async () => {
      // The pkg is built by `wasm-pack build crates/ivac-wasm --target web`
      // and linked via package.json. Letting vite resolve the import is
      // critical here: the browser can't fetch a bare `ivac-wasm` URL,
      // so an @vite-ignore'd dynamic import would fail silently in a
      // bundled app. Without the ignore, vite splits this into a chunk,
      // copies ivac_wasm_bg.wasm with a hashed name, and rewrites the
      // js's `import.meta.url`-relative .wasm fetch to match.
      const mod = (await import('ivac-wasm')) as unknown as WasmModule;
      if (typeof mod.default !== 'function') {
        throw new Error('ivac-wasm pkg missing default init export');
      }
      const init = await mod.default();
      if (!init.memory) {
        throw new Error('ivac-wasm init returned no memory');
      }
      return { Simulator: mod.Simulator, memory: init.memory };
    })();
  }
  return wasmPromise;
}

/// Project-state-shaped tool spec the WASM Simulator expects. Mirrors
/// what ivac_core::project::ToolEntry deserializes from (snake_case).
///
/// IMPORTANT: this is the SECOND wire seam after `buildTool`, both of
/// them feeding the same Rust `ToolKind` deserializer. The kind name
/// must go through `toWireToolKind` so the frontend `cone` becomes the
/// backend `kegel`.
export function toWireTool(t: ToolEntry): Record<string, unknown> {
  return {
    id: t.id,
    name: t.name,
    kind: toWireToolKind(t.kind),
    diameter: t.diameter,
    ...(t.tipDiameter !== undefined ? { tip_diameter: t.tipDiameter } : {}),
    ...(t.tipAngleDeg !== undefined ? { tip_angle_deg: t.tipAngleDeg } : {}),
    ...(t.dragoff !== undefined ? { dragoff: t.dragoff } : {}),
    flutes: t.flutes,
    speed: t.speed,
    plunge_rate: t.plungeRate,
    feed_rate: t.feedRate,
    coolant: t.coolant,
    ...(t.fluteLengthMm !== undefined ? { flute_length_mm: t.fluteLengthMm } : {}),
    ...(t.lengthMm !== undefined ? { length_mm: t.lengthMm } : {}),
    ...(t.compressionTransitionMm !== undefined
      ? { compression_transition_mm: t.compressionTransitionMm }
      : {}),
    ...(t.threadPitchMm !== undefined ? { thread_pitch_mm: t.threadPitchMm } : {}),
    ...(t.shankDiameterMm !== undefined ? { shank_diameter_mm: t.shankDiameterMm } : {}),
    ...(t.holder !== undefined ? { holder: t.holder } : {}),
    // Per-kind sim/holder metadata so the heightfield simulator can
    // pick the right cutter profile and holder-check uses the right
    // shank dims. Bull-nose collapses to a fillet profile in the sim;
    // form-profile (incl. the folded-in T-slot) carves its (z, r)
    // sample list when ≥2 rows are present.
    ...(t.cornerRadiusMm !== undefined ? { corner_radius_mm: t.cornerRadiusMm } : {}),
    // Beam / torch cut width — the laser_beam and plasma_torch sim
    // profiles carve a kerf-radius spot instead of a tool radius.
    ...(t.kerfMm !== undefined && t.kerfMm > 0 ? { kerf_mm: t.kerfMm } : {}),
    // Wear compensation — the sim carves at the effective diameter.
    ...(t.wearOffsetMm !== undefined && t.wearOffsetMm !== 0
      ? { wear_offset_mm: t.wearOffsetMm }
      : {}),
    ...(t.kind === 'form_profile' && t.formProfileMm !== undefined && t.formProfileMm.length >= 2
      ? { form_profile_mm: t.formProfileMm.map((s) => ({ z_mm: s.zMm, r_mm: s.rMm })) }
      : {}),
    ...(t.zShiftMm !== undefined ? { z_shift_mm: t.zShiftMm } : {}),
  };
}

/// `computeFootprint` moved to the THREE-free `./footprint`
/// module so the API layer can resolve the stock box without importing
/// this THREE-heavy driver. Imported for this module's own `build()` use
/// and re-exported for the existing 3D-scene import sites.
export { computeFootprint };

/// The in-browser (`?api=wasm`) trial runs the sim single-threaded
/// on the main thread, so a too-fine heightfield makes `set_toolpath` /
/// `advance` / mesh upload stutter the UI. Cap the grid harder there.
/// This is the single fidelity knob: a lower-res carve PREVIEW is an
/// acceptable trial trade — far better than laggy scrubbing. Native /
/// server / Tauri keep the user's full `maxSimulationCells`. ~250k cells
/// ≈ a 500×500 grid over the footprint — smooth, still legible.
export const WASM_TRIAL_SIM_CELL_CAP = 250_000;

/// Effective sim cell cap for the active transport. In wasm-trial mode
/// it's the tighter of the user's setting and [`WASM_TRIAL_SIM_CELL_CAP`];
/// everywhere else it's the user's setting verbatim. Pure so it's
/// unit-tested without the wasm module.
export function effectiveSimCellCap(userMaxCells: number, isWasm: boolean): number {
  const userCap = Math.max(1, userMaxCells);
  return isWasm ? Math.min(userCap, WASM_TRIAL_SIM_CELL_CAP) : userCap;
}

/// Compute cell size from the active tool diameter when settings is in
/// 'auto' mode. Targets ~tool_diameter/15, clamped 0.05..2.0 mm.
function computeCellSize(toolDiameter: number, settings: AppSettings): number {
  if (settings.cellResolutionMode === 'manual') {
    return Math.max(0.01, settings.cellResolutionMm);
  }
  return Math.max(0.05, Math.min(2.0, toolDiameter / 15));
}

export interface DriverOptions {
  scene: THREE.Scene;
  requestRender: () => void;
}

/// Module-level reference to the live driver, set by Scene3D on
/// mount / cleared on dispose. Lets file_ops trigger an STL export
/// without circular imports or threading a driver handle through every
/// component. There is only ever one Scene3D in the app.
let currentDriver: HeightfieldDriver | null = null;

/// Active driver, or `null` when no Scene3D is mounted. Used by the
/// "Export simulated stock as STL..." flow in file_ops.
export function getCurrentDriver(): HeightfieldDriver | null {
  return currentDriver;
}

/// Carve an entire program into `sim`, one contiguous per-tool run at a
/// time. Ops are contiguous in the toolpath, so a single `advance` per run
/// lets each op carve with its own cutter cross-section (a multi-op program
/// mixes tool diameters). `toolWire` maps a segment index to its serialized
/// tool. Shared by the back-surface build and the two-sided conflict pass.
function carveProgramToCompletion(
  sim: SimulatorWasm,
  total: number,
  toolForSeg: (segIdx: number) => ToolEntry,
): void {
  const wireCache = new Map<number, Record<string, unknown>>();
  const wireFor = (i: number): Record<string, unknown> => {
    const t = toolForSeg(i);
    let w = wireCache.get(t.id);
    if (!w) {
      w = toWireTool(t);
      wireCache.set(t.id, w);
    }
    return w;
  };
  let runStart = 0;
  while (runStart < total) {
    const runToolId = toolForSeg(runStart).id;
    let runEnd = runStart + 1;
    while (runEnd < total && toolForSeg(runEnd).id === runToolId) runEnd++;
    sim.advance(wireFor(runStart), runStart, runEnd);
    runStart = runEnd;
  }
}

/// The second carved surface for a two-sided (flip-stock) preview: a back-side
/// Owns the BACK `Simulator` of a two-sided (flip-stock) preview and carves
/// its whole program to completion — a static "finished underside". It does
/// NOT render: the finished back heightfield is folded into the FRONT mesh as
/// a per-cell floor (reflected about the stock mid-plane), so one watertight
/// solid spans the front carve (top) down to the back carve (floor). See
/// `./dual_surface` for the reflection frame math.
///
/// Owned by [`HeightfieldDriver`] only when a two-sided Generate produced a
/// back program; the single-sided front path never touches it. It shares the
/// driver's `WasmHandle` (module + memory), so the second sim is cheap.
///
/// v1 scope (ivac-rt1.11.4): the back is carved ONCE to completion while the
/// playhead keeps scrubbing the FRONT (the "preview follows front side"
/// contract). No deviation overlay or undercut voids on the back yet.
/// Two-sided conflict cells are surfaced separately — see
/// `HeightfieldDriver.getTwoSidedConflicts`, which pairs this surface's
/// finished heightfield against the front's.
class BackSurface {
  private sim: SimulatorWasm | null = null;

  constructor(private wasm: WasmHandle) {}

  /// Build the back sim and carve the whole back program to completion.
  /// `fp` / `cellSize` / `topZ` / `thickness` MIRROR the front build so both
  /// grids align cell-for-cell — a prerequisite for both the per-cell floor
  /// and the conflict pass. `toolForSeg` resolves each back segment's op tool,
  /// so a multi-op back program carves with the right cutter cross-sections.
  build(input: {
    backGenerated: GenerateResponse;
    toolForSeg: (segIdx: number) => ToolEntry;
    fp: { minX: number; minY: number; maxX: number; maxY: number };
    cellSize: number;
    topZ: number;
    thickness: number;
  }) {
    this.disposeSim();
    const { fp, cellSize, topZ, thickness } = input;
    const stockBottomZ = topZ - thickness;
    const sim = new this.wasm.Simulator(
      fp.minX,
      fp.minY,
      fp.maxX,
      fp.maxY,
      cellSize,
      topZ,
      stockBottomZ,
    );
    // The back program runs AFTER the physical flip, so the front's fixtures
    // don't apply to it — carve against a bare table.
    sim.set_fixtures([]);
    sim.set_toolpath(input.backGenerated.toolpath);
    // Carve the whole program to completion in per-tool runs (same split as
    // the front driver's bulk advance) — the back is a static "finished
    // underside".
    carveProgramToCompletion(sim, input.backGenerated.toolpath.length, input.toolForSeg);
    this.sim = sim;
  }

  /// Snapshot of the fully-carved back heightfield as a JS-owned COPY (so
  /// it survives WASM memory growth or this surface's teardown), plus its
  /// grid dims. Same grid as the front build — consumed both as the front
  /// mesh's reflected floor and by the driver's two-sided conflict pass.
  /// Null before `build()` / after `dispose()`.
  finalHeights(): { heights: Float32Array; cols: number; rows: number } | null {
    if (!this.sim) return null;
    const cols = this.sim.cols();
    const rows = this.sim.rows();
    const live = new Float32Array(this.wasm.memory.buffer, this.sim.data_ptr(), cols * rows);
    return { heights: live.slice(), cols, rows };
  }

  private disposeSim() {
    if (this.sim) {
      this.sim.free();
      this.sim = null;
    }
  }

  /// Free the back sim.
  dispose() {
    this.disposeSim();
  }
}

export class HeightfieldDriver {
  readonly group: THREE.Group;
  private sim: SimulatorWasm | null = null;
  private mesh: HeightfieldMeshPyramid | null = null;
  /// Undercut void renderer — the T-slot / dovetail cavity surfaces the
  /// single-valued dense heightfield structurally can't show. Created once
  /// and attached under `group`, so it inherits the sim mesh's
  /// visibility / teardown; fed a fresh snapshot after every carve.
  private undercut: UndercutMeshBuilder | null = null;
  /// Second carved surface for a two-sided (flip-stock) preview — the back
  /// program rendered reflected as the part underside. `null` for the common
  /// single-sided job; created in `build()` when a back program is present.
  private back: BackSurface | null = null;
  /// Two-sided conflict markers for the current build — columns where the
  /// finished front and back carves overlap or cut clean through. Empty for
  /// a single-sided job or before the first build. Recomputed by `build()`.
  private conflicts: ConflictMarker[] = [];
  private wasm: WasmHandle | null = null;
  /// Physical stock floor (`topZ − thickness`) captured at build() — the
  /// implicit `lo` of an uncut column, needed to resolve non-undercut
  /// neighbour solidity when meshing void walls.
  private stockBottomZ = 0;
  /// Last undercut column count pushed to the void builder. Lets a carve
  /// that leaves the sidecar empty clear the mesh exactly once instead of
  /// rebuilding empty geometry every frame of a 3-axis job.
  private lastUndercutCount = 0;
  /// Cached buffer view; valid until the next advance() that may grow
  /// WASM linear memory. Re-taken after every advance.
  private heightView: Float32Array | null = null;
  /// Target surface(s) for the red/green deviation overlay — one per enabled
  /// relief op, unioned deepest-cut-wins on the sim side — or `[]` when the
  /// overlay is off. Cached on the WASM sim (via `set_deviation_target`) at
  /// build() and whenever this changes; the terrain mesh is repainted from
  /// the sim's persistent class buffer after every carve while it's set.
  private deviationTargets: SurfaceField[] = [];
  /// On-target tolerance band half-width (mm) for the overlay.
  private deviationTolMm = 0.05;
  /// Zero-copy view of the sim's persistent deviation-class buffer
  /// (`deviation_ptr`), re-taken after every recompute — a growing WASM heap
  /// detaches it, same contract as `heightView`. `null` while the overlay is off.
  private deviationView: Uint8Array | null = null;
  /// Reference to the toolpath array that's currently cached on the
  /// WASM Simulator. Used to detect identity drift (e.g. a stale
  /// driver picking up a new Generate response) and trigger a single
  /// re-cache rather than re-deserializing per frame.
  private cachedToolpath: ToolpathSegment[] | null = null;
  /// Position in the toolpath that the heightfield is fully carved up
  /// to. Semantics: segments `[0, appliedSeg)` are bulk-carved; segment
  /// `appliedSeg` is carved up to `partialT ∈ [0, 1]`. Together
  /// they fully describe the rendered destruction state and let the
  /// driver issue tiny partial carves per render frame.
  private appliedSeg = 0;
  private partialT = 0;
  /// Cumulative sim warnings collected since the last replay. Cleared on
  /// dispose / reset; merged-into on every forward advance() so the UI
  /// can mark the offending segments as the user scrubs.
  private diagnostics: SimDiagnostics = { warnings: [] };
  private onDiagnosticsChange: ((d: SimDiagnostics) => void) | null = null;
  /// Target spacing, in segments, between heightmap checkpoints. Computed
  /// at build() from the program length and `MAX_CHECKPOINTS` so total
  /// checkpoint memory stays bounded regardless of program size. 0
  /// disables checkpointing (tiny programs where a full replay is cheap).
  private checkpointInterval = 0;
  /// Highest segment boundary a checkpoint has been taken at. Forward
  /// play snapshots again once `appliedSeg` advances `checkpointInterval`
  /// past this. Never lowered on rewind — snapshots above the playhead
  /// stay valid for a later forward scrub.
  private lastCheckpointSeg = 0;
  /// Per-checkpoint diagnostics snapshots, keyed by the same segment
  /// boundary as the Rust heightmap checkpoint. On a backward scrub the
  /// driver seeds `diagnostics` from here so the rewound prefix's
  /// warnings survive without re-running the (expensive) holder/fixture
  /// pass for `[0, replayFrom)`.
  private diagCheckpoints = new Map<number, SimDiagnostics>();
  /// Edges rebuild walks every triangle in the active heightfield
  /// (THREE.EdgesGeometry has no incremental API), so it must NOT run
  /// during continuous activity. Pure trailing debounce — every call
  /// to `scheduleEdgeRebuild` resets the timer, the rebuild fires
  /// only after `EDGE_REBUILD_MS` of quiet (no carve + no camera
  /// move). Continuous playback never hits it; idle frames after
  /// playback stops, do.
  private edgeRebuildTimer: ReturnType<typeof setTimeout> | null = null;
  private static readonly EDGE_REBUILD_MS = 400;
  /// Hard cap on active-level triangle count for which edges are
  /// computed at all. Above this, the EdgesGeometry rebuild cost
  /// (~25 ns / triangle in v8) exceeds a frame budget by itself, and
  /// the lines visually clutter the cell field anyway. The LOD
  /// pyramid normally pushes the active level coarser before this
  /// hits, but the cap is a hard ceiling regardless.
  private static readonly EDGE_MAX_TRIANGLES = 400_000;

  constructor(private opts: DriverOptions) {
    this.group = new THREE.Group();
    this.group.visible = false;
    opts.scene.add(this.group);
    // Void renderer lives under `group` (not the scene) so preview-mode
    // visibility and disposal cascade from the dense sim mesh. Persists
    // across build() rebuilds; each build() just swaps its geometry.
    this.undercut = new UndercutMeshBuilder({
      scene: this.group,
      requestRender: opts.requestRender,
    });
    // Register as the live driver so file_ops can reach it.
    // eslint-disable-next-line @typescript-eslint/no-this-alias -- singleton registry
    currentDriver = this;
  }

  async init(): Promise<void> {
    if (this.wasm) return;
    this.wasm = await loadWasm();
  }

  /// Build (or rebuild) the simulator + mesh for the given project
  /// state. Caller must ensure init() has resolved before the first
  /// build. Tearing down a previous sim/mesh is automatic.
  build(input: {
    imported: ImportResponse | null;
    generated: GenerateResponse | null;
    /// The BACK program of a two-sided (flip-stock) run, or `null` for a
    /// single-sided job. When present the front mesh floors at the stock
    /// mid-plane and a reflected back surface fills the bottom half.
    generatedBack?: GenerateResponse | null;
    tool: ToolEntry | null;
    /// Resolves the cutting tool for each FRONT toolpath segment (by op).
    /// Only consulted by the two-sided conflict pass (which carves a throwaway
    /// front sim to completion); the playhead-driven front carve gets its own
    /// resolver via `advanceTo`. Falls back to `tool` for every segment when
    /// omitted (fine for a single-op front).
    toolForSeg?: (segIdx: number) => ToolEntry;
    /// Resolves the cutting tool for each BACK toolpath segment (by op). Only
    /// consulted when `generatedBack` is present.
    toolForSegBack?: (segIdx: number) => ToolEntry;
    stock: {
      mode: 'auto' | 'manual';
      margin: number;
      thickness: number;
      customX: number;
      customY: number;
      /// Z of the stock top plane (default 0 = top at WCS z=0).
      offsetZ?: number;
    };
    settings: AppSettings;
    fixtures?: Fixture[];
  }) {
    if (!this.wasm || !input.imported || !input.generated || !input.tool) {
      this.dispose();
      return;
    }
    const fp = computeFootprint(input.imported, input.stock);
    const cellSize = computeCellSize(input.tool.diameter, input.settings);
    const cols = Math.ceil((fp.maxX - fp.minX) / cellSize) + 1;
    const rows = Math.ceil((fp.maxY - fp.minY) / cellSize) + 1;
    const cellCount = cols * rows;
    // Sim-side cap: `maxSimulationCells` bounds WASM heap allocation
    // (4 bytes / cell). Halve cell density only when the user's
    // setting is exceeded — accurately reflecting their preference.
    // The GPU-side cap is handled separately by the LOD pyramid
    // via `maxRenderTriangles`, so high sim accuracy no
    // longer forces a coarse mesh.
    // In the in-browser wasm trial the sim is single-threaded on
    // the main thread, so clamp harder to keep rebuild + scrub smooth.
    const isWasm = isWasmTransport();
    const simCellCap = effectiveSimCellCap(input.settings.maxSimulationCells, isWasm);
    let effectiveCellSize = cellSize;
    let coarsened = false;
    if (cellCount > simCellCap) {
      const scale = Math.sqrt(cellCount / simCellCap);
      effectiveCellSize = cellSize * scale;
      coarsened = true;
    }
    // Stock top plane Z (default 0). Carving descends from here;
    // the floor is topZ − thickness. Toolpath Z is absolute (WCS), so a
    // raised top models zeroing below the stock surface.
    const topZ = input.stock.offsetZ ?? 0;
    // Stepped voxel renderer needs a finite floor — match the physical
    // stock bottom so the rendered boxes have the right thickness when
    // viewed from below. Default to 10 mm so an unconfigured project
    // still has a visible stock height.
    const stockThickness = input.stock.thickness > 0 ? input.stock.thickness : 10.0;
    // Explicit span floor for the dexel field — the physical stock bottom.
    // Form (T-slot / dovetail) tools carve undercuts relative to it; 3-axis
    // jobs never reach it and stay on the dense top-down fast path.
    const stockBottomZ = topZ - stockThickness;
    this.stockBottomZ = stockBottomZ;
    this.dispose();
    this.sim = new this.wasm.Simulator(
      fp.minX,
      fp.minY,
      fp.maxX,
      fp.maxY,
      effectiveCellSize,
      topZ,
      stockBottomZ,
    );
    // Surface the coarsening as a sim warning so it shows up in
    // the diagnostics panel — silently coarsening the grid hid
    // tool-engagement and small-feature issues from the user.
    // But ONLY when it's user-config-driven (a low
    // maxSimulationCells the user can raise). The wasm-trial cap
    // coarsens by design and isn't user-actionable — warning there just
    // floods the diagnostics window (it's a sticky warning re-emitted
    // every advance), so stay silent in the trial.
    if (coarsened && !isWasm && typeof this.sim.push_cell_size_coarsened === 'function') {
      this.sim.push_cell_size_coarsened(cellSize, effectiveCellSize, 'max_simulation_cells');
    }
    // Pick the lowest LOD level whose mesh fits the user's
    // `maxRenderTriangles` budget. Skip building finer (heavier)
    // levels so total GPU memory stays predictable.
    const simCols = this.sim.cols();
    const simRows = this.sim.rows();
    const minLevel = pickMinLodLevelForBudget(simCols, simRows, input.settings.maxRenderTriangles);
    // The front mesh always floors at the true stock bottom. For a two-sided
    // job the block below installs a per-cell floor (the reflected back
    // surface) so this ONE mesh spans the front carve (top) down to the back
    // carve (floor) as a single watertight solid — no second reflected mesh,
    // and a front cut past the mid-plane shows its true depth instead of
    // clamping at a constant seam.
    const twoSided = input.generatedBack != null;
    this.mesh = new HeightfieldMeshPyramid(
      {
        cols: simCols,
        rows: simRows,
        cellSize: this.sim.cell_size(),
        originX: this.sim.origin_x(),
        originY: this.sim.origin_y(),
        topZ: this.sim.top_z(),
        floorZ: stockBottomZ,
        solidColor: input.settings.solidColor,
        solidOpacity: input.settings.solidOpacity,
        edgeColor: input.settings.edgeColor,
        edgeOpacity: input.settings.edgeOpacity,
      },
      3,
      minLevel,
    );
    this.group.add(this.mesh.group);
    if (input.fixtures && input.fixtures.length > 0) {
      this.sim.set_fixtures(input.fixtures);
    } else {
      this.sim.set_fixtures([]);
    }
    // Cache the toolpath on the WASM side ONCE per Generate so per-frame
    // advance() doesn't re-deserialize the whole segment array.
    // The cached toolpath is the reference identity tracked
    // by `cachedToolpath` below.
    this.sim.set_toolpath(input.generated.toolpath);
    this.cachedToolpath = input.generated.toolpath;
    this.resetCheckpoints(input.generated.toolpath.length);
    this.appliedSeg = 0;
    this.partialT = 0;
    this.diagnostics = { warnings: [] };
    this.notifyDiagnostics();
    // Cleared here so a single-sided rebuild drops any stale two-sided
    // markers; the two-sided branch below recomputes them.
    this.conflicts = [];
    // Match the void surfaces to the dense mesh's stock material so a
    // cavity reads as the same carved solid.
    this.undercut?.setStyle({
      solidColor: input.settings.solidColor,
      solidOpacity: input.settings.solidOpacity,
    });
    this.refreshHeightView();
    this.refreshUndercutMesh();
    // Re-arm the deviation overlay on the fresh sim/mesh if it's active.
    if (this.deviationTargets.length > 0) {
      this.sim.set_deviation_target(this.deviationTargets, this.sim.top_z(), this.deviationTolMm);
      this.applyDeviation();
    }

    // Two-sided: build (or rebuild) the reflected back surface, carving the
    // back program with its own per-op tools at the SAME cell size as the
    // front so the two grids align. `dispose()` above already dropped any
    // stale back, so this creates a fresh one; a single-sided rebuild simply
    // skips it (leaving `this.back` null).
    if (twoSided && input.generatedBack && input.toolForSegBack && this.wasm) {
      this.back = new BackSurface(this.wasm);
      this.back.build({
        backGenerated: input.generatedBack,
        toolForSeg: input.toolForSegBack,
        fp,
        cellSize: this.sim.cell_size(),
        topZ: this.sim.top_z(),
        thickness: stockThickness,
      });
      // Fold the finished back carve into the front mesh as its per-cell
      // floor (reflected about the mid-plane) so the ONE front mesh renders
      // the whole two-sided solid, then repaint against the new floor.
      this.applyBackFloor(this.sim.top_z(), stockThickness);
      // Flag columns where the finished front + back carves cross the stock.
      this.conflicts = this.computeTwoSidedConflicts({
        frontGenerated: input.generated,
        frontToolForSeg: input.toolForSeg ?? (() => input.tool as ToolEntry),
        fp,
        cellSize: this.sim.cell_size(),
        topZ: this.sim.top_z(),
        thickness: stockThickness,
      });
    }
  }

  /// World-anchored two-sided conflict markers for the current build —
  /// clusters of columns where the finished front carve and the reflected
  /// back carve overlap or cut clean through. Empty for a single-sided job
  /// or before the first build. Scene3D reads this right after `build()`.
  getTwoSidedConflicts(): ConflictMarker[] {
    return this.conflicts;
  }

  /// Install the finished back carve as the front mesh's per-cell floor,
  /// reflected about the stock mid-plane: `floor[i] = 2·midPlane −
  /// back[i]`. The front mesh then spans the front carve (top) down to the
  /// back carve (floor) as one watertight solid. A full `updateHeights`
  /// follows so tops re-clamp against the new floor and the underside
  /// steps close. Bails on a grid mismatch (the two sims share fp +
  /// cellSize, so this is a defensive guard, not an expected path).
  private applyBackFloor(topZ: number, thickness: number): void {
    if (!this.mesh || !this.sim || !this.back) return;
    const back = this.back.finalHeights();
    if (!back) return;
    if (back.cols !== this.sim.cols() || back.rows !== this.sim.rows()) return;
    const offset = backReflectionOffsetZ(topZ, thickness);
    const floor = new Float32Array(back.heights.length);
    for (let i = 0; i < floor.length; i++) floor[i] = offset - back.heights[i];
    this.mesh.setFloor(floor);
    if (this.heightView) this.mesh.updateHeights(this.heightView);
  }

  /// Detect two-sided conflicts by pairing the finished BACK heightfield
  /// (already carved to completion in `this.back`) against a FINISHED front.
  /// The live `this.sim` sits at the playhead, not the program end, so this
  /// carves a THROWAWAY front sim to completion on the SAME grid + floor.
  /// One extra full front carve per two-sided Generate (a rare, debounced
  /// user action) — negligible next to generation itself.
  private computeTwoSidedConflicts(p: {
    frontGenerated: GenerateResponse;
    frontToolForSeg: (segIdx: number) => ToolEntry;
    fp: { minX: number; minY: number; maxX: number; maxY: number };
    cellSize: number;
    topZ: number;
    thickness: number;
  }): ConflictMarker[] {
    if (!this.wasm || !this.back) return [];
    const back = this.back.finalHeights();
    if (!back) return [];
    const stockBottomZ = p.topZ - p.thickness;
    const sim = new this.wasm.Simulator(
      p.fp.minX,
      p.fp.minY,
      p.fp.maxX,
      p.fp.maxY,
      p.cellSize,
      p.topZ,
      stockBottomZ,
    );
    try {
      // The conflict is a property of the finished part, so the front's
      // fixtures (a collision concern, not a material one) don't apply.
      sim.set_fixtures([]);
      sim.set_toolpath(p.frontGenerated.toolpath);
      carveProgramToCompletion(sim, p.frontGenerated.toolpath.length, p.frontToolForSeg);
      const cols = sim.cols();
      const rows = sim.rows();
      // Grid mismatch would misalign the two fields — bail rather than
      // compare across incompatible layouts.
      if (cols !== back.cols || rows !== back.rows) return [];
      const live = new Float32Array(this.wasm.memory.buffer, sim.data_ptr(), cols * rows);
      const frontFinal = live.slice();
      return detectTwoSidedConflicts(frontFinal, back.heights, {
        cols,
        rows,
        cellSize: sim.cell_size(),
        originX: sim.origin_x(),
        originY: sim.origin_y(),
        topZ: sim.top_z(),
        thickness: p.thickness,
      });
    } finally {
      sim.free();
    }
  }

  /// Drop any heightmap/diagnostics checkpoints and size the checkpoint
  /// interval for a `total`-segment program. Spacing is chosen so at most
  /// `MAX_CHECKPOINTS` snapshots exist, bounding memory (each heightmap
  /// snapshot is cols·rows·4 bytes) regardless of program length. Small
  /// programs (≤ MIN_SEGMENTS_FOR_CHECKPOINTS) disable it — a full replay
  /// is already cheap there.
  private resetCheckpoints(total: number) {
    this.sim?.clear_checkpoints();
    this.diagCheckpoints.clear();
    this.lastCheckpointSeg = 0;
    this.checkpointInterval =
      total > MIN_SEGMENTS_FOR_CHECKPOINTS ? Math.ceil(total / MAX_CHECKPOINTS) : 0;
  }

  /// Snapshot the heightfield + diagnostics at the current clean segment
  /// boundary, if checkpointing is on and we've advanced far enough past
  /// the last snapshot. Called after a forward advance settles. Only
  /// snapshots at `partialT === 0` so the heightfield is exactly
  /// `[0, appliedSeg)` carved — a valid replay base.
  private maybeCheckpoint() {
    if (!this.sim || this.checkpointInterval <= 0) return;
    if (this.partialT !== 0 || this.appliedSeg <= 0) return;
    if (this.appliedSeg < this.lastCheckpointSeg + this.checkpointInterval) return;
    this.sim.checkpoint(this.appliedSeg);
    // Clone the warning list so later mutations of `this.diagnostics`
    // don't leak into the snapshot.
    this.diagCheckpoints.set(this.appliedSeg, { warnings: [...this.diagnostics.warnings] });
    this.lastCheckpointSeg = this.appliedSeg;
  }

  /// Replace the simulator's fixture set without rebuilding the mesh.
  /// Triggers a reset so the next advanceTo() call replays from segment
  /// 0 with the new obstacle list.
  setFixtures(fixtures: Fixture[]) {
    if (!this.sim) return;
    this.sim.set_fixtures(fixtures);
    this.sim.reset();
    // Checkpoint diagnostics snapshots captured fixture-collision
    // warnings against the OLD fixture set — drop them (the heightmap
    // snapshots are geometrically fine, but a stale diagnostics seed
    // would resurrect wrong collisions). Interval is unchanged.
    this.sim.clear_checkpoints();
    this.diagCheckpoints.clear();
    this.lastCheckpointSeg = 0;
    this.appliedSeg = 0;
    this.partialT = 0;
    this.diagnostics = { warnings: [] };
    this.notifyDiagnostics();
    // Clear the LOD pyramid's coarse pools so the next
    // updateHeights doesn't pick up stale carved data from before
    // the reset.
    this.mesh?.reset();
    this.refreshHeightView();
    this.refreshUndercutMesh();
  }

  /// Subscribe to diagnostics changes. Called with the current snapshot
  /// after every forward advance() that returns warnings, and after
  /// reset/dispose so listeners can clear stale UI markers.
  onDiagnostics(cb: (d: SimDiagnostics) => void) {
    this.onDiagnosticsChange = cb;
    cb(this.diagnostics);
  }

  /// Latest cumulative diagnostics snapshot. The UI may also subscribe
  /// via `onDiagnostics` for a push-based update.
  getDiagnostics(): SimDiagnostics {
    return this.diagnostics;
  }

  /// Advance the simulation to `headFraction` (a number in [0, 1] —
  /// project.playhead). Returns true if the mesh was modified.
  ///
  /// `cumLen` / `totalLen` are the arc-length cumulative table built
  /// alongside the toolpath; we map `headFraction` to `(segIdx, segT)`
  /// using arc length so the heightfield destruction tracks the 3D
  /// tool mesh and the gcode panel (which also use arc-length mapping).
  ///
  /// Carving happens at sub-segment resolution: segment `appliedSeg` is
  /// carved up to `partialT` and segments `[0, appliedSeg)` are fully
  /// done. Forward steps issue a small `partial_advance` to
  /// extend the in-flight segment, finalize it when crossing a
  /// boundary, and bulk-`advance` any skipped segments in between.
  advanceTo(
    headFraction: number,
    segments: ToolpathSegment[],
    /// Resolves the cutting tool for a given toolpath segment index. The
    /// sim must carve each segment with ITS op's tool (a v-bit cuts a V,
    /// an endmill a cylinder) — feeding one tool for the whole program
    /// made multi-op runs carve with the wrong cross-section. A bare
    /// ToolEntry is accepted for single-tool callers / tests.
    toolForSeg: ToolEntry | ((segIdx: number) => ToolEntry),
    cumLen?: Float64Array | null,
    totalLen?: number,
    /// When false (the default), a backward scrub leaves the
    /// heightfield untouched — cells retain their deepest-ever
    /// cuts, and the cursor (`appliedSeg` / `partialT`) does not
    /// move backward, so subsequent forward scrubs resume from the
    /// previous max position. When true, the driver runs the
    /// reset + forward-replay path so the heightfield exactly
    /// reflects the cuts up to the new playhead. Replay is O(N)
    /// in segments-replayed and currently has a known visual
    /// artifact on chunked / LOD meshes, so it lives behind
    /// a Settings toggle.
    exactRewind: boolean = false,
  ): boolean {
    const resolveTool =
      typeof toolForSeg === 'function' ? toolForSeg : () => toolForSeg as ToolEntry;
    if (!this.sim || !this.mesh) return false;
    const total = segments.length;
    if (total === 0) return false;

    let segIdx: number;
    let segT: number;
    if (cumLen && cumLen.length === total && totalLen && totalLen > 0) {
      const r = playheadToSegment(headFraction, cumLen, totalLen);
      if (r.segIdx < 0) return false;
      segIdx = r.segIdx;
      segT = r.segT;
    } else {
      const clamped = Math.max(0, Math.min(1, headFraction));
      const c = clamped * total;
      segIdx = Math.min(total - 1, Math.floor(c));
      segT = c - segIdx;
    }

    // On a backward scrub with exact rewind, find the nearest heightmap
    // checkpoint ≤ the target so the replay restores it and re-simulates
    // only the tail instead of the whole `[0, target]` prefix. `-1` (no
    // usable checkpoint) falls back to replay-from-0.
    const backward =
      segIdx < this.appliedSeg || (segIdx === this.appliedSeg && segT < this.partialT);
    let replayFrom = 0;
    if (backward && exactRewind && this.checkpointInterval > 0) {
      const nearest = this.sim.nearest_checkpoint(segIdx);
      if (nearest > 0) replayFrom = nearest;
    }

    const plan = planAdvance(this.appliedSeg, this.partialT, segIdx, segT, total, replayFrom);
    if (!plan) return false;

    // A backward scrub asks the planner to emit `reset: true`
    // plus a forward-replay sequence from t=0 to the new playhead.
    // Default behavior (exactRewind=false) skips the whole advance:
    // the heightmap is forward-monotone (cuts only deepen), so
    // leaving the cells alone shows the deepest-ever state and
    // subsequent forward scrubs correctly resume from the previous
    // max position. exactRewind=true runs the reset + replay so
    // the visible heights track the playhead — slow on long
    // programs and currently has a chunked-mesh artifact,
    // which is why it lives behind a Settings toggle.
    if (plan.reset && !exactRewind) {
      // Leave appliedSeg / partialT untouched. The heightmap and
      // mesh stay at the previous max-deep state. The PlaybackBar's
      // hint tells the user this is the cheap path.
      return false;
    }

    // Backward scrub WITH exactRewind: rewind the simulator and let the
    // planner's forward ops replay. When a checkpoint at `replayFrom` is
    // available we restore that heightfield (and seed diagnostics from
    // the matching snapshot) so only `[replayFrom, target]` is
    // re-simulated; otherwise we fall back to a full reset + replay from
    // 0. Either way the visible heightfield base is rebuilt BEFORE the
    // forward ops, else cells outside the replay's dirty AABB keep the
    // stale (deeper) heights from the previous playhead.
    if (plan.reset) {
      if (replayFrom > 0 && this.sim.restore_checkpoint(replayFrom)) {
        // Seed diagnostics from the checkpoint so the rewound prefix's
        // warnings survive; the replayed tail adds the rest (deduped).
        const seeded = this.diagCheckpoints.get(replayFrom);
        this.diagnostics = seeded ? { warnings: [...seeded.warnings] } : { warnings: [] };
      } else {
        this.sim.reset();
        this.diagnostics = { warnings: [] };
      }
      this.notifyDiagnostics();
      // Clear coarse pool data before the forward replay so the active
      // LOD level redraws from the restored (or uncut) base.
      this.mesh?.reset();
      this.refreshHeightView();
      if (this.heightView && this.mesh) this.mesh.updateHeights(this.heightView);
    }

    // Wire-tool cache (by tool id) so a multi-segment run doesn't
    // re-serialize the same tool spec repeatedly.
    const wireCache = new Map<number, Record<string, unknown>>();
    const wireFor = (segIdx: number): Record<string, unknown> => {
      const t = resolveTool(segIdx);
      let w = wireCache.get(t.id);
      if (!w) {
        w = toWireTool(t);
        wireCache.set(t.id, w);
      }
      return w;
    };
    // Defensive re-cache if the toolpath identity drifts from the
    // build()-time snapshot (e.g. a Generate response replaced
    // `project.gen.generated.toolpath` without going through build()).
    // The common path is a no-op compare.
    if (segments !== this.cachedToolpath) {
      this.sim.set_toolpath(segments);
      this.cachedToolpath = segments;
      // The toolpath changed under us — prior checkpoints snapshot a
      // different program and must not be restored.
      this.resetCheckpoints(segments.length);
    }

    let unionAabb: [number, number, number, number] | null = null;
    const unionWith = (a: Uint32Array | number[]) => {
      if (a.length !== 4) return;
      if (!unionAabb) {
        unionAabb = [a[0], a[1], a[2], a[3]];
      } else {
        if (a[0] < unionAabb[0]) unionAabb[0] = a[0];
        if (a[1] < unionAabb[1]) unionAabb[1] = a[1];
        if (a[2] > unionAabb[2]) unionAabb[2] = a[2];
        if (a[3] > unionAabb[3]) unionAabb[3] = a[3];
      }
    };

    if (plan.finalizePartial) {
      const { segIdx: fIdx, fromT } = plan.finalizePartial;
      unionWith(this.sim.partial_advance(wireFor(fIdx), fIdx, fromT, 1));
      this.collectDiagnostics();
    }
    if (plan.bulkAdvance) {
      // Split the contiguous range into runs of consecutive segments that
      // share a tool (ops are contiguous in the toolpath), advancing each
      // run with its own tool so per-op cutter shapes carve correctly.
      const { from, to } = plan.bulkAdvance;
      let runStart = from;
      while (runStart < to) {
        const runToolId = resolveTool(runStart).id;
        let runEnd = runStart + 1;
        while (runEnd < to && resolveTool(runEnd).id === runToolId) runEnd++;
        unionWith(this.sim.advance(wireFor(runStart), runStart, runEnd));
        runStart = runEnd;
      }
      this.collectDiagnostics();
    }
    if (plan.startPartial) {
      const { segIdx: sIdx, startT, endT } = plan.startPartial;
      unionWith(this.sim.partial_advance(wireFor(sIdx), sIdx, startT, endT));
      this.collectDiagnostics();
    }

    this.appliedSeg = plan.newAppliedSeg;
    this.partialT = plan.newPartialT;

    // Drop a heightmap + diagnostics checkpoint if forward progress has
    // crossed the interval at a clean segment boundary — this is what
    // makes a later backward scrub cheap.
    this.maybeCheckpoint();

    // Re-take the buffer view: any advance / partial_advance call may
    // have grown WASM linear memory and detached the prior view.
    this.refreshHeightView();
    if (this.heightView) {
      const a = unionAabb as [number, number, number, number] | null;
      // After a reset-driven backstep replay, do a FULL mesh
      // re-upload regardless of how small the forward replay's
      // dirty AABB came out. The partial-AABB path leaves cells
      // outside the AABB at whatever the pre-replay state was —
      // correct ON THE FIRST FRAME after the reset's full upload
      // (those cells are topZ), but defense-in-depth against the
      // chunked / LOD pyramid possibly retaining stale pool data
      // in some configuration. The cost is one extra full upload
      // per backstep, which the user opted into via the
      // Settings exact-rewind toggle.
      if (a && !plan.reset) {
        this.mesh.updateHeights(this.heightView, {
          ix0: a[0],
          iy0: a[1],
          ix1: a[2],
          iy1: a[3],
        });
      } else {
        this.mesh.updateHeights(this.heightView);
      }
      // Repaint the deviation overlay over the same dirty region (full on a
      // reset-driven replay, matching the mesh re-upload above). No-op when
      // the overlay is off.
      if (this.deviationTargets.length > 0) {
        this.applyDeviation(
          a && !plan.reset ? { ix0: a[0], iy0: a[1], ix1: a[2], iy1: a[3] } : undefined,
        );
      }
    }

    // Re-mesh the undercut voids from the (re-flattened) CSR sidecar. Uses
    // the just-refreshed `heightView` as the dense-top array; a no-op for
    // 3-axis jobs (empty sidecar).
    this.refreshUndercutMesh();

    this.scheduleEdgeRebuild();
    this.opts.requestRender();
    return true;
  }

  setVisible(visible: boolean) {
    this.group.visible = visible;
  }

  setSolidVisible(visible: boolean) {
    this.mesh?.setSolidVisible(visible);
    // Void surfaces are part of the solid stock — show them with it.
    this.undercut?.setVisible(visible);
    // Two-sided: the back is folded into the front mesh's floor, so the front
    // mesh's visibility already covers it — no separate back mesh to toggle.
  }

  setEdgesVisible(visible: boolean) {
    this.mesh?.setEdgesVisible(visible);
  }

  /// Live-apply settings changes (color / opacity). Resolution / max
  /// cells changes require a full rebuild via build().
  applyStyle(
    settings: Pick<AppSettings, 'solidColor' | 'solidOpacity' | 'edgeColor' | 'edgeOpacity'>,
  ) {
    this.mesh?.setStyle({
      solidColor: settings.solidColor,
      solidOpacity: settings.solidOpacity,
      edgeColor: settings.edgeColor,
      edgeOpacity: settings.edgeOpacity,
    });
    this.undercut?.setStyle({
      solidColor: settings.solidColor,
      solidOpacity: settings.solidOpacity,
    });
    // The back is folded into the front mesh's floor, so the front mesh's
    // setStyle (above) already restyles the whole two-sided solid.
    this.opts.requestRender();
  }

  /// Turn the target-surface deviation overlay on or off. Pass the target
  /// `SurfaceField`s (from `deviationTargets` — one per enabled relief op,
  /// unioned deepest-cut-wins) and the on-target tolerance to enable it; pass
  /// `[]` to clear it and restore the stock color. The targets are cached on
  /// the WASM sim so per-carve refreshes stay cheap, then the whole terrain is
  /// repainted once from the current carve state. Safe to call before build()
  /// — the pending targets are applied when the sim/mesh come up.
  setDeviationTarget(targets: SurfaceField[], toleranceMm: number) {
    this.deviationTargets = targets;
    this.deviationTolMm = toleranceMm;
    if (!this.sim || !this.mesh) return;
    if (targets.length > 0) {
      this.sim.set_deviation_target(targets, this.sim.top_z(), toleranceMm);
      this.applyDeviation();
    } else {
      this.sim.clear_deviation_target();
      this.deviationView = null;
      this.mesh.setDeviation(null);
    }
    this.opts.requestRender();
  }

  /// Repaint the deviation overlay from the current carve state. `aabb`
  /// restricts BOTH the Rust reclassify and the mesh re-upload to the dirty
  /// rectangle: only cells the tool just swept get re-sampled against the
  /// target, every other cell keeps its still-correct class in the sim's
  /// persistent buffer (mirroring the mesh's partial re-upload). Omit `aabb`
  /// for a full repaint (build / reset / target change). A no-op when the
  /// overlay is off.
  private applyDeviation(aabb?: { ix0: number; iy0: number; ix1: number; iy1: number }) {
    if (this.deviationTargets.length === 0 || !this.sim || !this.mesh || !this.wasm) return;
    if (aabb) {
      this.sim.deviation_recompute_in(aabb.ix0, aabb.iy0, aabb.ix1, aabb.iy1);
    } else {
      this.sim.deviation_recompute();
    }
    const len = this.sim.deviation_len();
    if (len === 0) return;
    // Re-take the zero-copy view: the preceding advance/recompute may have
    // grown WASM memory and detached any prior view (same as `heightView`).
    this.deviationView = new Uint8Array(this.wasm.memory.buffer, this.sim.deviation_ptr(), len);
    this.mesh.setDeviation(this.deviationView, aabb);
  }

  /// Drive the LOD pyramid's active level. Caller is Scene3D's
  /// render loop, which feeds `pixelsPerL0Cell` from the camera
  /// projection and `maxRenderTriangles` from settings; the pyramid
  /// picks the coarser of the distance- and budget-recommended
  /// levels. Returns the level actually applied so the caller can
  /// debug-log or display it.
  ///
  /// Distance hysteresis: switching to a COARSER level uses the
  /// `1.0 px / cell` threshold; switching back to a FINER level
  /// requires `1.2 px / cell` (20% gap). Without this, a tiny pan
  /// near the threshold would oscillate the active level every
  /// frame.
  setLodHint(pixelsPerL0Cell: number, maxRenderTriangles: number): number {
    if (!this.mesh) return 0;
    const current = this.mesh.getActiveLevel();
    const budgetLevel = this.mesh.recommendBudgetLevel(maxRenderTriangles);
    const coarsenLevel = this.mesh.recommendDistanceLevel(pixelsPerL0Cell, 1.0);
    const finerLevel = this.mesh.recommendDistanceLevel(pixelsPerL0Cell, 1.2);
    const coarsenTarget = Math.max(coarsenLevel, budgetLevel);
    const finerTarget = Math.max(finerLevel, budgetLevel);
    let next = current;
    if (coarsenTarget > current) {
      next = coarsenTarget;
    } else if (finerTarget < current) {
      next = finerTarget;
    }
    if (next !== current) {
      this.mesh.setActiveLevel(next);
      // The back is the front mesh's per-cell floor, so the pyramid re-pools
      // and installs it for the new level inside setActiveLevel — nothing
      // separate to swap here.
      // The new level's EdgesGeometry is stale — schedule a rebuild
      // through the same trailing-debounce path that handles carves
      // so a pan that crosses LOD thresholds doesn't stall on a
      // synchronous edge rebuild.
      this.scheduleEdgeRebuild();
      this.opts.requestRender();
    }
    return this.mesh.getActiveLevel();
  }

  /// Current active LOD level (or `null` when no mesh exists). Used
  /// by Scene3D for the debug overlay.
  getLodLevel(): number | null {
    return this.mesh ? this.mesh.getActiveLevel() : null;
  }

  /// L0 cell-size in mm so the camera-distance LOD heuristic can
  /// project a single cell to screen pixels.
  getCellSize(): number | null {
    return this.sim ? this.sim.cell_size() : null;
  }

  /// Serialize the carved heightfield as a binary STL. Returns
  /// `null` when there is no live simulator (no project loaded yet, or
  /// the driver was disposed). Walls drop to `stockBottomZ` at every
  /// perimeter sample for a watertight mesh.
  exportStl(stockBottomZ: number): Uint8Array | null {
    return this.sim ? this.sim.export_stl(stockBottomZ) : null;
  }

  /// Serialize the carved stock as a watertight voxel-solid binary STL
  /// (stair-stepped top, but hole-free through undercut voids — for slicer /
  /// boolean consumers). Returns `null` when there is no live simulator. The
  /// solid's floor is intrinsic to the field, so no `stockBottomZ` is needed.
  exportStlSolid(): Uint8Array | null {
    return this.sim ? this.sim.export_stl_solid() : null;
  }

  dispose() {
    if (this.mesh) {
      this.group.remove(this.mesh.group);
      this.mesh.dispose();
      this.mesh = null;
    }
    if (this.sim) {
      this.sim.free();
      this.sim = null;
    }
    // Tear down the two-sided back surface (frees its sim + mesh and detaches
    // its reflected group from `this.group`). Recreated by the next two-sided
    // build().
    if (this.back) {
      this.back.dispose();
      this.back = null;
    }
    this.conflicts = [];
    // Drop any void geometry but keep the builder alive for the next
    // build() (it's created once in the constructor).
    this.undercut?.clear();
    this.lastUndercutCount = 0;
    this.heightView = null;
    this.deviationView = null;
    this.appliedSeg = 0;
    this.partialT = 0;
    this.cachedToolpath = null;
    this.diagCheckpoints.clear();
    this.lastCheckpointSeg = 0;
    this.checkpointInterval = 0;
    if (this.diagnostics.warnings.length > 0) {
      this.diagnostics = { warnings: [] };
      this.notifyDiagnostics();
    }
  }

  destroy() {
    if (this.edgeRebuildTimer != null) {
      clearTimeout(this.edgeRebuildTimer);
      this.edgeRebuildTimer = null;
    }
    this.dispose();
    // Full teardown of the void builder (frees its material + detaches its
    // group from `this.group`) before the group leaves the scene.
    this.undercut?.dispose();
    this.undercut = null;
    this.opts.scene.remove(this.group);
    // Deregister so a stale handle can't be reached.
    if (currentDriver === this) currentDriver = null;
  }

  private collectDiagnostics() {
    if (!this.sim) return;
    const fresh = this.sim.take_diagnostics();
    if (!fresh || !Array.isArray(fresh.warnings) || fresh.warnings.length === 0) return;
    // Dedupe against what's already accumulated: the sim re-emits sticky
    // warnings (cell_size_coarsened) every advance and re-fires segment
    // warnings on scrub-back, which otherwise pile up duplicate rows and
    // flood the window (and inflate the critical count).
    const seen = new Set(this.diagnostics.warnings.map(simWarningKey));
    const added = fresh.warnings.filter((w) => {
      const k = simWarningKey(w);
      if (seen.has(k)) return false;
      seen.add(k);
      return true;
    });
    if (added.length === 0) return;
    this.diagnostics = {
      warnings: [...this.diagnostics.warnings, ...added],
    };
    this.notifyDiagnostics();
  }

  private notifyDiagnostics() {
    this.onDiagnosticsChange?.(this.diagnostics);
  }

  private refreshHeightView() {
    if (!this.wasm || !this.sim) {
      this.heightView = null;
      return;
    }
    const cols = this.sim.cols();
    const rows = this.sim.rows();
    this.heightView = new Float32Array(this.wasm.memory.buffer, this.sim.data_ptr(), cols * rows);
  }

  /// Re-mesh the undercut voids from the sim's CSR sidecar. Called after
  /// every carve (right after `refreshHeightView`, whose `heightView` this
  /// reuses as the dense-top array). Reads the WASM CSR buffers into a plain
  /// snapshot synchronously — the views are valid until the next advance —
  /// and hands it to the void builder. A pure 3-axis job's sidecar is empty,
  /// so this clears once and then stays a cheap early return.
  private refreshUndercutMesh() {
    if (!this.wasm || !this.sim || !this.undercut) return;
    const count = this.sim.undercut_column_count();
    if (count === 0) {
      // Only touch the mesh on the empty transition; otherwise every
      // 3-axis frame would rebuild empty geometry.
      if (this.lastUndercutCount !== 0) {
        this.undercut.clear();
        this.lastUndercutCount = 0;
      }
      return;
    }
    const top = this.heightView;
    if (!top) return;
    const mem = this.wasm.memory.buffer;
    const colIndex = new Uint32Array(
      mem,
      this.sim.undercut_col_index_ptr(),
      this.sim.undercut_col_index_len(),
    );
    const spanOffsets = new Uint32Array(
      mem,
      this.sim.undercut_span_offsets_ptr(),
      this.sim.undercut_span_offsets_len(),
    );
    const spans = new Float32Array(
      mem,
      this.sim.undercut_spans_ptr(),
      this.sim.undercut_spans_len(),
    );
    this.undercut.build({
      cols: this.sim.cols(),
      rows: this.sim.rows(),
      cellSize: this.sim.cell_size(),
      originX: this.sim.origin_x(),
      originY: this.sim.origin_y(),
      topZ: this.sim.top_z(),
      stockBottomZ: this.stockBottomZ,
      top,
      colIndex,
      spanOffsets,
      spans,
    });
    this.lastUndercutCount = count;
  }

  private scheduleEdgeRebuild() {
    if (!this.mesh) return;
    // Bail above the budget — too expensive to rebuild and visually
    // useless at that density. The LOD pyramid normally swaps to a
    // coarser level long before this, but the cap protects against
    // a user-configured `maxRenderTriangles` that pushes L0 over it.
    if (this.mesh.getActiveTriangleCount() > HeightfieldDriver.EDGE_MAX_TRIANGLES) {
      if (this.edgeRebuildTimer != null) {
        clearTimeout(this.edgeRebuildTimer);
        this.edgeRebuildTimer = null;
      }
      return;
    }
    // Pure trailing debounce: reset the timer on every call so the
    // rebuild only fires after EDGE_REBUILD_MS of quiet. Continuous
    // playback at 60 fps never sees it; idle frames after the user
    // stops scrubbing do.
    if (this.edgeRebuildTimer != null) clearTimeout(this.edgeRebuildTimer);
    this.edgeRebuildTimer = setTimeout(() => {
      this.edgeRebuildTimer = null;
      if (!this.mesh) return;
      // Re-check the cap in case the active level changed since the
      // timer was armed.
      if (this.mesh.getActiveTriangleCount() > HeightfieldDriver.EDGE_MAX_TRIANGLES) return;
      this.mesh.rebuildEdges();
      this.opts.requestRender();
    }, HeightfieldDriver.EDGE_REBUILD_MS);
  }
}
