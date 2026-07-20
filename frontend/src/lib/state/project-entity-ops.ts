// Scene-entity CRUD extracted from the ProjectState god root: fixtures,
// text layers, and relief sources — every workpiece-scene entity that
// isn't an operation, a tool, or the machine/stock config. Each edit
// routes through the undo/redo command bus; ProjectState keeps thin
// one-line delegators so the component-facing `project.*` API is
// unchanged.

import type { ProjectState } from './project.svelte';
import type { Fixture, FixtureKind, ReliefSource, TextLayer } from './project-types';
import { DEFAULT_FIXTURE_COLOR, defaultFixtureName } from './project-types';
import { invalidatePreview } from './text_preview.svelte';
import {
  addFixtureCommand,
  addReliefSourceCommand,
  addTextLayerCommand,
  deleteOperationCommand,
  deleteReliefSourceCommand,
  deleteTextLayerCommand,
  removeFixtureCommand,
  updateFixtureCommand,
  updateReliefSourceCommand,
  updateTextLayerCommand,
} from './commands';

/// Memoised bundled-font fetch — the DejaVu Sans bytes used as the
/// default font for imported DXF TEXT/MTEXT entities. Resolved once
/// per session and shared across every TextLayer created from
/// `imported.text_entities`. Returns base64 because that's the form
/// TextFontSource carries.
let _defaultFontBytesB64: Promise<string | null> | null = null;
function loadDefaultFontBytesB64(): Promise<string | null> {
  if (_defaultFontBytesB64) return _defaultFontBytesB64;
  _defaultFontBytesB64 = (async () => {
    try {
      const res = await fetch('/fonts/DejaVuSans.ttf');
      if (!res.ok) return null;
      const buf = new Uint8Array(await res.arrayBuffer());
      let binary = '';
      const chunk = 0x8000;
      for (let i = 0; i < buf.length; i += chunk) {
        binary += String.fromCharCode(...buf.subarray(i, i + chunk));
      }
      return btoa(binary);
    } catch {
      return null;
    }
  })();
  return _defaultFontBytesB64;
}

// ── fixtures ───────────────────────────────────────────────────────────

export function addFixture(
  p: ProjectState,
  kind: FixtureKind,
  origin: [number, number],
  z_bottom: number,
  z_top: number,
  name?: string,
): Fixture {
  const nextId = p.data.fixtures.reduce((m, f) => Math.max(m, f.id), 0) + 1;
  const f: Fixture = {
    id: nextId,
    name: name ?? defaultFixtureName(kind, nextId),
    kind,
    origin,
    z_bottom,
    z_top,
    color: DEFAULT_FIXTURE_COLOR,
  };
  p.history.exec(addFixtureCommand(f), p.target());
  p.sel.selectedFixtureId = f.id;
  return f;
}

export function updateFixture(p: ProjectState, id: number, patch: Partial<Fixture>) {
  if (Object.keys(patch).length === 0) return;
  if (!p.data.fixtures.some((f) => f.id === id)) return;
  p.history.exec(updateFixtureCommand(id, patch), p.target());
}

export function removeFixture(p: ProjectState, id: number) {
  if (!p.data.fixtures.some((f) => f.id === id)) return;
  p.history.exec(removeFixtureCommand(id), p.target());
  if (p.sel.selectedFixtureId === id) p.sel.selectedFixtureId = null;
}

// ── text layers ────────────────────────────────────────────────────────

/// Insert a text layer with the given configuration; `id` and the
/// default `name` are filled in if absent. Returns the inserted
/// layer (with the assigned id). Undoable.
export function addTextLayer(
  p: ProjectState,
  seed: Omit<TextLayer, 'id' | 'name'> & Partial<Pick<TextLayer, 'id' | 'name'>>,
): TextLayer {
  const nextId = seed.id ?? p.data.textLayers.reduce((m, t) => Math.max(m, t.id), 0) + 1;
  const previewText = seed.text.split(/\r?\n/, 1)[0] ?? '';
  const truncated = previewText.length > 20 ? `${previewText.slice(0, 20)}…` : previewText;
  const defaultName = `${seed.kind} — "${truncated}"`;
  const layer: TextLayer = { ...seed, id: nextId, name: seed.name ?? defaultName };
  p.history.exec(addTextLayerCommand(layer), p.target());
  return layer;
}

export function updateTextLayer(p: ProjectState, id: number, patch: Partial<TextLayer>) {
  if (Object.keys(patch).length === 0) return;
  if (!p.data.textLayers.some((t) => t.id === id)) return;
  p.history.exec(updateTextLayerCommand(id, patch), p.target());
}

/// Convert any `imported.text_entities` from the most recent setImported
/// call into editable `TextLayer` entries. Each entity gets the bundled
/// DejaVu Sans by default so the user sees the text immediately; they
/// can swap fonts later from the sidebar. No-op when nothing was
/// imported or no TEXT/MTEXT entities were present.
export async function convertImportedTextEntities(p: ProjectState): Promise<void> {
  const entry = p.data.imports[0];
  if (!entry) return;
  const entities = entry.source.text_entities;
  if (!entities || entities.length === 0) return;
  const bytes_b64 = await loadDefaultFontBytesB64();
  if (!bytes_b64) return;
  p.history.beginTransaction('Import text entities');
  try {
    for (const e of entities) {
      const isMtext = e.kind === 'MTEXT';
      addTextLayer(p, {
        kind: isMtext ? 'MTEXT' : 'TEXT',
        text: e.text,
        fontSource: { kind: 'bundled', path: '/fonts/DejaVuSans.ttf', bytes_b64 },
        sizeMm: e.size_mm,
        origin: { x: e.origin[0], y: e.origin[1] },
        rotationDeg: e.rotation_deg ?? 0,
        letterSpacingMm: 0,
        lineSpacingMm: 0,
        alignment: 'left',
        widthScale: 1.0,
        singleLine: false,
      });
    }
    p.history.commitTransaction();
  } catch (err) {
    p.history.cancelTransaction(p.target());
    throw err;
  }
  // Consume the queue so subsequent addImported() calls don't try
  // to convert the same entities again into duplicate TextLayers.
  // Plain mutation (not a command): this is bookkeeping after the
  // text-layer-add commands above, not user-undoable state.
  const cur = p.data.imports[0];
  if (cur) {
    p.data.imports = [
      { ...cur, source: { ...cur.source, text_entities: [] } },
      ...p.data.imports.slice(1),
    ];
  }
}

export function removeTextLayer(p: ProjectState, id: number) {
  if (!p.data.textLayers.some((t) => t.id === id)) return;
  const syntheticLayer = `__text_${id}`;
  // Drop the cached preview segments so the canvas doesn't keep
  // painting glyphs from a layer that no longer exists.
  invalidatePreview(id);
  // Cascade-delete any ops whose source targets the text layer's
  // synthetic geometry layer — leaving them around would make the
  // pipeline raise "no segments on layer __text_<id>".
  const dependentOps = p.data.operations.filter(
    (o) => Array.isArray(o.sourceLayers) && o.sourceLayers.includes(syntheticLayer),
  );
  if (dependentOps.length > 0) {
    p.history.beginTransaction('Delete text');
    for (const op of dependentOps) {
      p.history.exec(deleteOperationCommand(op.id), p.target());
    }
    p.history.exec(deleteTextLayerCommand(id), p.target());
    p.history.commitTransaction();
  } else {
    p.history.exec(deleteTextLayerCommand(id), p.target());
  }
  if (p.sel.selectedTextLayerId === id) p.sel.selectedTextLayerId = null;
}

// ── relief sources ───────────────────────────────────────────────────────

/// Insert a relief surface source (e.g. a decoded grayscale
/// image). `id` is assigned if absent. Returns the inserted source.
/// Undoable.
export function addReliefSource(
  p: ProjectState,
  seed: Omit<ReliefSource, 'id'> & Partial<Pick<ReliefSource, 'id'>>,
): ReliefSource {
  const nextId = seed.id ?? p.data.reliefSources.reduce((m, s) => Math.max(m, s.id), 0) + 1;
  const source: ReliefSource = { ...seed, id: nextId };
  p.history.exec(addReliefSourceCommand(source), p.target());
  return source;
}

export function updateReliefSource(p: ProjectState, id: number, patch: Partial<ReliefSource>) {
  if (Object.keys(patch).length === 0) return;
  if (!p.data.reliefSources.some((s) => s.id === id)) return;
  p.history.exec(updateReliefSourceCommand(id, patch), p.target());
}

export function removeReliefSource(p: ProjectState, id: number) {
  if (!p.data.reliefSources.some((s) => s.id === id)) return;
  p.history.exec(deleteReliefSourceCommand(id), p.target());
}
