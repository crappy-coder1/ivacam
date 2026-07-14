import { brightnessToRgba } from '../../cam/raster_preview';
import type { ReliefGrid, ReliefSource } from '../../state/project-types';
import { reliefDisplayBrightness } from '../../state/relief';
import type { ProjectFn } from './types';

/// Cache of the decoded preview image per relief source, keyed by source
/// id. Invalidated when the source's `grid` reference changes (origin /
/// cell edits keep the same grid object, so a drag never rebuilds the 256²
/// ImageData). Grayscale sources render their brightness; heightgrid (STL)
/// sources render their z normalized to a heatmap (see
/// `reliefDisplayBrightness`), so both kinds get a placement preview.
export class RasterImageCache {
  private cache = new Map<number, { grid: ReliefGrid; canvas: HTMLCanvasElement }>();

  canvasFor(src: ReliefSource): HTMLCanvasElement | null {
    if (src.cols <= 0 || src.rows <= 0) return null;
    const cached = this.cache.get(src.id);
    if (cached && cached.grid === src.grid) return cached.canvas;
    const cv = document.createElement('canvas');
    cv.width = src.cols;
    cv.height = src.rows;
    const ictx = cv.getContext('2d');
    if (!ictx) return null;
    const rgba = brightnessToRgba(reliefDisplayBrightness(src.grid), src.cols, src.rows);
    const img = ictx.createImageData(src.cols, src.rows);
    img.data.set(rgba);
    ictx.putImageData(img, 0, 0);
    this.cache.set(src.id, { grid: src.grid, canvas: cv });
    return cv;
  }
}

export interface RasterPlacementColors {
  accent: string;
  border: string;
}

/// Paint the faint placed raster images (+ selection / placement
/// border) on the overlay, under the interaction chrome — so a source
/// move repaints without touching the heavy bg layer.
export function drawRasterPlacements(
  ctx: CanvasRenderingContext2D,
  p: ProjectFn,
  scale: number,
  cache: RasterImageCache,
  placements: readonly { src: ReliefSource; selected: boolean }[],
  colors: RasterPlacementColors,
) {
  for (const { src, selected } of placements) {
    const cv = cache.canvasFor(src);
    if (!cv) continue;
    const wmm = src.cols * src.cell;
    const hmm = src.rows * src.cell;
    const [x0, y0] = p(src.origin.x, src.origin.y + hmm); // world top-left
    const wpx = wmm * scale;
    const hpx = hmm * scale;
    const prevAlpha = ctx.globalAlpha;
    const prevSmooth = ctx.imageSmoothingEnabled;
    ctx.globalAlpha = 0.5;
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(cv, x0, y0, wpx, hpx);
    ctx.globalAlpha = prevAlpha;
    ctx.imageSmoothingEnabled = prevSmooth;
    ctx.lineWidth = selected ? 2 : 1;
    ctx.strokeStyle = selected ? colors.accent : colors.border;
    if (!selected) ctx.setLineDash([4, 3]);
    ctx.strokeRect(x0, y0, wpx, hpx);
    ctx.setLineDash([]);
  }
}
