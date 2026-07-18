/// Pure status-bar + summary string builders lifted out of App.svelte
/// (ivac-3xwn.1). Rune-free so they unit-test under the logic-only vitest
/// config; the component keeps thin `$derived` wrappers that pass plain
/// snapshots + the `t` translator, mirroring warning-display.ts /
/// error-display.ts.
///
/// The status bar renders three layers (see App.svelte):
///   1. `modalStatusHint` — when a modal click-tool is active (approach
///      pick, tab placement), its instructions take precedence.
///   2. `statusInfoText` — idle context: selection union bbox, else the
///      import extent + segment count, else the "Ready" message.
///   3. `statusShortcutHints` — trailing shortcut reminder for the state.
import type { ImportResponse } from '../api/types';
import type { MsgKey } from '../i18n/keys';
import type { OpEntry } from './op_types';
import type { PickMode } from './selection.svelte';

export type Translate = (key: MsgKey, params?: Record<string, string | number>) => string;

/// Stock summary chip: "X × Y × Z mm" from a footprint + thickness. `f`
/// rounds to whole millimetres and guards non-finite values to "0".
export function formatStockDims(
  fp: { minX: number; minY: number; maxX: number; maxY: number },
  thickness: number,
): string {
  const x = Math.max(0, fp.maxX - fp.minX);
  const y = Math.max(0, fp.maxY - fp.minY);
  const z = Math.max(0, thickness);
  const f = (n: number) => (Number.isFinite(n) ? n.toFixed(0) : '0');
  return `${f(x)} × ${f(y)} × ${f(z)} mm`;
}

/// Does the selected op accept manual tab placement? Drives the tab-place
/// modal hint — only profile/pocket ops in manual or mixed tab mode.
export function acceptsManualTabs(op: OpEntry | null): boolean {
  return (
    !!op &&
    (op.kind === 'profile' || op.kind === 'pocket') &&
    (op.tabMode?.kind === 'manual' || op.tabMode?.kind === 'mixed')
  );
}

/// Modal click-tool instructions; null when no modal tool is active. The
/// approach-point hint only shows while its own op is the current selection.
export function modalStatusHint(
  pickMode: PickMode | null,
  selectedOpId: number | null,
  acceptsTabs: boolean,
  t: Translate,
): string | null {
  if (pickMode?.kind === 'approach-point' && pickMode.opId === selectedOpId) {
    return t('app.status.pick_approach');
  }
  if (acceptsTabs) return t('app.status.tab_placement');
  return null;
}

/// Idle context line. With a non-empty object selection, shows the union
/// bbox of the selected objects as (center · W × H); otherwise the import's
/// own bbox + segment count; with no import at all, the "Ready" message.
export function statusInfoText(
  imp: ImportResponse | null,
  selectedObjects: ReadonlySet<number>,
  t: Translate,
): string {
  if (!imp) return t('app.status.ready');
  const meta = imp.object_meta ?? [];
  if (selectedObjects.size > 0 && meta.length > 0) {
    // Object ids are NOT a dense 1-based index into `meta` — combineImports
    // namespaces later drawings' ids by an offset, so `meta[id - 1]` reads
    // the wrong entry once a second drawing is added. Resolve by id.
    const byId = new Map<number, (typeof meta)[number]>();
    for (const m of meta) byId.set(m.id, m);
    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    let counted = 0;
    for (const id of selectedObjects) {
      const m = byId.get(id);
      if (!m) continue;
      if (m.bbox.min_x < minX) minX = m.bbox.min_x;
      if (m.bbox.min_y < minY) minY = m.bbox.min_y;
      if (m.bbox.max_x > maxX) maxX = m.bbox.max_x;
      if (m.bbox.max_y > maxY) maxY = m.bbox.max_y;
      counted += 1;
    }
    if (counted > 0) {
      const cx = (minX + maxX) * 0.5;
      const cy = (minY + maxY) * 0.5;
      const w = Math.max(0, maxX - minX);
      const h = Math.max(0, maxY - minY);
      const tag =
        counted === 1
          ? t('app.status.object_one')
          : t('app.status.object_many', { count: counted });
      return `${tag} · center=(${cx.toFixed(2)}, ${cy.toFixed(2)}) · ${w.toFixed(2)} × ${h.toFixed(2)} mm`;
    }
  }
  const minX = imp.bbox.min_x.toFixed(2);
  const minY = imp.bbox.min_y.toFixed(2);
  const maxX = imp.bbox.max_x.toFixed(2);
  const maxY = imp.bbox.max_y.toFixed(2);
  return `bbox=(${minX},${minY})–(${maxX},${maxY}) · ${imp.segments.length} segments · unit_scale=${imp.unit_scale}`;
}

/// Trailing shortcut reminder; null when no import is loaded. Selection
/// multi-modifiers when something is selected, else the idle context-menu hint.
export function statusShortcutHints(
  hasImport: boolean,
  hasSelection: boolean,
  t: Translate,
): string | null {
  if (!hasImport) return null;
  return hasSelection ? t('app.status.hints_selection') : t('app.status.hints_idle');
}

/// Compose the full status-bar line from the info + shortcut layers.
export function composeStatusBar(info: string, hints: string | null): string {
  return hints ? `${info} · ${hints}` : info;
}
