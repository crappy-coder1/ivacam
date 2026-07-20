<script lang="ts">
  import { onMount } from 'svelte';
  import { project, isContourOp, type OpEntry } from '../state/project.svelte';
  import { opSourceCss } from '../state/op-color';
  import { STOCK_OUTLINE_LAYER } from '../state/stock-outline';
  import { consumeSelectHint } from '../state/ui-hints';
  import { buildObjectPolylines, type ObjectPolyline } from '../cam/tabs';
  import type { BBox } from '../api/types';
  import {
    buildHitIndex as buildHitIndexPure,
    queryHit,
    queryHitObjects,
    type HitIndex,
  } from '../canvas/spatial-index';
  import { fixtureAt } from '../canvas/fixture-hit';
  import { projectGhostTab, type GhostTab } from '../canvas/ghost-tab';
  import { reduceCanvasClick } from '../canvas/entity-selection';
  import { reducePointerDown } from '../canvas/pointer-down';
  import { reducePointerUp } from '../canvas/pointer-up';
  import { reducePointerMove, hoverCursor } from '../canvas/pointer-move';
  import {
    findTabAtPixel as hitTabAtPixel,
    patchTabPlacement,
    removeTabPlacement,
  } from '../canvas/tab-hit';
  import {
    computeViewportTransform,
    placementsBBox,
    segsBBox,
    type Rect,
  } from '../canvas/viewport';
  import { nearestTextLayer } from '../canvas/text-hit';
  import { ViewController } from '../canvas/view-controller.svelte';
  import { HoverState } from '../canvas/hover-state.svelte';
  import { TouchTracker } from '../canvas/touch-tracker';
  import { PointerDragController } from '../canvas/pointer-drag.svelte';
  import { withinTapTolerance, LONG_PRESS_MS } from '../canvas/touch-gestures';
  import { objectsContainedInBox } from '../canvas/box_select';
  import { resolveAci, hexToCss } from '../canvas/aci-color';
  import { drawSegment } from '../canvas/render/segment';
  import {
    drawGrid,
    drawAxes,
    drawWorkArea,
    drawStock,
    drawStockGizmo,
  } from '../canvas/render/chrome';
  import {
    hitStockHandle,
    dragStockBox,
    boxToStock,
    type StockHandleKind,
    type StockResizeKind,
    type WorldBox,
  } from '../canvas/stock-gizmo';
  import { RegionPathCache, drawRegions } from '../canvas/render/regions';
  import { drawTextPreview } from '../canvas/render/text';
  import { drawImportedWireframe, drawEntityHalos } from '../canvas/render/entities';
  import { drawTabs } from '../canvas/render/tabs';
  import { drawApproachPoint } from '../canvas/render/approach';
  import { drawFixtures } from '../canvas/render/fixtures';
  import { RasterImageCache, drawRasterPlacements } from '../canvas/render/raster';
  import { drawBoxSelect } from '../canvas/render/box-select';
  import {
    DEFAULT_OSNAP_SETTINGS,
    findOSnap,
    precomputeOSnapTargets,
    type OSnapCandidate,
    type OSnapTargets,
  } from '../canvas/osnap';
  import { pickerLabel, type PickerKind } from './OpKindPicker.svelte';
  import EntityContextMenu from './EntityContextMenu.svelte';
  import { createOpFromSelection } from '../state/op_creation';
  import { layout } from '../state/layout.svelte';
  import { t } from '../i18n';
  import { computeFootprint } from '../sim/driver';
  import {
    previewSegmentsFor,
    previewVersion,
    requestPreview,
    forceTextPreviewRefresh,
  } from '../state/text_preview.svelte';
  import type { ReliefSource, TextLayer } from '../state/project-types';

  interface Props {
    onShowHelp?: () => void;
    /// Hint the sidebar to switch its accordion to a given pane.
    /// Used by right-click "Add op from selection" so the new op
    /// row is visible in the Operations panel without an extra
    /// click on the sidebar.
    onActivateSidebarPane?: (pane: 'stock' | 'layers' | 'text' | 'operations') => void;
  }
  let { onShowHelp, onActivateSidebarPane }: Props = $props();

  let canvas: HTMLCanvasElement;
  /// Stacked overlay canvas for state-bearing repaints (selection halos,
  /// hover halo, ghost tab, approach point, box-select rect, fixtures,
  /// tabs, OSnap glyph). pointer-events: none in CSS so the bg canvas
  /// keeps receiving input. Splits the per-frame work so hover and
  /// selection don't repaint the (often huge) imported geometry layer.
  let canvasOverlay: HTMLCanvasElement;
  let container: HTMLDivElement;

  /// Cached resolved theme colors. A bare
  /// `getComputedStyle(container).getPropertyValue(name)` on every
  /// lookup fires a synchronous style recalc — and `draw()`
  /// invokes it 15-20× per frame. We memoise per CSS var until the
  /// Theme-observed CSS-var cache. `resetThemeCache()` clears it when
  /// the MutationObserver in onMount sees a `data-theme` change.
  let themeCache = new Map<string, string>();
  function themeVar(name: string, fallback: string): string {
    if (!container) return fallback;
    const cached = themeCache.get(name);
    if (cached !== undefined) return cached;
    const v = getComputedStyle(container).getPropertyValue(name).trim() || fallback;
    themeCache.set(name, v);
    return v;
  }
  function resetThemeCache() {
    themeCache = new Map();
  }

  /// Trigger both canvas layers — used by resize / theme / mount paths
  /// where the right answer is "repaint everything". The $effect blocks
  /// drive per-state-change repaints; this helper is for non-reactive
  /// triggers.
  function drawBoth() {
    drawBackground();
    drawOverlay();
  }

  // rAF-coalesced repaint scheduling for the reactive draw effects below.
  // A pan/zoom drag mutates view.panX/panY/zoom on every pointermove, and
  // each change would otherwise synchronously re-stroke the whole imported
  // wireframe (O(segments) ctx ops). Instead the effects SCHEDULE a redraw
  // and at most one background + one overlay paint runs per animation
  // frame — intermediate frames under load are dropped. Reactivity is
  // unaffected: the effects still `void` every dependency synchronously,
  // so tracking is exact; only the paint itself is deferred to the frame.
  let drawFrame = 0;
  let needBackground = false;
  let needOverlay = false;
  function flushDraw() {
    drawFrame = 0;
    if (needBackground) {
      needBackground = false;
      drawBackground();
    }
    if (needOverlay) {
      needOverlay = false;
      drawOverlay();
    }
  }
  function ensureDrawFrame() {
    if (drawFrame === 0) drawFrame = requestAnimationFrame(flushDraw);
  }
  function scheduleBackground() {
    needBackground = true;
    ensureDrawFrame();
  }
  function scheduleOverlay() {
    needOverlay = true;
    ensureDrawFrame();
  }

  onMount(() => {
    // Defer the resize-driven redraw to the next animation frame.
    // ResizeObserver fires synchronously during layout; if the
    // callback mutates the observed element's children (which
    // drawBoth does — it resizes the canvases inside `container`),
    // the browser logs "ResizeObserver loop completed with
    // undelivered notifications" and skips dispatching the next
    // batch. Coalescing into one rAF eliminates the warning and
    // makes the redraw cost predictable across multi-event resizes.
    let resizeFrame = 0;
    const ro = new ResizeObserver(() => {
      if (resizeFrame !== 0) return;
      resizeFrame = requestAnimationFrame(() => {
        resizeFrame = 0;
        drawBoth();
      });
    });
    ro.observe(container);
    drawBoth();
    // Re-paint when the user toggles their OS theme or picks a manual one.
    const mql = window.matchMedia('(prefers-color-scheme: light)');
    const onChange = () => {
      resetThemeCache();
      drawBoth();
    };
    mql.addEventListener('change', onChange);
    // Diff the data-theme value before redrawing — MutationObserver fires
    // on every attribute write, including same-value writes. draw() is
    // non-trivial for big imports.
    let lastTheme = document.documentElement.dataset.theme ?? '';
    const themeMo = new MutationObserver(() => {
      const cur = document.documentElement.dataset.theme ?? '';
      if (cur === lastTheme) return;
      lastTheme = cur;
      resetThemeCache();
      drawBoth();
    });
    themeMo.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['data-theme'],
    });
    return () => {
      ro.disconnect();
      mql.removeEventListener('change', onChange);
      themeMo.disconnect();
      if (drawFrame !== 0) cancelAnimationFrame(drawFrame);
    };
  });

  // Two-effect canvas paint (split bd: 'EntityCanvas2D draw effect').
  //
  // The bg canvas paints the heavy static layer: imported geometry in
  // base layer color, text-layer previews in base color, regions, grid,
  // axes, work-area. Repaints only on data / layout / zoom / pan / theme
  // changes — NEVER on hover or selection.
  //
  // The overlay canvas paints state-bearing items on top: selection /
  // hover / op-assignment halos, ghost tab, approach point, fixtures,
  // tab markers, box-select rect, selected-text-layer highlight, OSnap
  // glyphs. Hover and selection only retouch this layer.

  $effect(() => {
    void project.geometryView;
    void project.data.visibleLayers;
    void project.data.regionsVisible;
    void project.gen.generated;
    // Repaints on origin too (drag): cheap now that an origin change no
    // longer triggers a backend render — drawTextPreview just re-strokes
    // the cached glyphs at the live origin via a draw-time translation.
    void project.data.textLayers;
    void project.sel.selectedTextLayerId;
    void previewVersion.v;
    void project.data.machine.workArea;
    void project.data.stock;
    void project.data.settings.previewLineWidth;
    // A raster-engrave-only project (no imported geometry)
    // draws its grid / axes fit to the placement bbox, so the bg must
    // react to sources / ops appearing / moving. Same per-frame cost as
    // a pan (which already repaints this layer), so no new regression.
    void project.data.reliefSources;
    void project.data.operations;
    void view.zoom;
    void view.panX;
    void view.panY;
    scheduleBackground();
  });

  $effect(() => {
    void project.geometryView;
    void project.data.visibleLayers;
    void project.sel.selectedObjects;
    void project.data.operations;
    void project.sel.selectedOpId;
    void project.data.fixtures;
    void project.sel.selectedFixtureId;
    void project.sel.selectedTextLayerId;
    // Raster-engrave placement images live on the overlay
    // (faint, under the interaction chrome) so the heavy bg layer stays
    // pure. Repaint when a source moves / resizes.
    void project.data.reliefSources;
    void hover.objectIdx;
    void hover.textId;
    void ghostTab;
    void boxSelect;
    void view.zoom;
    void view.panX;
    void view.panY;
    scheduleOverlay();
  });

  // Keep the live-preview cache warm. Loops every text layer and asks
  // for a render — the helper deduplicates by content hash and
  // debounces, so this is cheap when nothing changed.
  $effect(() => {
    for (const layer of project.data.textLayers) {
      requestPreview(layer);
    }
  });

  /// Selected-op-driven tab placement mode. When the user
  /// has a profile / pocket op selected with `tabMode` === manual or
  /// mixed, the canvas behaves as a tab-placement surface: hover
  /// shows a ghost tab; click toggles a placement.
  const selectedOp = $derived(
    project.sel.selectedOpId == null
      ? null
      : (project.data.operations.find((o) => o.id === project.sel.selectedOpId) ?? null),
  );
  const tabPlacementActive = $derived(
    !!selectedOp &&
      (selectedOp.kind === 'profile' || selectedOp.kind === 'pocket') &&
      (selectedOp.tabMode?.kind === 'manual' || selectedOp.tabMode?.kind === 'mixed'),
  );
  /// Ghost tab while hovering the contour in placement mode. The
  /// `snap` field describes which secondary snap target the cursor
  /// landed on so the renderer can flash a small dot.
  let ghostTab = $state<{
    x: number;
    y: number;
    objectId: number;
    t: number;
    snap: 'contour' | 'vertex' | 'midpoint' | 'existing';
  } | null>(null);
  /// Track Alt-held state across the gesture — when true, snap
  /// to anything except the bare contour projection is disabled,
  /// matching the CAD-convention escape hatch.
  let altDown = $state(false);

  /// Track Shift-held state for the approach-point picker. When
  /// true, snap-to-vertex is disabled — the user is asking for a
  /// free-form pick anywhere in the canvas.
  let shiftDown = $state(false);

  /// Approach-point picker. Active when project.sel.pickMode is
  /// `{ kind: 'approach-point', opId: <selected op id> }`. Cursor
  /// becomes a crosshair, a preview marker tracks the mouse (snapped
  /// to source-object vertices unless Shift is held), and a click
  /// commits the point to `op.approachPoint` while staying in pick
  /// mode (sticky — ESC exits).
  const approachPickActive = $derived(
    project.sel.pickMode?.kind === 'approach-point' && project.sel.pickMode.opId === selectedOp?.id,
  );

  /// Live preview state while picking. `lastTransform`-relative data
  /// coords; `snap` carries the snap-kind classification so the
  /// renderer can paint the matching glyph (square / triangle / X / +).
  let approachPreview = $state<{
    x: number;
    y: number;
    snap: OSnapCandidate['kind'] | null;
  } | null>(null);

  /// Active live-commit pointer drags (approach marker / raster placement /
  /// text origin / stock gizmo) + the parked stock-handle pre-commit. Owned
  /// by lib/canvas/pointer-drag.svelte.ts, mirroring the ViewController /
  /// TouchTracker split. At most one is active at a time; each commits live
  /// (coalesced into one undo entry) on every move. Text patches coalesce
  /// via coalesceKeyForTextPatch.
  const drag = new PointerDragController();

  /// Cache of the decoded brightness image per relief source — see
  /// RasterImageCache (lib/canvas/render/raster.ts).
  const rasterImageCache = new RasterImageCache();

  /// Precomputed OSnap target collection. Rebuilt only when the
  /// imported geometry changes — never per pointermove.
  const osnapTargets = $derived<OSnapTargets>(
    approachPickActive || drag.approach != null
      ? precomputeOSnapTargets(project.geometryView)
      : { endpoints: [], midpoints: [], intersections: [], centers: [] },
  );

  /// OSnap settings come from `project.data.settings.osnap`. Falls
  /// back to the hardcoded defaults for the brief window between
  /// component mount and the settings hydration completing.
  const osnapSettings = $derived(project.data.settings.osnap ?? DEFAULT_OSNAP_SETTINGS);

  // Mouse → segment hit testing. We project each segment to canvas space
  // and pick the nearest one within `HIT_PIXEL_TOL`.
  const HIT_PIXEL_TOL = 8;
  /// Pointer-hover feedback: the object/text under the cursor (hover halo +
  /// grab cursor) and the cursor world coords (on-canvas HUD). Owned by
  /// lib/canvas/hover-state.svelte.ts, mirroring the ViewController split.
  const hover = new HoverState();
  let lastTransform: { scale: number; offX: number; offY: number } | null = null;
  /// Last-computed AUTO-FIT (base) transform — the scale/offset the
  /// canvas would use with zoom=1 and no pan. Stored separately so the
  /// wheel-zoom math can solve for a user pan that keeps the cursor
  /// over the same data-space point as the zoom multiplier changes.
  let lastBaseTransform: { scale: number; offX: number; offY: number } | null = null;

  /// User-applied pan + zoom on top of the auto-fit transform, plus the
  /// active middle-button pan drag. zoom = 1 + pan = 0 → auto-fit (the
  /// default after every new import). Wheel zooms around the cursor;
  /// middle-button drag pans; double-click empty space resets to default.
  /// Owned by lib/canvas/view-controller.svelte.ts — the transitions
  /// (wheel/pinch/pan/fit/import-reset) live there, not inline here.
  const view = new ViewController();

  /// Touch gesture bookkeeping (live pointer set, active pinch frame,
  /// long-press hold timer). Plain (non-reactive): it drives the reactive
  /// view (`view.zoom` / `view.pan*`) via pinch but nothing renders it
  /// directly. Owned by lib/canvas/touch-tracker.ts, mirroring how
  /// ViewController owns the pan/zoom state.
  const touch = new TouchTracker();

  /// Keyboardless multi-select. On a touchscreen with no hardware
  /// keyboard, shift/ctrl-tap are unreachable, so this toggle makes a
  /// plain tap behave like ctrl-click (toggle into the selection).
  /// Surfaced as an overlay button on touch-capable devices only.
  let addToSelection = $state(false);
  const isTouchDevice = typeof navigator !== 'undefined' && (navigator.maxTouchPoints ?? 0) > 0;

  /// Reset pan + zoom when the imported file changes (different filename
  /// or going from no-import to imported). Keeps mid-session zooms
  /// intact across normal redraws. The change-tracking lives in the
  /// controller; this effect just feeds it the live import key.
  $effect(() => {
    view.resetOnImportChange(project.transformedImport?.filename ?? null);
  });

  /// FreeCAD-style box-select state. Captured on pointerdown over
  /// empty canvas; commits to a box drag once the cursor has moved
  /// `BOX_DRAG_THRESHOLD` px (so a sloppy click on empty space still
  /// just clears the selection).
  let boxSelect = $state<{
    startX: number;
    startY: number;
    curX: number;
    curY: number;
    mode: 'replace' | 'add' | 'toggle';
    armed: boolean; // pointer is down but we haven't crossed the threshold yet
  } | null>(null);
  const BOX_DRAG_THRESHOLD = 4;

  /// Inverted index: objectId → opIds that reference it via
  /// `op.sourceObjects`. Drives the green / dim-green tinting on the
  /// canvas and the "click an assigned object activates its op"
  /// behaviour.
  const objectToOps = $derived.by<Map<number, number[]>>(() => {
    const out = new Map<number, number[]>();
    for (const op of project.data.operations) {
      const refs = op.sourceObjects;
      if (!refs) continue;
      for (const id of refs) {
        if (id <= 0) continue;
        const list = out.get(id);
        if (list) list.push(op.id);
        else out.set(id, [op.id]);
      }
    }
    return out;
  });

  /// Uniform-grid spatial index for segment hit testing. Without it,
  /// pixelHit() ran an O(N) scan on every pointermove — fine for tiny
  /// DXFs but a million distance computations per second of idle hover
  /// on a 10k-segment file. The grid is rebuilt when project.imported
  /// changes (rare); each query inspects only the cells overlapping the
  /// cursor + tolerance and bails early past that.
  // Spatial index (HitIndex type + buildHitIndex + the cell-walk
  // query loop) extracted to lib/canvas/spatial-index.ts so vitest
  // can exercise them without mounting the canvas.
  let hitIndex: HitIndex | null = null;

  $effect(() => {
    void project.geometryView;
    hitIndex = buildHitIndexPure(project.geometryView);
  });

  function pixelHit(canvasX: number, canvasY: number): number | null {
    const data = project.geometryView;
    if (!data || !lastTransform) return null;
    const { scale, offX, offY } = lastTransform;
    const dataX = (canvasX - offX) / scale;
    const dataY = (offY - canvasY) / scale;
    const tolData = HIT_PIXEL_TOL / scale;
    return queryHit(
      data,
      hitIndex,
      dataX,
      dataY,
      tolData,
      (l) =>
        // The synthetic stock-outline layer isn't in the
        // user's visibleLayers set, but it must always be hittable.
        l === STOCK_OUTLINE_LAYER || project.data.visibleLayers.has(l),
    );
  }

  /// Ordered (nearest-first) distinct object ids whose geometry lies
  /// within the hit tolerance of a canvas pixel — the candidate stack for
  /// tap-cycling. Mirrors `pixelHit`'s coordinate + visibility setup.
  function objectsUnderPoint(canvasX: number, canvasY: number): number[] {
    const data = project.geometryView;
    if (!data || !lastTransform) return [];
    const { scale, offX, offY } = lastTransform;
    const dataX = (canvasX - offX) / scale;
    const dataY = (offY - canvasY) / scale;
    const tolData = HIT_PIXEL_TOL / scale;
    return queryHitObjects(
      data,
      hitIndex,
      data.objects,
      dataX,
      dataY,
      tolData,
      (l) => l === STOCK_OUTLINE_LAYER || project.data.visibleLayers.has(l),
    );
  }

  /// Tap-cycling (ivac-0rbu). On touch a small object stacked under a
  /// larger one — or under a stock-gizmo handle's midpoint — can't be
  /// reached by a single nearest-hit pick. Repeated PLAIN taps in (almost)
  /// the same spot step through the stacked candidates: tap 1 selects the
  /// topmost, tap 2 replaces it with the next under it, wrapping around.
  /// Reset whenever the tap moves past `TAP_CYCLE_PX` or the candidate
  /// stack changes (pan/zoom/edit all shift one of those). Works on the
  /// mouse too, but only ever changes behaviour where ≥2 objects overlap
  /// within the hit tolerance — single-object taps are unaffected.
  const TAP_CYCLE_PX = 12;
  let tapCycle: { cx: number; cy: number; ids: number[]; pos: number } | null = null;
  function sameIdList(a: number[], b: number[]): boolean {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
    return true;
  }
  function cycledHitObjectId(canvasX: number, canvasY: number): number | null {
    const ids = objectsUnderPoint(canvasX, canvasY);
    if (ids.length === 0) {
      tapCycle = null;
      return null;
    }
    if (
      ids.length > 1 &&
      tapCycle &&
      Math.hypot(canvasX - tapCycle.cx, canvasY - tapCycle.cy) <= TAP_CYCLE_PX &&
      sameIdList(tapCycle.ids, ids)
    ) {
      const pos = (tapCycle.pos + 1) % ids.length;
      tapCycle = { cx: canvasX, cy: canvasY, ids, pos };
      return ids[pos];
    }
    tapCycle = { cx: canvasX, cy: canvasY, ids, pos: 0 };
    return ids[0];
  }

  // ---- Stock gizmo (phone on-canvas affordance, 7jug.15) --------------
  /// Touch-friendly grab radius for the stock handles. Larger than the
  /// 8px geometry tolerance because these are deliberate finger targets.
  const STOCK_HANDLE_PX = 14;
  const STOCK_HIT_PX = 22;
  /// Smallest stock dimension a resize drag may produce (mm).
  const STOCK_MIN_MM = 1;
  // Stock-gizmo drag (`drag.stock`) and the deferred stock-handle press
  // (`drag.pendingStock`, ivac-0rbu) are owned by the PointerDragController
  // declared above. The press parks in `pendingStock` and only promotes to a
  // real `stock` drag once the finger leaves the tap tolerance, so a small
  // object under the 22 px handle stays reachable by a stationary tap.

  /// Current stock footprint in world mm (same source the renderer uses).
  function currentStockBox(): WorldBox {
    return computeFootprint(
      project.stockSizingImport,
      project.data.stock,
      project.data.machine.workArea,
    );
  }
  /// Imported-geometry bbox centre (the manual-stock centring anchor), or
  /// null when nothing is loaded.
  function currentBboxCenter(): { x: number; y: number } | null {
    const imp = project.transformedImport;
    if (!imp) return null;
    const { min_x, min_y, max_x, max_y } = imp.bbox;
    return { x: (min_x + max_x) * 0.5, y: (min_y + max_y) * 0.5 };
  }
  /// Hit-test the stock gizmo handles at a canvas pixel. Narrow-only; the
  /// desktop sidebar owns stock there. Returns the grabbed handle or null.
  function stockHandleHit(cx: number, cy: number): StockHandleKind | null {
    if (!layout.isNarrow || !lastTransform) return null;
    const box = currentStockBox();
    if (box.maxX - box.minX <= 0 || box.maxY - box.minY <= 0) return null;
    return hitStockHandle(box, lastTransform, cx, cy, STOCK_HIT_PX);
  }

  /// Convert canvas-pixel coords to data-space (mm) using the last
  /// emitted transform. Returns `null` when no transform is staged
  /// (project hasn't rendered yet).
  function pxToData(cx: number, cy: number): { x: number; y: number } | null {
    if (!lastTransform) return null;
    const t = lastTransform;
    // Canvas Y axis is inverted vs data Y (canvas grows downward).
    return { x: (cx - t.offX) / t.scale, y: -(cy - t.offY) / t.scale };
  }

  /// Tolerance for the approach-point snap, in data units. Mirrors
  /// the existing pixel-tolerance pattern used by `pixelHit` so the
  /// snap radius stays constant in screen pixels across zoom levels.
  function approachSnapToleranceData(): number {
    if (!lastTransform) return 0;
    // ~6 px feels right — matches the marker hit radius below.
    return 6 / Math.max(Math.abs(lastTransform.scale), 1e-6);
  }

  /// Snap radius for the placed marker's drag handle, in data units.
  function approachMarkerHitRadiusData(): number {
    if (!lastTransform) return 0;
    return 7 / Math.max(Math.abs(lastTransform.scale), 1e-6);
  }

  function onPointerMove(e: PointerEvent) {
    const rect = canvas.getBoundingClientRect();
    const cx = e.clientX - rect.left;
    const cy = e.clientY - rect.top;
    // Cursor coordinate HUD. Track the world (data) position on
    // every move regardless of modal mode — users want to read X/Y
    // while pan/zoom/select/picking. pxToData returns null if the
    // transform isn't staged yet (no imported drawing).
    hover.cursor = pxToData(cx, cy);
    // Feed the live touch position into the gesture tracker so a live
    // pinch reads the fresh finger positions.
    if (touch.has(e.pointerId)) {
      touch.track(e.pointerId, { x: cx, y: cy });
    }

    // The move handler is a priority pipeline with two cleanups interleaved
    // between the mode returns; reducePointerMove (lib/canvas/pointer-move.ts)
    // owns that fragile order as a pure, tested decision. We resolve the
    // predicates here and run the verbatim body the winning mode names.
    const intent = reducePointerMove({
      pinchActive: touch.pinch != null,
      promoteStock:
        drag.pendingStock != null &&
        e.pointerId === drag.pendingStock.pointerId &&
        !withinTapTolerance(
          { x: drag.pendingStock.cx0, y: drag.pendingStock.cy0 },
          { x: cx, y: cy },
        ),
      stockDragMatches: drag.stock != null && e.pointerId === drag.stock.pointerId,
      longPressWandered:
        touch.longPressStart != null && !withinTapTolerance(touch.longPressStart, { x: cx, y: cy }),
      approachPickActive,
      approachDragMatches: drag.approach != null && e.pointerId === drag.approach.pointerId,
      rasterDragMatches: drag.raster != null && e.pointerId === drag.raster.pointerId,
      textDragMatches: drag.text != null && e.pointerId === drag.text.pointerId,
      // Lazy: mirror onPointerDown's marker hit-test so the cursor flips to
      // `grab` BEFORE the user mousedowns — without it the marker is
      // draggable but invisibly so. Gated out while panning / box-selecting.
      hoverMarkerHit: () => {
        const selOp =
          project.sel.selectedOpId == null
            ? null
            : project.data.operations.find((o) => o.id === project.sel.selectedOpId);
        if (
          selOp &&
          (selOp.kind === 'profile' || selOp.kind === 'pocket') &&
          selOp.approachPoint &&
          !view.panDrag &&
          !boxSelect
        ) {
          const data = pxToData(cx, cy);
          if (data) {
            const hitR = approachMarkerHitRadiusData();
            const [ax, ay] = selOp.approachPoint;
            const dx = data.x - ax;
            const dy = data.y - ay;
            if (dx * dx + dy * dy <= hitR * hitR) return true;
          }
        }
        return false;
      },
      panActive: view.panDrag != null,
      boxDragEngaged:
        boxSelect != null &&
        (!boxSelect.armed ||
          Math.hypot(cx - boxSelect.startX, cy - boxSelect.startY) >= BOX_DRAG_THRESHOLD),
    });

    // Interleaved cleanups (faithful to the original order): a held finger
    // that wandered past tap tolerance is a drag, not a hold, so cancel the
    // pending long-press menu; and leaving pick mode drops any staged preview.
    if (intent.cancelLongPress) touch.cancelLongPress();
    if (intent.clearApproachPreview && approachPreview) approachPreview = null;

    switch (intent.mode) {
      case 'pinch': {
        const a = touch.position(touch.pinch!.idA);
        const b = touch.position(touch.pinch!.idB);
        if (a && b && lastBaseTransform) {
          view.applyPinchFrame(
            lastBaseTransform,
            { a: touch.pinch!.prevA, b: touch.pinch!.prevB },
            { a, b },
          );
          touch.pinch!.prevA = { ...a };
          touch.pinch!.prevB = { ...b };
        }
        return;
      }
      case 'stock-drag': {
        // Promote a parked stock-handle press to a real drag once the finger
        // leaves the tap tolerance (ivac-0rbu) — below that it's still a
        // candidate tap that should select the object under the handle.
        if (intent.promoteStock && drag.pendingStock) {
          drag.promoteStock();
          canvas.style.cursor = 'grabbing';
        }
        // Live stock-gizmo drag (phone). Move pans the offset (mode kept);
        // resize rewrites the box and switches to manual. Each gesture
        // coalesces into one undo via the explicit setStock key.
        const s = drag.stock;
        if (s && e.pointerId === s.pointerId) {
          const cur = pxToData(cx, cy);
          if (cur) {
            // Round gizmo output to 0.01 mm so the stored stock dims stay
            // clean numbers (a raw drag yields long float tails).
            const r2 = (n: number) => Math.round(n * 100) / 100;
            if (s.kind === 'move') {
              project.setStock(
                {
                  offsetX: r2(s.startOffsetX + (cur.x - s.grab.x)),
                  offsetY: r2(s.startOffsetY + (cur.y - s.grab.y)),
                },
                'setStock:gizmo-move',
              );
            } else {
              const nextBox = dragStockBox(
                s.kind as StockResizeKind,
                s.startBox,
                s.grab,
                cur,
                STOCK_MIN_MM,
              );
              const patch = boxToStock(nextBox, currentBboxCenter());
              project.setStock(
                {
                  mode: patch.mode,
                  customX: r2(patch.customX),
                  customY: r2(patch.customY),
                  offsetX: r2(patch.offsetX),
                  offsetY: r2(patch.offsetY),
                },
                'setStock:gizmo-resize',
              );
            }
          }
          canvas.style.cursor = 'grabbing';
          e.preventDefault();
        }
        return;
      }
      case 'approach-pick': {
        // In approach-pick mode, the cursor IS the picker — update
        // the preview marker on every move.
        const data = pxToData(cx, cy);
        if (data) {
          const tol = approachSnapToleranceData();
          const snap = shiftDown
            ? null
            : findOSnap(osnapTargets, data.x, data.y, tol, osnapSettings);
          approachPreview = snap
            ? { x: snap.x, y: snap.y, snap: snap.kind }
            : { x: data.x, y: data.y, snap: null };
        } else {
          approachPreview = null;
        }
        canvas.style.cursor = 'crosshair';
        return;
      }
      case 'approach-drag': {
        // Live drag of an already-placed approach marker.
        const data = pxToData(cx, cy);
        if (data) {
          const tol = approachSnapToleranceData();
          const snap = shiftDown
            ? null
            : findOSnap(osnapTargets, data.x, data.y, tol, osnapSettings);
          const x = snap ? snap.x : data.x;
          const y = snap ? snap.y : data.y;
          project.updateOperation(drag.approach!.opId, { approachPoint: [x, y] });
          approachPreview = { x, y, snap: snap?.kind ?? null };
        }
        canvas.style.cursor = 'grabbing';
        return;
      }
      case 'raster-drag': {
        // Live drag of a raster-engrave placement image.
        // Commits straight to the source origin (coalesced ⇒ one undo
        // entry); the overlay repaint tracks the new origin reactively.
        const data = pxToData(cx, cy);
        if (data) {
          project.updateReliefSource(drag.raster!.sourceId, {
            origin: { x: data.x - drag.raster!.grabDX, y: data.y - drag.raster!.grabDY },
          });
        }
        canvas.style.cursor = 'grabbing';
        return;
      }
      case 'text-drag': {
        // Live drag of a text layer's origin (coalesced ⇒ one
        // undo entry); the bg repaint tracks the new origin reactively.
        const data = pxToData(cx, cy);
        if (data) {
          project.updateTextLayer(drag.text!.id, {
            origin: { x: data.x - drag.text!.grabDX, y: data.y - drag.text!.grabDY },
          });
        }
        canvas.style.cursor = 'grabbing';
        return;
      }
      case 'hover-marker': {
        canvas.style.cursor = 'grab';
        return;
      }
      case 'pan': {
        // Active pan drag: the controller translates the user-pan offsets
        // by the cursor delta since the previous frame, then re-anchors.
        view.movePan(e.clientX, e.clientY);
        return;
      }
      case 'box-drag': {
        // Box-select drag: the cursor crossed BOX_DRAG_THRESHOLD px from the
        // arm point. While dragging, suppress hover hit-testing so the cursor
        // stays a crosshair.
        boxSelect = { ...boxSelect!, curX: cx, curY: cy, armed: false };
        canvas.style.cursor = 'crosshair';
        return;
      }
      case 'hover': {
        // Text-stroke hover takes precedence (text is drawn on top);
        // when over a glyph stroke we suppress the geometry hover and flip
        // the cursor to a grab affordance so the drag is discoverable.
        const tdata = pxToData(cx, cy);
        const textHover = tdata ? textHitAtData(tdata.x, tdata.y) : null;
        const idx = textHover ? null : pixelHit(cx, cy);
        hover.setEntity(idx, textHover ? textHover.id : null);
        // Tab-placement mode — project cursor to the op's
        // closest source contour and stage a ghost tab. The ghost only
        // renders when the projection is within ~6 px of the cursor
        // (screen-space) so we don't spam ghosts the user wasn't aiming at.
        if (tabPlacementActive && lastTransform) {
          const ghost = ghostTabAt(cx, cy);
          if (
            !ghost ||
            !ghostTab ||
            ghost.objectId !== ghostTab.objectId ||
            Math.abs(ghost.t - ghostTab.t) > 1e-5
          ) {
            ghostTab = ghost;
          }
        } else if (ghostTab) {
          ghostTab = null;
        }
        canvas.style.cursor = hoverCursor(textHover != null, idx != null, tabPlacementActive);
        return;
      }
    }
  }

  function onPointerUp(e: PointerEvent) {
    // Release touch tracking + end any active gesture. A quick
    // down→up cancels the long-press (it was a tap, not a hold).
    if (e.pointerType === 'touch') {
      touch.untrack(e.pointerId);
    }
    touch.cancelLongPress();

    // The priority order of "which gesture is ending" lives in the pure
    // reducer (lib/canvas/pointer-up.ts); this handler resolves each
    // drag-state → pointerId match to a boolean and performs the side
    // effects the intent names. Pointer-capture release is common to
    // every branch, so it runs once after the switch.
    const intent = reducePointerUp({
      pinchMatches:
        touch.pinch != null && (e.pointerId === touch.pinch.idA || e.pointerId === touch.pinch.idB),
      pendingStockMatches: drag.pendingStock != null && e.pointerId === drag.pendingStock.pointerId,
      stockDragMatches: drag.stock != null && e.pointerId === drag.stock.pointerId,
      approachDragMatches: drag.approach != null && e.pointerId === drag.approach.pointerId,
      rasterDragMatches: drag.raster != null && e.pointerId === drag.raster.pointerId,
      textDragMatches: drag.text != null && e.pointerId === drag.text.pointerId,
      panMatches: view.panDrag != null && e.pointerId === view.panDrag.pointerId,
      boxSelectCommittable: boxSelect != null && !boxSelect.armed,
    });

    switch (intent.kind) {
      case 'end-pinch':
        // A finger leaving a pinch ends the gesture and drops any armed
        // box-select so the remaining finger's lift is a no-op.
        touch.pinch = null;
        boxSelect = null;
        break;
      case 'stock-tap': {
        // A parked stock-handle press that never became a drag is a TAP
        // (ivac-0rbu): only when geometry sits under the press point, fall
        // through to normal object selection there — so the object under
        // the handle gets selected (and tap-cycling can step through a
        // stack). When nothing is under it we leave the selection
        // untouched (a handle tap over empty stock stays a no-op — we
        // don't clear the user's selection). A plain tap, so no box-select
        // arming (the pointer is already up).
        const { cx0, cy0 } = drag.pendingStock!;
        drag.pendingStock = null;
        canvas.style.cursor = 'default';
        if (pixelHit(cx0, cy0) != null) {
          commitEntityTapSelection(cx0, cy0, {
            shiftKey: e.shiftKey,
            ctrlKey: e.ctrlKey,
            metaKey: e.metaKey,
          });
        }
        break;
      }
      case 'end-stock-drag':
        drag.stock = null;
        canvas.style.cursor = 'default';
        break;
      case 'end-approach-drag':
        drag.approach = null;
        canvas.style.cursor = 'default';
        approachPreview = null;
        break;
      case 'end-raster-drag':
        drag.raster = null;
        canvas.style.cursor = 'default';
        break;
      case 'end-text-drag':
        drag.text = null;
        canvas.style.cursor = 'default';
        // The 3D scene doesn't track text origin per-frame (it would mean
        // a GPU rebuild on every move); nudge it once so it picks up the
        // final dragged position via the draw-time translation.
        forceTextPreviewRefresh();
        break;
      case 'end-pan':
        view.endPan();
        canvas.style.cursor = 'default';
        break;
      case 'commit-box': {
        // Commit the rubber-band selection. (An armed box that never
        // crossed the threshold collapses to a plain "click on empty",
        // already handled in onPointerDown's empty-hit branch.)
        const { startX, startY, curX, curY, mode } = boxSelect!;
        const ids = objectsInBox(startX, startY, curX, curY);
        project.selectObjects(ids, mode);
        boxSelect = null;
        canvas.style.cursor = tabPlacementActive ? 'crosshair' : 'default';
        break;
      }
      case 'clear-box':
        boxSelect = null;
        canvas.style.cursor = tabPlacementActive ? 'crosshair' : 'default';
        break;
    }

    try {
      canvas.releasePointerCapture(e.pointerId);
    } catch {
      /* may already be released; harmless */
    }
  }

  /// Wheel = cursor-pivot zoom. deltaY < 0 (scroll up) zooms in; > 0
  /// zooms out. The pan offsets are adjusted so the data-space point
  /// under the cursor stays under the cursor across the zoom.
  function onWheel(e: WheelEvent) {
    if (!lastBaseTransform) return;
    e.preventDefault();
    const rect = canvas.getBoundingClientRect();
    view.wheelZoom(lastBaseTransform, e.clientX - rect.left, e.clientY - rect.top, e.deltaY);
  }

  /// Double-click on empty space = reset pan + zoom to auto-fit.
  function onDblClick(e: MouseEvent) {
    const rect = canvas.getBoundingClientRect();
    const cx = e.clientX - rect.left;
    const cy = e.clientY - rect.top;
    if (pixelHit(cx, cy) != null) return; // hit something — don't reset
    fitView();
  }
  /// Shared reset — invoked from the fit-view button, double-click
  /// empty space, and the keyboard `F` / `Home` shortcuts. Pulls the
  /// canvas back to its auto-fit baseline (no user pan, no user zoom).
  function fitView() {
    view.reset();
    drawBackground();
  }

  /// Return the set of object ids whose bbox lies fully INSIDE the
  /// screen rectangle drawn between (x0,y0) and (x1,y1) — Illustrator /
  /// Inkscape style containment select, so dragging the rubber-band
  /// across part of an object does NOT pick it. Works in DATA
  /// coordinates: we transform the rectangle once into data space and
  /// containment-test each object's bbox.
  function objectsInBox(x0: number, y0: number, x1: number, y1: number): number[] {
    const data = project.geometryView;
    if (!data || !lastTransform) return [];
    return objectsContainedInBox(
      data.object_meta ?? [],
      project.data.visibleLayers,
      lastTransform,
      x0,
      y0,
      x1,
      y1,
      STOCK_OUTLINE_LAYER,
    );
  }
  function onPointerLeave() {
    hover.clear();
    ghostTab = null;
    // A finger dragged off the canvas can't complete a hold.
    touch.cancelLongPress();
    canvas.style.cursor = tabPlacementActive ? 'crosshair' : 'default';
  }

  /// Cache of the per-object polylines for the current import. Cleared
  /// when the import changes; the projection helpers reuse it.
  let objectPolylinesCache: ObjectPolyline[] | null = null;
  let objectPolylinesCacheKey: unknown = null;
  function getObjectPolylines(): ObjectPolyline[] {
    const imp = project.geometryView;
    if (!imp) return [];
    if (objectPolylinesCacheKey !== imp) {
      objectPolylinesCache = buildObjectPolylines(imp);
      objectPolylinesCacheKey = imp;
    }
    return objectPolylinesCache ?? [];
  }

  /// Thin reactive wrapper over the pure `projectGhostTab` geometry
  /// (lib/canvas/ghost-tab.ts): pull the selected contour op, transform,
  /// and osnap state out of component scope and assemble the context.
  /// Returns null when there's no contour op selected or no transform yet.
  function ghostTabAt(cx: number, cy: number): GhostTab | null {
    const op = selectedOp;
    if (!op || !isContourOp(op) || !lastTransform) return null;
    return projectGhostTab(cx, cy, {
      transform: lastTransform,
      polylines: getObjectPolylines(),
      sourceObjects: op.sourceObjects,
      tabPlacements: op.tabPlacements ?? undefined,
      altDown,
      osnapTargets,
      osnapSettings,
    });
  }

  /// The context-menu + tab-popover child owns its own open/close state; the
  /// parent drives it imperatively — open on right-click / long-press, dismiss
  /// on outside-click / Escape — through this ref (ivac-3xwn.2). The parent
  /// still resolves the open decision's inputs (transform, hit-test, selection)
  /// since the child never touches `project`. See EntityContextMenu.svelte.
  let ctxMenuRef = $state<ReturnType<typeof EntityContextMenu>>();

  function onContextMenu(e: MouseEvent) {
    e.preventDefault();
    // A touch long-press may have already opened the menu; a native
    // contextmenu event firing on the same hold would re-anchor it. The
    // long-press path cancels its timer, so by here any native event is
    // a real mouse right-click — just (re)open at the cursor.
    touch.cancelLongPress();
    const rect = canvas.getBoundingClientRect();
    openContextMenuAt(e.clientX - rect.left, e.clientY - rect.top);
  }

  /// Open the canvas context menu at a canvas-relative pixel position.
  /// Shared by mouse right-click (`onContextMenu`) and the touch
  /// long-press so both reach the same tab-popover / op-picker /
  /// "set text origin here" actions.
  function openContextMenuAt(cx: number, cy: number) {
    // The child runs the pure open decision (tab popover vs op menu vs
    // suppressed-empty-hint); the parent just resolves the inputs it owns —
    // the tab hit-test, the live selection, the lazy once-per-session hint,
    // and the transform for the pixel → data-space projection.
    ctxMenuRef?.open(cx, cy, {
      tabHit: findTabAtPixel(cx, cy),
      hasTextSelected: project.sel.selectedTextLayerId != null,
      hasObjsSelected: project.sel.selectedObjects.size > 0,
      consumeSelectHint,
      transform: lastTransform,
    });
  }

  /// Set the currently-selected text layer's origin to the data-space
  /// position the user right-clicked at (passed up from the context menu).
  /// No-op when no text layer is selected.
  function setTextOrigin(dataX: number, dataY: number) {
    const id = project.sel.selectedTextLayerId;
    if (id == null) return;
    project.updateTextLayer(id, { origin: { x: dataX, y: dataY } });
  }

  /// Find an op's tab placement under the cursor (canvas-space).
  /// Walks every op (not just the selected) so right-click works
  /// regardless of which op is active — matches CAD intuition
  /// ('that tab right there'). Filters ops to contour + manual/mixed tab
  /// mode here, then defers the pure projection to lib/canvas/tab-hit.
  function findTabAtPixel(cx: number, cy: number): { opId: number; placementIdx: number } | null {
    if (!lastTransform) return null;
    const ops = project.data.operations
      .filter(isContourOp)
      .filter((op) => {
        const mode = op.tabMode?.kind ?? 'off';
        return mode === 'manual' || mode === 'mixed';
      })
      .map((op) => ({ opId: op.id, placements: op.tabPlacements ?? [] }));
    return hitTabAtPixel(cx, cy, lastTransform, getObjectPolylines(), ops);
  }

  /// Update one tab placement's width / height override. Routes
  /// through updateOperation so it's a single undoable history entry.
  function patchTabOverride(
    opId: number,
    placementIdx: number,
    patch: { widthOverrideMm?: number | undefined; heightOverrideMm?: number | undefined },
  ) {
    const op = project.data.operations.find((o) => o.id === opId);
    if (!op || !isContourOp(op)) return;
    const next = patchTabPlacement(op.tabPlacements ?? [], placementIdx, patch);
    if (next) project.updateOperation(opId, { tabPlacements: next });
  }

  /// Delete one tab placement (via toggleTabPlacement — its remove
  /// branch fires when the target is within tolerance).
  function deleteTabPlacement(opId: number, placementIdx: number) {
    const op = project.data.operations.find((o) => o.id === opId);
    if (!op || !isContourOp(op)) return;
    const next = removeTabPlacement(op.tabPlacements ?? [], placementIdx);
    if (!next) return;
    project.updateOperation(opId, { tabPlacements: next });
    // The child closes its own popover after the delete.
  }

  function onCtxKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      // The child dismisses the popover / menu and reports which. A closed
      // popover CONSUMES Escape (return); a closed menu only preventDefaults so
      // the lower Escape handlers below (approach-picker, box-select) still run.
      const dismissed = ctxMenuRef?.handleEscape();
      if (dismissed === 'popover') {
        e.preventDefault();
        return;
      }
      if (dismissed === 'menu') e.preventDefault();
    }
    // F / Home reset the 2D view to its auto-fit baseline.
    // Mirrors the new `.fit-btn` overlay button and the 3D pane's
    // equivalent. Bail when the user is typing — we don't want F to
    // wipe an unrelated input.
    if (
      (e.key === 'f' || e.key === 'F' || e.key === 'Home') &&
      !e.ctrlKey &&
      !e.metaKey &&
      !e.altKey
    ) {
      const t = e.target as HTMLElement | null;
      const tag = t?.tagName ?? '';
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || t?.isContentEditable) {
        return;
      }
      fitView();
      e.preventDefault();
    }
    // ESC finalizes the approach-point picker (sticky mode exit).
    if (e.key === 'Escape' && approachPickActive) {
      project.sel.pickMode = null;
      approachPreview = null;
      canvas.style.cursor = 'default';
      e.preventDefault();
    }
    // Escape mid-drag cancels the box-select without changing the
    // current selection — FreeCAD-style.
    if (e.key === 'Escape' && boxSelect) {
      boxSelect = null;
      canvas.style.cursor = tabPlacementActive ? 'crosshair' : 'default';
      e.preventDefault();
    }
  }

  function onCtxDocClick(e: MouseEvent) {
    // The child owns the outside-click dismissal (it holds the open state and
    // self-bails cheaply when nothing is open). Wired to <svelte:window>.
    ctxMenuRef?.handleDocClick(e);
  }

  function pickFromCtx(kind: PickerKind) {
    const sel = [...project.sel.selectedObjects];
    if (sel.length === 0) return;
    createOpFromSelection(project, kind, pickerLabel(kind), sel);
    // Bounce the sidebar to Operations so the freshly-added op
    // row is visible without a second click on the sidebar. The child
    // closes its own menu after the pick.
    onActivateSidebarPane?.('operations');
  }

  function onPointerDown(e: PointerEvent) {
    const rect = canvas.getBoundingClientRect();
    const cx = e.clientX - rect.left;
    const cy = e.clientY - rect.top;

    // Stock gizmo (phone, 7jug.15): a handle press pre-empts
    // pan / select / long-press. Only on the FIRST pointer — a second
    // finger promotes to a pinch (handled below, which cancels the drag).
    //
    // ivac-0rbu: we DON'T start the resize on down. We park the press in
    // `drag.pendingStock`; pointermove promotes it to a real `drag.stock`
    // once the finger leaves the tap tolerance, and a stationary
    // tap-and-release (pointerup) falls through to object selection — so a
    // small object UNDER the handle is still reachable by tapping.
    if (e.pointerType !== 'touch' || touch.size === 0) {
      const handle = stockHandleHit(cx, cy);
      if (handle) {
        const grab = pxToData(cx, cy) ?? { x: 0, y: 0 };
        drag.pendingStock = {
          kind: handle,
          pointerId: e.pointerId,
          cx0: cx,
          cy0: cy,
          startBox: currentStockBox(),
          grab,
          startOffsetX: project.data.stock.offsetX ?? 0,
          startOffsetY: project.data.stock.offsetY ?? 0,
        };
        if (e.pointerType === 'touch') touch.track(e.pointerId, { x: cx, y: cy });
        try {
          canvas.setPointerCapture(e.pointerId);
        } catch {
          /* harmless */
        }
        // Cursor stays a pointer until the press actually becomes a drag.
        e.preventDefault();
        return;
      }
    }

    // Touch gesture tracking. A second finger promotes the
    // gesture to a pinch-zoom / two-finger pan; the first finger held
    // still becomes a long-press → context menu. Mouse / pen fall
    // straight through to the button-based paths below.
    if (e.pointerType === 'touch') {
      touch.track(e.pointerId, { x: cx, y: cy });
      if (touch.size >= 2) {
        // Abandon whatever the first finger armed (selection already
        // happened on its down; a box-select / marker drag would fight
        // the pinch), then diff finger movement from here.
        touch.cancelLongPress();
        boxSelect = null;
        drag.clearAll();
        if (touch.beginPinch()) {
          for (const id of touch.pinchIds!) {
            try {
              canvas.setPointerCapture(id);
            } catch {}
          }
        }
        canvas.style.cursor = 'default';
        e.preventDefault();
        return;
      }
      // First finger down — arm the long-press hold. The selection
      // logic below still runs (tap-to-select on down); the timer just
      // overlays a context-menu open if the finger stays put.
      touch.cancelLongPress();
      // Snapshot the selection BEFORE the pointer-down selection logic
      // (below) runs. A long-press is meant to open the "new operation
      // from selection" menu — but on touch a press that lands just off a
      // thin line reads as an empty-space tap and clears the selection, so
      // the menu would then show only the "select something first" hint.
      // Restore the snapshot in the timer if that happened, so long-press
      // never drops the user's selection.
      const selBefore = {
        objs: [...project.sel.selectedObjects],
        textId: project.sel.selectedTextLayerId,
      };
      touch.armLongPress(
        { x: cx, y: cy },
        () => {
          // Fire only while exactly one finger is still down — a pinch
          // would have cleared this. Open at the press position.
          if (touch.size !== 1) return;
          if (
            project.sel.selectedObjects.size === 0 &&
            project.sel.selectedTextLayerId == null &&
            (selBefore.objs.length > 0 || selBefore.textId != null)
          ) {
            if (selBefore.objs.length > 0) project.selectObjects(selBefore.objs, 'replace');
            project.sel.selectedTextLayerId = selBefore.textId;
          }
          openContextMenuAt(cx, cy);
        },
        LONG_PRESS_MS,
      );
    }

    // The branch ORDER (pan → pick → marker drag → raster → text → tab
    // → fixture → entity selection) lives in the pure reducer
    // (lib/canvas/pointer-down.ts); this handler supplies the lazy
    // hit-tests and performs the side effects the intent names.
    const intent = reducePointerDown({
      button: e.button,
      approachPickActive,
      tabPlacementActive,
      approachMarkerHit: () => {
        if (
          !selectedOp ||
          (selectedOp.kind !== 'profile' && selectedOp.kind !== 'pocket') ||
          !selectedOp.approachPoint
        ) {
          return false;
        }
        const data = pxToData(cx, cy);
        if (!data) return false;
        const hitR = approachMarkerHitRadiusData();
        const [ax, ay] = selectedOp.approachPoint;
        const dx = data.x - ax;
        const dy = data.y - ay;
        return dx * dx + dy * dy <= hitR * hitR;
      },
      rasterHit: () => {
        const data = pxToData(cx, cy);
        const hit = data ? rasterPlacementAtData(data.x, data.y) : null;
        if (!data || !hit) return null;
        return {
          opId: hit.op.id,
          sourceId: hit.src.id,
          grabDX: data.x - hit.src.origin.x,
          grabDY: data.y - hit.src.origin.y,
        };
      },
      textHit: () => {
        const data = pxToData(cx, cy);
        const hit = data ? textHitAtData(data.x, data.y) : null;
        if (!data || !hit) return null;
        return { id: hit.id, grabDX: data.x - hit.origin.x, grabDY: data.y - hit.origin.y };
      },
      tabGhost: () => {
        const ghost = ghostTabAt(cx, cy);
        return ghost ? { objectId: ghost.objectId, t: ghost.t } : null;
      },
      fixtureHit: () => fixtureHit(cx, cy),
    });

    const grabPointer = () => {
      try {
        canvas.setPointerCapture(e.pointerId);
      } catch {
        /* not all browsers / older versions; harmless */
      }
      canvas.style.cursor = 'grabbing';
      e.preventDefault();
    };

    switch (intent.kind) {
      case 'pan':
        view.startPan(e.clientX, e.clientY, e.pointerId);
        grabPointer();
        return;
      case 'approach-commit': {
        // Commits the snapped (or free, if Shift) cursor position.
        const data = pxToData(cx, cy);
        if (data && selectedOp) {
          const tol = approachSnapToleranceData();
          const snap = shiftDown
            ? null
            : findOSnap(osnapTargets, data.x, data.y, tol, osnapSettings);
          const x = snap ? snap.x : data.x;
          const y = snap ? snap.y : data.y;
          project.updateOperation(selectedOp.id, { approachPoint: [x, y] });
          approachPreview = { x, y, snap: snap?.kind ?? null };
        }
        e.preventDefault();
        return;
      }
      case 'approach-exit':
        project.sel.pickMode = null;
        approachPreview = null;
        e.preventDefault();
        return;
      case 'ignore':
      case 'tab-miss':
        return;
      case 'approach-drag':
        drag.approach = { opId: selectedOp!.id, pointerId: e.pointerId };
        grabPointer();
        return;
      case 'raster-drag':
        project.sel.selectedOpId = intent.grab.opId;
        drag.raster = {
          sourceId: intent.grab.sourceId,
          pointerId: e.pointerId,
          grabDX: intent.grab.grabDX,
          grabDY: intent.grab.grabDY,
        };
        grabPointer();
        return;
      case 'text-drag':
        project.sel.selectedTextLayerId = intent.grab.id;
        project.clearSelection();
        project.selectFixture(null);
        drag.text = { ...intent.grab, pointerId: e.pointerId };
        grabPointer();
        return;
      case 'tab-toggle':
        // Tolerance in t-units: ~3 px of contour length. Without an
        // exact polyline length we conservatively use 0.01 (1% of contour).
        project.toggleTabPlacement(selectedOp!.id, intent.at, 0.01);
        return;
      case 'fixture-select':
        project.selectFixture(intent.id);
        project.sel.selectedTextLayerId = null; // keep selection single-domain
        return;
      case 'entity-click':
        break;
    }

    commitEntityTapSelection(
      cx,
      cy,
      { shiftKey: e.shiftKey, ctrlKey: e.ctrlKey, metaKey: e.metaKey },
      { armBoxSelectPointerId: e.pointerId },
    );
  }

  /// Resolve an entity tap at canvas pixel `(cx, cy)` into selection side
  /// effects: deselect any active text layer, map the tap → object id (a
  /// plain tap cycles through stacked objects — ivac-0rbu), run the pure
  /// `reduceCanvasClick` reducer, and dispatch its actions.
  ///
  /// `armBoxSelectPointerId` is the pointer to capture when an empty-space
  /// plain tap arms a box-select — supplied on the pointer-DOWN path. The
  /// deferred stock-handle TAP path (pointer-up) passes `undefined` so a
  /// tap that misses geometry just clears the selection without arming a
  /// box-select against an already-lifted pointer.
  function commitEntityTapSelection(
    cx: number,
    cy: number,
    mods: { shiftKey: boolean; ctrlKey: boolean; metaKey: boolean },
    opts?: { armBoxSelectPointerId?: number },
  ) {
    // A left-click on geometry / empty space (not consumed by a
    // text-stroke hit above) deselects any active text layer, so text
    // and object selection stay mutually exclusive.
    project.sel.selectedTextLayerId = null;

    // Map the tap → its 1-based object id (or null for empty space). The
    // pure reducer in lib/canvas/entity-selection.ts resolves modifiers
    // and emits the action list; we dispatch and arm the box-select store.
    //
    // A PLAIN tap (no modifier / add-to-selection) cycles through stacked
    // objects so a small one hidden under a larger one stays reachable
    // (ivac-0rbu). Modifier / add-to-selection taps keep the single
    // nearest hit and reset the cycle — multi-select and cycling don't
    // compose.
    const plainTap = !mods.shiftKey && !mods.ctrlKey && !mods.metaKey && !addToSelection;
    let hitObjectId: number | null;
    if (plainTap) {
      hitObjectId = cycledHitObjectId(cx, cy);
    } else {
      tapCycle = null;
      const idx = pixelHit(cx, cy);
      hitObjectId = idx == null ? null : (project.geometryView?.objects?.[idx] ?? 0);
    }
    const actions = reduceCanvasClick(
      {
        hitObjectId,
        shiftKey: mods.shiftKey,
        // The "add to selection" toggle stands in for ctrl on a
        // keyboardless touch device — a plain tap then toggles.
        ctrlKey: mods.ctrlKey || addToSelection,
        metaKey: mods.metaKey,
      },
      objectToOps,
    );
    for (const action of actions) {
      switch (action.kind) {
        case 'clear-selection':
          project.clearSelection();
          break;
        case 'clear-fixture-selection':
          project.selectFixture(null);
          break;
        case 'select-objects':
          project.selectObjects(action.ids, action.mode);
          break;
        case 'series-select-to':
          project.seriesSelectTo(action.id);
          break;
        case 'set-active-op':
          if (project.sel.selectedOpId !== action.opId) project.sel.selectedOpId = action.opId;
          break;
        case 'arm-box-select':
          if (opts?.armBoxSelectPointerId == null) break;
          boxSelect = {
            startX: cx,
            startY: cy,
            curX: cx,
            curY: cy,
            mode: action.mode,
            armed: true,
          };
          // Capture so pointermove keeps firing if the user drags past
          // the canvas edge — otherwise box-select would freeze at the
          // last point inside the canvas.
          try {
            canvas.setPointerCapture(opts.armBoxSelectPointerId);
          } catch {
            /* not all browsers / older versions; harmless */
          }
          break;
      }
    }
  }

  /// Returns the id of the fixture under the cursor, or null. Hit-test
  /// runs in data coordinates: a click is "inside" a Box / Cylinder if
  /// the point is inside their AABB / disc, and inside a Polygon by
  /// Hit-test fixtures in canvas pixel space. Pure shape-inclusion
  /// logic delegated to `lib/canvas/fixture-hit.ts`;
  /// the component just converts canvas-pixel to data-space and
  /// passes the current fixture list.
  function fixtureHit(canvasX: number, canvasY: number): number | null {
    if (!lastTransform) return null;
    const { scale, offX, offY } = lastTransform;
    const dataX = (canvasX - offX) / scale;
    const dataY = (offY - canvasY) / scale;
    return fixtureAt(project.data.fixtures, dataX, dataY);
  }

  function colorFor(c: number): string {
    const r = resolveAci(c);
    return r.kind === 'fixed' ? hexToCss(r.hex) : themeVar(r.token, hexToCss(r.fallback));
  }

  /// Idempotent canvas-size + DPR sync. Returns the painting context +
  /// CSS-pixel dimensions, or null when the host isn't mounted yet.
  function setupCanvas(
    c: HTMLCanvasElement | undefined,
  ): { ctx: CanvasRenderingContext2D; w: number; h: number } | null {
    if (!c || !container) return null;
    const ctx = c.getContext('2d');
    if (!ctx) return null;
    const dpr = window.devicePixelRatio || 1;
    const w = container.clientWidth;
    const h = container.clientHeight;
    // Only reallocate the backing store on real size changes — setting
    // canvas.width on every redraw allocates a fresh GPU buffer and
    // clears it.
    const targetW = w * dpr;
    const targetH = h * dpr;
    if (c.width !== targetW) c.width = targetW;
    if (c.height !== targetH) c.height = targetH;
    if (c.style.width !== `${w}px`) c.style.width = `${w}px`;
    if (c.style.height !== `${h}px`) c.style.height = `${h}px`;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    return { ctx, w, h };
  }

  /// Auto-fit transform compute. Reads bbox + user pan/zoom and writes
  /// `lastTransform` / `lastBaseTransform` so the pointer handlers + the
  /// overlay layer see the same projection as the bg layer.
  /// Thin reactive wrapper over `computeViewportTransform` (lib/canvas/
  /// viewport.ts): pulls the live user pan/zoom out of $state, computes
  /// the active transform, and caches both the base and active values
  /// so hit-tests can read them without recomputing.
  function computeTransform(
    bbox: BBox,
    w: number,
    h: number,
  ): {
    scale: number;
    offX: number;
    offY: number;
    project2: (x: number, y: number) => [number, number];
  } {
    const t = computeViewportTransform(bbox, { w, h }, view.userView);
    lastBaseTransform = { scale: t.baseScale, offX: t.baseOffX, offY: t.baseOffY };
    lastTransform = { scale: t.scale, offX: t.offX, offY: t.offY };
    return { scale: t.scale, offX: t.offX, offY: t.offY, project2: t.project2 };
  }

  /// The bbox the viewport auto-fits to. STABLE on purpose: a text-only
  /// project's `geometryView` is the synthetic stock outline, which is sized
  /// to the text and MOVES with it — fitting the view to that would make the
  /// canvas chase the text during a drag (a tiny finger move reads as a huge
  /// jump). Prefer real imported geometry; else the work-area / placement
  /// bed (stable); only fall back to the live geometryView bbox if neither
  /// exists.
  function viewportFitBBox(): BBox | null {
    const imp = project.transformedImport;
    if (imp && imp.segments.length > 0) return imp.bbox;
    const fb = placementFallbackBBox();
    if (fb) return fb;
    const gv = project.geometryView;
    return gv && gv.segments.length > 0 ? gv.bbox : null;
  }

  /// Heavy static layer — repaints only on geometry / layout / theme /
  /// zoom / pan changes. State-bearing repaints (hover / selection /
  /// ghost tab / box-select / approach point) happen on the overlay
  /// canvas via drawOverlay() and do NOT invalidate this layer.
  function drawBackground() {
    const setup = setupCanvas(canvas);
    if (!setup) return;
    const { ctx, w, h } = setup;
    ctx.fillStyle = themeVar('--bg-app', '#0d0d0d');
    ctx.fillRect(0, 0, w, h);

    const data = project.geometryView;
    const hasGeom = !!data && data.segments.length > 0;
    const fitBBox = viewportFitBBox();
    if (!fitBBox) {
      ctx.fillStyle = themeVar('--canvas-empty', '#555');
      ctx.font = '13px system-ui, sans-serif';
      ctx.fillText('Open a file to view geometry', 16, 24);
      return;
    }

    const { scale, offX, offY, project2 } = computeTransform(fitBBox, w, h);

    drawGrid(ctx, w, h, scale, offX, offY, {
      minor: themeVar('--grid-minor', '#1a1a1a'),
      major: themeVar('--grid-major', '#262626'),
    });
    drawAxes(ctx, w, h, offX, offY, {
      x: themeVar('--axis-x', '#882222'),
      y: themeVar('--axis-y', '#226622'),
    });
    drawWorkArea(ctx, project2, project.data.machine.workArea, themeVar('--text-muted', '#888'));
    const stockBox = computeFootprint(
      project.stockSizingImport,
      project.data.stock,
      project.data.machine.workArea,
    );
    drawStock(ctx, project2, stockBox, themeVar('--stock-edge', '#888'));
    // Phone: overlay grab handles so the stock rect is directly
    // manipulable (move + resize) without a sidebar (7jug.15). Desktop
    // edits stock via the StockPanel, so the gizmo is narrow-only.
    if (layout.isNarrow) {
      drawStockGizmo(
        ctx,
        stockBox,
        scale,
        offX,
        offY,
        STOCK_HANDLE_PX,
        themeVar('--stock-edge', '#888'),
        themeVar('--accent', '#2d6cdf'),
        themeVar('--bg-elevated', '#222'),
        drag.stock?.kind ?? null,
      );
    }

    // Imported-geometry chrome (regions + base wireframe) — only when a
    // DXF is loaded. A placement-only project skips straight to the text
    // previews below (raster images live on the overlay).
    if (hasGeom && data) {
      // Filled-region preview painted under the wireframe so contours
      // stay legible. Regions come from the backend (pipeline.rs
      // build_region_previews).
      const regions = project.gen.generated?.regions ?? [];
      if (regions.length > 0 && project.data.regionsVisible) {
        drawRegions(
          ctx,
          regionPathCache,
          regions,
          scale,
          offX,
          offY,
          project.sel.selectedOpId,
          themeVar('--accent', '#2d6cdf'),
        );
      }

      // Imported segments — paint in BASE layer color only. State-bearing
      // overlays (selection / hover / op-assignment halos) go on the
      // overlay canvas, so editing those does NOT invalidate this layer.
      const visibleLayersSnap = new Set(project.data.visibleLayers);
      visibleLayersSnap.add(STOCK_OUTLINE_LAYER); // synthetic layer always drawn
      drawImportedWireframe(
        ctx,
        project2,
        data.segments,
        visibleLayersSnap,
        project.data.settings.previewLineWidth,
        colorFor,
      );
    }

    // Text-layer previews. Rendered with OR without imported geometry so
    // a text-only engrave project is visible (and draggable) on a bare
    // canvas. The cache is filled by requestPreview() in the top-of-file
    // effect; the active layer (selectedTextLayerId) gets the highlight.
    if (project.data.textLayers.length > 0) {
      drawTextPreview(
        ctx,
        project2,
        project.data.textLayers.map((layer) => ({
          // Segments come back translated to the layer's current origin, so
          // a drag repositions the glyphs with no re-render.
          segments: previewSegmentsFor(layer.id, layer.origin) ?? [],
          isActive: project.sel.selectedTextLayerId === layer.id,
        })),
        {
          accent: themeVar('--accent', '#2d6cdf'),
          halo: themeVar('--text-strong', '#ffffff'),
          idle: themeVar('--obj-assigned-other', '#2a6f3b'),
        },
      );
    }
  }

  /// State-bearing overlay — selection halos + accent strokes, hover
  /// halo, op-assignment tints, fixtures, tab markers + ghost tab,
  /// approach-point marker + OSnap glyph, box-select rect. Cleared and
  /// repainted on every interaction-state change; the bg canvas stays
  /// put.
  function drawOverlay() {
    const setup = setupCanvas(canvasOverlay);
    if (!setup) return;
    const { ctx, w, h } = setup;
    // Transparent clear — bg shows through.
    ctx.clearRect(0, 0, w, h);

    const data = project.geometryView;
    const hasGeom = !!data && data.segments.length > 0;
    // Mirror drawBackground — same stable fit bbox so the overlay projection
    // matches the bg layer (and doesn't chase a dragged text layer).
    const fitBBox = viewportFitBBox();
    if (!fitBBox) return;
    const { scale, project2 } = computeTransform(fitBBox, w, h);

    const accent = themeVar('--accent', '#2d6cdf');

    // Faint raster-engrave placement images, painted
    // first so selection halos / chrome layer over them.
    drawRasterPlacements(
      ctx,
      project2,
      scale,
      rasterImageCache,
      rasterPlacements.map(({ op, src }) => ({
        src,
        selected: project.sel.selectedOpId === op.id,
      })),
      { accent, border: themeVar('--border', '#555') },
    );

    // Hover highlight for the text layer under the cursor (the
    // selected-layer highlight stays on the bg in drawTextPreview). Drawn
    // on the overlay so frequent hover repaints don't touch the bg layer.
    if (hover.textId != null && hover.textId !== project.sel.selectedTextLayerId) {
      const hoverLayer = project.data.textLayers.find((l) => l.id === hover.textId);
      const segs = hoverLayer ? previewSegmentsFor(hover.textId, hoverLayer.origin) : null;
      if (segs && segs.length > 0) {
        const hoverColor = themeVar('--accent-strong', '#6e9ce6');
        ctx.lineWidth = 1.6;
        ctx.strokeStyle = hoverColor;
        for (const seg of segs) drawSegment(ctx, seg, project2);
      }
    }

    if (hasGeom && data) {
      const visibleLayersSnap = new Set(project.data.visibleLayers);
      visibleLayersSnap.add(STOCK_OUTLINE_LAYER); // synthetic layer always drawn
      drawEntityHalos(ctx, project2, {
        segments: data.segments,
        objects: data.objects,
        visibleLayers: visibleLayersSnap,
        selectedObjects: new Set(project.sel.selectedObjects),
        hoverObjectId: hover.objectIdx == null ? 0 : (data.objects?.[hover.objectIdx] ?? 0),
        objectToOps,
        selectedOpId: project.sel.selectedOpId,
        opColor: opSourceCss,
        colors: {
          hover: themeVar('--accent-strong', '#6e9ce6'),
          // Uses --text-strong so the contrast halo inverts automatically
          // in light theme.
          halo: themeVar('--text-strong', '#ffffff'),
          accent,
        },
      });
    }

    drawFixtures(ctx, project2, project.data.fixtures, project.sel.selectedFixtureId, accent);
    drawTabs(
      ctx,
      project2,
      scale,
      project.data.operations.filter(isContourOp),
      getObjectPolylines(),
      // Ghost: selected op + manual/mixed mode + cursor over contour.
      ghostTab && tabPlacementActive && selectedOp && isContourOp(selectedOp)
        ? { tab: ghostTab, op: selectedOp }
        : null,
      {
        fill: themeVar('--tab-marker', '#ffd23a'),
        auto: themeVar('--tab-auto', '#ffeb88'),
        stroke: themeVar('--bg-app', '#0d0d0d'),
        accent,
      },
    );
    // approachPoint lives on ContourFields, currently shared only by
    // Profile + Pocket on the FE type side. (The BE accepts it on
    // Engrave / DragKnife too; expanding the FE types is a follow-up.)
    if (selectedOp && (selectedOp.kind === 'profile' || selectedOp.kind === 'pocket')) {
      drawApproachPoint(
        ctx,
        project2,
        selectedOp.approachPoint ?? null,
        approachPickActive || drag.approach != null ? approachPreview : null,
        {
          marker: themeVar('--accent', '#3aa'),
          snap: themeVar('--success', '#3c3'),
          ring: themeVar('--text', '#000'),
        },
      );
    }
    if (boxSelect && !boxSelect.armed) {
      drawBoxSelect(ctx, boxSelect, accent);
    }
  }

  /// The distinct relief sources referenced by raster-engrave ops, each
  /// paired with the (first) op referencing it — so the selection
  /// highlight + drag target know which op an image belongs to.
  // Cached between repaints (recomputed only when ops / relief sources
  // change) instead of rebuilt on every overlay paint + pointer hit-test.
  const rasterPlacements = $derived.by<{ op: OpEntry; src: ReliefSource }[]>(() => {
    const out: { op: OpEntry; src: ReliefSource }[] = [];
    const seen = new Set<number>();
    for (const op of project.data.operations) {
      if (op.kind !== 'raster_engrave' || !op.enabled) continue;
      const src = project.data.reliefSources.find((s) => s.id === op.sourceId);
      if (!src || src.cols <= 0 || src.rows <= 0 || seen.has(src.id)) continue;
      seen.add(src.id);
      out.push({ op, src });
    }
    return out;
  });

  /// Hit-test a data-space point against the placed raster images,
  /// preferring the selected op's image (so overlapping placements stay
  /// grabbable) then the topmost. Returns the placement or null.
  function rasterPlacementAtData(x: number, y: number): { op: OpEntry; src: ReliefSource } | null {
    // Copy before sorting — `rasterPlacements` is a shared cached derived,
    // and `.sort()` mutates in place.
    const ordered = [...rasterPlacements].sort(
      (a, b) =>
        (a.op.id === project.sel.selectedOpId ? 1 : 0) -
        (b.op.id === project.sel.selectedOpId ? 1 : 0),
    );
    for (let i = ordered.length - 1; i >= 0; i--) {
      const { src } = ordered[i];
      const wmm = src.cols * src.cell;
      const hmm = src.rows * src.cell;
      if (
        x >= src.origin.x &&
        x <= src.origin.x + wmm &&
        y >= src.origin.y &&
        y <= src.origin.y + hmm
      ) {
        return ordered[i];
      }
    }
    return null;
  }

  /// A viewport bbox for a project with no imported
  /// geometry + no visible stock, so placement-only entities (raster
  /// images, text layers) still render + drag. Prefers the machine work
  /// area — a STABLE reference, so the view doesn't jiggle while an
  /// origin is dragged (the entity moves within the bed). Falls back to
  /// the union of all placement extents (+10% margin) when no bed is
  /// defined. Null when there's nothing placeable to frame.
  function placementFallbackBBox(): BBox | null {
    const rects: Rect[] = [];
    for (const { src } of rasterPlacements) {
      rects.push({
        minX: src.origin.x,
        minY: src.origin.y,
        maxX: src.origin.x + src.cols * src.cell,
        maxY: src.origin.y + src.rows * src.cell,
      });
    }
    for (const layer of project.data.textLayers) {
      const bb = segsBBox(previewSegmentsFor(layer.id, layer.origin) ?? []);
      if (bb) rects.push(bb);
    }
    if (rects.length === 0) return null;
    const wa = project.data.machine.workArea;
    if (wa && wa.x > 0 && wa.y > 0) {
      return { min_x: 0, min_y: 0, max_x: wa.x, max_y: wa.y };
    }
    return placementsBBox(rects);
  }

  /// The text layer whose glyph STROKE is under a data-space point
  /// (within the screen-constant pixel tolerance), or null. Uses the
  /// rendered preview segments so the mostly-whitespace bbox doesn't
  /// steal clicks; topmost layer wins a tie. Drives both hover and
  /// click-select / drag.
  /// Grabbing a text layer means landing within tolerance of a thin glyph
  /// STROKE — fiddly with a fingertip. Give touch a much larger pixel
  /// tolerance so the whole word is comfortably grabbable/movable, without
  /// loosening geometry selection (which keeps HIT_PIXEL_TOL).
  const TEXT_TOUCH_TOL = 24;
  function textHitAtData(x: number, y: number): TextLayer | null {
    if (!lastTransform || project.data.textLayers.length === 0) return null;
    const px = isTouchDevice ? TEXT_TOUCH_TOL : HIT_PIXEL_TOL;
    const tol = px / Math.max(Math.abs(lastTransform.scale), 1e-6);
    const hit = nearestTextLayer(
      project.data.textLayers.map((l) => ({
        id: l.id,
        segments: previewSegmentsFor(l.id, l.origin) ?? [],
      })),
      x,
      y,
      tol,
    );
    return hit ? (project.data.textLayers.find((l) => l.id === hit.id) ?? null) : null;
  }

  /// Path2D cache for region previews — see RegionPathCache
  /// (lib/canvas/render/regions.ts).
  const regionPathCache = new RegionPathCache();
</script>

<svelte:window
  onkeydown={(e) => {
    onCtxKeydown(e);
    if (e.key === 'Alt' || e.altKey) altDown = true;
    if (e.key === 'Shift' || e.shiftKey) shiftDown = true;
  }}
  onkeyup={(e) => {
    if (e.key === 'Alt' || !e.altKey) altDown = false;
    if (e.key === 'Shift' || !e.shiftKey) shiftDown = false;
  }}
  onblur={() => {
    altDown = false;
    shiftDown = false;
  }}
  onclick={onCtxDocClick}
/>
<div class="canvas-host" bind:this={container}>
  <canvas
    bind:this={canvas}
    class="bg"
    onpointermove={onPointerMove}
    onpointerleave={onPointerLeave}
    onpointerdown={onPointerDown}
    onpointerup={onPointerUp}
    onpointercancel={onPointerUp}
    oncontextmenu={onContextMenu}
    onwheel={onWheel}
    ondblclick={onDblClick}
  ></canvas>
  <canvas bind:this={canvasOverlay} class="overlay"></canvas>
  {#if project.sel.selectedEntities.size > 0}
    <div class="selection-hud">
      {t('canvas.selection_hud', { count: project.sel.selectedEntities.size })}
    </div>
  {/if}
  {#if hover.cursor}
    <div class="cursor-hud" aria-hidden="true">
      x: {hover.cursor.x.toFixed(2)} &nbsp; y: {hover.cursor.y.toFixed(2)} mm
      {#if shiftDown && (approachPickActive || tabPlacementActive)}
        <!-- Visible cue that Shift is suppressing snap.
             Without this the snap glyph just silently disappears and
             the user can't tell why their click stops locking. -->
        <span class="snap-off">{t('canvas.snap_off')}</span>
      {/if}
    </div>
  {/if}
  {#if project.transformedImport && project.data.operations.length === 0}
    <div class="firstrun-hint" role="status">
      {#if isTouchDevice}
        <span class="firstrun-step">1</span>
        <span>{t('canvas.firstrun.tap_select')}</span>
        <span class="firstrun-arrow">→</span>
        <span class="firstrun-step">2</span>
        <span>{t('canvas.firstrun.multiselect')}</span>
        <span class="firstrun-arrow">→</span>
        <span class="firstrun-step">3</span>
        <span>{t('canvas.firstrun.longpress_op')}</span>
      {:else}
        <span class="firstrun-step">1</span>
        <span>{t('canvas.firstrun.click_select')}</span>
        <span class="firstrun-arrow">→</span>
        <span class="firstrun-step">2</span>
        <span>{t('canvas.firstrun.rightclick_op')}</span>
      {/if}
    </div>
  {/if}
  <EntityContextMenu
    bind:this={ctxMenuRef}
    operations={project.data.operations}
    hasTextSelected={project.sel.selectedTextLayerId != null}
    hasObjsSelected={project.sel.selectedObjects.size > 0}
    onPatchTab={patchTabOverride}
    onDeleteTab={deleteTabPlacement}
    onSetTextOrigin={setTextOrigin}
    onPick={pickFromCtx}
  />
  <!-- Fit-to-view affordance mirroring Scene3D's .fit-btn.
       Doubleclick on empty space already resets, but that's undocumented
       — adding the button gives an obvious affordance and matches the 3D
       pane. F / Home shortcuts also call fitView when canvas has focus. -->
  <button
    type="button"
    class="fit-btn"
    onclick={fitView}
    title={t('canvas.fit_view.title')}
    aria-label={t('canvas.fit_view.aria')}
  >
    ⌖
  </button>
  {#if isTouchDevice}
    <!-- Keyboardless multi-select toggle. On touch there's no
         shift/ctrl-tap, so this latches "tap = add/remove from
         selection". Touch devices only — mouse users have modifiers. -->
    <button
      type="button"
      class="multiselect-btn"
      class:active={addToSelection}
      aria-pressed={addToSelection}
      onclick={() => (addToSelection = !addToSelection)}
      title={t('canvas.add_to_selection.title')}
      aria-label={t('canvas.add_to_selection.aria')}
    >
      ⧉
    </button>
  {/if}
  {#if onShowHelp}
    <button
      type="button"
      class="help-btn"
      onclick={onShowHelp}
      title={t('canvas.help.title')}
      aria-label={t('canvas.help.aria')}>?</button
    >
  {/if}
  <!-- Phone Stock/Layers are now the 2D bottom-left "S+L" sheet
       (ivac-yc25), not floating canvas chips. The stock drag
       gizmo still lives on the rect itself. -->
</div>

<style>
  .canvas-host {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
    background: var(--bg-app);
  }
  canvas {
    display: block;
    user-select: none;
    touch-action: none;
  }
  /* Stack bg + overlay canvases so the overlay paints state-bearing
     items (selection / hover / ghost tab / fixtures / tabs / approach /
     box-select / OSnap glyph) without invalidating the heavy imported-
     geometry layer. The overlay must not eat pointer events. */
  canvas.bg,
  canvas.overlay {
    position: absolute;
    top: 0;
    left: 0;
  }
  canvas.overlay {
    pointer-events: none;
  }
  .selection-hud {
    position: absolute;
    top: 0.5rem;
    left: 0.5rem;
    background: color-mix(in srgb, var(--accent) 80%, transparent);
    color: white;
    padding: 0.2rem 0.5rem;
    border-radius: 3px;
    font-size: 0.72rem;
    pointer-events: none;
  }
  /* First-run hint when imported && no ops. Center bottom so it
     hangs below the geometry without covering it. Auto-dismisses the
     instant the user adds an op. */
  .firstrun-hint {
    position: absolute;
    bottom: 1.2rem;
    left: 50%;
    transform: translateX(-50%);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-wrap: wrap;
    gap: 0.4rem 0.55rem;
    padding: 0.5rem 0.9rem;
    background: color-mix(in srgb, var(--bg-elevated) 92%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent) 35%, var(--border));
    border-radius: 16px;
    color: var(--text-strong);
    font-size: 0.78rem;
    box-shadow: 0 6px 18px var(--shadow-modal);
    pointer-events: none;
    max-width: calc(100% - 2rem);
    /* Above the layer / stock-info chips (var(--z-floating)) so the hint
       isn't clipped behind them on the phone layout. */
    z-index: calc(var(--z-floating) + 1);
  }
  .firstrun-step {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.4rem;
    height: 1.4rem;
    border-radius: 50%;
    background: var(--accent);
    color: #fff;
    font-weight: 700;
    font-size: 0.72rem;
  }
  .firstrun-arrow {
    color: var(--text-muted);
    font-size: 0.85rem;
  }
  /* Cursor world-coordinate HUD. Top-right corner so it doesn't
     fight the selection-hud (top-left). Monospace tabular-nums so the
     numbers don't dance as the cursor moves. */
  .cursor-hud {
    position: absolute;
    /* Sits below the fit-btn / help-btn cluster so it doesn't collide
       with the round overlay buttons in the top-right corner. */
    top: 2.4rem;
    right: 0.5rem;
    background: color-mix(in srgb, var(--bg-elevated) 85%, transparent);
    color: var(--text);
    padding: 0.2rem 0.5rem;
    border: 1px solid var(--border);
    border-radius: 3px;
    font-size: 0.72rem;
    font-family: ui-monospace, monospace;
    font-variant-numeric: tabular-nums;
    pointer-events: none;
    white-space: nowrap;
  }
  .cursor-hud .snap-off {
    margin-left: 0.5rem;
    padding: 0 0.3rem;
    border-radius: 2px;
    background: color-mix(in srgb, var(--warn) 28%, transparent);
    color: var(--text-strong);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.02em;
  }
  /* Fit-to-view button — visual twin of Scene3D's .fit-btn, sits to
     the LEFT of the help-btn so both float in the same top-right
     cluster regardless of which pane the user is in. */
  .fit-btn {
    position: absolute;
    top: 0.5rem;
    right: 2.5rem;
    width: 1.6rem;
    height: 1.6rem;
    border-radius: 50%;
    border: 1px solid var(--border);
    background: var(--bg-elevated);
    color: var(--text-muted);
    cursor: pointer;
    font-size: 1rem;
    line-height: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    opacity: 0.7;
    transition:
      opacity 0.12s ease,
      color 0.12s ease;
  }
  .fit-btn:hover,
  .fit-btn:focus-visible {
    opacity: 1;
    color: var(--text-strong);
  }
  /* Keyboardless multi-select toggle — visual twin of .fit-btn,
     sits one slot further left. Latches an accent fill while active. */
  .multiselect-btn {
    position: absolute;
    top: 0.5rem;
    right: 4.5rem;
    width: 1.6rem;
    height: 1.6rem;
    border-radius: 50%;
    border: 1px solid var(--border);
    background: var(--bg-elevated);
    color: var(--text-muted);
    cursor: pointer;
    font-size: 0.95rem;
    line-height: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    opacity: 0.7;
    transition:
      opacity 0.12s ease,
      color 0.12s ease,
      background 0.12s ease;
  }
  .multiselect-btn:hover,
  .multiselect-btn:focus-visible {
    opacity: 1;
    color: var(--text-strong);
  }
  .multiselect-btn.active {
    opacity: 1;
    color: var(--accent-contrast, #fff);
    background: var(--accent);
    border-color: var(--accent);
  }
  .help-btn {
    position: absolute;
    top: 0.5rem;
    right: 0.5rem;
    width: 1.6rem;
    height: 1.6rem;
    border-radius: 50%;
    border: 1px solid var(--border);
    background: var(--bg-elevated);
    color: var(--text-muted);
    cursor: pointer;
    font-size: 0.85rem;
    font-weight: bold;
    line-height: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    opacity: 0.7;
    transition:
      opacity 0.12s ease,
      color 0.12s ease;
  }
  .help-btn:hover,
  .help-btn:focus-visible {
    opacity: 1;
    color: var(--text-strong);
  }

  /* Lift the corner buttons ABOVE the gesture catch layers (the
     pull-to-refresh strip at the top and the EdgeSwipeNav edge zones at
     the sides, both at --z-floating). Without this, those transparent
     zones sit over the buttons and swallow the tap — the help (?) button
     at right:0.5rem is under the right edge zone, and the fit button was
     under the pull strip. Higher z-index lets a tap on the button hit the
     button while gestures elsewhere still reach the zones. */
  .fit-btn,
  .multiselect-btn,
  .help-btn {
    z-index: calc(var(--z-floating) + 2);
  }

  /* Touch: (1) enlarge the 1.6rem corner buttons to a ~40px target, and
     (2) drop them BELOW the pull-to-refresh catch strip (top 28px of the
     canvas) — that strip captured pointer events over the buttons, so a
     tap at top:0.5rem hit the refresh zone instead of the button. Moving
     them to top:2.4rem clears the strip so they're tappable again. */
  @media (pointer: coarse) {
    .fit-btn,
    .multiselect-btn,
    .help-btn {
      top: 2.4rem;
      width: 2.5rem;
      height: 2.5rem;
      opacity: 0.85;
    }
    /* Right offsets clear the 22px (≈1.4rem) EdgeSwipeNav right edge zone
       so the buttons don't physically overlap it (the top:2.4rem already
       clears the pull strip). */
    .help-btn {
      right: 1.75rem;
    }
    .fit-btn {
      right: 4.6rem;
    }
    .multiselect-btn {
      right: 7.45rem;
    }
  }
</style>
