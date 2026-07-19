import { describe, it, expect } from 'vitest';
import { detectTwoSidedConflicts, type ConflictGrid } from './two_sided_conflict';

// Shared stock frame: top at Z=0, 10 mm thick → mid-plane at −5, stock
// bottom at −10, reflection offset (2·midPlane) at −10. Uncut cells sit at
// front=back=0 → remaining = 0 + 0 − (−10) = 10 mm (clear).
function grid(cols: number, rows: number): ConflictGrid {
  return { cols, rows, cellSize: 2, originX: 10, originY: 20, topZ: 0, thickness: 10 };
}

/// A cols×rows heightfield with every cell uncut (at topZ = 0).
function uncut(cols: number, rows: number): Float32Array {
  return new Float32Array(cols * rows); // zero-filled = topZ
}

describe('detectTwoSidedConflicts', () => {
  it('returns [] when the two carves never cross', () => {
    // Both sides cut 3 mm: remaining = −3 + −3 + 10 = 4 mm > 0 everywhere.
    const front = new Float32Array(9).fill(-3);
    const back = new Float32Array(9).fill(-3);
    expect(detectTwoSidedConflicts(front, back, grid(3, 3))).toEqual([]);
  });

  it('flags a single overlapping cell and anchors it at the cell center', () => {
    const front = uncut(3, 3);
    const back = uncut(3, 3);
    // Cell (1,1): front 6 mm + back 6 mm deep → 2 mm overlap past the stock.
    front[4] = -6;
    back[4] = -6;
    const markers = detectTwoSidedConflicts(front, back, grid(3, 3));
    expect(markers).toHaveLength(1);
    const m = markers[0];
    expect(m.x).toBeCloseTo(10 + 1.5 * 2, 6); // 13
    expect(m.y).toBeCloseTo(20 + 1.5 * 2, 6); // 23
    expect(m.z).toBeCloseTo(-5, 6); // overlap midpoint = mid-plane
    expect(m.cellCount).toBe(1);
    expect(m.overlapMm).toBeCloseTo(2, 6);
  });

  it('clusters a contiguous overlap region into one marker at its worst cell', () => {
    const front = uncut(3, 3);
    const back = uncut(3, 3);
    // 2×2 block (0,0),(1,0),(0,1),(1,1) all overlap; (1,1) is deepest.
    for (const i of [0, 1, 3]) {
      front[i] = -6;
      back[i] = -6;
    }
    front[4] = -7;
    back[4] = -7; // cell (1,1): 4 mm overlap
    const markers = detectTwoSidedConflicts(front, back, grid(3, 3));
    expect(markers).toHaveLength(1);
    expect(markers[0].cellCount).toBe(4);
    expect(markers[0].overlapMm).toBeCloseTo(4, 6);
    expect(markers[0].x).toBeCloseTo(13, 6); // anchored at (1,1)
    expect(markers[0].y).toBeCloseTo(23, 6);
  });

  it('keeps diagonal-only conflict cells as separate regions (4-connectivity)', () => {
    const front = uncut(3, 3);
    const back = uncut(3, 3);
    // Cells (0,0) and (2,2) touch only at a corner → two markers.
    for (const i of [0, 8]) {
      front[i] = -6;
      back[i] = -6;
    }
    const markers = detectTwoSidedConflicts(front, back, grid(3, 3));
    expect(markers).toHaveLength(2);
    expect(markers.every((m) => m.cellCount === 1)).toBe(true);
  });

  it('flags a clean-through (front cuts full thickness, back uncut)', () => {
    const front = uncut(2, 2);
    const back = uncut(2, 2);
    // Cell 0: front cuts the whole 10 mm to the stock bottom; back uncut.
    // remaining = −10 + 0 + 10 = 0 ⇒ still a conflict (sides meet).
    front[0] = -10;
    const markers = detectTwoSidedConflicts(front, back, grid(2, 2));
    expect(markers).toHaveLength(1);
    expect(markers[0].overlapMm).toBeCloseTo(0, 6);
  });

  it('is defensive against undersized buffers', () => {
    expect(detectTwoSidedConflicts(new Float32Array(2), new Float32Array(9), grid(3, 3))).toEqual(
      [],
    );
    expect(detectTwoSidedConflicts(new Float32Array(0), new Float32Array(0), grid(0, 0))).toEqual(
      [],
    );
  });
});
