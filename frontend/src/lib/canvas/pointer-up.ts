// Pure pointer-up reduction for EntityCanvas2D. The canvas pointerup
// handler is a priority chain of "which gesture is ending" — pinch,
// stock-handle tap-vs-drag, marker / raster / text / pan drags, and a
// box-select commit tail. Like reducePointerDown, the PRIORITY ORDER of
// those checks is the business logic: exactly ONE gesture owns the lift,
// and the order decides which. This module owns that order as a pure
// decision; the component resolves each drag-state → pointerId match to a
// boolean and performs the side effects the returned intent names
// (release capture, reset cursor, select-under-tap, commit box).

export interface PointerUpEnv {
  /// The active pinch's two pointers include this one — the lift ends
  /// the two-finger gesture.
  pinchMatches: boolean;
  /// A parked (not-yet-promoted) stock-handle press on this pointer —
  /// the lift is a TAP, not a resize (ivac-0rbu).
  pendingStockMatches: boolean;
  stockDragMatches: boolean;
  approachDragMatches: boolean;
  rasterDragMatches: boolean;
  textDragMatches: boolean;
  panMatches: boolean;
  /// A box-select that crossed the drag threshold (armed === false) is
  /// waiting to be committed on lift. A still-armed box never became a
  /// drag, so it just clears.
  boxSelectCommittable: boolean;
}

export type PointerUpIntent =
  /// End the two-finger gesture; also drop any armed box-select so the
  /// remaining finger's lift is a no-op.
  | { kind: 'end-pinch' }
  /// A stationary stock-handle press released without dragging — fall
  /// through to object selection at the press point when geometry sits
  /// under it (so a small object under the handle stays tap-reachable).
  | { kind: 'stock-tap' }
  | { kind: 'end-stock-drag' }
  /// End the approach-marker drag; the component also clears the preview.
  | { kind: 'end-approach-drag' }
  | { kind: 'end-raster-drag' }
  /// End the text-origin drag; the component also nudges the 3D preview.
  | { kind: 'end-text-drag' }
  | { kind: 'end-pan' }
  /// Commit the rubber-band: select the objects contained in the box.
  | { kind: 'commit-box' }
  /// Nothing live ended (or an armed box that never dragged) — just drop
  /// any box-select state and reset.
  | { kind: 'clear-box' };

export function reducePointerUp(env: PointerUpEnv): PointerUpIntent {
  if (env.pinchMatches) return { kind: 'end-pinch' };
  if (env.pendingStockMatches) return { kind: 'stock-tap' };
  if (env.stockDragMatches) return { kind: 'end-stock-drag' };
  if (env.approachDragMatches) return { kind: 'end-approach-drag' };
  if (env.rasterDragMatches) return { kind: 'end-raster-drag' };
  if (env.textDragMatches) return { kind: 'end-text-drag' };
  if (env.panMatches) return { kind: 'end-pan' };
  if (env.boxSelectCommittable) return { kind: 'commit-box' };
  return { kind: 'clear-box' };
}
