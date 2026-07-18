import { describe, it, expect } from 'vitest';
import {
  SIDEBAR_DEFAULT,
  clampSidebar,
  clampGcode,
  defaultGcodeHeight,
  reclampPanels,
} from './workspace-layout';

describe('clampSidebar', () => {
  it('passes a comfortable value through on a wide viewport', () => {
    expect(clampSidebar(SIDEBAR_DEFAULT, 1920)).toBe(360);
  });
  it('enforces the 240 px floor', () => {
    expect(clampSidebar(100, 1920)).toBe(240);
  });
  it('caps the ceiling at 720 px on a very wide viewport', () => {
    expect(clampSidebar(1000, 3000)).toBe(720);
  });
  it('tracks a 60% ceiling on a narrower viewport', () => {
    // ceiling = min(720, round(800 * 0.6)) = 480
    expect(clampSidebar(600, 800)).toBe(480);
  });
  it('keeps the 240 floor even when 60% of the viewport is below it', () => {
    // ceiling would be 180 but the floor wins → 240
    expect(clampSidebar(300, 300)).toBe(240);
  });
});

describe('clampGcode', () => {
  it('passes a mid value through', () => {
    expect(clampGcode(300, 1000)).toBe(300);
  });
  it('enforces the 120 px floor', () => {
    expect(clampGcode(50, 1000)).toBe(120);
  });
  it('caps at 70% of the viewport height', () => {
    expect(clampGcode(900, 1000)).toBe(700);
  });
});

describe('defaultGcodeHeight', () => {
  it('is ~35% of the viewport, rounded', () => {
    expect(defaultGcodeHeight(1000)).toBe(350);
    expect(defaultGcodeHeight(773)).toBe(271);
  });
});

describe('reclampPanels', () => {
  it('reports no change when both panels already fit', () => {
    const r = reclampPanels({ sidebarWidth: 360, gcodeHeight: 350 }, { width: 1920, height: 1000 });
    expect(r).toEqual({ sidebarWidth: 360, gcodeHeight: 350, changed: false });
  });
  it('clamps both and flags the change when the viewport shrinks', () => {
    const r = reclampPanels(
      { sidebarWidth: 1000, gcodeHeight: 900 },
      { width: 1920, height: 1000 },
    );
    expect(r).toEqual({ sidebarWidth: 720, gcodeHeight: 700, changed: true });
  });
  it('flags a change when only one panel needs clamping', () => {
    const r = reclampPanels({ sidebarWidth: 360, gcodeHeight: 900 }, { width: 1920, height: 1000 });
    expect(r.changed).toBe(true);
    expect(r.sidebarWidth).toBe(360);
    expect(r.gcodeHeight).toBe(700);
  });
});
