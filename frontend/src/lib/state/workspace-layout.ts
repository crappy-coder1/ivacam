/// Pure resizable-panel geometry lifted out of App.svelte (ivac-3xwn.1).
/// The clamp/default math previously read `window.innerWidth/innerHeight`
/// inline, which made it impossible to unit-test; here the viewport comes in
/// as a parameter so the rules are exercised headlessly (mirrors the
/// FloatingPanel clamp extraction from oytm). The component keeps the `$state`
/// panel sizes + persistence and just calls these for the arithmetic.

/// Default sidebar width in px on a cold start (no persisted value yet).
export const SIDEBAR_DEFAULT = 360;

/// Clamp a sidebar width against the viewport. Hard floor of 240 px (under
/// that the OperationsList grid overlaps); the ceiling tracks the viewport
/// (60 %, capped to [240, 720]) so a too-wide persisted value can't crowd the
/// canvas to zero on a smaller monitor.
export function clampSidebar(v: number, viewportWidth: number): number {
  const ceiling = Math.max(240, Math.min(720, Math.round(viewportWidth * 0.6)));
  return Math.max(240, Math.min(ceiling, v));
}

/// Clamp a G-code panel height to [120 px, 70 % of viewport] so it can't run
/// off a shrunk window or crowd the 3D scene to nothing.
export function clampGcode(v: number, viewportHeight: number): number {
  return Math.max(120, Math.min(Math.round(viewportHeight * 0.7), v));
}

/// Default G-code panel height: ~35 % of the viewport.
export function defaultGcodeHeight(viewportHeight: number): number {
  return Math.round(viewportHeight * 0.35);
}

/// Re-clamp both panels against a new viewport. Returns the clamped sizes and
/// whether either actually changed — the caller persists only when `changed`
/// so an inert resize doesn't churn the workspace store.
export function reclampPanels(
  cur: { sidebarWidth: number; gcodeHeight: number },
  viewport: { width: number; height: number },
): { sidebarWidth: number; gcodeHeight: number; changed: boolean } {
  const sidebarWidth = clampSidebar(cur.sidebarWidth, viewport.width);
  const gcodeHeight = clampGcode(cur.gcodeHeight, viewport.height);
  return {
    sidebarWidth,
    gcodeHeight,
    changed: sidebarWidth !== cur.sidebarWidth || gcodeHeight !== cur.gcodeHeight,
  };
}
