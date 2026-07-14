/// STL file → relief height grid, rasterized by the Rust core through the
/// wasm `fromStl` binding. Mirrors `relief_image.ts`'s client-side image
/// decode: the conversion runs at load, and only the resulting Z grid is
/// stored on the `ReliefSource` (as a `heightgrid` `ReliefGrid`), so the raw
/// STL bytes never enter the project JSON. Unlike the image path (browser
/// canvas), this needs the Rust rasterizer, so it loads the wasm module
/// on-demand regardless of the active transport.

import { loadWasmModule } from '../api/wasm';

/// The rasterized height grid — the serialized `SurfaceField` shape the
/// wasm `fromStl` export returns.
export interface HeightGridResult {
  /// World XY of the grid's min corner.
  origin: { x: number; y: number };
  /// Square cell size (mm), derived from the mesh bbox + `maxDim`.
  cell: number;
  cols: number;
  rows: number;
  /// Row-major real target Z per cell (mm), model top shifted to 0.
  z: number[];
}

/// Rasterize an STL `File` into a relief height grid. `maxDim` bounds the
/// longer XY side's cell count (mirrors the image decode's 256 budget), so
/// a physically large model can't blow up the grid. Resolves to `null` when
/// the mesh has no XY footprint (e.g. a fully vertical model — nothing to
/// surface from above). Throws if the bytes aren't a valid STL or the wasm
/// module can't load in this build.
export async function rasterizeStlFile(file: File, maxDim = 256): Promise<HeightGridResult | null> {
  const m = await loadWasmModule();
  if (!m.fromStl) throw new Error('STL rasterization is unavailable in this build');
  const bytes = new Uint8Array(await file.arrayBuffer());
  const out = m.fromStl(bytes, maxDim) as HeightGridResult | null;
  return out ?? null;
}
