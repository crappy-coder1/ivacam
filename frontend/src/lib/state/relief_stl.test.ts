/// `rasterizeStlFile` reads the picked File into bytes and hands them to
/// the active transport's `rasterizeStl` — it must NOT touch wasm directly
/// (the whole point of ivac-fm06). These tests inject a fake client so the
/// delegation + byte round-trip is pinned without a real transport.

import { describe, expect, it, vi } from 'vitest';
import { rasterizeStlFile } from './relief_stl';
import type { WiacClient } from '../api/client';
import type { SurfaceField } from '../api/types';

function fakeClient(over: Partial<WiacClient>): WiacClient {
  // Only rasterizeStl is exercised; the rest throw if unexpectedly called.
  return new Proxy(over as WiacClient, {
    get(target, prop, recv) {
      if (prop in target) return Reflect.get(target, prop, recv);
      return () => {
        throw new Error(`unexpected client call: ${String(prop)}`);
      };
    },
  });
}

const FIELD: SurfaceField = {
  origin: { x: 0, y: 0 },
  cell: 1,
  cols: 1,
  rows: 1,
  z: [-1],
};

describe('rasterizeStlFile', () => {
  it('reads the file bytes and delegates to client.rasterizeStl with maxDim', async () => {
    const rasterizeStl = vi.fn().mockResolvedValue(FIELD);
    const file = new File([new Uint8Array([1, 2, 3, 4])], 'part.stl');
    const got = await rasterizeStlFile(file, 128, fakeClient({ rasterizeStl }));

    expect(got).toEqual(FIELD);
    expect(rasterizeStl).toHaveBeenCalledTimes(1);
    const [bytes, maxDim] = rasterizeStl.mock.calls[0] as [Uint8Array, number];
    expect(Array.from(bytes)).toEqual([1, 2, 3, 4]);
    expect(maxDim).toBe(128);
  });

  it('propagates a null result (mesh has no XY footprint)', async () => {
    const rasterizeStl = vi.fn().mockResolvedValue(null);
    const file = new File([new Uint8Array([9])], 'vertical.stl');
    const got = await rasterizeStlFile(file, 256, fakeClient({ rasterizeStl }));
    expect(got).toBeNull();
  });

  it('defaults maxDim to 256', async () => {
    const rasterizeStl = vi.fn().mockResolvedValue(FIELD);
    const file = new File([new Uint8Array([1])], 'part.stl');
    await rasterizeStlFile(file, undefined, fakeClient({ rasterizeStl }));
    expect(rasterizeStl.mock.calls[0][1]).toBe(256);
  });
});
