import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { TouchTracker } from './touch-tracker';

describe('TouchTracker pointer set', () => {
  it('tracks, reads back, and drops pointers', () => {
    const t = new TouchTracker();
    expect(t.size).toBe(0);
    expect(t.has(1)).toBe(false);

    t.track(1, { x: 10, y: 20 });
    expect(t.has(1)).toBe(true);
    expect(t.size).toBe(1);
    expect(t.position(1)).toEqual({ x: 10, y: 20 });

    // re-track updates in place, not appends
    t.track(1, { x: 11, y: 22 });
    expect(t.size).toBe(1);
    expect(t.position(1)).toEqual({ x: 11, y: 22 });

    t.untrack(1);
    expect(t.has(1)).toBe(false);
    expect(t.size).toBe(0);
    expect(t.position(1)).toBeUndefined();
  });
});

describe('TouchTracker pinch', () => {
  it('beginPinch snapshots the first two live pointers by insertion order', () => {
    const t = new TouchTracker();
    t.track(7, { x: 1, y: 1 });
    t.track(9, { x: 3, y: 5 });
    t.track(11, { x: 9, y: 9 });

    expect(t.beginPinch()).toBe(true);
    expect(t.pinch).toEqual({
      idA: 7,
      idB: 9,
      prevA: { x: 1, y: 1 },
      prevB: { x: 3, y: 5 },
    });
    expect(t.pinchIds).toEqual([7, 9]);
  });

  it('beginPinch copies positions so later moves do not mutate the frame', () => {
    const t = new TouchTracker();
    t.track(1, { x: 1, y: 1 });
    t.track(2, { x: 2, y: 2 });
    t.beginPinch();
    t.track(1, { x: 100, y: 100 }); // finger moves after the frame was taken
    expect(t.pinch!.prevA).toEqual({ x: 1, y: 1 });
  });

  it('beginPinch is a no-op with fewer than two pointers', () => {
    const t = new TouchTracker();
    t.track(1, { x: 0, y: 0 });
    expect(t.beginPinch()).toBe(false);
    expect(t.pinch).toBeNull();
    expect(t.pinchIds).toBeNull();
  });
});

describe('TouchTracker long-press', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('fires onFire after the delay with the anchor cleared first', () => {
    const t = new TouchTracker();
    let sawStartAtFire: unknown = 'unset';
    t.armLongPress(
      { x: 4, y: 5 },
      () => {
        sawStartAtFire = t.longPressStart;
      },
      500,
    );

    expect(t.longPressStart).toEqual({ x: 4, y: 5 });
    vi.advanceTimersByTime(499);
    expect(sawStartAtFire).toBe('unset'); // not yet
    vi.advanceTimersByTime(1);
    // onFire ran and observed the anchor already cleared
    expect(sawStartAtFire).toBeNull();
    expect(t.longPressStart).toBeNull();
  });

  it('cancelLongPress prevents onFire and clears the anchor', () => {
    const t = new TouchTracker();
    const onFire = vi.fn();
    t.armLongPress({ x: 0, y: 0 }, onFire, 500);
    t.cancelLongPress();
    expect(t.longPressStart).toBeNull();
    vi.advanceTimersByTime(1000);
    expect(onFire).not.toHaveBeenCalled();
  });

  it('cancelLongPress is idempotent when unarmed', () => {
    const t = new TouchTracker();
    expect(() => t.cancelLongPress()).not.toThrow();
    expect(t.longPressStart).toBeNull();
  });
});
