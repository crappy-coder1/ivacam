// Machine / stock / work-offset / profile mutations extracted from the
// ProjectState god root. Every edit here routes through the undo/redo
// command bus; the machine + profile paths additionally run the
// mode-switch staleness assessment. ProjectState keeps one-line
// delegators so the component-facing `project.*` API is unchanged.

import type { ProjectState } from './project.svelte';
import type { MachineSettings, StockConfig, ToolEntry, WorkOffset } from './project-types';
import type { MachineProfile } from './workspace';
import { effectiveModes } from './tool_family';
import { assessModeSwitch } from './mode_switch';
import { modeNotice } from './mode_notice.svelte';
import { defaultToolForMode } from './tool_mode_defaults';
import { profilePayload } from './machine_profiles';
import { computeFootprint } from '../sim/driver';
import {
  addToolCommand,
  applyMachineProfileCommand,
  assignToolToOpsCommand,
  setMachineCommand,
  setStockCommand,
  setWorkOffsetCommand,
} from './commands';

export function setMachine(p: ProjectState, next: MachineSettings) {
  const prevModes = effectiveModes(p.data.machine);
  p.history.exec(setMachineCommand(next), p.target());
  // Machine change invalidates the cached gcode: work area / units /
  // post-processor dialect / rapid feeds all feed into the run, so a
  // toolpath generated against the prior machine isn't safe to draw
  // against the new envelope or download into the new dialect's file.
  // The user has to regen; clearing here lets the GcodePanel + Scene3D
  // empty-state messaging show the stale-vs-fresh distinction
  // immediately instead of silently lying.
  p.gen.generated = null;
  // Mode / capability change: surface ops now referencing
  // incompatible tools (or a library with nothing the machine can
  // run) as ONE non-modal notice. Never rewrites anything itself;
  // never blocks the toggle. A switch back to a config where
  // everything fits clears the notice (assess returns null).
  // Compared on the EFFECTIVE mode set so dropping a capability
  // (mill+plasma → plasma-only) triggers the same check a primary-
  // mode flip does.
  const nextModes = effectiveModes(next);
  const modesChanged =
    nextModes.length !== prevModes.length || nextModes.some((m) => !prevModes.includes(m));
  if (modesChanged) {
    modeNotice.current = assessModeSwitch(next, p.data.operations, p.data.tools);
  }
}

/// The mode-switch notice's "assign to all" action: point every
/// affected op at `toolId`, or — when the library has no compatible
/// tool (`toolId == null`) — create the mode's default tool and
/// assign that. One undoable transaction via the command bus.
export function assignToolToOps(p: ProjectState, opIds: readonly number[], toolId: number | null) {
  if (opIds.length === 0) return;
  if (toolId == null) {
    const nextId = p.data.tools.reduce((m, t) => Math.max(m, t.id), 0) + 1;
    const tool = defaultToolForMode(p.data.machine.mode, nextId);
    p.history.exec(assignToolToOpsCommand(opIds, tool.id, tool), p.target());
  } else {
    p.history.exec(assignToolToOpsCommand(opIds, toolId), p.target());
  }
}

/// The mode-switch notice's seed action for a singleton mode with an
/// empty compatible set: add the mode's default tool (torch / beam /
/// knife) to the library. Undoable like any tool-library edit.
export function seedDefaultToolForMode(p: ProjectState) {
  const nextId = p.data.tools.reduce((m, t) => Math.max(m, t.id), 0) + 1;
  p.history.exec(addToolCommand(defaultToolForMode(p.data.machine.mode, nextId)), p.target());
}

/// Switch the project to a workspace machine profile: its machine
/// config + tool library replace the project's working copies and
/// the profile reference moves, all as one undoable step. Runs the
/// same staleness + mode-switch assessment as a manual machine edit
/// — the right library coming along doesn't guarantee the existing
/// ops fit the new machine.
export function applyMachineProfile(p: ProjectState, profile: MachineProfile) {
  const prevModes = effectiveModes(p.data.machine);
  const { machine, tools } = profilePayload(profile);
  p.history.exec(applyMachineProfileCommand(machine, tools, profile.id), p.target());
  p.gen.generated = null;
  const nextModes = effectiveModes(machine);
  const modesChanged =
    nextModes.length !== prevModes.length || nextModes.some((m) => !prevModes.includes(m));
  if (modesChanged) {
    modeNotice.current = assessModeSwitch(machine, p.data.operations, p.data.tools);
  }
}

/// Detach the project from its machine profile: machine + tools stay
/// exactly as they are (they become project-local again); only the
/// reference clears, so edits stop mirroring back to the profile.
export function detachMachineProfile(p: ProjectState) {
  if (p.data.machineProfileId == null) return;
  p.history.exec(
    applyMachineProfileCommand(
      JSON.parse(JSON.stringify(p.data.machine)) as MachineSettings,
      JSON.parse(JSON.stringify(p.data.tools)) as ToolEntry[],
      null,
    ),
    p.target(),
  );
}

export function setStock(p: ProjectState, patch: Partial<StockConfig>, coalesceKey?: string) {
  if (Object.keys(patch).length === 0) return;
  p.history.exec(setStockCommand(patch, coalesceKey), p.target());
}

/// Undoable WorkOffset edit. Routes through the command bus so the
/// X/Y/Z spinners + WCS picker in StockPanel + the warnings-panel
/// Apply-Fix button all coalesce into history entries identical to
/// the stock-dim flow.
export function setWorkOffset(p: ProjectState, patch: Partial<WorkOffset>) {
  if (Object.keys(patch).length === 0) return;
  p.history.exec(setWorkOffsetCommand(patch), p.target());
}

/// Snap the WCS origin to the geometry/stock footprint's bottom-left
/// corner — the single source of truth behind the
/// `stock_origin_outside_geometry_bbox` Apply-Fix action (desktop
/// GenerateBar + phone PhoneWarnings both call this). Reads
/// `stockSizingImport` so it also works for text-only projects, where
/// `transformedImport` is null. Undoable via `setWorkOffset`; callers
/// trigger a re-generate afterwards.
export function snapWorkOffsetToFootprint(p: ProjectState): void {
  const fp = computeFootprint(p.stockSizingImport, p.data.stock, p.data.machine.workArea);
  setWorkOffset(p, { x_mm: fp.minX, y_mm: fp.minY });
}
