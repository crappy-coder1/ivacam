import { describe, it, expect } from 'vitest';
import { HOLDER_PRESETS, applyPresetPatch } from './tool_presets';
import type { ToolEntry } from './project-types';

// Baseline endmill row; tests override diameter / holder fields as needed.
function tool(over: Partial<ToolEntry> = {}): ToolEntry {
  const raw: Record<string, unknown> = {
    id: 1,
    name: 'T1',
    kind: 'endmill',
    diameter: 6,
    flutes: 2,
    speed: 18000,
    plungeRate: 200,
    feedRate: 1200,
    coolant: 'off',
    ...over,
  };
  return raw as unknown as ToolEntry;
}

describe('HOLDER_PRESETS', () => {
  it('exposes the expected, stable set of preset labels', () => {
    expect(HOLDER_PRESETS.map((p) => p.label)).toEqual([
      'ER11 (≤7 mm)',
      'ER16 (≤10 mm)',
      'ER20 (≤13 mm)',
      'Direct shank',
      'No holder',
    ]);
  });
});

describe('applyPresetPatch', () => {
  it('returns null for an unknown label (stale/blank selection)', () => {
    expect(applyPresetPatch(tool(), 'ER99')).toBeNull();
    expect(applyPresetPatch(tool(), '')).toBeNull();
  });

  it('ER11 sets a cone holder + default flute/shank on a bare tool', () => {
    const patch = applyPresetPatch(tool({ diameter: 6 }), 'ER11 (≤7 mm)');
    expect(patch).toEqual({
      fluteLengthMm: 15,
      shankDiameterMm: 6, // min(diameter 6, cap 6)
      holder: { kind: 'cone', bottom_diameter_mm: 19, top_diameter_mm: 30, length_mm: 35 },
    });
  });

  it('preserves flute/shank the user already set (?? guard), still swaps the holder', () => {
    const patch = applyPresetPatch(
      tool({ diameter: 6, fluteLengthMm: 99, shankDiameterMm: 88 }),
      'ER11 (≤7 mm)',
    );
    expect(patch?.fluteLengthMm).toBe(99);
    expect(patch?.shankDiameterMm).toBe(88);
    expect(patch?.holder).toEqual({
      kind: 'cone',
      bottom_diameter_mm: 19,
      top_diameter_mm: 30,
      length_mm: 35,
    });
  });

  it('clamps the default shank to the collet cap for a large-diameter tool', () => {
    // ER16 cap is 8; a 20 mm tool clamps to 8, a 5 mm tool stays 5.
    expect(applyPresetPatch(tool({ diameter: 20 }), 'ER16 (≤10 mm)')?.shankDiameterMm).toBe(8);
    expect(applyPresetPatch(tool({ diameter: 5 }), 'ER16 (≤10 mm)')?.shankDiameterMm).toBe(5);
    // ER20 cap is 12.
    expect(applyPresetPatch(tool({ diameter: 20 }), 'ER20 (≤13 mm)')?.shankDiameterMm).toBe(12);
  });

  it('Direct shank clears the holder and matches shank to the cutting diameter', () => {
    const patch = applyPresetPatch(tool({ diameter: 10 }), 'Direct shank');
    expect(patch).toEqual({ fluteLengthMm: 15, shankDiameterMm: 10, holder: undefined });
  });

  it('No holder clears every holder field', () => {
    const patch = applyPresetPatch(tool({ diameter: 10, fluteLengthMm: 30 }), 'No holder');
    expect(patch).toEqual({
      fluteLengthMm: undefined,
      shankDiameterMm: undefined,
      holder: undefined,
    });
  });
});
