import { describe, expect, it } from 'vitest';
import type { PipelineWarning } from '../api/types';
import en from '../i18n/messages/en.json';
import { hasWarningTemplate, warningMessage, type Translate } from './warning-display';

/// Params the pipeline attaches to `warn.<kind>` warnings via `.with_param`
/// (crates/ivac-core/src/pipeline/**). A template may only interpolate these —
/// referencing anything else would render a literal `{token}` at runtime
/// because the Rust side never supplies it. Extend this set (and the matching
/// `.with_param` call) when a new warning param is introduced.
const KNOWN_WARN_PARAMS = new Set([
  'allowance',
  'allowed',
  'approach_x',
  'approach_y',
  'axis',
  'bbox_max_x',
  'bbox_max_y',
  'bbox_min_x',
  'bbox_min_y',
  'bore_radius',
  'carve_width',
  'changes',
  'clamped',
  'cols',
  'count',
  'cut_depth',
  'delta',
  'diameter',
  'digest',
  'distance',
  'effective_width',
  'engagement',
  'feed',
  'first_line',
  'first_reason',
  'first_text',
  'first_tool',
  'flute_length',
  'kind_name',
  'layer',
  'limit',
  'line',
  'loop_radius',
  'max_pixels',
  'min_gap',
  'mode',
  'n_skipped',
  'n_total',
  'neck_mm',
  'object_id',
  'object_index',
  'op_name',
  'orbit',
  'other_diameter',
  'other_name',
  'padding',
  'pass',
  'perimeter',
  'pitch',
  'placed_count',
  'plunge',
  'profile_radius',
  'radius',
  'reach',
  'reason',
  'reference_tool',
  'requested_count',
  'ring_cap',
  'rings',
  'rows',
  'rpm',
  'source_id',
  'speed',
  'spot_depth',
  'spot_tool_id',
  'step',
  'stride',
  'tab_width',
  'text',
  'threshold',
  'tip',
  'tip_angle',
  'tip_radius',
  'tolerance_mm',
  'tolerance_pct',
  'tool_kind',
  'tool_name',
  'tool_radius',
  'transition',
  'variable',
  'variant',
  'vertex_index',
  'vertex_total',
  'wcs_x',
  'wcs_y',
  'width',
  'width_cap',
  'z_range',
]);

// A stub translate that echoes the key + params so we can assert which path ran.
const t: Translate = (key, params) => `T:${key}:${JSON.stringify(params ?? {})}`;

describe('warningMessage', () => {
  it('falls back to the English message for a kind with no warn.<kind> template', () => {
    const w = {
      kind: 'definitely_not_a_real_warning_kind',
      message: 'English fallback text',
    } as PipelineWarning;
    expect(hasWarningTemplate(w.kind)).toBe(false);
    expect(warningMessage(w, t)).toBe('English fallback text');
  });

  it('tolerates a warning with no params object', () => {
    const w = { kind: 'still_not_real', message: 'msg only' } as PipelineWarning;
    expect(warningMessage(w, t)).toBe('msg only');
  });

  it('renders warn.<kind> through the translator when a template exists', () => {
    // thread_no_depth ships a warn.* template (Commit C), so the localized
    // path runs — the English message is ignored in favor of the template.
    const w = {
      kind: 'thread_no_depth',
      message: 'English message that must NOT be shown',
      params: { op_name: 'Contour 1' },
    } as PipelineWarning;
    expect(hasWarningTemplate(w.kind)).toBe(true);
    // The stub `t` echoes key + params so we can prove the localized path ran.
    expect(warningMessage(w, t)).toBe('T:warn.thread_no_depth:{"op_name":"Contour 1"}');
  });

  it('selects warn.<kind>.<variant> when a variant discriminator is set', () => {
    // tool_kind_mismatch ships per-reason variant templates (os2k.13); the
    // `variant` param picks warn.tool_kind_mismatch.pocket_drill over the base.
    const w = {
      kind: 'tool_kind_mismatch',
      message: 'English fallback that must NOT be shown',
      params: { op_name: 'Pocket 1', tool_name: 'D3', variant: 'pocket_drill' },
    } as PipelineWarning;
    expect(warningMessage(w, t)).toBe(
      'T:warn.tool_kind_mismatch.pocket_drill:{"op_name":"Pocket 1","tool_name":"D3","variant":"pocket_drill"}',
    );
  });

  it('falls back to the base warn.<kind> when the variant key is unknown/empty', () => {
    // out_of_work_area has a base template; an empty variant selects it (the
    // no-gcode-line case), never a bogus warn.out_of_work_area. key.
    const w = {
      kind: 'out_of_work_area',
      message: 'English fallback',
      params: { count: '2', variant: '' },
    } as PipelineWarning;
    expect(warningMessage(w, t)).toBe('T:warn.out_of_work_area:{"count":"2","variant":""}');
  });
});

describe('warn.<kind> template ↔ pipeline param drift', () => {
  it('every warn.* template only interpolates params the pipeline provides', () => {
    for (const [key, value] of Object.entries(en as Record<string, string>)) {
      if (!key.startsWith('warn.')) continue;
      for (const m of value.matchAll(/\{(\w+)\}/g)) {
        expect(
          KNOWN_WARN_PARAMS.has(m[1]),
          `${key} references {${m[1]}} which no .with_param() sets`,
        ).toBe(true);
      }
    }
  });
});
