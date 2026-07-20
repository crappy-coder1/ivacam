/// Reactive view (pan / zoom) controller for EntityCanvas2D — the owning
/// module for the canvas's user-view interaction STATE and its small
/// transitions (ivac-3xwn.3). Before this, `userZoom` / `userPanX` /
/// `userPanY` / `panDrag` lived as loose `$state` in the component and
/// every new view gesture piled another transition onto the template;
/// this gives them a home OUTSIDE the .svelte file, mirroring the
/// scene3d/* builder classes that kept Scene3D from regrowing.
///
/// The fragile MATH is pure + unit-tested in `viewport.ts`
/// (`zoomAroundCursor`) and `touch-gestures.ts` (`applyPinch`); this class
/// is the thin rune-bound wrapper that applies their results to reactive
/// state — the same split as `workspace.ts` (tested) + `workspace.svelte.ts`
/// (rune wrapper), so it needs no separate test.
///
/// It deliberately does NOT own the per-frame transform CACHE
/// (`lastTransform` / `lastBaseTransform`): those are render OUTPUTS the
/// draw pass writes and the hit-testers read imperatively, not user-view
/// intent. The component passes the live base transform into `wheelZoom` /
/// `applyPinchFrame` (mirroring how the pure helpers already take `base`).

import { applyPinch, type BaseView, type PointerPos } from './touch-gestures';
import { zoomAroundCursor, type UserView } from './viewport';

/// Active middle-button pan drag: the previous frame's screen anchor plus
/// the captured pointer id. `null` when no pan is in flight.
export interface PanDrag {
  startX: number;
  startY: number;
  pointerId: number;
}

export class ViewController {
  /// User zoom multiplied on top of the auto-fit scale (1 = fit).
  zoom = $state(1);
  /// User pan offsets in canvas pixels, added after fit-and-zoom.
  panX = $state(0);
  panY = $state(0);
  /// Active middle-button pan drag; null when idle.
  panDrag = $state<PanDrag | null>(null);

  /// Last imported-file key the view was reset for — so a new import
  /// snaps back to auto-fit while normal redraws keep the mid-session view.
  #lastImportKey: string | null = null;

  /// Snapshot of the user view for the pure viewport math.
  get userView(): UserView {
    return { zoom: this.zoom, panX: this.panX, panY: this.panY };
  }

  /// True while a middle-button pan drag is in flight.
  get panning(): boolean {
    return this.panDrag != null;
  }

  /// Pull the view back to its auto-fit baseline (no user pan/zoom).
  /// Invoked from the fit-view button, double-click empty space, and the
  /// `F` / `Home` shortcuts.
  reset() {
    this.zoom = 1;
    this.panX = 0;
    this.panY = 0;
  }

  /// Reset the view when the imported file changes (different filename, or
  /// going from no-import to imported); a no-op on unchanged redraws so a
  /// mid-session zoom survives. Pass `project.transformedImport?.filename`.
  resetOnImportChange(key: string | null) {
    if (key !== this.#lastImportKey) {
      this.#lastImportKey = key;
      this.reset();
    }
  }

  /// Cursor-pivot wheel zoom. `base` is the live auto-fit transform (the
  /// component's `lastBaseTransform`); `(cursorX, cursorY)` is the
  /// canvas-relative pointer. Keeps the data point under the cursor fixed.
  wheelZoom(base: BaseView, cursorX: number, cursorY: number, deltaY: number) {
    this.#apply(zoomAroundCursor(base, this.userView, cursorX, cursorY, deltaY));
  }

  /// Apply one two-finger pinch frame: `prev`/`curr` are the two touch
  /// positions last frame vs. this frame. `base` is the live auto-fit
  /// transform. Zooms + pans around the pinch centroid.
  applyPinchFrame(
    base: BaseView,
    prev: { a: PointerPos; b: PointerPos },
    curr: { a: PointerPos; b: PointerPos },
  ) {
    this.#apply(applyPinch(this.userView, base, prev, curr));
  }

  /// Begin a pan drag captured on middle-button down.
  startPan(clientX: number, clientY: number, pointerId: number) {
    this.panDrag = { startX: clientX, startY: clientY, pointerId };
  }

  /// Advance an in-flight pan by the cursor delta since the last frame,
  /// then re-anchor. No-op when no pan is active.
  movePan(clientX: number, clientY: number) {
    const drag = this.panDrag;
    if (!drag) return;
    this.panX += clientX - drag.startX;
    this.panY += clientY - drag.startY;
    this.panDrag = { ...drag, startX: clientX, startY: clientY };
  }

  /// End any in-flight pan drag.
  endPan() {
    this.panDrag = null;
  }

  #apply(next: UserView) {
    this.zoom = next.zoom;
    this.panX = next.panX;
    this.panY = next.panY;
  }
}
