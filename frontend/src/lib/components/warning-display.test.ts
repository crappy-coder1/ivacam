import { describe, expect, it } from 'vitest';
import type { PipelineWarning } from '../api/types';
import { hasWarningTemplate, warningMessage, type Translate } from './warning-display';

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
});
