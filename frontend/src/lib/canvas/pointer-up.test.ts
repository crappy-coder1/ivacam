import { describe, it, expect } from 'vitest';
import { reducePointerUp, type PointerUpEnv } from './pointer-up';

function env(over: Partial<PointerUpEnv>): PointerUpEnv {
  return {
    pinchMatches: false,
    pendingStockMatches: false,
    stockDragMatches: false,
    approachDragMatches: false,
    rasterDragMatches: false,
    textDragMatches: false,
    panMatches: false,
    boxSelectCommittable: false,
    ...over,
  };
}

describe('reducePointerUp', () => {
  it('nothing live ending → clear-box (default reset)', () => {
    expect(reducePointerUp(env({})).kind).toBe('clear-box');
  });

  it('a committable box-select commits on lift', () => {
    expect(reducePointerUp(env({ boxSelectCommittable: true })).kind).toBe('commit-box');
  });

  it('pinch lift wins over every other ending gesture', () => {
    const out = reducePointerUp(
      env({
        pinchMatches: true,
        pendingStockMatches: true,
        stockDragMatches: true,
        panMatches: true,
        boxSelectCommittable: true,
      }),
    );
    expect(out.kind).toBe('end-pinch');
  });

  it('a parked stock press releases as a tap (before drag endings)', () => {
    expect(reducePointerUp(env({ pendingStockMatches: true, panMatches: true })).kind).toBe(
      'stock-tap',
    );
  });

  it('each drag ends under its own intent, in priority order', () => {
    expect(reducePointerUp(env({ stockDragMatches: true })).kind).toBe('end-stock-drag');
    expect(reducePointerUp(env({ approachDragMatches: true })).kind).toBe('end-approach-drag');
    expect(reducePointerUp(env({ rasterDragMatches: true })).kind).toBe('end-raster-drag');
    expect(reducePointerUp(env({ textDragMatches: true })).kind).toBe('end-text-drag');
    expect(reducePointerUp(env({ panMatches: true })).kind).toBe('end-pan');
  });

  it('stock drag outranks the lower drags and the box commit', () => {
    const out = reducePointerUp(
      env({
        stockDragMatches: true,
        approachDragMatches: true,
        rasterDragMatches: true,
        textDragMatches: true,
        panMatches: true,
        boxSelectCommittable: true,
      }),
    );
    expect(out.kind).toBe('end-stock-drag');
  });

  it('an active drag suppresses the box commit tail', () => {
    // A drag matching this pointer means the lift is not a box commit,
    // even if a stale box-select is somehow still committable.
    expect(reducePointerUp(env({ panMatches: true, boxSelectCommittable: true })).kind).toBe(
      'end-pan',
    );
  });
});
