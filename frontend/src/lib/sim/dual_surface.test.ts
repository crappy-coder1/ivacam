import { describe, it, expect } from 'vitest';
import { midPlaneZ, backReflectionOffsetZ } from './dual_surface';

/// The reflection realized by the back group: `scale.z = −1` composes with
/// `position.z = offset` to give `worldZ = offset − localZ`.
const reflect = (offset: number, localZ: number): number => offset - localZ;

describe('dual-surface frame math', () => {
  it('places the mid-plane halfway down the stock', () => {
    expect(midPlaneZ(0, 6)).toBe(-3);
    // A raised stock top (offsetZ) shifts the whole span with it.
    expect(midPlaneZ(2, 10)).toBe(-3);
  });

  it('reflects the back top face onto the stock bottom', () => {
    const topZ = 0;
    const thickness = 6;
    const off = backReflectionOffsetZ(topZ, thickness);
    // The back mesh's top plane (local topZ) lands at the stock bottom.
    expect(reflect(off, topZ)).toBeCloseTo(topZ - thickness);
  });

  it('leaves the mid-plane fixed — the shared seam where front and back meet', () => {
    const topZ = 0;
    const thickness = 6;
    const off = backReflectionOffsetZ(topZ, thickness);
    const mid = midPlaneZ(topZ, thickness);
    expect(reflect(off, mid)).toBeCloseTo(mid);
  });

  it('tiles the full thickness: reflected back [bottom, mid] abuts front [mid, top]', () => {
    const topZ = 5; // raised top
    const thickness = 8;
    const off = backReflectionOffsetZ(topZ, thickness);
    const mid = midPlaneZ(topZ, thickness);
    const bottom = topZ - thickness;
    // Back local span [mid, topZ] reflects to [bottom, mid].
    expect(reflect(off, topZ)).toBeCloseTo(bottom);
    expect(reflect(off, mid)).toBeCloseTo(mid);
    // Offset is exactly 2·topZ − thickness.
    expect(off).toBe(2 * topZ - thickness);
  });
});
