import { describe, it, expect, vi } from 'vitest';
import { reduceContextMenuOpen, type CtxOpenEnv } from './context-menu';

/// A selection-present environment with no tab under the cursor and an
/// identity-ish transform — the plain "open the op menu" case.
function baseEnv(over: Partial<CtxOpenEnv> = {}): CtxOpenEnv {
  return {
    tabHit: null,
    hasTextSelected: false,
    hasObjsSelected: true,
    consumeSelectHint: () => true,
    transform: { scale: 1, offX: 0, offY: 0 },
    ...over,
  };
}

describe('reduceContextMenuOpen — priority', () => {
  it('a tab under the cursor opens the popover, outranking any selection', () => {
    const r = reduceContextMenuOpen(30, 40, baseEnv({ tabHit: { opId: 7, placementIdx: 2 } }));
    expect(r).toEqual({ kind: 'tab', tabPopover: { x: 30, y: 40, opId: 7, placementIdx: 2 } });
  });

  it('the tab popover wins even when nothing is selected (no hint burned)', () => {
    const spy = vi.fn(() => true);
    const r = reduceContextMenuOpen(5, 6, {
      tabHit: { opId: 1, placementIdx: 0 },
      hasTextSelected: false,
      hasObjsSelected: false,
      consumeSelectHint: spy,
      transform: null,
    });
    expect(r.kind).toBe('tab');
    expect(spy).not.toHaveBeenCalled();
  });

  it('objects selected → op menu, without consuming the hint', () => {
    const spy = vi.fn(() => true);
    const r = reduceContextMenuOpen(
      0,
      0,
      baseEnv({ hasObjsSelected: true, consumeSelectHint: spy }),
    );
    expect(r.kind).toBe('menu');
    expect(spy).not.toHaveBeenCalled();
  });

  it('text selected → op menu, without consuming the hint', () => {
    const spy = vi.fn(() => true);
    const r = reduceContextMenuOpen(
      0,
      0,
      baseEnv({ hasTextSelected: true, hasObjsSelected: false, consumeSelectHint: spy }),
    );
    expect(r.kind).toBe('menu');
    expect(spy).not.toHaveBeenCalled();
  });
});

describe('reduceContextMenuOpen — empty right-click hint', () => {
  it('empty selection with the hint still owed → menu (and the hint is spent)', () => {
    const spy = vi.fn(() => true);
    const r = reduceContextMenuOpen(
      0,
      0,
      baseEnv({ hasObjsSelected: false, consumeSelectHint: spy }),
    );
    expect(r.kind).toBe('menu');
    expect(spy).toHaveBeenCalledTimes(1);
  });

  it('empty selection with the hint already spent → nothing opens', () => {
    const spy = vi.fn(() => false);
    const r = reduceContextMenuOpen(
      0,
      0,
      baseEnv({ hasObjsSelected: false, consumeSelectHint: spy }),
    );
    expect(r).toEqual({ kind: 'none' });
    expect(spy).toHaveBeenCalledTimes(1);
  });
});

describe('reduceContextMenuOpen — data-space projection', () => {
  it('projects the cursor pixel to data mm (y flipped) using the transform', () => {
    // scale 2, offX 100, offY 50: dataX=(cx-100)/2, dataY=(50-cy)/2.
    const r = reduceContextMenuOpen(
      140,
      10,
      baseEnv({ transform: { scale: 2, offX: 100, offY: 50 } }),
    );
    expect(r).toEqual({ kind: 'menu', ctxMenu: { x: 140, y: 10, dataX: 20, dataY: 20 } });
  });

  it('falls back to data (0,0) before the first draw (null transform)', () => {
    const r = reduceContextMenuOpen(140, 10, baseEnv({ transform: null }));
    expect(r).toEqual({ kind: 'menu', ctxMenu: { x: 140, y: 10, dataX: 0, dataY: 0 } });
  });
});
