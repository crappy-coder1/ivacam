import { describe, expect, it } from 'vitest';
import {
  computeArrowChevron,
  arrowSpacingMm,
  moveBoost,
  resolveSegmentColor,
  fadeColor,
  tessellateArc,
  lowerBoundSeg,
  type ArrowParams,
  type Rgb,
} from './toolpath_buffers';

const P: ArrowParams = {
  minLen: 1.0,
  maxSize: 4.0,
  sizeFrac: 0.2,
  halfWing: Math.tan((30 * Math.PI) / 180),
};

describe('arrowSpacingMm', () => {
  it('disables arrows at density 0 (Infinity spacing)', () => {
    expect(arrowSpacingMm(0)).toBe(Infinity);
  });
  it('packs arrows closer as density rises', () => {
    expect(arrowSpacingMm(1)).toBeCloseTo(3.0);
    expect(arrowSpacingMm(2)).toBeCloseTo(1.5);
  });
});

describe('computeArrowChevron', () => {
  it('returns null for a segment shorter than minLen', () => {
    expect(computeArrowChevron({ x: 0, y: 0, z: 0 }, { x: 0.5, y: 0, z: 0 }, P)).toBeNull();
  });

  it('builds a chevron pointing along a +X move with ±normal wings', () => {
    const c = computeArrowChevron({ x: 0, y: 0, z: 0 }, { x: 10, y: 0, z: 0 }, P);
    expect(c).not.toBeNull();
    // A = min(10*0.2, 4) = 2; apex at midpoint (5,0,0); wings 2mm back
    // (x=3) and ±A*halfWing in Y.
    const side = 2 * P.halfWing;
    expect(c!.mid).toEqual([5, 0, 0]);
    expect(c!.wing1[0]).toBeCloseTo(3);
    expect(c!.wing1[1]).toBeCloseTo(side);
    expect(c!.wing1[2]).toBeCloseTo(0);
    expect(c!.wing2[0]).toBeCloseTo(3);
    expect(c!.wing2[1]).toBeCloseTo(-side);
    // Wings are symmetric about the move axis.
    expect(c!.wing1[1]).toBeCloseTo(-c!.wing2[1]);
  });

  it('caps arrow size at maxSize on a long move', () => {
    const c = computeArrowChevron({ x: 0, y: 0, z: 0 }, { x: 100, y: 0, z: 0 }, P);
    // A = min(100*0.2=20, 4) = 4 → wings 4mm behind the midpoint (x=46).
    expect(c!.wing1[0]).toBeCloseTo(46);
    expect(c!.wing2[0]).toBeCloseTo(46);
  });

  it('falls back to a +X normal for a pure-Z (plunge) move', () => {
    const c = computeArrowChevron({ x: 0, y: 0, z: 0 }, { x: 0, y: 0, z: 5 }, P);
    expect(c).not.toBeNull();
    // A = min(5*0.2=1, 4) = 1; apex (0,0,2.5); wings 1mm back in Z (z=1.5)
    // and ±halfWing in X (the fallback normal).
    const side = 1 * P.halfWing;
    expect(c!.mid).toEqual([0, 0, 2.5]);
    expect(c!.wing1[0]).toBeCloseTo(side);
    expect(c!.wing1[2]).toBeCloseTo(1.5);
    expect(c!.wing2[0]).toBeCloseTo(-side);
    expect(c!.wing2[2]).toBeCloseTo(1.5);
  });
});

describe('moveBoost', () => {
  it('rapids are dimmest, plunge/retract mid, cuts brightest', () => {
    expect(moveBoost('rapid')).toBe(0.5);
    expect(moveBoost('plunge')).toBe(0.85);
    expect(moveBoost('retract')).toBe(0.85);
    expect(moveBoost('cut')).toBe(1.15);
    expect(moveBoost('arc')).toBe(1.15);
    // Unknown kinds fall into the "brightest" default.
    expect(moveBoost('whatever')).toBe(1.15);
  });
});

describe('resolveSegmentColor', () => {
  const moveTint: Rgb = [0.2, 0.6, 1.0];
  const opColor: Rgb = [0.4, 0.5, 0.6];

  it('op_id 0 uses the move tint verbatim', () => {
    expect(resolveSegmentColor(0, 'cut', moveTint, opColor)).toEqual([0.2, 0.6, 1.0]);
    // Move kind is irrelevant when unstamped.
    expect(resolveSegmentColor(0, 'rapid', moveTint, opColor)).toEqual([0.2, 0.6, 1.0]);
  });

  it('a stamped op scales its hue color by the move boost', () => {
    const [r, g, b] = resolveSegmentColor(3, 'cut', moveTint, opColor);
    expect(r).toBeCloseTo(0.4 * 1.15);
    expect(g).toBeCloseTo(0.5 * 1.15);
    expect(b).toBeCloseTo(0.6 * 1.15);
    // Rapid dims the same op color.
    const rapid = resolveSegmentColor(3, 'rapid', moveTint, opColor);
    expect(rapid[0]).toBeCloseTo(0.4 * 0.5);
  });
});

describe('fadeColor', () => {
  const base: Rgb = [0.8, 0.4, 0.2];
  it('past moves keep the full base color', () => {
    expect(fadeColor(base, true, 0.25, 0.05)).toEqual([0.8, 0.4, 0.2]);
  });
  it('future moves dim to base*factor + offset (visible floor, not black)', () => {
    const [r, g, b] = fadeColor(base, false, 0.25, 0.05);
    expect(r).toBeCloseTo(0.8 * 0.25 + 0.05);
    expect(g).toBeCloseTo(0.4 * 0.25 + 0.05);
    expect(b).toBeCloseTo(0.2 * 0.25 + 0.05);
    // A black base still floors at the offset so it's not invisible.
    expect(fadeColor([0, 0, 0], false, 0.25, 0.05)).toEqual([0.05, 0.05, 0.05]);
  });
});

describe('tessellateArc', () => {
  const step = Math.PI / 90; // 2°

  it('walks a CCW quarter circle: interior points sit on the circle, endpoints exact', () => {
    // Quarter circle about the origin, R=10, from (10,0) to (0,10), CCW (G3).
    const from = { x: 10, y: 0, z: 0 };
    const to = { x: 0, y: 10, z: 0 };
    const pts = tessellateArc(from, to, { cx: 0, cy: 0, ccw: true }, step);
    // 90° / 2° = 45 render chords ⇒ 46 points.
    expect(pts.length).toBe(46);
    // Endpoints copied verbatim (no float drift at the joins).
    expect(pts[0]).toEqual(from);
    expect(pts[pts.length - 1]).toEqual(to);
    // Every interior point lies on the R=10 circle...
    for (const p of pts) {
      expect(Math.hypot(p.x, p.y)).toBeCloseTo(10, 9);
    }
    // ...and the sweep goes CCW through the first quadrant (a midpoint bulges
    // out to ~45°, not straight across the chord).
    const mid = pts[Math.floor(pts.length / 2)];
    expect(mid.x).toBeGreaterThan(0);
    expect(mid.y).toBeGreaterThan(0);
    // Roughly on the 45° diagonal (~(7.07, 7.07)) — the arc bulges out, it
    // doesn't cut straight across the chord.
    expect(mid.x).toBeCloseTo(Math.SQRT1_2 * 10, 0);
    expect(mid.y).toBeCloseTo(Math.SQRT1_2 * 10, 0);
  });

  it('interpolates Z linearly across the sweep (helical arc)', () => {
    const from = { x: 10, y: 0, z: 0 };
    const to = { x: 0, y: 10, z: 4 };
    const pts = tessellateArc(from, to, { cx: 0, cy: 0, ccw: true }, step);
    expect(pts[0].z).toBe(0);
    expect(pts[pts.length - 1].z).toBe(4);
    // Z rises monotonically from 0 to 4.
    for (let i = 1; i < pts.length; i++) expect(pts[i].z).toBeGreaterThanOrEqual(pts[i - 1].z);
  });

  it('coincident endpoints ⇒ a full revolution in the requested direction', () => {
    const p = { x: 10, y: 0, z: 0 };
    const pts = tessellateArc(p, { ...p }, { cx: 0, cy: 0, ccw: true }, step);
    // 360° / 2° = 180 chords ⇒ 181 points, all on the circle.
    expect(pts.length).toBe(181);
    for (const q of pts) expect(Math.hypot(q.x, q.y)).toBeCloseTo(10, 9);
  });

  it('CW (G2) sweeps the other way than CCW for the same endpoints', () => {
    const from = { x: 10, y: 0, z: 0 };
    const to = { x: 0, y: 10, z: 0 };
    const ccw = tessellateArc(from, to, { cx: 0, cy: 0, ccw: true }, step);
    const cw = tessellateArc(from, to, { cx: 0, cy: 0, ccw: false }, step);
    // CCW takes the short way (first quadrant, y>0 midpoint); CW the long way
    // (270° around, a midpoint in the third quadrant with x<0, y<0).
    const cwMid = cw[Math.floor(cw.length / 2)];
    expect(cwMid.x).toBeLessThan(0);
    expect(cwMid.y).toBeLessThan(0);
    // The long way around is many more chords than the short way.
    expect(cw.length).toBeGreaterThan(ccw.length);
  });

  it('degenerate (start on center) falls back to a straight chord', () => {
    const from = { x: 0, y: 0, z: 0 };
    const to = { x: 5, y: 5, z: 0 };
    expect(tessellateArc(from, to, { cx: 0, cy: 0, ccw: true }, step)).toEqual([from, to]);
  });
});

describe('lowerBoundSeg', () => {
  // Render-line → backend-seg map for two arcs (seg 0 spans 3 lines, seg 1
  // spans 2) surrounding straight moves — exactly the on-read tessellation
  // shape the playhead fade walks.
  const segs = [0, 0, 0, 1, 1, 2, 3, 3];
  const at = (i: number) => segs[i];

  it('finds the first render line at or after a backend segment boundary', () => {
    // "seg 0 is past" ⇒ boundary = first line with seg >= 1 ⇒ index 3.
    expect(lowerBoundSeg(at, segs.length, 1)).toBe(3);
    // seg >= 2 ⇒ index 5; seg >= 3 ⇒ index 6.
    expect(lowerBoundSeg(at, segs.length, 2)).toBe(5);
    expect(lowerBoundSeg(at, segs.length, 3)).toBe(6);
  });

  it('boundary 0 is the start; past-the-end targets return len', () => {
    expect(lowerBoundSeg(at, segs.length, 0)).toBe(0);
    expect(lowerBoundSeg(at, segs.length, 4)).toBe(segs.length);
    expect(lowerBoundSeg(at, segs.length, 99)).toBe(segs.length);
  });

  it('a missing backend seg maps to where it would begin (skipped/disabled ops)', () => {
    // No render line carries seg 2 here (e.g. a disabled op between 1 and 4).
    const sparse = [0, 1, 1, 4, 4];
    const sat = (i: number) => sparse[i];
    // Boundary "seg <= 2 past" ⇒ first line with seg >= 3 ⇒ the seg-4 run at 3.
    expect(lowerBoundSeg(sat, sparse.length, 3)).toBe(3);
    expect(lowerBoundSeg(sat, sparse.length, 2)).toBe(3);
  });
});
