import { describe, it, expect } from 'vitest';
import type { ImportResponse } from '../api/types';
import type { OpEntry } from './op_types';
import type { PickMode } from './selection.svelte';
import {
  formatStockDims,
  acceptsManualTabs,
  modalStatusHint,
  statusInfoText,
  statusShortcutHints,
  composeStatusBar,
  type Translate,
} from './status-bar-text';

/// Echoing translator so assertions can see which key + params were used.
const t: Translate = (key, params) => (params ? `${key}:${JSON.stringify(params)}` : key);

// Loose input so a `drill` kind can still carry a `tabMode` in the negative
// cases without tripping the discriminated-union excess-property check.
const op = (over: Record<string, unknown>): OpEntry =>
  ({ id: 1, kind: 'profile', ...over }) as unknown as OpEntry;

describe('formatStockDims', () => {
  it('renders width × depth × thickness rounded to whole mm', () => {
    expect(formatStockDims({ minX: 0, minY: 0, maxX: 120.4, maxY: 80.6 }, 18)).toBe(
      '120 × 81 × 18 mm',
    );
  });
  it('clamps negative extents/thickness to 0', () => {
    expect(formatStockDims({ minX: 10, minY: 10, maxX: 5, maxY: 5 }, -3)).toBe('0 × 0 × 0 mm');
  });
  it('guards non-finite values to "0"', () => {
    expect(formatStockDims({ minX: 0, minY: 0, maxX: Infinity, maxY: NaN }, 5)).toBe(
      '0 × 0 × 5 mm',
    );
  });
});

describe('acceptsManualTabs', () => {
  it('is true for profile/pocket ops in manual or mixed tab mode', () => {
    expect(acceptsManualTabs(op({ kind: 'profile', tabMode: { kind: 'manual' } }))).toBe(true);
    expect(acceptsManualTabs(op({ kind: 'pocket', tabMode: { kind: 'mixed' } }))).toBe(true);
  });
  it('is false for auto tab mode, other op kinds, or no op', () => {
    expect(acceptsManualTabs(op({ kind: 'profile', tabMode: { kind: 'auto' } }))).toBe(false);
    expect(acceptsManualTabs(op({ kind: 'drill', tabMode: { kind: 'manual' } }))).toBe(false);
    expect(acceptsManualTabs(op({ kind: 'profile' }))).toBe(false);
    expect(acceptsManualTabs(null)).toBe(false);
  });
});

describe('modalStatusHint', () => {
  const pick: PickMode = { kind: 'approach-point', opId: 7 };
  it('shows the approach hint only while its own op is selected', () => {
    expect(modalStatusHint(pick, 7, false, t)).toBe('app.status.pick_approach');
    expect(modalStatusHint(pick, 3, false, t)).toBeNull();
  });
  it('shows the tab-placement hint when the op accepts manual tabs', () => {
    expect(modalStatusHint(null, 3, true, t)).toBe('app.status.tab_placement');
  });
  it('is null when no modal tool is active', () => {
    expect(modalStatusHint(null, 3, false, t)).toBeNull();
  });
  it('prefers the approach hint over the tab hint', () => {
    expect(modalStatusHint(pick, 7, true, t)).toBe('app.status.pick_approach');
  });
});

const imp = (over: Partial<ImportResponse> = {}): ImportResponse =>
  ({
    object_meta: [{ id: 5, bbox: { min_x: 0, min_y: 0, max_x: 10, max_y: 20 } }],
    bbox: { min_x: -1, min_y: -2, max_x: 3, max_y: 4 },
    segments: [{}, {}, {}],
    unit_scale: 1,
    ...over,
  }) as unknown as ImportResponse;

describe('statusInfoText', () => {
  it('is the ready message when there is no import', () => {
    expect(statusInfoText(null, new Set(), t)).toBe('app.status.ready');
  });
  it('shows the import bbox + segment count with no selection', () => {
    expect(statusInfoText(imp(), new Set(), t)).toBe(
      'bbox=(-1.00,-2.00)–(3.00,4.00) · 3 segments · unit_scale=1',
    );
  });
  it('shows the selection union bbox when objects are selected', () => {
    expect(statusInfoText(imp(), new Set([5]), t)).toBe(
      'app.status.object_one · center=(5.00, 10.00) · 10.00 × 20.00 mm',
    );
  });
  it('pluralizes and passes the count for multi-object selections', () => {
    const two = imp({
      object_meta: [
        { id: 5, bbox: { min_x: 0, min_y: 0, max_x: 10, max_y: 20 } },
        { id: 9, bbox: { min_x: -4, min_y: 2, max_x: 6, max_y: 30 } },
      ],
    } as unknown as Partial<ImportResponse>);
    expect(statusInfoText(two, new Set([5, 9]), t)).toContain('app.status.object_many:{"count":2}');
  });
  it('falls back to the import bbox when selected ids are absent from meta', () => {
    expect(statusInfoText(imp(), new Set([999]), t)).toContain('bbox=(-1.00,-2.00)');
  });
});

describe('statusShortcutHints', () => {
  it('is null without an import', () => {
    expect(statusShortcutHints(false, false, t)).toBeNull();
  });
  it('shows the selection hints when something is selected', () => {
    expect(statusShortcutHints(true, true, t)).toBe('app.status.hints_selection');
  });
  it('shows the idle hints otherwise', () => {
    expect(statusShortcutHints(true, false, t)).toBe('app.status.hints_idle');
  });
});

describe('composeStatusBar', () => {
  it('joins info and hints with a separator', () => {
    expect(composeStatusBar('info', 'hints')).toBe('info · hints');
  });
  it('is just the info line when there are no hints', () => {
    expect(composeStatusBar('info', null)).toBe('info');
  });
});
