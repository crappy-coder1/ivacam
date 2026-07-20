import { describe, it, expect } from 'vitest';
import {
  computeViewportTransform,
  placementsBBox,
  zoomAroundCursor,
  ZOOM_MIN,
  ZOOM_MAX,
  ZOOM_STEP,
  type UserView,
} from './viewport';
import type { BBox } from '../api/types';

const SQUARE_BBOX: BBox = { min_x: 0, min_y: 0, max_x: 100, max_y: 100 };

describe('computeViewportTransform', () => {
  it('user view {zoom:1, pan:0,0} ⇒ active transform equals base transform', () => {
    const t = computeViewportTransform(
      SQUARE_BBOX,
      { w: 600, h: 400 },
      { zoom: 1, panX: 0, panY: 0 },
    );
    expect(t.scale).toBeCloseTo(t.baseScale);
    expect(t.offX).toBeCloseTo(t.baseOffX);
    expect(t.offY).toBeCloseTo(t.baseOffY);
  });

  it('zoom multiplies the base scale; pan adds to the offsets', () => {
    const base = computeViewportTransform(
      SQUARE_BBOX,
      { w: 600, h: 400 },
      { zoom: 1, panX: 0, panY: 0 },
    );
    const zoomed = computeViewportTransform(
      SQUARE_BBOX,
      { w: 600, h: 400 },
      { zoom: 2, panX: 10, panY: -5 },
    );
    expect(zoomed.scale).toBeCloseTo(base.baseScale * 2);
    expect(zoomed.offX).toBeCloseTo(base.baseOffX + 10);
    expect(zoomed.offY).toBeCloseTo(base.baseOffY - 5);
  });

  it('fit-to-view leaves the configured margin on the limiting axis', () => {
    // 100×100 data in a 200×800 canvas; X is the limiting axis. With
    // margin=32 the available width is 136 px; baseScale = 136/100 = 1.36.
    const t = computeViewportTransform(
      SQUARE_BBOX,
      { w: 200, h: 800 },
      { zoom: 1, panX: 0, panY: 0 },
      32,
    );
    expect(t.baseScale).toBeCloseTo(136 / 100);
  });

  it('project2 flips Y (DXF y-up, canvas y-down)', () => {
    const t = computeViewportTransform(
      SQUARE_BBOX,
      { w: 200, h: 200 },
      { zoom: 1, panX: 0, panY: 0 },
    );
    const [px0, py0] = t.project2(0, 0);
    const [px100, py100] = t.project2(0, 100);
    // Larger data-Y maps to a SMALLER canvas-Y (top of screen).
    expect(py100).toBeLessThan(py0);
    // X axis is not flipped: 0 maps less-than 100.
    expect(px0).toBeLessThanOrEqual(px100 + 1);
  });

  it('handles degenerate (zero-extent) bboxes without dividing by zero', () => {
    const t = computeViewportTransform(
      { min_x: 5, min_y: 5, max_x: 5, max_y: 5 },
      { w: 200, h: 200 },
      { zoom: 1, panX: 0, panY: 0 },
    );
    expect(Number.isFinite(t.scale)).toBe(true);
    expect(Number.isFinite(t.offX)).toBe(true);
    expect(Number.isFinite(t.offY)).toBe(true);
  });
});

describe('zoomAroundCursor (ivac-3xwn.3)', () => {
  const BASE = { scale: 2, offX: 100, offY: 300 };

  /// Data-space point under a canvas pixel for a given base + user view —
  /// the invariant the cursor-pivot zoom must preserve.
  function dataUnderCursor(view: UserView, cx: number, cy: number): [number, number] {
    const scale = BASE.scale * view.zoom;
    const offX = BASE.offX + view.panX;
    const offY = BASE.offY + view.panY;
    return [(cx - offX) / scale, (offY - cy) / scale];
  }

  it('scroll up (deltaY < 0) multiplies the zoom by ZOOM_STEP', () => {
    const next = zoomAroundCursor(BASE, { zoom: 1, panX: 0, panY: 0 }, 250, 150, -100);
    expect(next.zoom).toBeCloseTo(ZOOM_STEP);
  });

  it('scroll down (deltaY > 0) divides the zoom by ZOOM_STEP', () => {
    const next = zoomAroundCursor(BASE, { zoom: 1, panX: 0, panY: 0 }, 250, 150, 100);
    expect(next.zoom).toBeCloseTo(1 / ZOOM_STEP);
  });

  it('keeps the data point under the cursor fixed across the zoom', () => {
    const view: UserView = { zoom: 1, panX: 0, panY: 0 };
    const before = dataUnderCursor(view, 250, 150);
    const next = zoomAroundCursor(BASE, view, 250, 150, -100);
    const after = dataUnderCursor(next, 250, 150);
    expect(after[0]).toBeCloseTo(before[0]);
    expect(after[1]).toBeCloseTo(before[1]);
  });

  it('preserves the anchor from an already panned + zoomed view', () => {
    const view: UserView = { zoom: 3, panX: -40, panY: 25 };
    const before = dataUnderCursor(view, 310, 90);
    const next = zoomAroundCursor(BASE, view, 310, 90, 100);
    const after = dataUnderCursor(next, 310, 90);
    expect(after[0]).toBeCloseTo(before[0]);
    expect(after[1]).toBeCloseTo(before[1]);
  });

  it('clamps zoom-in at ZOOM_MAX', () => {
    const next = zoomAroundCursor(BASE, { zoom: ZOOM_MAX, panX: 0, panY: 0 }, 250, 150, -100);
    expect(next.zoom).toBe(ZOOM_MAX);
  });

  it('clamps zoom-out at ZOOM_MIN', () => {
    const next = zoomAroundCursor(BASE, { zoom: ZOOM_MIN, panX: 0, panY: 0 }, 250, 150, 100);
    expect(next.zoom).toBe(ZOOM_MIN);
  });
});

describe('placementsBBox (rt1.12 fvb0)', () => {
  it('returns null for an empty list', () => {
    expect(placementsBBox([])).toBeNull();
  });

  it('unions rects and pads by a fraction of the span', () => {
    const bb = placementsBBox(
      [
        { minX: 0, minY: 0, maxX: 100, maxY: 50 },
        { minX: 120, minY: 10, maxX: 140, maxY: 90 },
      ],
      0.1,
    );
    // union = [0,0]..[140,90]; margin = 10% of 140 / 90 = 14 / 9.
    expect(bb).toEqual({ min_x: -14, min_y: -9, max_x: 154, max_y: 99 });
  });

  it('floors the margin at 1 unit for a zero-span axis', () => {
    // A single 1-wide, 0-tall rect: x span 1 ⇒ margin max(1, 0.1) = 1;
    // y span 0 ⇒ margin max(1, 0) = 1.
    const bb = placementsBBox([{ minX: 5, minY: 5, maxX: 6, maxY: 5 }], 0.1);
    expect(bb).toEqual({ min_x: 4, min_y: 4, max_x: 7, max_y: 6 });
  });
});
