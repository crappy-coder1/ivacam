import { describe, expect, it } from 'vitest';
import type { PipelineWarning } from '../api/types';
import en from '../i18n/messages/en.json';
import { hasWarningTemplate, warningMessage, type Translate } from './warning-display';

/// Params the pipeline attaches to `warn.<kind>` warnings via `.with_param`
/// (crates/ivac-core/src/pipeline/**). A template may only interpolate these —
/// referencing anything else would render a literal `{token}` at runtime
/// because the Rust side never supplies it. Extend this set (and the matching
/// `.with_param` call) when a new warning param is introduced.
const KNOWN_WARN_PARAMS = new Set(['op_name', 'tool_name', 'diameter']);

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
