import { describe, it, expect } from 'vitest';
import { resolveExportExtension, applyProfileLineEnding } from './gcode-export';

describe('resolveExportExtension', () => {
  it('prefers the extension the post declared', () => {
    expect(resolveExportExtension('cps', 'nc', 'tap')).toBe('nc');
    expect(resolveExportExtension('linuxcnc', 'nc')).toBe('nc');
  });

  it('falls back to the machine PostProfile extension (the fixed bug)', () => {
    expect(resolveExportExtension('linuxcnc', undefined, 'tap')).toBe('tap');
    expect(resolveExportExtension('grbl', undefined, 'gcode')).toBe('gcode');
    // Leading dots and padding in the profile field are tolerated.
    expect(resolveExportExtension('linuxcnc', undefined, ' .tap ')).toBe('tap');
  });

  it('falls back to the per-dialect default', () => {
    expect(resolveExportExtension('linuxcnc')).toBe('ngc');
    expect(resolveExportExtension('grbl')).toBe('ngc');
    expect(resolveExportExtension('hpgl')).toBe('plt');
  });

  it('ignores empty declarations at every rung', () => {
    expect(resolveExportExtension('hpgl', '', '   ')).toBe('plt');
    expect(resolveExportExtension('cps', '.', '')).toBe('ngc');
  });
});

describe('applyProfileLineEnding', () => {
  const body = 'G21\nG90\nM30\n';

  it('leaves the buffer alone with no profile setting or LF', () => {
    expect(applyProfileLineEnding(body)).toBe(body);
    expect(applyProfileLineEnding(body, 'lf')).toBe(body);
    expect(applyProfileLineEnding(body, '\n')).toBe(body);
  });

  it('rewrites to CRLF for both spellings', () => {
    expect(applyProfileLineEnding(body, 'crlf')).toBe('G21\r\nG90\r\nM30\r\n');
    expect(applyProfileLineEnding(body, '\r\n')).toBe('G21\r\nG90\r\nM30\r\n');
    expect(applyProfileLineEnding(body, 'CRLF')).toBe('G21\r\nG90\r\nM30\r\n');
  });

  it('is idempotent on already-CRLF text', () => {
    const crlf = applyProfileLineEnding(body, 'crlf');
    expect(applyProfileLineEnding(crlf, 'crlf')).toBe(crlf);
  });

  it('supports bare CR for vintage controllers', () => {
    expect(applyProfileLineEnding(body, 'cr')).toBe('G21\rG90\rM30\r');
  });
});
