// Two-sided (flip-stock) conflict detection. A pure, THREE-free companion
// to `./dual_surface`: given the two sims' FINISHED heightfields, find the
// columns where the front carve and the (reflected) back carve overlap —
// or cut clean through — so the preview can flag them.
//
// Both heightfields live in the SAME top-down numeric frame (value = the
// remaining surface's Z, starting at `topZ` and only ever dropping toward
// the stock bottom `topZ - thickness`). The back surface is rendered
// reflected about the stock mid-plane (see `dual_surface.ts`), so its
// reflected top face sits at `backReflectionOffsetZ - back[i]`.
//
// A column has enough material iff the front cut floor stays ABOVE the
// back's reflected top:
//     front[i]  >  (offset - back[i])         where offset = 2·midPlane
//   ⇔ front[i] + back[i]  >  offset
// so the remaining thickness is `front[i] + back[i] - offset`. When that is
// ≤ 0 the two carves have crossed the stock — an overlap (both sides bit
// into the same material) or an exact clean-through (they just meet). Both
// are conflicts the user must see before cutting.

import { backReflectionOffsetZ, midPlaneZ } from './dual_surface';

/// Grid + stock geometry shared by the two aligned heightfields.
export interface ConflictGrid {
  cols: number;
  rows: number;
  cellSize: number;
  originX: number;
  originY: number;
  topZ: number;
  thickness: number;
}

/// One contiguous conflict region, collapsed to a single world-anchored
/// marker at its worst (deepest-overlap) cell.
export interface ConflictMarker {
  /// World XY of the worst cell's center.
  x: number;
  y: number;
  /// World Z at the overlap midpoint of the worst cell — a point inside
  /// the crossing, between the front cut floor and the back's reflected
  /// top, so the marker sits where the two carves collide.
  z: number;
  /// Grid cells in the contiguous conflict region (4-connected).
  cellCount: number;
  /// Worst overlap depth across the region, in mm (always > 0). How far
  /// the two carves cross past each other at the anchor cell.
  overlapMm: number;
}

/// Cells whose combined front+back cut leaves this much material or less
/// (mm) count as a conflict. Slightly positive so an exact clean-through
/// (sides meeting at zero remaining) still flags, while sub-micron float
/// noise on a clean job does not.
const CONFLICT_EPS_MM = 1e-4;

/// Detect two-sided conflict regions from the finished front + back
/// heightfields (same grid, same top-down frame). Conflicting cells are
/// clustered 4-connected so one marker represents each contiguous problem
/// area, anchored at that region's worst cell. Returns `[]` when the two
/// carves never cross (the common, good case).
export function detectTwoSidedConflicts(
  front: Float32Array,
  back: Float32Array,
  grid: ConflictGrid,
): ConflictMarker[] {
  const { cols, rows, cellSize, originX, originY, topZ, thickness } = grid;
  const n = cols * rows;
  if (n <= 0 || front.length < n || back.length < n) return [];

  // remaining[i] = front[i] + back[i] - offset; ≤ EPS ⇒ conflict.
  const offset = backReflectionOffsetZ(topZ, thickness);
  const remaining = new Float32Array(n);
  // 0 = clear, 1 = conflict (unvisited), 2 = conflict (visited).
  const state = new Uint8Array(n);
  let any = false;
  for (let i = 0; i < n; i++) {
    const r = front[i] + back[i] - offset;
    remaining[i] = r;
    if (r <= CONFLICT_EPS_MM) {
      state[i] = 1;
      any = true;
    }
  }
  if (!any) return [];

  const markers: ConflictMarker[] = [];
  const stack: number[] = [];
  for (let start = 0; start < n; start++) {
    if (state[start] !== 1) continue;
    // Flood-fill this region (iterative DFS), tracking its worst cell.
    state[start] = 2;
    stack.length = 0;
    stack.push(start);
    let worstIdx = start;
    let worstR = remaining[start];
    let cellCount = 0;
    while (stack.length > 0) {
      const idx = stack.pop() as number;
      cellCount++;
      if (remaining[idx] < worstR) {
        worstR = remaining[idx];
        worstIdx = idx;
      }
      const ix = idx % cols;
      const iy = (idx - ix) / cols;
      if (ix > 0 && state[idx - 1] === 1) {
        state[idx - 1] = 2;
        stack.push(idx - 1);
      }
      if (ix + 1 < cols && state[idx + 1] === 1) {
        state[idx + 1] = 2;
        stack.push(idx + 1);
      }
      if (iy > 0 && state[idx - cols] === 1) {
        state[idx - cols] = 2;
        stack.push(idx - cols);
      }
      if (iy + 1 < rows && state[idx + cols] === 1) {
        state[idx + cols] = 2;
        stack.push(idx + cols);
      }
    }
    const wix = worstIdx % cols;
    const wiy = (worstIdx - wix) / cols;
    // Overlap band (world Z): front cut floor `front[worstIdx]` up to the
    // back's reflected top `offset - back[worstIdx]`; they cross, so the
    // midpoint is a point inside the collision.
    const backReflectedTop = offset - back[worstIdx];
    markers.push({
      x: originX + (wix + 0.5) * cellSize,
      y: originY + (wiy + 0.5) * cellSize,
      z: (front[worstIdx] + backReflectedTop) / 2,
      cellCount,
      overlapMm: -worstR,
    });
  }
  return markers;
}

/// Re-exported for callers that want the mid-plane directly (e.g. tests /
/// diagnostics) without reaching into `dual_surface`.
export { midPlaneZ };
