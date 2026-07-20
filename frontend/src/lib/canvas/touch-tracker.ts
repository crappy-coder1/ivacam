/// Mutable bookkeeping for multi-touch canvas gestures in EntityCanvas2D
/// (ivac-1hxn slice): which touch pointers are live and where, the active
/// two-finger pinch frame, and the pending long-press hold timer. Before
/// this these lived as loose `activePointers` / `pinch` / `longPress*`
/// fields plus a `cancelLongPress()` helper in the component; pulling them
/// into an owning class mirrors the ViewController precedent
/// (view-controller.svelte.ts) and keeps EntityCanvas2D from regrowing.
///
/// These are PLAIN (non-reactive) fields on purpose — nothing renders them
/// directly. They drive the reactive ViewController via pinch and open the
/// context menu via long-press, so this is a vanilla class, unit-tested
/// without the rune runtime. The fragile pinch MATH stays pure in
/// `touch-gestures.ts` (`applyPinch`); this only tracks state.

import type { PointerPos } from './touch-gestures';

/// The two fingers of an active pinch: their ids plus the previous frame's
/// positions the next move diffs against.
export interface PinchFrame {
  idA: number;
  idB: number;
  prevA: PointerPos;
  prevB: PointerPos;
}

export class TouchTracker {
  /// Every live touch pointerId → its last canvas-relative position, in
  /// insertion order, so the first two entries are the pinch pair.
  #pointers = new Map<number, PointerPos>();

  /// Active two-finger pinch frame; `null` when fewer than two fingers are
  /// down or the gesture has ended. Public + mutable so the pinch-advance
  /// step can re-anchor `prevA`/`prevB` in place each frame.
  pinch: PinchFrame | null = null;

  /// The armed hold→context-menu timer, or `null` when unarmed.
  #longPressTimer: ReturnType<typeof setTimeout> | null = null;

  /// Anchor of the pending long-press hold (for the tap-tolerance check);
  /// `null` when unarmed.
  longPressStart: PointerPos | null = null;

  // --- live pointer set ---

  /// Record (or update) a live touch pointer's position.
  track(id: number, pos: PointerPos): void {
    this.#pointers.set(id, pos);
  }

  /// Whether a pointer id is currently being tracked.
  has(id: number): boolean {
    return this.#pointers.has(id);
  }

  /// The last recorded position of a tracked pointer, or `undefined`.
  position(id: number): PointerPos | undefined {
    return this.#pointers.get(id);
  }

  /// Drop a lifted touch pointer.
  untrack(id: number): void {
    this.#pointers.delete(id);
  }

  /// Number of live touch pointers.
  get size(): number {
    return this.#pointers.size;
  }

  // --- pinch ---

  /// Promote the first two live pointers into a pinch frame. Returns
  /// `false` (leaving `pinch` untouched) when fewer than two are down.
  beginPinch(): boolean {
    const entries = [...this.#pointers.entries()];
    const first = entries[0];
    const second = entries[1];
    if (!first || !second) return false;
    this.pinch = {
      idA: first[0],
      idB: second[0],
      prevA: { ...first[1] },
      prevB: { ...second[1] },
    };
    return true;
  }

  /// The two pointer ids in the active pinch, for pointer-capture; `null`
  /// when no pinch is in flight.
  get pinchIds(): [number, number] | null {
    return this.pinch ? [this.pinch.idA, this.pinch.idB] : null;
  }

  // --- long press ---

  /// Cancel any pending long-press hold (finger moved, lifted, or a second
  /// finger arrived). Idempotent.
  cancelLongPress(): void {
    if (this.#longPressTimer != null) {
      clearTimeout(this.#longPressTimer);
      this.#longPressTimer = null;
    }
    this.longPressStart = null;
  }

  /// Arm the hold→menu timer anchored at `start`. When it fires it first
  /// clears its own handle + anchor (so `onFire` sees the disarmed state,
  /// mirroring the original inline timer), then invokes `onFire`.
  armLongPress(start: PointerPos, onFire: () => void, delayMs: number): void {
    this.longPressStart = start;
    this.#longPressTimer = setTimeout(() => {
      this.#longPressTimer = null;
      this.longPressStart = null;
      onFire();
    }, delayMs);
  }
}
