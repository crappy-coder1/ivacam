import { describe, expect, it } from 'vitest';
import { buildOpEntry, type OpDefaultsCtx } from '../state/op_defaults';
import type { OpEntry, ReliefMillOp } from '../state/op_types';
import type { ReliefSource } from '../state/project-types';
import { deviationTargets, reliefTargetSurface } from './deviation_target';

// A valid relief_mill op via the shared factory, with per-test overrides.
function reliefOp(overrides: Partial<ReliefMillOp> = {}): ReliefMillOp {
  const ctx: OpDefaultsCtx = {
    nextId: 1,
    tools: [{ id: 1, kind: 'ball_nose' } as unknown as OpDefaultsCtx['tools'][number]],
    reliefSources: [],
    selectionIds: [],
    objectMeta: [],
  };
  return { ...(buildOpEntry('relief_mill', ctx) as ReliefMillOp), ...overrides };
}

function grayscaleSource(id: number, brightness: number[]): ReliefSource {
  return {
    id,
    name: 'gs',
    origin: { x: 0, y: 0 },
    cell: 1,
    cols: brightness.length,
    rows: 1,
    grid: { kind: 'grayscale', brightness },
  };
}

function heightgridSource(id: number, z: number[]): ReliefSource {
  return {
    id,
    name: 'hg',
    origin: { x: 3, y: 4 },
    cell: 2,
    cols: z.length,
    rows: 1,
    grid: { kind: 'heightgrid', z },
  };
}

describe('reliefTargetSurface', () => {
  it('remaps grayscale brightness across [zMin, zMax] (bright = high)', () => {
    const op = reliefOp({ sourceId: 1, zMinMm: -5, zMaxMm: 0, invert: false });
    const surf = reliefTargetSurface(op, [grayscaleSource(1, [0, 0.5, 1])]);
    expect(surf).not.toBeNull();
    // dark (0) → deepest (-5); mid (0.5) → -2.5; bright (1) → top (0).
    expect(surf!.z).toEqual([-5, -2.5, 0]);
    expect(surf!.cols).toBe(3);
    expect(surf!.origin).toEqual({ x: 0, y: 0 });
  });

  it('inverts the grayscale mapping when op.invert is set', () => {
    const op = reliefOp({ sourceId: 1, zMinMm: -4, zMaxMm: 0, invert: true });
    const surf = reliefTargetSurface(op, [grayscaleSource(1, [0, 1])]);
    // invert: dark → top (0), bright → deep (-4).
    expect(surf!.z).toEqual([0, -4]);
  });

  it('tolerates a flipped [zMin, zMax] span (lo is always deepest)', () => {
    const op = reliefOp({ sourceId: 1, zMinMm: 0, zMaxMm: -4, invert: false });
    const surf = reliefTargetSurface(op, [grayscaleSource(1, [0, 1])]);
    expect(surf!.z).toEqual([-4, 0]);
  });

  it('clamps out-of-range brightness before remapping', () => {
    const op = reliefOp({ sourceId: 1, zMinMm: -2, zMaxMm: 0, invert: false });
    const surf = reliefTargetSurface(op, [grayscaleSource(1, [-0.5, 1.5])]);
    expect(surf!.z).toEqual([-2, 0]);
  });

  it('uses a heightgrid Z directly and carries its placement', () => {
    const op = reliefOp({ sourceId: 2, zMinMm: -10, zMaxMm: 0 });
    const surf = reliefTargetSurface(op, [heightgridSource(2, [0, -1, -3])]);
    // Heightgrid Z is not remapped by the op's z range.
    expect(surf!.z).toEqual([0, -1, -3]);
    expect(surf!.cell).toBe(2);
    expect(surf!.origin).toEqual({ x: 3, y: 4 });
  });

  it('returns null when the op references a missing source', () => {
    const op = reliefOp({ sourceId: 99 });
    expect(reliefTargetSurface(op, [grayscaleSource(1, [0.5])])).toBeNull();
  });
});

describe('deviationTargets', () => {
  it('collects every enabled relief_mill op with a resolvable source, in order', () => {
    const disabled = reliefOp({ sourceId: 1, enabled: false });
    const first = reliefOp({ sourceId: 2, enabled: true });
    const second = reliefOp({ sourceId: 3, enabled: true });
    const ops: OpEntry[] = [disabled, first, second];
    const targets = deviationTargets(ops, [
      heightgridSource(1, [-9]),
      heightgridSource(2, [-1, -2]),
      heightgridSource(3, [-5]),
    ]);
    // Disabled op excluded; the two enabled ones kept in document order.
    expect(targets.map((t) => t.z)).toEqual([
      [-1, -2],
      [-5],
    ]);
  });

  it('skips a relief op whose source is missing but keeps the resolvable ones', () => {
    const orphan = reliefOp({ sourceId: 1, enabled: true });
    const good = reliefOp({ sourceId: 2, enabled: true });
    const targets = deviationTargets([orphan, good], [heightgridSource(2, [-5])]);
    expect(targets.map((t) => t.z)).toEqual([[-5]]);
  });

  it('returns [] when there is no relief op at all', () => {
    const profile = buildOpEntry('profile', {
      nextId: 1,
      tools: [{ id: 1, kind: 'end_mill' } as unknown as OpDefaultsCtx['tools'][number]],
      reliefSources: [],
      selectionIds: [],
      objectMeta: [],
    });
    expect(deviationTargets([profile], [heightgridSource(1, [-1])])).toEqual([]);
  });
});
