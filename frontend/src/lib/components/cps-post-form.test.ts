import { describe, it, expect } from 'vitest';
import { formControls, sparseProperties, cpsSelectionKey, toCpsPostRequest } from './cps-post-form';
import type { PostMeta } from '../api/types';

const meta = {
  description: 'GRBL (ivaCAM)',
  vendor: 'ivaCAM',
  extension: 'gcode',
  capabilities: 1,
  properties: [
    {
      name: 'pauseOnToolChange',
      title: 'Pause on tool change',
      description: 'Emit M0.',
      kind: { type: 'bool' },
      default: true,
    },
    {
      name: 'spindleWarmupSeconds',
      title: 'Spindle warm-up (s)',
      description: '',
      kind: { type: 'number' },
      default: 0,
    },
    {
      name: 'safeMode',
      title: 'Safe retracts',
      description: '',
      kind: {
        type: 'enum',
        values: [
          { id: 'G28', title: 'G28' },
          { id: 'G53', title: 'G53' },
        ],
      },
      default: 'G28',
    },
  ],
} as unknown as PostMeta;

describe('formControls', () => {
  it('builds one control per declared property in order, using defaults', () => {
    const controls = formControls(meta, {});
    expect(controls.map((c) => c.name)).toEqual([
      'pauseOnToolChange',
      'spindleWarmupSeconds',
      'safeMode',
    ]);
    expect(controls[0]).toMatchObject({ kind: 'bool', value: true });
    expect(controls[1]).toMatchObject({ kind: 'number', value: 0 });
    expect(controls[2].kind).toBe('enum');
    expect(controls[2].values?.map((v) => v.id)).toEqual(['G28', 'G53']);
  });

  it('overrides shadow the declared defaults', () => {
    const controls = formControls(meta, { pauseOnToolChange: false, safeMode: 'G53' });
    expect(controls[0].value).toBe(false);
    expect(controls[1].value).toBe(0); // untouched → default
    expect(controls[2].value).toBe('G53');
  });

  it('falls back to the property name when no title is declared', () => {
    const bare = {
      properties: [
        { name: 'raw', title: '', description: '', kind: { type: 'string' }, default: '' },
      ],
    } as unknown as PostMeta;
    expect(formControls(bare, {})[0].title).toBe('raw');
  });
});

describe('sparseProperties', () => {
  it('keeps only genuine deviations from the post defaults', () => {
    const out = sparseProperties(meta, {
      pauseOnToolChange: true, // == default → dropped
      spindleWarmupSeconds: 2, // != default → kept
      safeMode: 'G28', // == default → dropped
    });
    expect(out).toEqual({ spindleWarmupSeconds: 2 });
  });

  it('drops values for properties the post no longer declares', () => {
    const out = sparseProperties(meta, { goneAway: 'stale', spindleWarmupSeconds: 1 });
    expect(out).toEqual({ spindleWarmupSeconds: 1 });
  });
});

describe('cpsSelectionKey', () => {
  it('is empty for no selection', () => {
    expect(cpsSelectionKey(undefined)).toBe('');
  });

  it('changes when the bundled post changes', () => {
    const a = cpsSelectionKey({ source: 'bundled', bundledId: 'grbl', properties: {} });
    const b = cpsSelectionKey({ source: 'bundled', bundledId: 'fanuc', properties: {} });
    expect(a).not.toBe(b);
  });

  it('changes when a property value changes, and is order-independent', () => {
    const base = { source: 'bundled' as const, bundledId: 'grbl', properties: { a: 1, b: 2 } };
    const reordered = { source: 'bundled' as const, bundledId: 'grbl', properties: { b: 2, a: 1 } };
    const changed = { source: 'bundled' as const, bundledId: 'grbl', properties: { a: 1, b: 3 } };
    expect(cpsSelectionKey(base)).toBe(cpsSelectionKey(reordered));
    expect(cpsSelectionKey(base)).not.toBe(cpsSelectionKey(changed));
  });

  it('changes when a file post is re-opened with edited content', () => {
    const before = cpsSelectionKey({
      source: 'file',
      filename: 'mine.cps',
      script: 'function onOpen() {}',
      properties: {},
    });
    const after = cpsSelectionKey({
      source: 'file',
      filename: 'mine.cps',
      script: 'function onOpen() { writeln("%"); }',
      properties: {},
    });
    expect(before).not.toBe(after);
  });
});

describe('toCpsPostRequest', () => {
  it('maps a bundled selection to the wire shape', () => {
    expect(
      toCpsPostRequest({ source: 'bundled', bundledId: 'grbl', properties: { useM30: false } }),
    ).toEqual({
      source: { kind: 'bundled', id: 'grbl' },
      properties: { useM30: false },
    });
  });

  it('maps a file selection to an inline script', () => {
    expect(
      toCpsPostRequest({
        source: 'file',
        filename: 'mine.cps',
        script: 'function onOpen() {}',
        properties: {},
      }),
    ).toEqual({
      source: { kind: 'inline', script: 'function onOpen() {}', filename: 'mine.cps' },
      properties: {},
    });
  });

  it('returns undefined for an incomplete selection (nothing to generate with)', () => {
    expect(toCpsPostRequest(undefined)).toBeUndefined();
    expect(toCpsPostRequest({ source: 'bundled', properties: {} })).toBeUndefined();
    expect(toCpsPostRequest({ source: 'file', filename: 'x.cps', properties: {} })).toBeUndefined();
  });
});
