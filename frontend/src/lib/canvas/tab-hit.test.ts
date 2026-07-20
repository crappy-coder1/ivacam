import { describe, it, expect } from 'vitest';
import { findTabAtPixel, patchTabPlacement, removeTabPlacement, type TabHitOp } from './tab-hit';
import type { ObjectPolyline } from '../cam/tabs';

// A unit square (0..10 mm) as a closed polyline. polylineAtT walks it by
// normalized arc-length t, so t=0 is the first vertex (0,0).
const SQUARE: ObjectPolyline = {
  objectId: 1,
  pts: [
    { x: 0, y: 0 },
    { x: 10, y: 0 },
    { x: 10, y: 10 },
    { x: 0, y: 10 },
  ],
  closed: true,
};

// Identity-ish transform: scale 1, no offset -> screen == data (y flipped).
const T = { scale: 1, offX: 0, offY: 0 };

describe('findTabAtPixel', () => {
  it('returns null when no op has a placement near the cursor', () => {
    const ops: TabHitOp[] = [{ opId: 7, placements: [{ objectId: 1, t: 0 }] }];
    // Placement at t=0 -> data (0,0) -> screen (0, 0). Probe far away.
    expect(findTabAtPixel(500, 500, T, [SQUARE], ops)).toBeNull();
  });

  it('hits a placement within the 10-px tolerance', () => {
    const ops: TabHitOp[] = [{ opId: 7, placements: [{ objectId: 1, t: 0 }] }];
    // t=0 -> data (0,0) -> screen sx=0, sy=0. Probe 6px away (inside tol).
    expect(findTabAtPixel(4, 4, T, [SQUARE], ops)).toEqual({ opId: 7, placementIdx: 0 });
  });

  it('just outside the tolerance (>10px) misses', () => {
    const ops: TabHitOp[] = [{ opId: 7, placements: [{ objectId: 1, t: 0 }] }];
    expect(findTabAtPixel(8, 8, T, [SQUARE], ops)).toBeNull(); // dist ~11.3 > 10
  });

  it('picks the NEAREST placement when two are in range', () => {
    // t=0 -> (0,0); a second op with a placement at t=0.25 -> (10,0) i.e.
    // screen (10,0). Probe near (0,0): op 7 wins.
    const ops: TabHitOp[] = [
      { opId: 7, placements: [{ objectId: 1, t: 0 }] },
      { opId: 9, placements: [{ objectId: 1, t: 0.25 }] },
    ];
    expect(findTabAtPixel(2, 1, T, [SQUARE], ops)).toEqual({ opId: 7, placementIdx: 0 });
    // Probe near (10,0): op 9 wins, and reports ITS placement index.
    expect(findTabAtPixel(9, 1, T, [SQUARE], ops)).toEqual({ opId: 9, placementIdx: 0 });
  });

  it('reports the correct placementIdx within a multi-placement op', () => {
    const ops: TabHitOp[] = [
      {
        opId: 7,
        placements: [
          { objectId: 1, t: 0 },
          { objectId: 1, t: 0.25 },
        ],
      },
    ];
    expect(findTabAtPixel(10, 1, T, [SQUARE], ops)).toEqual({ opId: 7, placementIdx: 1 });
  });

  it('skips placements whose object is missing from the polylines', () => {
    const ops: TabHitOp[] = [{ opId: 7, placements: [{ objectId: 999, t: 0 }] }];
    expect(findTabAtPixel(0, 0, T, [SQUARE], ops)).toBeNull();
  });

  it('honors scale + offset (screen = data*scale + off, y flipped)', () => {
    const t2 = { scale: 2, offX: 100, offY: 50 };
    // t=0 -> data (0,0) -> screen (0*2+100, 50-0*2) = (100, 50).
    expect(
      findTabAtPixel(100, 50, t2, [SQUARE], [{ opId: 3, placements: [{ objectId: 1, t: 0 }] }]),
    ).toEqual({ opId: 3, placementIdx: 0 });
  });
});

describe('patchTabPlacement', () => {
  const base = [
    { objectId: 1, t: 0, widthOverrideMm: undefined as number | undefined },
    { objectId: 1, t: 0.5, widthOverrideMm: undefined as number | undefined },
  ];

  it('merges the patch into the target index only, leaving others intact', () => {
    const next = patchTabPlacement(base, 1, { widthOverrideMm: 4 });
    expect(next).not.toBeNull();
    expect(next![1].widthOverrideMm).toBe(4);
    expect(next![0]).toEqual(base[0]);
  });

  it('does not mutate the input array', () => {
    const snapshot = JSON.parse(JSON.stringify(base));
    patchTabPlacement(base, 0, { widthOverrideMm: 9 });
    expect(base).toEqual(snapshot);
  });

  it('returns null for an out-of-bounds index (no-op)', () => {
    expect(patchTabPlacement(base, -1, { widthOverrideMm: 1 })).toBeNull();
    expect(patchTabPlacement(base, 2, { widthOverrideMm: 1 })).toBeNull();
  });
});

describe('removeTabPlacement', () => {
  const base = [
    { objectId: 1, t: 0 },
    { objectId: 1, t: 0.5 },
    { objectId: 1, t: 0.9 },
  ];

  it('removes exactly the target index', () => {
    expect(removeTabPlacement(base, 1)).toEqual([base[0], base[2]]);
  });

  it('does not mutate the input array', () => {
    const snapshot = JSON.parse(JSON.stringify(base));
    removeTabPlacement(base, 0);
    expect(base).toEqual(snapshot);
  });

  it('returns null for an out-of-bounds index (no-op)', () => {
    expect(removeTabPlacement(base, -1)).toBeNull();
    expect(removeTabPlacement(base, 3)).toBeNull();
  });
});
