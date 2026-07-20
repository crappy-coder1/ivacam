// Global project state, Svelte 5 runes.
// Holds the most recently imported geometry plus UI flags.

import type {
  GenerateResponse,
  TwoSidedGenerateResponse,
  ImportResponse,
  Segment,
  SimDiagnostics,
  WiacError,
} from '../api/types';
import { History } from './history';
import { computeUnsavedWork } from './unsaved';
import { invalidatePreview, previewSegmentsFor, previewVersion } from './text_preview.svelte';
import { GeneratedState, type PipelineNoteEvent } from './generated.svelte';
import { SelectionState, type PickMode, type SelectionMode } from './selection.svelte';

export type { PickMode };
import {
  ProjectDataState,
  DEFAULT_SETTINGS,
  saveSettings,
  type AppSettings,
} from './project-data.svelte';

export { DEFAULT_SETTINGS };
export type { AppSettings };
// Bring the union types into scope locally; project-types and op_types
// re-export them through this module so callers can import them here too.
import type { OpEntry, OpKind } from './op_types';
import * as importOps from './import-ops';
import * as fileOps from './project-file-ops';
import * as machineOps from './project-machine-ops';
import * as selectionOps from './project-selection-ops';

// Pure-TypeScript data shapes live in project-types.ts so vitest specs
// and non-Svelte helpers can import them without booting the rune
// runtime. They're re-exported below for backwards-compat
// with the 40+ call sites that already import from this module.
import {
  DEFAULT_FIXTURE_COLOR,
  defaultAxesConfig,
  defaultFixtureName,
  identityFileTransform,
  isIdentityFileTransform,
  prettyOpKind,
} from './project-types';
import type {
  AxesConfig,
  AxisFormat,
  AxisLimits,
  CoolantMode,
  CutDirection,
  DrillCycle,
  FileTransform,
  Fixture,
  FixtureKind,
  FormProfileSample,
  HalfpipeProfile,
  HolderShape,
  ImportEntry,
  MachineSettings,
  PatternConfig,
  PlungeStrategy,
  PocketStrategy,
  PostProfile,
  ProjectFile,
  ReliefSource,
  SpindleDirection,
  StockConfig,
  TabPlacement,
  TabPlacementMode,
  TextAlignment,
  TextFontSource,
  TextLayer,
  TextLayerKind,
  ToolEntry,
  Wcs,
  WorkOffset,
} from './project-types';
import { defaultWorkOffset, isDefaultWorkOffset } from './project-types';
import { combineImports } from './file-transform';

export {
  DEFAULT_FIXTURE_COLOR,
  defaultAxesConfig,
  defaultFixtureName,
  defaultWorkOffset,
  identityFileTransform,
  isDefaultWorkOffset,
  isIdentityFileTransform,
  prettyOpKind,
};
export type {
  AxesConfig,
  AxisFormat,
  AxisLimits,
  CoolantMode,
  CutDirection,
  DrillCycle,
  FileTransform,
  Fixture,
  FixtureKind,
  FormProfileSample,
  HalfpipeProfile,
  HolderShape,
  ImportEntry,
  MachineSettings,
  PatternConfig,
  PlungeStrategy,
  PocketStrategy,
  PostProfile,
  ProjectFile,
  ReliefSource,
  SpindleDirection,
  StockConfig,
  TabPlacement,
  TabPlacementMode,
  TextAlignment,
  TextFontSource,
  TextLayer,
  TextLayerKind,
  ToolEntry,
  Wcs,
  WorkOffset,
};
// OpEntry union + variant types live in op_types.ts; re-export from
// here for backwards-compat with call sites that imported them from
// state/project.svelte.
export type {
  ChamferOp,
  ContourFields,
  CycleMarkerOp,
  DragKnifeOp,
  DrillOp,
  EngraveOp,
  FrameShape,
  GcodeIncludeOp,
  HomingOp,
  LeadKind,
  OpBase,
  OpEntry,
  OpField,
  OpFieldValue,
  OpKind,
  OpOfKind,
  OpPatch,
  PauseOp,
  PocketOp,
  PowerCurve,
  PowerCurveKind,
  ProbeOp,
  ProfileOffset,
  ProfileOp,
  RasterEngraveOp,
  RasterLink,
  ReliefMillOp,
  ScanDirection,
  SourceCombine,
  TabType,
  ThreadOp,
  ToolKind,
  VCarveOp,
  WaterlineRoughOp,
} from './op_types';
export { isContourOp, isPathOp } from './op_types';

import { computeFootprint } from '../sim/driver';
import { augmentWithStockOutline } from './stock-outline';
import { buildOpEntry } from './op_defaults';
import { effectiveModes } from './tool_family';

/// Memoised bundled-font fetch — the DejaVu Sans bytes used as the
/// default font for imported DXF TEXT/MTEXT entities. Resolved once
/// per session and shared across every TextLayer created from
/// `imported.text_entities`. Returns base64 because that's the form
/// TextFontSource carries.
let _defaultFontBytesB64: Promise<string | null> | null = null;
function loadDefaultFontBytesB64(): Promise<string | null> {
  if (_defaultFontBytesB64) return _defaultFontBytesB64;
  _defaultFontBytesB64 = (async () => {
    try {
      const res = await fetch('/fonts/DejaVuSans.ttf');
      if (!res.ok) return null;
      const buf = new Uint8Array(await res.arrayBuffer());
      let binary = '';
      const chunk = 0x8000;
      for (let i = 0; i < buf.length; i += chunk) {
        binary += String.fromCharCode(...buf.subarray(i, i + chunk));
      }
      return btoa(binary);
    } catch {
      return null;
    }
  })();
  return _defaultFontBytesB64;
}

import {
  addFixtureCommand,
  addOperationCommand,
  addReliefSourceCommand,
  addTextLayerCommand,
  addToolCommand,
  deleteOperationCommand,
  deleteReliefSourceCommand,
  deleteTextLayerCommand,
  deleteToolCommand,
  duplicateOperationCommand,
  removeFixtureCommand,
  reorderOperationCommand,
  replaceToolsCommand,
  setGroupOpsByToolCommand,
  toggleTabPlacementCommand,
  updateFixtureCommand,
  updateOperationCommand,
  updateReliefSourceCommand,
  updateTextLayerCommand,
  type CommandTarget,
} from './commands';

export class ProjectState {
  /// Project-data slice. Owns `imported`,
  /// `operations`, `tools`, `machine`, `stock`, `fixtures`,
  /// `textLayers`, `dirty`, `visibleLayers`, `regionsVisible`, and
  /// `settings` — i.e. every field the undo/redo command bus mutates.
  /// Consumers read `project.data.<field>` directly; writes go through
  /// the command bus.
  data = new ProjectDataState();

  /// All imports merged into one ImportResponse with each entry's
  /// fileTransform applied. Every visual consumer (canvas
  /// / 3D scene / OSnap / sim / build-project payload / footprint) reads
  /// this rather than `imported`, so the user sees N drawings on stock
  /// with independent layout transforms.
  ///
  /// Single-entry case short-circuits to `applyFileTransform(entry.source,
  /// entry.fileTransform)` — same identity-fast path as Phase 1. Multi-
  /// entry case namespaces object ids (entries[0] keeps ids 1..N, later
  /// entries get the next range) so existing op references stay valid.
  transformedImport = $derived.by<ImportResponse | null>(() => {
    return combineImports(this.data.imports);
  });

  /// Raw geometry for AUTO-STOCK sizing — `transformedImport` with its bbox
  /// expanded to enclose the editable text layers too. Text is rendered by
  /// the backend at pipeline time, so it never lands in `transformedImport`;
  /// without this, auto-stock for a text-only (or text-overflowing) project
  /// can't see the text and falls back to the whole work area. `null` when
  /// there's no geometry AND no rendered text yet. Carries NO stock outline
  /// (that would loop: outline ← footprint ← bbox ← outline) — only the raw
  /// bbox matters here, which is all `computeFootprint` reads. Reactive on
  /// the text-preview cache so it tightens as glyphs finish rendering.
  stockSizingImport = $derived.by<ImportResponse | null>(() => {
    const base = this.transformedImport;
    if (this.data.textLayers.length === 0) return base; // unchanged path
    void previewVersion.v;
    let box = base ? { ...base.bbox } : null;
    for (const layer of this.data.textLayers) {
      const segs = previewSegmentsFor(layer.id, layer.origin);
      if (!segs || segs.length === 0) continue;
      for (const s of segs) {
        const mnx = Math.min(s.start.x, s.end.x);
        const mxx = Math.max(s.start.x, s.end.x);
        const mny = Math.min(s.start.y, s.end.y);
        const mxy = Math.max(s.start.y, s.end.y);
        if (!box) box = { min_x: mnx, min_y: mny, max_x: mxx, max_y: mxy };
        else {
          box.min_x = Math.min(box.min_x, mnx);
          box.min_y = Math.min(box.min_y, mny);
          box.max_x = Math.max(box.max_x, mxx);
          box.max_y = Math.max(box.max_y, mxy);
        }
      }
    }
    if (!box) return base;
    if (base) return { ...base, bbox: box };
    // Text-only: a minimal synthetic carrying just the bbox. computeFootprint
    // reads only `.bbox`; empty segments keep every other path untouched.
    return {
      bbox: box,
      filename: 'text',
      format: 'text',
      layers: [],
      segments: [],
      objects: [],
      object_meta: [],
      unit_scale: 1,
      warnings: [],
    } as ImportResponse;
  });

  /// Geometry the canvas selects + the wire payload sends —
  /// `transformedImport` plus a synthetic, selectable stock-outline
  /// object when the stock is shown, so an op (chamfer/profile/…) can
  /// target the workpiece edge. Returns the SAME object as
  /// `transformedImport` when the stock is hidden or the footprint is
  /// degenerate (referential identity), so the canvas / build path see
  /// no change in the common case. Auto-stock sizing keeps reading the
  /// raw `transformedImport` (see `computeFootprint` callers) — feeding
  /// this back would loop (outline ← footprint ← bbox ← outline).
  geometryView = $derived.by<ImportResponse | null>(() => {
    const base = this.transformedImport;
    if (!this.data.stock.visible) return base;
    // Size the footprint from the text-inclusive bbox so a text-only project
    // gets a fitting auto-stock (and a non-null geometryView, which unblocks
    // Generate). The outline still augments the RAW `base` so the canvas
    // doesn't double-draw text (text stays a separate textLayers entity).
    const fp = computeFootprint(
      this.stockSizingImport,
      this.data.stock,
      this.data.machine.workArea,
    );
    return augmentWithStockOutline(base, fp);
  });

  loading = $state(false);
  loadingMessage = $state<string | null>(null);
  /// Last error surfaced to the user. `string` for legacy paths (file
  /// upload, save dialogs, etc.); `WiacError` for backend pipeline /
  /// import errors so the toast can render recovery hints + auto-fix.
  error = $state<string | WiacError | null>(null);

  /// Generate-pipeline slice. Holds `generated`,
  /// `generating`, `pipelineState`/`pipelineProgress`, the cached-count
  /// stats, `toolpathCumLen` / `toolpathTotalLen`, `simDiagnostics`,
  /// plus the lifecycle methods. Consumers read/write
  /// `project.gen.<field>` directly.
  gen = new GeneratedState();

  /// UI-selection slice. Holds hoverSegment, the
  /// selectedObjects / anchor / entities sets, plus the selectedOpId /
  /// selectedFixtureId / selectedTextLayerId / toolsDialogFocusId
  /// pointers. Consumers read/write `project.sel.<field>` directly.
  sel = new SelectionState();

  /// Toolpath scrub position in [0, 1]. Read by Scene3D for the tool-tip
  /// indicator and by PlaybackBar for the slider. Interpreted as a
  /// fraction of total ARC LENGTH (not segment count), so cutter speed
  /// stays consistent across short connectors and long edges. The
  /// playhead → segment mapping uses `toolpathCumLen` below.
  playhead = $state(1.0);

  /// Undoable UI entry point for the tool-grouping toggle. The plain
  /// write lives on the data slice for command apply/revert and
  /// load/clear paths, which manage dirty + generated + history
  /// themselves. Routes through
  /// the command bus (so Ctrl+Z reverses it) and invalidates the cached
  /// toolpath — the reorder changes emitted-program order, so a toolpath
  /// generated against the prior setting isn't safe to draw/download.
  setGroupOpsByTool(v: boolean) {
    if (this.data.groupOpsByTool === v) return;
    this.history.exec(setGroupOpsByToolCommand(v), this.target());
    this.gen.generated = null;
    this.gen.toolpathCumLen = null;
  }

  /// True when discarding the current project would lose work the user
  /// hasn't saved to disk — either unsaved edits (`dirty`) OR a drawing
  /// that was imported but never saved as a project. Gates the
  /// "discard?" confirmation before any destructive load (open file /
  /// open project / sample / drag-drop) and the desktop quit prompt. See
  /// [`computeUnsavedWork`] for the rule and why it's broader than
  /// `dirty`.
  get hasUnsavedWork(): boolean {
    const empty =
      !this.transformedImport &&
      this.data.operations.length === 0 &&
      this.data.textLayers.length === 0 &&
      this.data.reliefSources.length === 0;
    return computeUnsavedWork({
      empty,
      dirty: this.data.dirty,
      savedToProject: this.savedToProject,
    });
  }

  /// Undo / redo. Per-session only; not serialized to .vc-project.json.
  /// View-state (selection, playhead, layer visibility, settings) is
  /// excluded — see history.ts for the full list.
  history = new History();

  /// Reactive mirror of `history.version` so `$derived` expressions in
  /// the UI re-run when the stacks change. We can't make `History` a
  /// `.svelte.ts` module today: vitest's test config
  /// (frontend/vitest.config.ts) skips the Svelte plugin to avoid the
  /// vite 5 / vite 8 plugin mismatch, and every History test would
  /// fail with "$state is not defined". This mirror can be dropped
  /// once the test runner can handle the runes (it's a vitest
  /// + plugin-svelte upgrade, not a code-level change).
  historyVersion = $state(0);

  /// Absolute path of the currently-open project, or null if the user
  /// hasn't loaded one yet. Drives both the per-project workspace state
  /// look-up and the watch set for source-change events.
  /// Set explicitly via `setActiveProjectPath` from the open-project
  /// flows. Not part of `snapshot()` — workspace state follows the
  /// path, the path is per-machine.
  activeProjectPath = $state<string | null>(null);

  /// True when the current content lives in a saved `.ivac-project` file
  /// — set when a project is loaded from one (`restore`) or written to
  /// one (`saveProject`), cleared on a raw drawing import (`setImported`)
  /// and on `clearProject`. Unlike `activeProjectPath` (which also points
  /// at the source DXF/SVG after an import), this distinguishes "saved as
  /// a project" from "imported but never saved". Drives `hasUnsavedWork`.
  /// Runtime-only — not part of `snapshot()`.
  savedToProject = $state(false);

  /// Source-file change indicator. Populated when the watcher fires and
  /// the user has `autoReloadSources` disabled. SourceStaleToast renders
  /// from this and clears it on Reload / Ignore. Auto-reloads bypass it.
  sourceFileStaleNotice = $state<{ path: string; auto_reload: boolean } | null>(null);

  constructor() {
    this.history.subscribe(() => {
      this.historyVersion = this.history.version;
    });
  }

  /// Reset every project-scoped field (tools + machine persist by
  /// design) — see state/project-file-ops.ts.
  clearProject() {
    fileOps.clearProject(this);
  }

  /// Switch the active project path and apply persisted per-project
  /// workspace state — see state/project-file-ops.ts.
  setActiveProjectPath(path: string | null) {
    fileOps.setActiveProjectPath(this, path);
  }

  /// Persist the per-project view state (deferred off the effect
  /// flush) — see state/project-file-ops.ts.
  persistPerProjectState() {
    fileOps.persistPerProjectState(this);
  }

  /// Cast to `CommandTarget` for command builders. Single helper so the
  /// `as unknown as` dance lives in one place.
  /// Command-bus view of this state — used by History.exec callers
  /// here and in state/import-ops.ts.
  target(): CommandTarget {
    return this.data;
  }

  undo(): boolean {
    return this.history.undo(this.target());
  }
  redo(): boolean {
    return this.history.redo(this.target());
  }
  /// Public façade for `history.cancelTransaction` that hides the
  /// `CommandTarget` cast — call sites in the UI can stay free of
  /// `as unknown as never` workarounds.
  cancelTransaction(): void {
    this.history.cancelTransaction(this.target());
  }
  // The four accessors below touch `this.historyVersion` to subscribe
  // the rune scheduler. The mirror lives on this class (which is
  // already rune-aware) so `History.subscribe` can bump it from plain
  // TS — see the class doc on `historyVersion`.
  canUndo(): boolean {
    void this.historyVersion;
    return this.history.undoSize > 0;
  }
  canRedo(): boolean {
    void this.historyVersion;
    return this.history.redoSize > 0;
  }
  undoLabel(): string | null {
    void this.historyVersion;
    return this.history.undoLabel();
  }
  redoLabel(): string | null {
    void this.historyVersion;
    return this.history.redoLabel();
  }

  setSimDiagnostics(d: SimDiagnostics | null) {
    this.gen.simDiagnostics = d;
  }

  /// Persist `settings` to localStorage. Cheap (one JSON.stringify on a
  /// tiny object) so we just call it on every mutation rather than
  /// debouncing — the dialog won't fire updates fast enough to matter.
  saveSettings() {
    saveSettings(this.data.settings);
  }

  updateSettings(patch: Partial<AppSettings>) {
    this.data.settings = { ...this.data.settings, ...patch };
    this.saveSettings();
  }

  /// Click-toggle a tab placement on an op. `toleranceT` is
  /// the parameter-space distance under which a click on an existing
  /// nearby tab removes it (Estlcam-style toggle). Single undoable
  /// history entry per click.
  toggleTabPlacement(opId: number, placement: { objectId: number; t: number }, toleranceT: number) {
    this.history.exec(toggleTabPlacementCommand(opId, placement, toleranceT), this.target());
  }

  // ── fixtures ─────────────────────────────────────────────────────────

  addFixture(
    kind: FixtureKind,
    origin: [number, number],
    z_bottom: number,
    z_top: number,
    name?: string,
  ): Fixture {
    const nextId = this.data.fixtures.reduce((m, f) => Math.max(m, f.id), 0) + 1;
    const f: Fixture = {
      id: nextId,
      name: name ?? defaultFixtureName(kind, nextId),
      kind,
      origin,
      z_bottom,
      z_top,
      color: DEFAULT_FIXTURE_COLOR,
    };
    this.history.exec(addFixtureCommand(f), this.target());
    this.sel.selectedFixtureId = f.id;
    return f;
  }

  updateFixture(id: number, patch: Partial<Fixture>) {
    if (Object.keys(patch).length === 0) return;
    if (!this.data.fixtures.some((f) => f.id === id)) return;
    this.history.exec(updateFixtureCommand(id, patch), this.target());
  }

  removeFixture(id: number) {
    if (!this.data.fixtures.some((f) => f.id === id)) return;
    this.history.exec(removeFixtureCommand(id), this.target());
    if (this.sel.selectedFixtureId === id) this.sel.selectedFixtureId = null;
  }

  selectFixture(id: number | null) {
    this.sel.selectFixture(id);
  }

  /// Append another drawing as its own ImportEntry —
  /// see state/import-ops.ts.
  addImported(r: ImportResponse, sourcePath?: string | null) {
    importOps.addImported(this, r, sourcePath);
  }

  /// Remove an import by its ImportEntry.id (undoable) — see
  /// state/import-ops.ts.
  removeImport(id: number) {
    importOps.removeImport(this, id);
  }
  /// Replace imports[0] with a freshly-imported drawing (project
  /// boundary: clears history, resets view state, auto-places the
  /// drawing and infers the WCS default) — see state/import-ops.ts.
  setImported(r: ImportResponse, sourcePath?: string | null) {
    importOps.setImported(this, r, sourcePath);
  }

  /// Desktop source-file watcher + reload — see state/import-ops.ts.
  async refreshSourceWatch(): Promise<void> {
    return importOps.refreshSourceWatch(this);
  }

  async stopSourceWatch(): Promise<void> {
    return importOps.stopSourceWatch();
  }

  async reimportFromPath(path: string): Promise<boolean> {
    return importOps.reimportFromPath(this, path);
  }

  /// Canvas-click object toggle (undoable) — see
  /// state/project-selection-ops.ts.
  toggleObject(id: number, additive = false) {
    selectionOps.toggleObject(this, id, additive);
  }

  /// Bulk selection update with FreeCAD-style modifier semantics
  /// (undoable) — see state/project-selection-ops.ts.
  selectObjects(ids: Iterable<number>, mode: SelectionMode) {
    selectionOps.selectObjects(this, ids, mode);
  }

  /// Series-select from the current anchor to `targetId` along the
  /// bbox-centroid line — see state/project-selection-ops.ts.
  seriesSelectTo(targetId: number) {
    selectionOps.seriesSelectTo(this, targetId);
  }

  /// Clear the object selection (undoable) — see
  /// state/project-selection-ops.ts.
  clearSelection() {
    selectionOps.clearSelection(this);
  }

  setGenerated(r: GenerateResponse) {
    this.gen.generated = r;
    // Single-program run clears any prior two-sided back program so a
    // single-sided regenerate can't leave a stale back behind.
    this.gen.generatedBack = null;
    this.gen.generatedVersion += 1;
    // Pre-compute cumulative arc length over the toolpath so playback
    // can advance by physical distance instead of segment count. See
    // `playheadToSegment` for the inverse lookup.
    const tp = r.toolpath;
    if (tp.length > 0) {
      const cum = new Float64Array(tp.length);
      let acc = 0;
      for (let i = 0; i < tp.length; i++) {
        const s = tp[i];
        const dx = s.to.x - s.from.x;
        const dy = s.to.y - s.from.y;
        const dz = s.to.z - s.from.z;
        acc += Math.hypot(dx, dy, dz);
        cum[i] = acc;
      }
      this.gen.toolpathCumLen = cum;
      this.gen.toolpathTotalLen = acc;
    } else {
      this.gen.toolpathCumLen = null;
      this.gen.toolpathTotalLen = 0;
    }
    // A fresh toolpath invalidates the previous heightfield-sim run: its
    // warnings (collisions, rapid-through-material) described the OLD
    // program. Clear them so they don't linger against the new toolpath;
    // the 3D pane's sim re-runs (keyed on generatedVersion) and repopulates
    // when it's visible. Without this, a stale critical sim warning kept
    // showing in the warning chip after a fix-and-regenerate.
    this.gen.simDiagnostics = null;
    this.data.dirty = false;
    this.error = null;
    this.playhead = 1.0;
  }

  /// Store a two-sided (flip-stock) result: the FRONT program becomes the
  /// primary `generated` (all existing consumers see it), and the BACK
  /// program is held alongside for the Front/Back gcode tabs + dual-surface
  /// preview. Delegates the front-program bookkeeping to `setGenerated`
  /// (which first clears `generatedBack`), then sets the back.
  setGeneratedTwoSided(r: TwoSidedGenerateResponse) {
    this.setGenerated(r.front);
    this.gen.generatedBack = r.back ?? null;
  }

  setError(err: string | WiacError) {
    this.error = err;
  }

  clearError() {
    this.error = null;
  }

  /// Pipeline-state lifecycle helpers. Most delegate to
  /// the generated-state slice; `failGenerate` lives here because it
  /// crosses slices (error + pipelineState reset).
  beginGenerate() {
    this.error = null;
    this.gen.beginGenerate();
  }

  notePipelineEvent(ev: PipelineNoteEvent) {
    this.gen.notePipelineEvent(ev);
  }

  finishGenerate() {
    this.gen.finishGenerate();
  }

  cancelGenerate() {
    this.gen.cancelGenerate();
  }

  /// Pipeline failure path. Routes the error through setError and
  /// snaps the generate slice back to idle. Spans two slices, so
  /// stays on the parent rather than living on either.
  failGenerate(err: string | WiacError) {
    this.setError(err);
    this.gen.pipelineState = 'idle';
  }

  endGenerate() {
    this.gen.endGenerate();
  }

  toggleLayer(name: string) {
    importOps.toggleLayer(this, name);
  }

  /// Delete every imported segment on `layerName` across all imports
  /// (undoable) — see state/import-ops.ts.
  removeImportedLayer(layerName: string) {
    importOps.removeImportedLayer(this, layerName);
  }

  /// Snapshot for project save (view state intentionally omitted) —
  /// see state/project-file-ops.ts.
  snapshot(): ProjectFile {
    return fileOps.snapshotProject(this);
  }

  /// Load a saved .ivac-project file into the live state — see
  /// state/project-file-ops.ts for the precedence rules.
  restore(file: ProjectFile) {
    fileOps.restoreProject(this, file);
  }

  /// Append rendered AddTextDialog segments to the imported geometry
  /// and return the new 1-based object ids — see state/import-ops.ts.
  appendImportedSegments(segments: Segment[], layerName: string, singleLine: boolean): number[] {
    return importOps.appendImportedSegments(this, segments, layerName, singleLine);
  }

  // ── operation helpers ────────────────────────────────────────────────

  addOperation(kind: OpKind): OpEntry {
    // The per-kind default field set lives in the pure `buildOpEntry`
    // registry (op_defaults.ts) so it's one source of truth, unit-tested
    // without the rune runtime. This method only gathers the live context
    // and runs the result through the command bus. When the user has
    // objects selected on the canvas, geometry kinds pin to that set (most
    // users select first, then click "+ Pocket"); empty selection keeps the
    // All default.
    const op = buildOpEntry(kind, {
      nextId: this.data.operations.reduce((m, o) => Math.max(m, o.id), 0) + 1,
      tools: this.data.tools,
      reliefSources: this.data.reliefSources,
      selectionIds: [...this.sel.selectedObjects],
      objectMeta: this.transformedImport?.object_meta ?? [],
      modes: effectiveModes(this.data.machine),
    });
    this.history.exec(addOperationCommand(op), this.target());
    this.sel.selectedOpId = op.id;
    return op;
  }

  removeOperation(id: number) {
    if (!this.data.operations.some((o) => o.id === id)) return;
    this.history.exec(deleteOperationCommand(id), this.target());
    if (this.sel.selectedOpId === id) this.sel.selectedOpId = null;
  }

  /// Insert a text layer with the given configuration; `id` and the
  /// default `name` are filled in if absent. Returns the inserted
  /// layer (with the assigned id). Undoable.
  addTextLayer(
    seed: Omit<TextLayer, 'id' | 'name'> & Partial<Pick<TextLayer, 'id' | 'name'>>,
  ): TextLayer {
    const nextId = seed.id ?? this.data.textLayers.reduce((m, t) => Math.max(m, t.id), 0) + 1;
    const previewText = seed.text.split(/\r?\n/, 1)[0] ?? '';
    const truncated = previewText.length > 20 ? `${previewText.slice(0, 20)}…` : previewText;
    const defaultName = `${seed.kind} — "${truncated}"`;
    const layer: TextLayer = { ...seed, id: nextId, name: seed.name ?? defaultName };
    this.history.exec(addTextLayerCommand(layer), this.target());
    return layer;
  }

  /// Insert a relief surface source (e.g. a decoded grayscale
  /// image). `id` is assigned if absent. Returns the inserted source.
  /// Undoable.
  addReliefSource(
    seed: Omit<ReliefSource, 'id'> & Partial<Pick<ReliefSource, 'id'>>,
  ): ReliefSource {
    const nextId = seed.id ?? this.data.reliefSources.reduce((m, s) => Math.max(m, s.id), 0) + 1;
    const source: ReliefSource = { ...seed, id: nextId };
    this.history.exec(addReliefSourceCommand(source), this.target());
    return source;
  }

  updateReliefSource(id: number, patch: Partial<ReliefSource>) {
    if (Object.keys(patch).length === 0) return;
    if (!this.data.reliefSources.some((s) => s.id === id)) return;
    this.history.exec(updateReliefSourceCommand(id, patch), this.target());
  }

  removeReliefSource(id: number) {
    if (!this.data.reliefSources.some((s) => s.id === id)) return;
    this.history.exec(deleteReliefSourceCommand(id), this.target());
  }

  updateTextLayer(id: number, patch: Partial<TextLayer>) {
    if (Object.keys(patch).length === 0) return;
    if (!this.data.textLayers.some((t) => t.id === id)) return;
    this.history.exec(updateTextLayerCommand(id, patch), this.target());
  }

  /// Convert any `imported.text_entities` from the most recent setImported
  /// call into editable `TextLayer` entries. Each entity gets the bundled
  /// DejaVu Sans by default so the user sees the text immediately; they
  /// can swap fonts later from the sidebar. No-op when nothing was
  /// imported or no TEXT/MTEXT entities were present.
  async convertImportedTextEntities(): Promise<void> {
    const entry = this.data.imports[0];
    if (!entry) return;
    const entities = entry.source.text_entities;
    if (!entities || entities.length === 0) return;
    const bytes_b64 = await loadDefaultFontBytesB64();
    if (!bytes_b64) return;
    this.history.beginTransaction('Import text entities');
    try {
      for (const e of entities) {
        const isMtext = e.kind === 'MTEXT';
        this.addTextLayer({
          kind: isMtext ? 'MTEXT' : 'TEXT',
          text: e.text,
          fontSource: { kind: 'bundled', path: '/fonts/DejaVuSans.ttf', bytes_b64 },
          sizeMm: e.size_mm,
          origin: { x: e.origin[0], y: e.origin[1] },
          rotationDeg: e.rotation_deg ?? 0,
          letterSpacingMm: 0,
          lineSpacingMm: 0,
          alignment: 'left',
          widthScale: 1.0,
          singleLine: false,
        });
      }
      this.history.commitTransaction();
    } catch (err) {
      this.history.cancelTransaction(this.target());
      throw err;
    }
    // Consume the queue so subsequent addImported() calls don't try
    // to convert the same entities again into duplicate TextLayers.
    // Plain mutation (not a command): this is bookkeeping after the
    // text-layer-add commands above, not user-undoable state.
    const cur = this.data.imports[0];
    if (cur) {
      this.data.imports = [
        { ...cur, source: { ...cur.source, text_entities: [] } },
        ...this.data.imports.slice(1),
      ];
    }
  }

  removeTextLayer(id: number) {
    if (!this.data.textLayers.some((t) => t.id === id)) return;
    const syntheticLayer = `__text_${id}`;
    // Drop the cached preview segments so the canvas doesn't keep
    // painting glyphs from a layer that no longer exists.
    invalidatePreview(id);
    // Cascade-delete any ops whose source targets the text layer's
    // synthetic geometry layer — leaving them around would make the
    // pipeline raise "no segments on layer __text_<id>".
    const dependentOps = this.data.operations.filter(
      (o) => Array.isArray(o.sourceLayers) && o.sourceLayers.includes(syntheticLayer),
    );
    if (dependentOps.length > 0) {
      this.history.beginTransaction('Delete text');
      for (const op of dependentOps) {
        this.history.exec(deleteOperationCommand(op.id), this.target());
      }
      this.history.exec(deleteTextLayerCommand(id), this.target());
      this.history.commitTransaction();
    } else {
      this.history.exec(deleteTextLayerCommand(id), this.target());
    }
    if (this.sel.selectedTextLayerId === id) this.sel.selectedTextLayerId = null;
  }

  /// Deep-clone the op and insert it immediately after the original.
  /// Returns the new op or null if `id` is unknown.
  duplicateOperation(id: number): OpEntry | null {
    const src = this.data.operations.find((o) => o.id === id);
    if (!src) return null;
    const nextId = this.data.operations.reduce((m, o) => Math.max(m, o.id), 0) + 1;
    // JSON-roundtrip clone: Svelte 5 `$state` proxies make
    // structuredClone throw DataCloneError in production builds — the
    // dup button would die with an uncaught exception and look dead.
    const copy: OpEntry = {
      ...(JSON.parse(JSON.stringify(src)) as OpEntry),
      id: nextId,
      name: `${src.name} (copy)`,
    };
    this.history.exec(duplicateOperationCommand(id, copy, id), this.target());
    this.sel.selectedOpId = copy.id;
    return copy;
  }

  updateOperation(id: number, patch: Partial<OpEntry>) {
    if (Object.keys(patch).length === 0) return;
    if (!this.data.operations.some((o) => o.id === id)) return;
    this.history.exec(updateOperationCommand(id, patch), this.target());
  }

  /// Reorder. Skipped when source and target index are the same so a
  /// stray drag-and-drop with no actual move doesn't dirty the project.
  /// (A real reorder still flips dirty so the status badge surfaces it,
  /// but the previously-generated gcode stays on screen until the user
  /// clicks Generate again.)
  reorderOperation(id: number, toIndex: number) {
    const cur = this.data.operations.findIndex((o) => o.id === id);
    if (cur < 0) return;
    const clamped = Math.max(0, Math.min(toIndex, this.data.operations.length - 1));
    if (clamped === cur) return;
    this.history.exec(reorderOperationCommand(id, clamped), this.target());
  }

  // (op grouping removed — ops are a flat list)

  // ── tool library ─────────────────────────────────────────────────────

  /// Replace the entire tool library in one undoable step. Used by the
  /// Tool library dialog's commit button.
  replaceTools(nextTools: ToolEntry[]) {
    if (nextTools.length === 0) return;
    this.history.exec(replaceToolsCommand(nextTools), this.target());
  }

  addTool(tool: ToolEntry) {
    this.history.exec(addToolCommand(tool), this.target());
  }

  removeTool(id: number) {
    if (!this.data.tools.some((t) => t.id === id)) return;
    this.history.exec(deleteToolCommand(id), this.target());
  }

  // ── machine / stock ──────────────────────────────────────────────────

  /// Undoable machine-config swap; clears cached gcode + runs the
  /// mode-switch staleness assessment — see state/project-machine-ops.ts.
  setMachine(next: MachineSettings) {
    machineOps.setMachine(this, next);
  }

  /// The mode-switch notice's "assign to all" action — see
  /// state/project-machine-ops.ts.
  assignToolToOps(opIds: readonly number[], toolId: number | null) {
    machineOps.assignToolToOps(this, opIds, toolId);
  }

  /// The mode-switch notice's seed action for a singleton mode — see
  /// state/project-machine-ops.ts.
  seedDefaultToolForMode() {
    machineOps.seedDefaultToolForMode(this);
  }

  /// Switch the project to a workspace machine profile (config + tools
  /// + reference, one undoable step) — see state/project-machine-ops.ts.
  applyMachineProfile(profile: import('./workspace').MachineProfile) {
    machineOps.applyMachineProfile(this, profile);
  }

  /// Detach the project from its machine profile — see
  /// state/project-machine-ops.ts.
  detachMachineProfile() {
    machineOps.detachMachineProfile(this);
  }

  /// Undoable stock-config edit — see state/project-machine-ops.ts.
  setStock(patch: Partial<StockConfig>, coalesceKey?: string) {
    machineOps.setStock(this, patch, coalesceKey);
  }

  /// Undoable WorkOffset edit — see state/project-machine-ops.ts.
  setWorkOffset(patch: Partial<WorkOffset>) {
    machineOps.setWorkOffset(this, patch);
  }

  /// Snap the WCS origin to the footprint's bottom-left corner — see
  /// state/project-machine-ops.ts.
  snapWorkOffsetToFootprint(): void {
    machineOps.snapWorkOffsetToFootprint(this);
  }

  /// Per-import file-transform patch. Undoable, spinner
  /// nudges coalesce; marker re-projection — see state/import-ops.ts.
  patchFileTransformForImport(
    importId: number,
    patch: Partial<Omit<FileTransform, 'translate'>> & {
      translate?: Partial<FileTransform['translate']>;
    },
    coalesceKey?: string,
  ) {
    importOps.patchFileTransformForImport(this, importId, patch, coalesceKey);
  }

  resetFileTransformForImport(importId: number) {
    importOps.resetFileTransformForImport(this, importId);
  }
}

export const project = new ProjectState();

// These helpers live in `sim/warnings.ts` and `sim/playhead.ts` so
// vitest can import them without booting the Svelte rune runtime.
// Re-exported here for backwards-compat with existing call sites.
export { simWarningSeverity, simWarningSegmentIdx, simWarningSummary } from '../sim/warnings';
export { playheadToSegment } from '../sim/playhead';
