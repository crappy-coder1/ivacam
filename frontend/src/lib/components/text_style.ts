// Pure helpers shared between AddTextDialog.svelte and tests.
// Maps a style choice to the OperationKind + patch the dialog applies
// after rendering text geometry. Keeping it free of Svelte runes lets
// vitest cover the table-driven mapping without spinning up the runtime.

import type { FrameShape, OpKind, ProfileOffset, SourceCombine } from '../state/op_types';
import type { ToolKind } from '../state/op_types';
import type { MsgKey } from '../i18n/keys';

export type TextStyle =
  | 'engraving'
  | 'carve_inside'
  | 'carve_outside'
  | 'pocket_inside'
  | 'pocket_outside'
  | 'outline_inside'
  | 'outline_outside'
  | 'plain';

/// `label` / `help` are i18n *keys* (resolved with `t()` at the call site),
/// not display strings — so the style picker localizes like the rest of the
/// UI instead of hard-coding English. Kept as data here so this module stays
/// rune-free and vitest-coverable.
export interface StyleSpec {
  label: MsgKey;
  toolKind: ToolKind | null;
  defaultDepth: number | null;
  help: MsgKey;
}

export const STYLE_TABLE: Record<TextStyle, StyleSpec> = {
  engraving: {
    label: 'dialog.text.style.engraving',
    toolKind: 'engraver',
    defaultDepth: -0.5,
    help: 'dialog.text.style.engraving.help',
  },
  carve_inside: {
    label: 'dialog.text.style.carve_inside',
    toolKind: 'v_bit',
    defaultDepth: -3.0,
    help: 'dialog.text.style.carve_inside.help',
  },
  carve_outside: {
    label: 'dialog.text.style.carve_outside',
    toolKind: 'v_bit',
    defaultDepth: -3.0,
    help: 'dialog.text.style.carve_outside.help',
  },
  pocket_inside: {
    label: 'dialog.text.style.pocket_inside',
    toolKind: 'endmill',
    defaultDepth: -2.0,
    help: 'dialog.text.style.pocket_inside.help',
  },
  pocket_outside: {
    label: 'dialog.text.style.pocket_outside',
    toolKind: 'endmill',
    defaultDepth: -2.0,
    help: 'dialog.text.style.pocket_outside.help',
  },
  outline_inside: {
    label: 'dialog.text.style.outline_inside',
    toolKind: 'endmill',
    defaultDepth: -2.0,
    help: 'dialog.text.style.outline_inside.help',
  },
  outline_outside: {
    label: 'dialog.text.style.outline_outside',
    toolKind: 'endmill',
    defaultDepth: -2.0,
    help: 'dialog.text.style.outline_outside.help',
  },
  plain: {
    label: 'dialog.text.style.plain',
    toolKind: null,
    defaultDepth: null,
    help: 'dialog.text.style.plain.help',
  },
};

export interface StyleOpDescriptor {
  kind: OpKind;
  name: string;
  toolId: number;
  depth: number;
  sourceObjects?: number[];
  sourceCombine?: SourceCombine;
  offset?: ProfileOffset;
  frameShape?: FrameShape;
  framePaddingMm?: number;
}

/// Build the descriptor of the op the dialog should add for a given
/// style. Returns null for `plain`. `objectIds` is the list returned by
/// `appendImportedSegments`; `toolDiameter` drives the auto-padding for
/// the *Outside frames.
export function describeStyleOp(
  style: TextStyle,
  objectIds: number[],
  toolId: number,
  toolDiameter: number,
  depth: number,
): StyleOpDescriptor | null {
  const sources = objectIds.length > 0 ? objectIds : undefined;
  switch (style) {
    case 'engraving':
      return {
        kind: 'engrave',
        name: 'Engrave Text',
        toolId,
        depth,
        sourceObjects: sources,
        offset: 'on',
      };
    case 'carve_inside':
      return {
        kind: 'vcarve',
        name: 'V-Carve Text (inside)',
        toolId,
        depth,
        sourceObjects: sources,
        sourceCombine: objectIds.length > 1 ? 'union' : 'auto',
      };
    case 'carve_outside':
      return {
        kind: 'vcarve',
        name: 'V-Carve Text (outside)',
        toolId,
        depth,
        sourceObjects: sources,
        sourceCombine: 'difference',
        frameShape: 'rectangle',
        framePaddingMm: 3 * toolDiameter,
      };
    case 'pocket_inside':
      return {
        kind: 'pocket',
        name: 'Pocket Text (inside)',
        toolId,
        depth,
        sourceObjects: sources,
        sourceCombine: 'auto',
      };
    case 'pocket_outside':
      return {
        kind: 'pocket',
        name: 'Pocket Text (outside)',
        toolId,
        depth,
        sourceObjects: sources,
        sourceCombine: 'difference',
        frameShape: 'rectangle',
        framePaddingMm: 3 * toolDiameter,
      };
    case 'outline_inside':
      return {
        kind: 'profile',
        name: 'Outline Text (inside)',
        toolId,
        depth,
        sourceObjects: sources,
        offset: 'inside',
      };
    case 'outline_outside':
      return {
        kind: 'profile',
        name: 'Outline Text (outside)',
        toolId,
        depth,
        sourceObjects: sources,
        offset: 'outside',
      };
    case 'plain':
      return null;
  }
}

/// True when the chosen font is filled-outline but the user picked the
/// Engraving style — the dialog renders a chip suggesting a single-line
/// font swap.
export function engravingMismatch(
  style: TextStyle,
  singleLine: boolean | null,
  previewLength: number,
): boolean {
  return style === 'engraving' && singleLine === false && previewLength > 0;
}
