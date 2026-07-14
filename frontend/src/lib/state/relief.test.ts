import { describe, it, expect } from 'vitest';
import { reliefBrightness, isHeightgrid, reliefDisplayBrightness } from './relief';
import type { ReliefSource } from './project-types';

function source(grid: ReliefSource['grid']): ReliefSource {
  return { id: 1, name: 's', origin: { x: 0, y: 0 }, cell: 1, cols: 2, rows: 2, grid };
}

describe('relief grid helpers', () => {
  it('reliefBrightness returns the grid for grayscale, null for heightgrid', () => {
    const b = [0.1, 0.2, 0.3, 0.4];
    expect(reliefBrightness(source({ kind: 'grayscale', brightness: b }))).toBe(b);
    expect(reliefBrightness(source({ kind: 'heightgrid', z: [0, -1, -2, -3] }))).toBeNull();
  });

  it('isHeightgrid discriminates the two kinds', () => {
    expect(isHeightgrid(source({ kind: 'grayscale', brightness: [0] }))).toBe(false);
    expect(isHeightgrid(source({ kind: 'heightgrid', z: [0] }))).toBe(true);
  });

  it('reliefDisplayBrightness passes grayscale through unchanged', () => {
    const b = [0, 0.5, 1];
    expect(reliefDisplayBrightness({ kind: 'grayscale', brightness: b })).toBe(b);
  });

  it('reliefDisplayBrightness normalizes heightgrid z to [0,1] (deep→0, top→1)', () => {
    // z range [-4, 0] → 0 maps to 1 (top/bright), -4 maps to 0 (deep/dark).
    const out = reliefDisplayBrightness({ kind: 'heightgrid', z: [0, -2, -4] });
    expect(out).toEqual([1, 0.5, 0]);
  });

  it('reliefDisplayBrightness maps a flat heightgrid to all-top (1)', () => {
    expect(reliefDisplayBrightness({ kind: 'heightgrid', z: [-3, -3, -3, -3] })).toEqual([
      1, 1, 1, 1,
    ]);
  });
});
