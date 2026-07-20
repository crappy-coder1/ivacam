// Pure open-decision for EntityCanvas2D's right-click controller. The
// fragile part of "what does a right-click open" is a 3-way priority chain:
// a tab under the cursor opens the per-tab popover; otherwise a truly-empty
// right-click (no selection) is suppressed unless a once-per-session hint is
// still owed; otherwise the op-picker menu opens, anchored with the cursor's
// data-space position so "set text origin here" can plant its target. That
// ordering + the lazy, side-effecting hint consumption is the business logic,
// so it lives here as a pure decision the EntityContextMenu child applies to
// its own state. The caller resolves the environment (it owns the canvas
// transform + hit-tests); this module never touches `project` or the DOM.

export interface CtxMenuState {
  x: number;
  y: number;
  dataX: number;
  dataY: number;
}

export interface TabPopoverState {
  x: number;
  y: number;
  opId: number;
  placementIdx: number;
}

export interface CtxOpenEnv {
  /// The tab placement under the cursor, resolved by the caller (it owns the
  /// transform + polylines). Non-null takes precedence over the op menu.
  tabHit: { opId: number; placementIdx: number } | null;
  hasTextSelected: boolean;
  hasObjsSelected: boolean;
  /// Consume the once-per-session "select something to add an op" hint.
  /// LAZY + side-effecting: only invoked for a truly-empty right-click (no
  /// tab, no selection), so a right-click over a tab or with a live
  /// selection never burns the hint. Returns whether the hint was still owed.
  consumeSelectHint: () => boolean;
  /// Canvas transform for the pixel → data-space projection; null before the
  /// first draw (the menu then anchors at data 0,0, matching the old code).
  transform: { scale: number; offX: number; offY: number } | null;
}

/// The outcome of a right-click: open the tab popover, open the op menu, or
/// open nothing (empty right-click with the hint already spent). The caller
/// applies this to its `ctxMenu` / `tabPopover` state — each result fully
/// determines both (the unnamed one is cleared).
export type CtxOpenResult =
  | { kind: 'tab'; tabPopover: TabPopoverState }
  | { kind: 'menu'; ctxMenu: CtxMenuState }
  | { kind: 'none' };

export function reduceContextMenuOpen(cx: number, cy: number, env: CtxOpenEnv): CtxOpenResult {
  // A tab under the cursor opens its popover before anything else — a
  // right-click lands on "that tab right there" regardless of selection.
  if (env.tabHit) {
    return {
      kind: 'tab',
      tabPopover: { x: cx, y: cy, opId: env.tabHit.opId, placementIdx: env.tabHit.placementIdx },
    };
  }
  // With nothing selected the menu is just a "select something" hint — show
  // it at most once per session (shared with the 3D pane) instead of nagging
  // on every empty right-click. The hint check is lazy via `&&`: it only runs
  // (and only consumes) when there's genuinely no selection.
  if (!env.hasTextSelected && !env.hasObjsSelected && !env.consumeSelectHint()) {
    return { kind: 'none' };
  }
  // Convert canvas pixels → data-space mm so menu actions (like "set text
  // origin here") can plant their target at the cursor without redoing the
  // projection.
  const t = env.transform;
  const dataX = t ? (cx - t.offX) / t.scale : 0;
  const dataY = t ? (t.offY - cy) / t.scale : 0;
  return { kind: 'menu', ctxMenu: { x: cx, y: cy, dataX, dataY } };
}
