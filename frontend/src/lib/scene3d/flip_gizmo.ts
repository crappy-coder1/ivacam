/// Pure geometry for the two-sided "flip stock" 3D gizmo — the hinge axis
/// line + the 180° roll arrow that make the flip axis unmistakable in the
/// scene (ivac-rt1.11.5 "U"). Kept THREE-free so the math is unit-testable
/// without a WebGL context (same split as `./footprint` and `../sim/warnings`);
/// `StockBoxBuilder` turns these plain point triples into meshes.
///
/// The flip AXIS is the single most error-prone choice in two-sided work —
/// pick the wrong one and the back-face cuts land mirrored the wrong way, i.e.
/// scrap — so the visual spells it out: a line along the axis the stock hinges
/// about, and an arc rolling up over the top and plunging down the far edge.

export interface Vec3 {
  x: number;
  y: number;
  z: number;
}

export interface FlipGizmoInput {
  /// Stock footprint (from `computeFootprint`).
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
  /// Stock top plane z (WCS) — the gizmo sits on the top face.
  topZ: number;
  /// Which axis the stock hinges about. `'x'` = turn over about a line
  /// parallel to X (mirrors Y); `'y'` = about a line parallel to Y (mirrors X).
  axis: 'x' | 'y';
  /// Arc tessellation (polyline segment count). Default 32.
  arcSegments?: number;
}

export interface FlipGizmo {
  /// The hinge line on the top face, through the stock centre, running
  /// parallel to `axis` and slightly overhanging the stock edges.
  hinge: { a: Vec3; b: Vec3 };
  /// Half-width of the roll arc (mm) along the perpendicular — how far the arc
  /// endpoints reach toward the stock edges.
  spanHalf: number;
  /// Apex height of the roll arc (mm) above the top plane. Kept well below
  /// `spanHalf` so the arch is a wide, flat sweep that stays inside a fitted
  /// view rather than a tall dome that shoots off-screen.
  height: number;
  /// Polyline (`arcSegments + 1` points) of the 180° roll arc — a half-ellipse
  /// in the plane perpendicular to the hinge, arcing over the top from the near
  /// edge, over the apex, down to the far edge.
  arc: Vec3[];
  /// Arrowhead at the far end of the arc: world position + unit direction
  /// (the arc tangent there, pointing down into the far edge) for a cone.
  arrow: { tip: Vec3; dir: Vec3 };
}

/// Slight overhang so the hinge line pokes past the stock edges and reads as
/// an axis rather than an edge.
const HINGE_OVERHANG = 1.06;
/// Arc half-width as a fraction of the perpendicular span — under half so the
/// endpoints land just inside the stock outline while still spanning most of it.
const ARC_SPAN_FRAC = 0.4;
/// Arc apex height as a fraction of the perpendicular span — a low, wide arch.
const ARC_HEIGHT_FRAC = 0.2;
/// Floors so the gizmo stays visible on tiny stock.
const MIN_SPAN_MM = 2;
const MIN_HEIGHT_MM = 1;

export function computeFlipGizmo(input: FlipGizmoInput): FlipGizmo {
  const { minX, minY, maxX, maxY, topZ, axis } = input;
  const segments = Math.max(2, input.arcSegments ?? 32);
  const cx = (minX + maxX) * 0.5;
  const cy = (minY + maxY) * 0.5;
  const sizeX = maxX - minX;
  const sizeY = maxY - minY;

  const alongX = axis === 'x';
  // Unit vector along the hinge, and the horizontal unit perpendicular to it.
  const ux = alongX ? 1 : 0;
  const uy = alongX ? 0 : 1;
  const px = alongX ? 0 : 1;
  const py = alongX ? 1 : 0;
  const axisDim = alongX ? sizeX : sizeY;
  const perpDim = alongX ? sizeY : sizeX;

  const hingeHalf = axisDim * 0.5 * HINGE_OVERHANG;
  const spanHalf = Math.max(MIN_SPAN_MM, perpDim * ARC_SPAN_FRAC);
  const height = Math.max(MIN_HEIGHT_MM, perpDim * ARC_HEIGHT_FRAC);

  const hinge = {
    a: { x: cx + ux * hingeHalf, y: cy + uy * hingeHalf, z: topZ },
    b: { x: cx - ux * hingeHalf, y: cy - uy * hingeHalf, z: topZ },
  };

  // Half-ellipse in the perpendicular–vertical plane through the stock centre:
  // θ=0 at the near edge (+perp, top plane), θ=π/2 at the apex (over the top),
  // θ=π at the far edge (−perp, top plane). Wide (`spanHalf`) but low (`height`).
  const arc: Vec3[] = [];
  for (let i = 0; i <= segments; i++) {
    const th = (Math.PI * i) / segments;
    const c = Math.cos(th);
    const s = Math.sin(th);
    arc.push({
      x: cx + px * spanHalf * c,
      y: cy + py * spanHalf * c,
      z: topZ + height * s,
    });
  }

  // Tangent of the arc at θ=π: d/dθ[perp·spanHalf·cosθ + z·height·sinθ] =
  // perp·(−spanHalf·sinθ) + z·(height·cosθ) → at θ=π that is (0, 0, −height),
  // i.e. straight down into the far edge — the point that was on top plunging
  // under as the stock rolls over.
  const arrow = {
    tip: arc[arc.length - 1],
    dir: { x: 0, y: 0, z: -1 },
  };

  return { hinge, spanHalf, height, arc, arrow };
}
