// Tab-placement hit-test + edit transforms for EntityCanvas2D's context
// menu. Extracted from the component so the right-click "which tab is under
// the cursor" projection and the popover's width/height patch + delete stay
// pure and unit-testable, without a canvas or the rune runtime. The
// component keeps the thin glue that reads $state/project, filters ops, and
// routes the returned arrays through project.updateOperation (one undo entry).

import { polylineAtT, type ObjectPolyline } from '../cam/tabs';

/// A tab-bearing op reduced to what the hit-test needs: its id and the
/// placements to probe. The caller filters ops (contour + manual/mixed tab
/// mode) and drops the op-model coupling before calling in.
export interface TabHitOp {
  opId: number;
  placements: ReadonlyArray<{ objectId: number; t: number }>;
}

/// Find the op tab placement whose screen position is nearest the cursor
/// within a 10-px tolerance, scanning EVERY supplied op (not just the
/// selected one) so a right-click lands on "that tab right there" regardless
/// of which op is active. Returns the winning (opId, placementIdx) or null
/// when none is within tolerance.
export function findTabAtPixel(
  cx: number,
  cy: number,
  transform: { scale: number; offX: number; offY: number },
  objects: ObjectPolyline[],
  ops: TabHitOp[],
): { opId: number; placementIdx: number } | null {
  const { scale, offX, offY } = transform;
  const tolPx = 10;
  let best: { opId: number; placementIdx: number; d2: number } | null = null;
  for (const op of ops) {
    for (let i = 0; i < op.placements.length; i++) {
      const tp = op.placements[i];
      const obj = objects.find((o) => o.objectId === tp.objectId);
      if (!obj) continue;
      const { point } = polylineAtT(obj.pts, tp.t, obj.closed);
      const sx = point.x * scale + offX;
      const sy = offY - point.y * scale;
      const d2 = (cx - sx) * (cx - sx) + (cy - sy) * (cy - sy);
      if (d2 > tolPx * tolPx) continue;
      if (best && d2 >= best.d2) continue;
      best = { opId: op.opId, placementIdx: i, d2 };
    }
  }
  return best ? { opId: best.opId, placementIdx: best.placementIdx } : null;
}

/// Return the placements with `idx` merged with `patch` (width / height
/// override), or null when `idx` is out of bounds (a no-op the caller skips).
export function patchTabPlacement<T extends object>(
  placements: readonly T[],
  idx: number,
  patch: Partial<T>,
): T[] | null {
  if (idx < 0 || idx >= placements.length) return null;
  return placements.map((p, i) => (i === idx ? { ...p, ...patch } : p));
}

/// Return the placements with `idx` removed, or null when `idx` is out of
/// bounds (a no-op the caller skips).
export function removeTabPlacement<T>(placements: readonly T[], idx: number): T[] | null {
  if (idx < 0 || idx >= placements.length) return null;
  return placements.filter((_, i) => i !== idx);
}
