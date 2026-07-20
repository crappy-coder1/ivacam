/// Holder-shape presets for the tool library (ivac-bzpt). Extracted from
/// ToolLibraryDialog.svelte so the preset table and the "which fields does a
/// preset touch" logic is pure and unit-testable instead of stuck inline in
/// the dialog script. Each preset resolves to a PARTIAL patch merged onto the
/// tool; the `?? current` guards mean a preset never clobbers a value the user
/// already set — except where clearing IS the intent (`holder: undefined`).
///
/// Picking a preset is the fastest way to populate the holder spec; the user
/// can always edit the individual fields afterwards.

import type { ToolEntry } from './project-types';

export type HolderPreset = {
  /// Menu label shown in the preset dropdown — also the lookup key.
  label: string;
  /// The patch this preset applies to a tool, computed from the tool's
  /// current values so already-populated fields survive.
  apply: (t: ToolEntry) => Partial<ToolEntry>;
};

/// Common ER-collet stacks plus direct-shank / no-holder. ER presets are the
/// bounding cone of nut + spindle for the named collet size; lengths are total
/// stick-out from the spindle face. Conservative — real hardware varies a few
/// mm across vendors.
///
///  - ER11 / ER16 / ER20: cone holder sized to the collet, shank clamped to
///    the collet's max grip (min of the cutting diameter and the collet cap).
///  - Direct shank: no holder above the shank — just sets the shank diameter
///    to the cutting diameter and clears the holder.
///  - No holder: clears every holder field, restoring legacy behavior.
export const HOLDER_PRESETS: readonly HolderPreset[] = [
  {
    label: 'ER11 (≤7 mm)',
    apply: (t) => ({
      fluteLengthMm: t.fluteLengthMm ?? 15,
      shankDiameterMm: t.shankDiameterMm ?? Math.min(t.diameter, 6),
      holder: {
        kind: 'cone',
        bottom_diameter_mm: 19,
        top_diameter_mm: 30,
        length_mm: 35,
      },
    }),
  },
  {
    label: 'ER16 (≤10 mm)',
    apply: (t) => ({
      fluteLengthMm: t.fluteLengthMm ?? 20,
      shankDiameterMm: t.shankDiameterMm ?? Math.min(t.diameter, 8),
      holder: {
        kind: 'cone',
        bottom_diameter_mm: 28,
        top_diameter_mm: 42,
        length_mm: 45,
      },
    }),
  },
  {
    label: 'ER20 (≤13 mm)',
    apply: (t) => ({
      fluteLengthMm: t.fluteLengthMm ?? 25,
      shankDiameterMm: t.shankDiameterMm ?? Math.min(t.diameter, 12),
      holder: {
        kind: 'cone',
        bottom_diameter_mm: 34,
        top_diameter_mm: 50,
        length_mm: 50,
      },
    }),
  },
  {
    label: 'Direct shank',
    apply: (t) => ({
      fluteLengthMm: t.fluteLengthMm ?? 15,
      shankDiameterMm: t.shankDiameterMm ?? t.diameter,
      holder: undefined,
    }),
  },
  {
    label: 'No holder',
    apply: () => ({
      fluteLengthMm: undefined,
      shankDiameterMm: undefined,
      holder: undefined,
    }),
  },
];

/// Resolve a preset label to the patch it would apply to `tool`, or null when
/// the label is unknown (a stale or blank dropdown selection).
export function applyPresetPatch(tool: ToolEntry, label: string): Partial<ToolEntry> | null {
  const p = HOLDER_PRESETS.find((x) => x.label === label);
  return p ? p.apply(tool) : null;
}
