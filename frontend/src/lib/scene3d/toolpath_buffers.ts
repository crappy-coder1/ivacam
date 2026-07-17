// Pure toolpath-buffer geometry math, extracted from Scene3D.svelte
// so the direction-arrow chevron math — the kind of vector
// geometry that silently regresses — can be unit-tested without a live
// THREE renderer. Scene3D still owns the buffer assembly + GPU upload;
// these are the side-effect-free pieces it calls.

export interface Vec3 {
  x: number;
  y: number;
  z: number;
}

/// Tunables for the direction-arrow chevrons drawn on cutting moves.
export interface ArrowParams {
  /// Segments shorter than this (mm) never get an arrow.
  minLen: number;
  /// Absolute cap on arrow size (mm).
  maxSize: number;
  /// Arrow size as a fraction of segment length.
  sizeFrac: number;
  /// Half-wing spread: `tan(wing_angle)`. The default 30° wings use
  /// `Math.tan((30 * Math.PI) / 180)`.
  halfWing: number;
}

/// The two wing line-segments of a direction-arrow chevron. Each wing runs
/// from a back-set wing tip to the shared apex at the segment midpoint
/// (matching the `mid → tip` line pairs the fat-line buffer expects).
export interface ArrowChevron {
  /// Segment midpoint — the chevron apex, shared by both wings.
  mid: [number, number, number];
  /// `+normal`-side wing tip.
  wing1: [number, number, number];
  /// `-normal`-side wing tip.
  wing2: [number, number, number];
}

/// Arrow spacing (mm) from the user's density setting. Density 0 ⇒
/// `Infinity` (no segment ever qualifies ⇒ arrows disabled).
export function arrowSpacingMm(density: number): number {
  return density > 0 ? 3.0 / density : Infinity;
}

/// Compute the direction-arrow chevron for a cut move from `from` to `to`,
/// or `null` when the segment is shorter than `p.minLen` (too short to
/// carry a legible arrow). Spacing / move-kind eligibility is the caller's
/// concern — this is pure geometry.
///
/// The arrow points along the move direction: the apex sits at the segment
/// midpoint and the two wings sweep back by `A` along the reversed
/// direction and out by `A * halfWing` along the in-plane normal, where
/// `A = min(len * sizeFrac, maxSize)`. A near-pure-Z move (plunge /
/// retract, no meaningful XY component) uses a fixed `+X` normal so the
/// chevron stays visible from a top-down camera.
export function computeArrowChevron(from: Vec3, to: Vec3, p: ArrowParams): ArrowChevron | null {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const dz = to.z - from.z;
  const len = Math.sqrt(dx * dx + dy * dy + dz * dz);
  if (len < p.minLen) return null;

  const A = Math.min(len * p.sizeFrac, p.maxSize);
  const ux = dx / len;
  const uy = dy / len;
  const uz = dz / len;

  // In-plane normal: rotate the forward direction 90° CCW in XY when the
  // move has a meaningful horizontal component (the common case). A
  // pure-Z move falls back to +X so the arrow reads from any angle.
  let nx: number;
  let ny: number;
  let nz: number;
  const xyLen = Math.hypot(ux, uy);
  if (xyLen > 0.01) {
    nx = -uy / xyLen;
    ny = ux / xyLen;
    nz = 0;
  } else {
    nx = 1;
    ny = 0;
    nz = 0;
  }

  const mx = (from.x + to.x) * 0.5;
  const my = (from.y + to.y) * 0.5;
  const mz = (from.z + to.z) * 0.5;
  const side = A * p.halfWing;
  return {
    mid: [mx, my, mz],
    wing1: [mx - A * ux + side * nx, my - A * uy + side * ny, mz - A * uz + side * nz],
    wing2: [mx - A * ux - side * nx, my - A * uy - side * ny, mz - A * uz - side * nz],
  };
}

/// Planar arc descriptor carried on a tessellated `G2`/`G3` chord —
/// mirrors the backend `ArcXY` wire type (`toolpath[i].arc`). `(cx, cy)` is
/// the arc center in world XY; the radius is implied by the chord's `from`
/// point; `ccw` is the sweep direction (G3 = `true`, G2 = `false`).
export interface ArcXY {
  cx: number;
  cy: number;
  ccw: boolean;
}

/// Tessellate one arc-tagged toolpath chord into a smooth render polyline —
/// the on-read half of bd ivac-58nl.9. The backend now emits a COARSE G2/G3
/// chord stream (the sim carves each chord as its exact analytic sub-arc via
/// the `arc` descriptor, so density no longer bounds sim accuracy), which
/// would draw as visible polyline teeth if rendered verbatim. This walks the
/// chord's true sub-arc at `stepRad` per render-chord so the wireframe stays
/// smooth regardless of how coarse the payload is.
///
/// Returns the full point list INCLUDING the exact `from` / `to` endpoints
/// (interior points computed from the swept angle, endpoints copied verbatim
/// so adjacent chords meet with no gap). The Z is interpolated linearly across
/// the sweep (a helical G2/G3). Sweep resolution mirrors
/// `preview::interpret_with_index` on the Rust side (coincident endpoints ⇒ a
/// full revolution in the requested direction).
export function tessellateArc(from: Vec3, to: Vec3, arc: ArcXY, stepRad: number): Vec3[] {
  const TAU = Math.PI * 2;
  const { cx, cy, ccw } = arc;
  const r = Math.hypot(from.x - cx, from.y - cy);
  if (r < 1e-9) return [from, to]; // start on the center — degenerate
  const thetaStart = Math.atan2(from.y - cy, from.x - cx);
  const thetaEnd = Math.atan2(to.y - cy, to.x - cx);
  const coincident = Math.abs(from.x - to.x) < 1e-9 && Math.abs(from.y - to.y) < 1e-9;
  let sweep = thetaEnd - thetaStart;
  if (ccw) {
    if (coincident) sweep = TAU;
    else if (sweep <= 1e-9) sweep += TAU;
  } else if (coincident) sweep = -TAU;
  else if (sweep >= -1e-9) sweep -= TAU;
  const n = Math.max(1, Math.ceil(Math.abs(sweep) / Math.max(stepRad, 1e-6)));
  if (n <= 1) return [from, to];
  const dz = to.z - from.z;
  const pts: Vec3[] = [from];
  for (let k = 1; k < n; k++) {
    const theta = thetaStart + (sweep * k) / n;
    pts.push({
      x: cx + r * Math.cos(theta),
      y: cy + r * Math.sin(theta),
      z: from.z + (dz * k) / n,
    });
  }
  pts.push(to);
  return pts;
}

/// Lower bound over a sorted-by-`seg` sequence: the first index `i ∈ [0, len)`
/// whose `segAt(i) >= target`, or `len` if none. Used by the playhead fade to
/// map a backend segment boundary to a render-line boundary now that one arc
/// segment can span many render lines (bd ivac-58nl.9) — the per-segment color
/// entries are pushed in backend-segment order, so they're sorted by `seg`.
export function lowerBoundSeg(segAt: (i: number) => number, len: number, target: number): number {
  let lo = 0;
  let hi = len;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (segAt(mid) < target) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/// An RGB triple, each channel in [0, 1] (the fat-line buffer's color
/// layout). Not clamped here — the toolpath base colors can ride a
/// move-kind boost slightly past 1.0, exactly as the inline math did.
export type Rgb = [number, number, number];

/// Brightness multiplier per move kind — rapids dimmest,
/// plunge/retract mid, cuts/arcs (and anything else) brightest. Pulled
/// out of `rebuildToolpathGeometry` so the emphasis ladder is one
/// testable place instead of an inline ternary.
export function moveBoost(kind: string): number {
  if (kind === 'rapid') return 0.5;
  if (kind === 'plunge' || kind === 'retract') return 0.85;
  return 1.15;
}

/// Final base color for a toolpath segment. `op_id === 0`
/// (legacy / unstamped moves) uses the move-kind tint verbatim; a
/// stamped op uses its hue color scaled by the move-kind boost so the
/// cut/rapid/plunge emphasis reads on top of the per-op hue. THREE +
/// theme lookups stay in the caller — pass the already-resolved
/// `moveTint` (themed per kind) and `opColor` (op hue → RGB) triples.
export function resolveSegmentColor(opId: number, kind: string, moveTint: Rgb, opColor: Rgb): Rgb {
  if (opId === 0) return [moveTint[0], moveTint[1], moveTint[2]];
  const b = moveBoost(kind);
  return [opColor[0] * b, opColor[1] * b, opColor[2] * b];
}

/// Playhead fade. A `past` move (already cut) renders at full
/// `base` color; a `future` move dims to `base * factor + offset` — the
/// offset keeps a faded line visible (a non-black floor) instead of
/// collapsing to the background. Pulled out of `applyToolpathFade`.
export function fadeColor(base: Rgb, past: boolean, factor: number, offset: number): Rgb {
  if (past) return [base[0], base[1], base[2]];
  return [base[0] * factor + offset, base[1] * factor + offset, base[2] * factor + offset];
}
