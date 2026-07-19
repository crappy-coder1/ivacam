import { describe, it, expect, vi } from 'vitest';
import { reducePointerMove, hoverCursor, type PointerMoveEnv } from './pointer-move';

/// All predicates false + a hover-marker miss = the plain-hover tail.
function baseEnv(over: Partial<PointerMoveEnv> = {}): PointerMoveEnv {
  return {
    pinchActive: false,
    promoteStock: false,
    stockDragMatches: false,
    longPressWandered: false,
    approachPickActive: false,
    approachDragMatches: false,
    rasterDragMatches: false,
    textDragMatches: false,
    hoverMarkerHit: () => false,
    panActive: false,
    boxDragEngaged: false,
    ...over,
  };
}

describe('reducePointerMove — priority order', () => {
  it('nothing active falls through to hover', () => {
    expect(reducePointerMove(baseEnv()).mode).toBe('hover');
  });

  // The chain, top to bottom. Each row sets its own predicate PLUS every
  // lower-priority predicate, and must still win — that pins the order.
  const order: Array<[keyof PointerMoveEnv, string]> = [
    ['pinchActive', 'pinch'],
    ['stockDragMatches', 'stock-drag'],
    ['approachPickActive', 'approach-pick'],
    ['approachDragMatches', 'approach-drag'],
    ['rasterDragMatches', 'raster-drag'],
    ['textDragMatches', 'text-drag'],
    ['panActive', 'pan'],
    ['boxDragEngaged', 'box-drag'],
  ];
  // hoverMarkerHit (a lazy callback, not a boolean flag) sits between
  // text-drag and pan; its slot is pinned by the dedicated test below.
  order.forEach(([flag, mode], i) => {
    it(`${String(flag)} outranks everything below it -> ${mode}`, () => {
      const over: Partial<PointerMoveEnv> = {};
      for (let j = i; j < order.length; j++) over[order[j][0]] = true as never;
      expect(reducePointerMove(baseEnv(over)).mode).toBe(mode);
    });
  });

  it('promoteStock enters stock-drag (and flags the promotion) even with no live stockDrag', () => {
    const intent = reducePointerMove(baseEnv({ promoteStock: true }));
    expect(intent.mode).toBe('stock-drag');
    expect(intent.promoteStock).toBe(true);
  });

  it('a live stock drag enters stock-drag WITHOUT the promotion flag', () => {
    const intent = reducePointerMove(baseEnv({ stockDragMatches: true }));
    expect(intent.mode).toBe('stock-drag');
    expect(intent.promoteStock).toBe(false);
  });

  it('hover-marker sits between the drags and pan/box', () => {
    // Loses to text-drag above it...
    expect(
      reducePointerMove(baseEnv({ textDragMatches: true, hoverMarkerHit: () => true })).mode,
    ).toBe('text-drag');
    // ...wins over pan/box below it.
    expect(
      reducePointerMove(
        baseEnv({ hoverMarkerHit: () => true, panActive: true, boxDragEngaged: true }),
      ).mode,
    ).toBe('hover-marker');
  });
});

describe('reducePointerMove — lazy hover-marker hit-test', () => {
  it('is NOT evaluated when a higher-priority mode wins (cheap early branch)', () => {
    const spy = vi.fn(() => true);
    reducePointerMove(baseEnv({ stockDragMatches: true, hoverMarkerHit: spy }));
    expect(spy).not.toHaveBeenCalled();
  });

  it('IS evaluated once the drags are ruled out', () => {
    const spy = vi.fn(() => false);
    reducePointerMove(baseEnv({ hoverMarkerHit: spy }));
    expect(spy).toHaveBeenCalledTimes(1);
  });
});

describe('reducePointerMove — interleaved cleanups', () => {
  it('pinch and stock-drag short-circuit ABOVE both cleanups', () => {
    for (const over of [
      { pinchActive: true },
      { stockDragMatches: true },
      { promoteStock: true },
    ]) {
      const intent = reducePointerMove(
        baseEnv({ ...over, longPressWandered: true, approachPickActive: true }),
      );
      expect(intent.cancelLongPress).toBe(false);
      expect(intent.clearApproachPreview).toBe(false);
    }
  });

  it('a wandered long-press is cancelled for every mode below stock', () => {
    for (const mode of ['approach-pick', 'approach-drag', 'pan', 'hover'] as const) {
      const over: Partial<PointerMoveEnv> = { longPressWandered: true };
      if (mode === 'approach-pick') over.approachPickActive = true;
      if (mode === 'approach-drag') over.approachDragMatches = true;
      if (mode === 'pan') over.panActive = true;
      expect(reducePointerMove(baseEnv(over)).cancelLongPress).toBe(true);
    }
  });

  it('approach-pick suppresses the approach-preview clear; every other lower mode clears it', () => {
    expect(reducePointerMove(baseEnv({ approachPickActive: true })).clearApproachPreview).toBe(
      false,
    );
    for (const over of [
      { approachDragMatches: true },
      { rasterDragMatches: true },
      { textDragMatches: true },
      { hoverMarkerHit: () => true },
      { panActive: true },
      { boxDragEngaged: true },
      {}, // plain hover
    ]) {
      expect(reducePointerMove(baseEnv(over)).clearApproachPreview).toBe(true);
    }
  });
});

describe('hoverCursor', () => {
  it('text hover always wins with a grab affordance', () => {
    expect(hoverCursor(true, true, true)).toBe('grab');
    expect(hoverCursor(true, false, false)).toBe('grab');
  });

  it('empty space is the base cursor (crosshair while placing tabs, else default)', () => {
    expect(hoverCursor(false, false, false)).toBe('default');
    expect(hoverCursor(false, false, true)).toBe('crosshair');
  });

  it('geometry under the cursor is a cell target in tab mode, else pointer', () => {
    expect(hoverCursor(false, true, false)).toBe('pointer');
    expect(hoverCursor(false, true, true)).toBe('cell');
  });
});
