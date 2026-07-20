// Object-selection orchestration extracted from the ProjectState god
// root. Every path here commits the selection change through the
// undo/redo command bus so Ctrl+Z reverts it. ProjectState keeps thin
// delegators so the component-facing `project.*` API is unchanged.

import type { ProjectState } from './project.svelte';
import { computeSelectionUpdate, selectionsEqual, type SelectionMode } from './selection.svelte';
import { selectObjectsCommand } from './commands';
// Pure 2D geometry primitive extracted to `lib/canvas/selection-geometry.ts`
// so vitest specs can exercise it without mounting the canvas.
import { lineCrossesBBox } from '../canvas/selection-geometry';

export function toggleObject(p: ProjectState, id: number, additive = false) {
  if (id <= 0) return;
  // Route through the same command path as `selectObjects` so the
  // canvas-click toggle ends up in the undo/redo stack.
  selectObjects(p, [id], additive ? 'toggle' : 'replace');
}

/// Bulk selection update — used by box-select and any other path
/// that needs to commit a set of object ids with FreeCAD-style
/// modifier semantics in one go. Pushes the change through the
/// History so Ctrl+Z reverts the selection.
export function selectObjects(p: ProjectState, ids: Iterable<number>, mode: SelectionMode) {
  const prevSelected = new Set(p.sel.selectedObjects);
  const prevAnchor = p.sel.selectionAnchorObjectId;
  const { selected: nextSelected, anchor: nextAnchor } = computeSelectionUpdate(
    prevSelected,
    prevAnchor,
    ids,
    mode,
  );
  pushSelectionChange(p, prevSelected, prevAnchor, nextSelected, nextAnchor);
}

/// Internal: emit a single selection-change command. Used by
/// `selectObjects`, `clearSelection`, `seriesSelectTo`, and any
/// future selection helper that needs to land in the undo stack.
/// Skips the push when prev == next (no-op selection updates
/// shouldn't waste an undo slot).
function pushSelectionChange(
  p: ProjectState,
  prevSelected: Set<number>,
  prevAnchor: number | null,
  nextSelected: Set<number>,
  nextAnchor: number | null,
) {
  if (selectionsEqual(prevSelected, nextSelected) && prevAnchor === nextAnchor) return;
  p.history.exec(
    selectObjectsCommand(
      p.sel,
      { selected: prevSelected, anchor: prevAnchor },
      { selected: nextSelected, anchor: nextAnchor },
    ),
    p.target(),
  );
}

/// Series-select: extend the selection from the current anchor object
/// to `targetId`, picking every visible object whose bbox is crossed
/// by the straight line between the two bbox centroids. Falls back to
/// a plain replace when no anchor exists. Honors visibleLayers so
/// hidden chains can't be accidentally swept in.
export function seriesSelectTo(p: ProjectState, targetId: number) {
  if (targetId <= 0) return;
  const anchorId = p.sel.selectionAnchorObjectId;
  const meta = p.transformedImport?.object_meta ?? [];
  if (anchorId == null || anchorId === targetId || meta.length === 0) {
    selectObjects(p, [targetId], 'replace');
    return;
  }
  const visible = p.data.visibleLayers;
  const byId = new Map<number, (typeof meta)[number]>();
  for (const m of meta) byId.set(m.id, m);
  const a = byId.get(anchorId);
  const t = byId.get(targetId);
  if (!a || !t) {
    selectObjects(p, [targetId], 'replace');
    return;
  }
  const c0 = { x: (a.bbox.min_x + a.bbox.max_x) * 0.5, y: (a.bbox.min_y + a.bbox.max_y) * 0.5 };
  const c1 = { x: (t.bbox.min_x + t.bbox.max_x) * 0.5, y: (t.bbox.min_y + t.bbox.max_y) * 0.5 };
  const picked: number[] = [anchorId, targetId];
  for (const m of meta) {
    if (m.id === anchorId || m.id === targetId) continue;
    if (!visible.has(m.layer)) continue;
    if (lineCrossesBBox(c0, c1, m.bbox)) picked.push(m.id);
  }
  // Compute the post-add selection + override the anchor to `targetId`
  // so consecutive Shift+clicks chain (anchor → click → click → click).
  // Single command so Ctrl+Z restores both selection and anchor in
  // one undo step.
  const prevSelected = new Set(p.sel.selectedObjects);
  const prevAnchor = p.sel.selectionAnchorObjectId;
  const { selected: nextSelected } = computeSelectionUpdate(
    prevSelected,
    prevAnchor,
    picked,
    'add',
  );
  pushSelectionChange(p, prevSelected, prevAnchor, nextSelected, targetId);
}

export function clearSelection(p: ProjectState) {
  const prevSelected = new Set(p.sel.selectedObjects);
  const prevAnchor = p.sel.selectionAnchorObjectId;
  pushSelectionChange(p, prevSelected, prevAnchor, new Set(), null);
}
