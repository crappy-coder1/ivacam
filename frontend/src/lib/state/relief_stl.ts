/// STL file → relief height grid, rasterized by the Rust core through the
/// active transport (`WiacClient.rasterizeStl`). Mirrors `relief_image.ts`'s
/// client-side image decode: the conversion runs at load, and only the
/// resulting Z grid is stored on the `ReliefSource` (as a `heightgrid`
/// `ReliefGrid`), so the raw STL bytes never enter the project JSON. Unlike
/// the image path (browser canvas), STL rasterization needs the Rust
/// rasterizer — but rather than pull a second wasm instance onto the main
/// thread (or the wasm bundle into the tauri/http builds), it routes through
/// whichever transport is active: the wasm worker rasterizes on its existing
/// instance, tauri/http hit the native core. See ivac-fm06.

import { defaultClient } from '../api/http';
import type { WiacClient } from '../api/client';
import type { SurfaceField } from '../api/types';

/// The rasterized height grid — the serialized `SurfaceField` the core
/// returns (`{origin, cell, cols, rows, z}`, real target Z with the model
/// top shifted to 0).
export type HeightGridResult = SurfaceField;

/// Rasterize an STL `File` into a relief height grid. `maxDim` bounds the
/// longer XY side's cell count (mirrors the image decode's 256 budget), so
/// a physically large model can't blow up the grid. Resolves to `null` when
/// the mesh has no XY footprint (e.g. a fully vertical model — nothing to
/// surface from above). Throws if the bytes aren't a valid STL or the active
/// transport can't rasterize (e.g. a wasm build without the `fromStl`
/// export). `client` is injectable for tests; it defaults to the active
/// transport.
export async function rasterizeStlFile(
  file: File,
  maxDim = 256,
  client: WiacClient = defaultClient(),
): Promise<HeightGridResult | null> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  return client.rasterizeStl(bytes, maxDim);
}
