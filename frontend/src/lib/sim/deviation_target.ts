/// Build the target `SurfaceField` for the deviation overlay from a project's
/// relief op + source, mirroring the pipeline op driver
/// (`crates/ivac-core/src/pipeline/op_drivers/surface_mill.rs`) so the overlay
/// compares the carved stock against exactly the surface the relief op was
/// generated from.

import type { SurfaceField } from '../api/types';
import type { OpEntry, ReliefMillOp } from '../state/op_types';
import type { ReliefSource } from '../state/project-types';

/// Turn a `relief_mill` op + its referenced source into the target surface
/// the op cuts toward, or `null` when the source is missing. Two grid kinds
/// map depth differently (kept in lock-step with the Rust op driver):
///   * grayscale — brightness in [0,1] is linearly remapped across
///     `[min(zMin,zMax), max(zMin,zMax)]`, `invert` flipping the sense
///     (matches `SurfaceField::from_grayscale`);
///   * heightgrid — the Z is real geometry (STL top already at 0), used
///     directly (matches `SurfaceField::new`).
export function reliefTargetSurface(
  op: ReliefMillOp,
  sources: ReliefSource[],
): SurfaceField | null {
  const source = sources.find((s) => s.id === op.sourceId);
  if (!source) return null;
  const { origin, cell, cols, rows, grid } = source;
  let z: number[];
  if (grid.kind === 'heightgrid') {
    z = grid.z;
  } else {
    const lo = Math.min(op.zMinMm, op.zMaxMm);
    const hi = Math.max(op.zMinMm, op.zMaxMm);
    const span = hi - lo;
    z = grid.brightness.map((b) => {
      let t = b < 0 ? 0 : b > 1 ? 1 : b;
      if (op.invert) t = 1 - t;
      // t = 1 (bright) → hi (shallow/top); t = 0 (dark) → lo (deep).
      return lo + t * span;
    });
  }
  return { origin: { x: origin.x, y: origin.y }, cell, cols, rows, z };
}

/// Pick the deviation-overlay target for a project: the first ENABLED
/// `relief_mill` op in document order whose source resolves. Returns `null`
/// when there is no such op (the overlay has nothing to compare against).
///
/// v1 uses a single relief op — a project milling several distinct reliefs
/// only verifies the first; unioning multiple targets is a follow-up.
export function activeDeviationTarget(
  ops: OpEntry[],
  sources: ReliefSource[],
): SurfaceField | null {
  for (const op of ops) {
    if (op.kind === 'relief_mill' && op.enabled) {
      const surf = reliefTargetSurface(op, sources);
      if (surf) return surf;
    }
  }
  return null;
}
