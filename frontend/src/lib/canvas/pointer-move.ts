// Pure pointer-move reduction for EntityCanvas2D. Unlike pointerdown /
// pointerup — which are clean 1-of-N dispatches — the pointermove handler
// is a SEQUENTIAL pipeline: a priority chain of "which mode owns this move"
// with two cleanup side effects interleaved BETWEEN the mode returns. The
// order is the fragile business logic (a cleanup that runs a branch too
// early or late silently breaks long-press or the approach preview), so
// this module owns it as a pure decision. The component supplies the
// already-resolved predicates (and a lazy hover-marker hit-test, so a cheap
// early branch never pays for the pxToData + radius probe), then performs
// the verbatim body the returned mode names plus the two flagged cleanups.

export interface PointerMoveEnv {
  /// A two-finger pinch is active — recompute zoom/pan and consume the move
  /// before ANY cleanup runs.
  pinchActive: boolean;
  /// A parked stock-handle press on this pointer that has now wandered past
  /// the tap tolerance — promote it to a live resize/move drag this frame
  /// (ivac-0rbu). Like a live stock drag, it short-circuits above the
  /// cleanups.
  promoteStock: boolean;
  /// A live stock-gizmo drag owns this pointer.
  stockDragMatches: boolean;
  /// A long-press-for-context-menu timer is armed and the finger has
  /// wandered past the tap tolerance (so it's a drag, not a hold). Cancelled
  /// for every mode below stock — see `cancelLongPress` on the intent.
  longPressWandered: boolean;
  /// Approach-point pick mode — the cursor IS the picker (suppresses the
  /// approach-preview clear so the live preview survives).
  approachPickActive: boolean;
  approachDragMatches: boolean;
  rasterDragMatches: boolean;
  textDragMatches: boolean;
  /// Cursor is inside the placed approach marker's grab circle for a
  /// selected profile / pocket op, with no pan / box drag in flight. Lazy:
  /// only evaluated once the higher-priority drags are ruled out.
  hoverMarkerHit: () => boolean;
  panActive: boolean;
  /// A box-select that is already dragging (armed === false) or has just
  /// crossed the drag threshold this frame; an armed box still under the
  /// threshold falls through to plain hover instead.
  boxDragEngaged: boolean;
}

export type PointerMoveMode =
  | 'pinch'
  | 'stock-drag'
  | 'approach-pick'
  | 'approach-drag'
  | 'raster-drag'
  | 'text-drag'
  | 'hover-marker'
  | 'pan'
  | 'box-drag'
  | 'hover';

export interface PointerMoveIntent {
  mode: PointerMoveMode;
  /// Promote the parked stock grab to a live drag BEFORE running the
  /// stock-drag body. Only ever set together with mode === 'stock-drag'.
  promoteStock: boolean;
  /// Cancel the pending long-press context menu. Fires for every mode
  /// EXCEPT the two that short-circuit above the cleanup (pinch, stock-drag).
  cancelLongPress: boolean;
  /// Clear any staged approach-preview marker. Fires past the pinch/stock
  /// short-circuit for every mode except approach-pick (which owns it).
  clearApproachPreview: boolean;
}

export function reducePointerMove(env: PointerMoveEnv): PointerMoveIntent {
  // Pinch consumes the move before any cleanup runs.
  if (env.pinchActive) {
    return {
      mode: 'pinch',
      promoteStock: false,
      cancelLongPress: false,
      clearApproachPreview: false,
    };
  }
  // A parked stock press that wandered promotes to a drag; a live stock
  // drag continues. Either way stock owns the move and short-circuits above
  // the long-press / approach-preview cleanups.
  if (env.promoteStock || env.stockDragMatches) {
    return {
      mode: 'stock-drag',
      promoteStock: env.promoteStock,
      cancelLongPress: false,
      clearApproachPreview: false,
    };
  }
  // From here every branch runs the two interleaved cleanups first: a
  // wandered long-press is cancelled, and the approach-preview is cleared
  // unless we're actively picking.
  const cancelLongPress = env.longPressWandered;
  if (env.approachPickActive) {
    return {
      mode: 'approach-pick',
      promoteStock: false,
      cancelLongPress,
      clearApproachPreview: false,
    };
  }
  const rest = (mode: PointerMoveMode): PointerMoveIntent => ({
    mode,
    promoteStock: false,
    cancelLongPress,
    clearApproachPreview: true,
  });
  if (env.approachDragMatches) return rest('approach-drag');
  if (env.rasterDragMatches) return rest('raster-drag');
  if (env.textDragMatches) return rest('text-drag');
  if (env.hoverMarkerHit()) return rest('hover-marker');
  if (env.panActive) return rest('pan');
  if (env.boxDragEngaged) return rest('box-drag');
  return rest('hover');
}

/// The hover cursor when no drag / pick mode owns the pointer. Text strokes
/// take precedence (a `grab` affordance advertises the drag); empty space is
/// the base cursor (`crosshair` while placing tabs, else `default`); geometry
/// under the cursor is a `cell` target in tab-placement mode, else `pointer`.
export function hoverCursor(
  hasTextHover: boolean,
  hasGeometryHit: boolean,
  tabPlacementActive: boolean,
): 'grab' | 'crosshair' | 'default' | 'cell' | 'pointer' {
  if (hasTextHover) return 'grab';
  if (!hasGeometryHit) return tabPlacementActive ? 'crosshair' : 'default';
  return tabPlacementActive ? 'cell' : 'pointer';
}
