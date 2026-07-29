// Pure helpers behind `CpsPostConfig.svelte` — kept rune-free so they
// unit-test under the logic-only vitest config (same split as
// error-display.ts).

import type { PostMeta } from '../api/types';

export type PropertyValue = boolean | number | string;

export interface PropertyControl {
  name: string;
  title: string;
  description: string;
  kind: 'bool' | 'number' | 'integer' | 'enum' | 'string';
  /// Effective value: the stored override when present, else the post's
  /// declared default.
  value: PropertyValue;
  /// Selectable values for `kind === 'enum'`.
  values?: { id: string; title: string }[];
}

/// The property-form model: one control per declared property, in the
/// post's declaration order, resolved against the sparse overrides.
export function formControls(
  meta: PostMeta,
  overrides: Record<string, PropertyValue>,
): PropertyControl[] {
  return (meta.properties ?? []).map((property) => {
    // The generated type models `kind` as a tagged union; normalize to
    // the flat shape the template switches on.
    const kindTag = (property.kind as { type?: string }).type ?? 'string';
    const values = (property.kind as { values?: { id: string; title: string }[] }).values;
    const stored = overrides[property.name];
    return {
      name: property.name,
      title: property.title || property.name,
      description: property.description ?? '',
      kind: kindTag as PropertyControl['kind'],
      value: stored ?? (property.default as PropertyValue),
      values,
    };
  });
}

/// Drop every entry that equals the post's declared default, so what we
/// persist is only the user's genuine deviations (a post changing its
/// defaults then takes effect instead of being masked by stale values).
export function sparseProperties(
  meta: PostMeta,
  candidate: Record<string, PropertyValue>,
): Record<string, PropertyValue> {
  const defaults = new Map<string, PropertyValue>(
    (meta.properties ?? []).map((p) => [p.name, p.default as PropertyValue]),
  );
  const out: Record<string, PropertyValue> = {};
  for (const [name, value] of Object.entries(candidate)) {
    // Unknown names (a post no longer declaring a property) drop out
    // with the equal-to-default ones.
    if (!defaults.has(name)) continue;
    if (defaults.get(name) !== value) out[name] = value;
  }
  return out;
}

/// Stable identity of a post selection, used by GenerateBar to
/// invalidate cached gcode when the post OR any property changes.
export function cpsSelectionKey(
  config:
    | {
        source: 'bundled' | 'file';
        bundledId?: string;
        filename?: string;
        script?: string;
        properties: Record<string, PropertyValue>;
      }
    | undefined,
): string {
  if (!config) return '';
  const props = Object.keys(config.properties)
    .sort()
    .map((k) => `${k}=${String(config.properties[k])}`)
    .join(',');
  const source =
    config.source === 'bundled'
      ? `bundled:${config.bundledId ?? ''}`
      : // Hash the script so editing the file on disk and re-opening it
        // invalidates too (filename alone would not).
        `file:${config.filename ?? ''}:${cheapHash(config.script ?? '')}`;
  return `${source}|${props}`;
}

/// FNV-1a — enough to detect a changed script; not a security hash.
function cheapHash(text: string): string {
  let hash = 0x811c_9dc5;
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 0x0100_0193) >>> 0;
  }
  return hash.toString(16);
}

/// Wire form of a machine's post selection for `GenerateRequest.cps_post`.
export function toCpsPostRequest(
  config:
    | {
        source: 'bundled' | 'file';
        bundledId?: string;
        filename?: string;
        script?: string;
        properties: Record<string, PropertyValue>;
      }
    | undefined,
): { source: Record<string, unknown>; properties: Record<string, PropertyValue> } | undefined {
  if (!config) return undefined;
  if (config.source === 'bundled') {
    if (!config.bundledId) return undefined;
    return {
      source: { kind: 'bundled', id: config.bundledId },
      properties: config.properties,
    };
  }
  if (!config.script) return undefined;
  return {
    source: { kind: 'inline', script: config.script, filename: config.filename },
    properties: config.properties,
  };
}
