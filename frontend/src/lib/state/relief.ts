/// Small helpers over a `ReliefSource`'s tagged `grid`, so call sites don't
/// repeat the discriminant narrowing. A grayscale source carries real
/// brightness (usable by raster-engrave + the histogram); a heightgrid (STL)
/// source carries real Z that only the relief-mill op reads. For DISPLAY we
/// derive a normalized brightness for either kind so both get a 2D placement
/// preview / heatmap.

import type { ReliefGrid, ReliefSource } from './project-types';

/// The grayscale brightness grid, or `null` for a heightgrid (STL) source —
/// which has no brightness to modulate laser power or bin into a histogram.
export function reliefBrightness(s: ReliefSource): number[] | null {
  return s.grid.kind === 'grayscale' ? s.grid.brightness : null;
}

/// True for an STL height-grid source (real Z), false for a grayscale image.
export function isHeightgrid(s: ReliefSource): boolean {
  return s.grid.kind === 'heightgrid';
}

/// A normalized [0, 1] grid for DISPLAY, defined for both kinds: the raw
/// brightness for a grayscale source, or the height grid mapped from its own
/// z-range (deepest → 0 / dark, model top → 1 / bright) for a heightgrid
/// source. A flat grid maps to all-1 (everything at the top). The returned
/// array is fresh per call for a heightgrid — cache it against `grid` if you
/// key on reference.
export function reliefDisplayBrightness(grid: ReliefGrid): number[] {
  if (grid.kind === 'grayscale') return grid.brightness;
  const z = grid.z;
  let lo = Infinity;
  let hi = -Infinity;
  for (const v of z) {
    if (v < lo) lo = v;
    if (v > hi) hi = v;
  }
  const span = hi - lo;
  if (!(span > 0)) return z.map(() => 1);
  return z.map((v) => (v - lo) / span);
}
