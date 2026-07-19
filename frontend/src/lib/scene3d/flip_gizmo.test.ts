import { describe, it, expect } from 'vitest';
import { computeFlipGizmo, type FlipGizmoInput } from './flip_gizmo';

/// A 100×60 stock centred on the origin, top at z=0.
const BASE: FlipGizmoInput = { minX: -50, minY: -30, maxX: 50, maxY: 30, topZ: 0, axis: 'x' };

describe('computeFlipGizmo', () => {
  it('lays the hinge along X and overhangs the stock for axis=x', () => {
    const g = computeFlipGizmo(BASE);
    // Runs parallel to X (constant Y=centre, constant Z=top).
    expect(g.hinge.a.y).toBeCloseTo(0);
    expect(g.hinge.b.y).toBeCloseTo(0);
    expect(g.hinge.a.z).toBeCloseTo(0);
    expect(g.hinge.b.z).toBeCloseTo(0);
    // Symmetric about the centre and overhangs half-span 50 (×1.06 = 53).
    expect(g.hinge.a.x).toBeCloseTo(53);
    expect(g.hinge.b.x).toBeCloseTo(-53);
  });

  it('lays the hinge along Y for axis=y', () => {
    const g = computeFlipGizmo({ ...BASE, axis: 'y' });
    expect(g.hinge.a.x).toBeCloseTo(0);
    expect(g.hinge.b.x).toBeCloseTo(0);
    // half-span in Y is 30 → ×1.06 = 31.8.
    expect(g.hinge.a.y).toBeCloseTo(31.8);
    expect(g.hinge.b.y).toBeCloseTo(-31.8);
  });

  it('sizes the arc span + height from the span perpendicular to the hinge', () => {
    // axis=x → perpendicular span is Y (60) → span 0.4·60 = 24, height 0.2·60 = 12.
    expect(computeFlipGizmo(BASE).spanHalf).toBeCloseTo(24);
    expect(computeFlipGizmo(BASE).height).toBeCloseTo(12);
    // axis=y → perpendicular span is X (100) → span 40, height 20.
    expect(computeFlipGizmo({ ...BASE, axis: 'y' }).spanHalf).toBeCloseTo(40);
    expect(computeFlipGizmo({ ...BASE, axis: 'y' }).height).toBeCloseTo(20);
  });

  it('is a wide, flat arch — apex height well below the half-span', () => {
    const g = computeFlipGizmo(BASE);
    expect(g.height).toBeLessThan(g.spanHalf * 0.6);
  });

  it('arcs from the near edge over an apex above the top to the far edge', () => {
    const g = computeFlipGizmo(BASE);
    const first = g.arc[0];
    const mid = g.arc[Math.floor(g.arc.length / 2)];
    const last = g.arc[g.arc.length - 1];
    // Endpoints sit on the top plane, offset ±spanHalf along the perp (Y) axis.
    expect(first.z).toBeCloseTo(0);
    expect(last.z).toBeCloseTo(0);
    expect(first.y).toBeCloseTo(g.spanHalf);
    expect(last.y).toBeCloseTo(-g.spanHalf);
    // Apex rises `height` above the top plane, over the centre.
    expect(mid.z).toBeCloseTo(g.height);
    expect(mid.y).toBeCloseTo(0);
    // The arc never dips below the stock top.
    expect(Math.min(...g.arc.map((p) => p.z))).toBeGreaterThanOrEqual(-1e-9);
  });

  it('every arc point lies on the half-ellipse (spanHalf × height) about the top centre', () => {
    const g = computeFlipGizmo(BASE);
    for (const p of g.arc) {
      // axis=x → the ellipse lives in the Y–Z plane (X stays at centre).
      expect(p.x).toBeCloseTo(0);
      const e = (p.y / g.spanHalf) ** 2 + (p.z / g.height) ** 2;
      expect(e).toBeCloseTo(1);
    }
  });

  it('points the arrowhead straight down into the far edge', () => {
    const g = computeFlipGizmo(BASE);
    expect(g.arrow.tip).toEqual(g.arc[g.arc.length - 1]);
    expect(g.arrow.dir).toEqual({ x: 0, y: 0, z: -1 });
  });

  it('honours the stock top plane offset', () => {
    const g = computeFlipGizmo({ ...BASE, topZ: 12 });
    expect(g.hinge.a.z).toBeCloseTo(12);
    expect(g.arc[0].z).toBeCloseTo(12); // near edge on the (raised) top plane
    expect(g.arc[Math.floor(g.arc.length / 2)].z).toBeCloseTo(12 + g.height);
  });

  it('keeps a visible span + height on tiny stock (floors)', () => {
    const g = computeFlipGizmo({ minX: 0, minY: 0, maxX: 3, maxY: 3, topZ: 0, axis: 'x' });
    expect(g.spanHalf).toBe(2); // 0.4·3 = 1.2 → clamped to the 2 mm floor
    expect(g.height).toBe(1); // 0.2·3 = 0.6 → clamped to the 1 mm floor
  });

  it('produces arcSegments + 1 points', () => {
    expect(computeFlipGizmo({ ...BASE, arcSegments: 8 }).arc).toHaveLength(9);
  });
});
