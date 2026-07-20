// Operation + tool-library CRUD extracted from the ProjectState god
// root: the CAM entities the undo/redo command bus mutates. Every edit
// here routes through the command bus; ProjectState keeps thin one-line
// delegators so the component-facing `project.*` API is unchanged.

import type { ProjectState } from './project.svelte';
import type { OpEntry, OpKind } from './op_types';
import type { ToolEntry } from './project-types';
import { buildOpEntry } from './op_defaults';
import { effectiveModes } from './tool_family';
import {
  addOperationCommand,
  addToolCommand,
  deleteOperationCommand,
  deleteToolCommand,
  duplicateOperationCommand,
  reorderOperationCommand,
  replaceToolsCommand,
  setGroupOpsByToolCommand,
  toggleTabPlacementCommand,
  updateOperationCommand,
} from './commands';

// ── operations ───────────────────────────────────────────────────────────

/// Undoable UI entry point for the tool-grouping toggle. The plain write
/// lives on the data slice for command apply/revert and load/clear paths,
/// which manage dirty + generated + history themselves. Routes through the
/// command bus (so Ctrl+Z reverses it) and invalidates the cached toolpath
/// — the reorder changes emitted-program order, so a toolpath generated
/// against the prior setting isn't safe to draw/download.
export function setGroupOpsByTool(p: ProjectState, v: boolean) {
  if (p.data.groupOpsByTool === v) return;
  p.history.exec(setGroupOpsByToolCommand(v), p.target());
  p.gen.generated = null;
  p.gen.toolpathCumLen = null;
}

/// Click-toggle a tab placement on an op. `toleranceT` is the
/// parameter-space distance under which a click on an existing nearby tab
/// removes it (Estlcam-style toggle). Single undoable history entry per
/// click.
export function toggleTabPlacement(
  p: ProjectState,
  opId: number,
  placement: { objectId: number; t: number },
  toleranceT: number,
) {
  p.history.exec(toggleTabPlacementCommand(opId, placement, toleranceT), p.target());
}

export function addOperation(p: ProjectState, kind: OpKind): OpEntry {
  // The per-kind default field set lives in the pure `buildOpEntry`
  // registry (op_defaults.ts) so it's one source of truth, unit-tested
  // without the rune runtime. This method only gathers the live context
  // and runs the result through the command bus. When the user has
  // objects selected on the canvas, geometry kinds pin to that set (most
  // users select first, then click "+ Pocket"); empty selection keeps the
  // All default.
  const op = buildOpEntry(kind, {
    nextId: p.data.operations.reduce((m, o) => Math.max(m, o.id), 0) + 1,
    tools: p.data.tools,
    reliefSources: p.data.reliefSources,
    selectionIds: [...p.sel.selectedObjects],
    objectMeta: p.transformedImport?.object_meta ?? [],
    modes: effectiveModes(p.data.machine),
  });
  p.history.exec(addOperationCommand(op), p.target());
  p.sel.selectedOpId = op.id;
  return op;
}

export function removeOperation(p: ProjectState, id: number) {
  if (!p.data.operations.some((o) => o.id === id)) return;
  p.history.exec(deleteOperationCommand(id), p.target());
  if (p.sel.selectedOpId === id) p.sel.selectedOpId = null;
}

/// Deep-clone the op and insert it immediately after the original.
/// Returns the new op or null if `id` is unknown.
export function duplicateOperation(p: ProjectState, id: number): OpEntry | null {
  const src = p.data.operations.find((o) => o.id === id);
  if (!src) return null;
  const nextId = p.data.operations.reduce((m, o) => Math.max(m, o.id), 0) + 1;
  // JSON-roundtrip clone: Svelte 5 `$state` proxies make structuredClone
  // throw DataCloneError in production builds — the dup button would die
  // with an uncaught exception and look dead.
  const copy: OpEntry = {
    ...(JSON.parse(JSON.stringify(src)) as OpEntry),
    id: nextId,
    name: `${src.name} (copy)`,
  };
  p.history.exec(duplicateOperationCommand(id, copy, id), p.target());
  p.sel.selectedOpId = copy.id;
  return copy;
}

export function updateOperation(p: ProjectState, id: number, patch: Partial<OpEntry>) {
  if (Object.keys(patch).length === 0) return;
  if (!p.data.operations.some((o) => o.id === id)) return;
  p.history.exec(updateOperationCommand(id, patch), p.target());
}

/// Reorder. Skipped when source and target index are the same so a stray
/// drag-and-drop with no actual move doesn't dirty the project. (A real
/// reorder still flips dirty so the status badge surfaces it, but the
/// previously-generated gcode stays on screen until the user clicks
/// Generate again.)
export function reorderOperation(p: ProjectState, id: number, toIndex: number) {
  const cur = p.data.operations.findIndex((o) => o.id === id);
  if (cur < 0) return;
  const clamped = Math.max(0, Math.min(toIndex, p.data.operations.length - 1));
  if (clamped === cur) return;
  p.history.exec(reorderOperationCommand(id, clamped), p.target());
}

// ── tool library ─────────────────────────────────────────────────────────

/// Replace the entire tool library in one undoable step. Used by the Tool
/// library dialog's commit button.
export function replaceTools(p: ProjectState, nextTools: ToolEntry[]) {
  if (nextTools.length === 0) return;
  p.history.exec(replaceToolsCommand(nextTools), p.target());
}

export function addTool(p: ProjectState, tool: ToolEntry) {
  p.history.exec(addToolCommand(tool), p.target());
}

export function removeTool(p: ProjectState, id: number) {
  if (!p.data.tools.some((t) => t.id === id)) return;
  p.history.exec(deleteToolCommand(id), p.target());
}
