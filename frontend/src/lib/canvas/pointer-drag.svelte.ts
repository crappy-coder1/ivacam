/// Reactive active-pointer-DRAG controller for EntityCanvas2D (ivac-1hxn
/// slice): the four mutually-exclusive drag gestures the canvas commits
/// live — repositioning an approach marker, a raster-engrave placement, a
/// text-layer origin, and the stock gizmo — plus the pre-commit "parked"
/// stock-handle press. Before this these were five loose states in the
/// component; giving them an owning module mirrors the ViewController /
/// TouchTracker precedent (lib/canvas/*.svelte.ts + touch-tracker.ts) and
/// gives the drag lifecycle a home OUTSIDE the .svelte file, so the
/// component can't silently regrow.
///
/// At most one of `approach` / `raster` / `text` / `stock` is ever set at a
/// time — they're armed in mutually-exclusive pointerdown branches, each
/// keyed to the captured pointer id. `pendingStock` is stock's two-phase
/// pre-commit (ivac-0rbu): a handle press parks here and only PROMOTES to
/// `stock` once the finger leaves the tap tolerance, so a small object under
/// the handle stays tappable.
///
/// `approach` must be reactive: EntityCanvas2D's `osnapTargets` $derived
/// reads it to precompute snap targets when a marker drag starts OUTSIDE
/// pick mode. The other three are read only from event handlers but stay
/// `$state` for consistency. `pendingStock` is intentionally NON-reactive (a
/// plain field): nothing reactive observes it, matching the original loose
/// `let`.
///
/// A thin rune wrapper with no fragile math — so, like HoverState /
/// ViewController, it needs no separate unit test.

import type { StockHandleKind, WorldBox } from './stock-gizmo';

/// Repositioning an already-placed approach marker (Option C: hybrid pick +
/// draggable). Captured on pointerdown inside the marker's hit circle.
export interface ApproachDrag {
  opId: number;
  pointerId: number;
}

/// Repositioning a raster-engrave placement image. `grabDX/DY` is the
/// data-space pointer→origin offset at grab time, so the origin tracks the
/// cursor without jumping.
export interface RasterDrag {
  sourceId: number;
  pointerId: number;
  grabDX: number;
  grabDY: number;
}

/// Repositioning a text layer's origin. `grabDX/DY` as for RasterDrag.
export interface TextDrag {
  id: number;
  pointerId: number;
  grabDX: number;
  grabDY: number;
}

/// Active stock-gizmo drag. `move` pans the offset (mode preserved); resize
/// kinds rewrite the box and switch to manual mode. `startBox` is the world
/// footprint at grab; `grab` is the world point first touched;
/// `startOffset*` seeds the move delta.
export interface StockDrag {
  kind: StockHandleKind;
  pointerId: number;
  startBox: WorldBox;
  grab: { x: number; y: number };
  startOffsetX: number;
  startOffsetY: number;
}

/// A stock-handle press that hasn't yet committed to a resize/move drag
/// (ivac-0rbu). Carries the press pixel (`cx0/cy0`) plus everything needed
/// to build the `StockDrag` once the finger crosses the tap tolerance.
export interface PendingStockGrab {
  kind: StockHandleKind;
  pointerId: number;
  cx0: number;
  cy0: number;
  startBox: WorldBox;
  grab: { x: number; y: number };
  startOffsetX: number;
  startOffsetY: number;
}

export class PointerDragController {
  /// Repositioning an already-placed approach marker; null when idle.
  approach = $state<ApproachDrag | null>(null);
  /// Repositioning a raster-engrave placement; null when idle.
  raster = $state<RasterDrag | null>(null);
  /// Repositioning a text-layer origin; null when idle.
  text = $state<TextDrag | null>(null);
  /// Active stock-gizmo drag; null when idle.
  stock = $state<StockDrag | null>(null);
  /// Parked stock-handle press awaiting promotion. Non-reactive: nothing
  /// reactive observes it (read only from the pointer handlers).
  pendingStock: PendingStockGrab | null = null;

  /// Promote a parked stock-handle press to a live `stock` drag, clearing
  /// the pending slot. Called once the finger leaves the tap tolerance
  /// (ivac-0rbu). No-op if nothing is parked. The caller owns the cursor
  /// change (`grabbing`) since that's a DOM side effect.
  promoteStock(): void {
    const p = this.pendingStock;
    if (!p) return;
    this.stock = {
      kind: p.kind,
      pointerId: p.pointerId,
      startBox: p.startBox,
      grab: p.grab,
      startOffsetX: p.startOffsetX,
      startOffsetY: p.startOffsetY,
    };
    this.pendingStock = null;
  }

  /// Drop every active + pending drag. Used when a second finger promotes
  /// the gesture to a pinch — whatever the first finger armed must yield.
  clearAll(): void {
    this.approach = null;
    this.raster = null;
    this.text = null;
    this.stock = null;
    this.pendingStock = null;
  }
}
