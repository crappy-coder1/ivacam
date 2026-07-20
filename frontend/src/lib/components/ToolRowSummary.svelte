<script lang="ts">
  /// One collapsed tool-library row: the 13 grid cells (id + expand
  /// toggle, name, kind, diameter, tip ⌀, tip ∠, flutes, speed, feed,
  /// plunge, default step, coolant, delete). Extracted from
  /// ToolLibraryDialog (ivac-u9iy) as a pure-view child mirroring
  /// ToolRowExpandedEditor: a `tool` snapshot in, edits out via callbacks.
  ///
  /// The PARENT owns the `.row` grid wrapper — its column template is
  /// shared with the sticky header row so headers and cells stay aligned —
  /// and renders this child inside it. So the child emits its cells as
  /// bare grid items with no wrapper element of its own.
  import { t } from '../i18n';
  import type { ToolEntry, ToolKind, CoolantMode } from '../state/project.svelte';
  import { KIND_DISPLAY_LABELS } from '../state/tool_family';
  import { suggestToolName } from '../state/tool_naming';
  import {
    diameterInvalid,
    speedInvalid,
    feedInvalid,
    plungeInvalid,
    fieldApplies,
    fieldDisabledReason,
  } from '../state/tool_validation';

  interface Props {
    tool: ToolEntry;
    /// Whether the row's expanded editor is open — drives the ▾/▸ glyph
    /// and the expand button's aria state.
    expanded: boolean;
    /// False when this is the last remaining tool (delete disabled — the
    /// library keeps at least one row).
    canDelete: boolean;
    onUpdateField: <K extends keyof ToolEntry>(key: K, value: ToolEntry[K]) => void;
    onKindChange: (kind: ToolKind) => void;
    onToggleExpanded: () => void;
    onRemove: () => void;
  }
  let {
    tool,
    expanded,
    canDelete,
    onUpdateField,
    onKindChange,
    onToggleExpanded,
    onRemove,
  }: Props = $props();

  // Kind + coolant option lists / labels live in tool_family so the row,
  // the parent's filter dropdown, and the disabled-reason tooltips read
  // from one source.
  const kindLabels = KIND_DISPLAY_LABELS;
  const kindOptions = Object.keys(kindLabels) as ToolKind[];
  const coolantLabels: Record<CoolantMode, () => string> = {
    off: () => t('tools.coolant.off'),
    mist: () => t('tools.coolant.mist'),
    flood: () => t('tools.coolant.flood'),
  };
  const coolantOptions = Object.keys(coolantLabels) as CoolantMode[];
</script>

<span class="id">
  <button
    class="expand"
    type="button"
    aria-expanded={expanded}
    aria-label={expanded
      ? t('tools.row.expand.collapse.aria', { id: tool.id })
      : t('tools.row.expand.expand.aria', { id: tool.id })}
    title={expanded ? t('tools.row.expand.collapse.title') : t('tools.row.expand.expand.title')}
    onclick={onToggleExpanded}>{expanded ? '▾' : '▸'} {tool.id}</button
  >
</span>
<input
  type="text"
  value={tool.name}
  placeholder={suggestToolName(tool)}
  title={t('tools.row.name.title')}
  oninput={(e) => onUpdateField('name', (e.currentTarget as HTMLInputElement).value)}
/>
<select
  value={tool.kind}
  onchange={(e) => onKindChange((e.currentTarget as HTMLSelectElement).value as ToolKind)}
>
  {#each kindOptions as k (k)}
    <option value={k}>{kindLabels[k]}</option>
  {/each}
</select>
<input
  type="number"
  step="0.1"
  min="0.01"
  value={tool.diameter}
  class:invalid={diameterInvalid(tool)}
  title={diameterInvalid(tool) ? t('tools.row.diameter.invalid.title') : ''}
  onchange={(e) =>
    onUpdateField('diameter', parseFloat((e.currentTarget as HTMLInputElement).value) || 0)}
/>
<input
  type="number"
  step="0.05"
  min="0"
  value={tool.tipDiameter ?? ''}
  placeholder={fieldApplies('tipDiameter', tool.kind) ? '—' : t('tools.field.na')}
  disabled={!fieldApplies('tipDiameter', tool.kind)}
  class:invalid={tool.tipDiameter !== undefined && tool.tipDiameter < 0}
  title={!fieldApplies('tipDiameter', tool.kind)
    ? fieldDisabledReason('tipDiameter', tool.kind)
    : tool.tipDiameter !== undefined && tool.tipDiameter < 0
      ? t('tools.row.tip_diameter.invalid.title')
      : ''}
  onchange={(e) => {
    // Reject negative tip ⌀ — Rust setup_resolver.rs:669 does .max(0.0)
    // on this, so a typo like -0.5 silently becomes 0 and the depth math
    // changes without warning. Treat any negative input as "unset" (same
    // pattern as defaultStep) so the user must enter a valid value.
    const v = (e.currentTarget as HTMLInputElement).value;
    if (v === '') {
      onUpdateField('tipDiameter', undefined);
      return;
    }
    const n = parseFloat(v);
    onUpdateField('tipDiameter', isNaN(n) || n < 0 ? undefined : n);
  }}
/>
<input
  type="number"
  step="1"
  min="1"
  max="179"
  value={tool.tipAngleDeg ?? ''}
  placeholder={fieldApplies('tipAngleDeg', tool.kind) ? '60' : t('tools.field.na')}
  disabled={!fieldApplies('tipAngleDeg', tool.kind)}
  title={fieldApplies('tipAngleDeg', tool.kind)
    ? t('tools.row.tip_angle.title')
    : fieldDisabledReason('tipAngleDeg', tool.kind)}
  onchange={(e) => {
    const v = (e.currentTarget as HTMLInputElement).value;
    onUpdateField('tipAngleDeg', v === '' ? undefined : parseFloat(v));
  }}
/>
<input
  type="number"
  step="1"
  min="1"
  value={tool.flutes}
  disabled={!fieldApplies('flutes', tool.kind)}
  title={fieldApplies('flutes', tool.kind) ? '' : fieldDisabledReason('flutes', tool.kind)}
  onchange={(e) =>
    onUpdateField('flutes', parseInt((e.currentTarget as HTMLInputElement).value, 10) || 1)}
/>
<input
  type="number"
  step="500"
  min="1"
  value={tool.speed}
  disabled={!fieldApplies('speed', tool.kind)}
  class:invalid={speedInvalid(tool)}
  title={!fieldApplies('speed', tool.kind)
    ? fieldDisabledReason('speed', tool.kind)
    : speedInvalid(tool)
      ? t('tools.row.speed.invalid.title')
      : ''}
  onchange={(e) =>
    onUpdateField('speed', parseInt((e.currentTarget as HTMLInputElement).value, 10) || 0)}
/>
<input
  type="number"
  step="50"
  min="1"
  value={tool.feedRate}
  class:invalid={feedInvalid(tool)}
  title={feedInvalid(tool)
    ? t('tools.row.feed.invalid.title')
    : tool.kind === 'drill'
      ? t('tools.row.feed.drill.title')
      : ''}
  onchange={(e) =>
    onUpdateField('feedRate', parseInt((e.currentTarget as HTMLInputElement).value, 10) || 0)}
/>
<input
  type="number"
  step="50"
  min="1"
  value={tool.plungeRate}
  disabled={!fieldApplies('plunge', tool.kind)}
  class:invalid={plungeInvalid(tool)}
  title={!fieldApplies('plunge', tool.kind)
    ? fieldDisabledReason('plunge', tool.kind)
    : plungeInvalid(tool)
      ? t('tools.row.plunge.invalid.title')
      : ''}
  onchange={(e) =>
    onUpdateField('plungeRate', parseInt((e.currentTarget as HTMLInputElement).value, 10) || 0)}
/>
<input
  type="number"
  step="0.05"
  max="0"
  value={tool.defaultStep ?? ''}
  placeholder={fieldApplies('defaultStep', tool.kind) ? '—' : t('tools.field.na')}
  disabled={!fieldApplies('defaultStep', tool.kind)}
  title={fieldApplies('defaultStep', tool.kind)
    ? tool.defaultStep !== undefined && tool.defaultStep >= 0
      ? t('tools.row.dflt_step.invalid.title')
      : t('tools.row.dflt_step.title')
    : fieldDisabledReason('defaultStep', tool.kind)}
  class:invalid={tool.defaultStep !== undefined && tool.defaultStep >= 0}
  onchange={(e) => {
    const v = (e.currentTarget as HTMLInputElement).value;
    if (v === '') {
      onUpdateField('defaultStep', undefined);
      return;
    }
    const n = parseFloat(v);
    onUpdateField('defaultStep', isNaN(n) || n >= 0 ? undefined : n);
  }}
/>
<select
  value={tool.coolant}
  onchange={(e) =>
    onUpdateField('coolant', (e.currentTarget as HTMLSelectElement).value as CoolantMode)}
>
  {#each coolantOptions as c (c)}
    <option value={c}>{coolantLabels[c]()}</option>
  {/each}
</select>
<button
  class="del"
  onclick={onRemove}
  disabled={!canDelete}
  title={!canDelete ? t('tools.row.delete.disabled') : t('tools.row.delete.title')}
  aria-label={!canDelete
    ? t('tools.row.delete.disabled')
    : t('tools.row.delete.aria', { name: tool.name })}>×</button
>

<style>
  .id {
    text-align: center;
    color: var(--text-faint);
    font-variant-numeric: tabular-nums;
  }
  .expand {
    background: transparent;
    border: 0;
    color: var(--text-faint);
    cursor: pointer;
    padding: 0;
    font-size: 0.78rem;
    font-variant-numeric: tabular-nums;
    width: 100%;
    text-align: center;
  }
  /* Base input styling — the parent keeps its own copy for the filter
     bar; Svelte scopes each component's styles, so the row cells need
     their own here. */
  input,
  select {
    background: var(--bg-input);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0.18rem 0.32rem;
    font-size: 0.78rem;
    min-width: 0;
    width: 100%;
    box-sizing: border-box;
  }
  input.invalid {
    border-color: var(--danger);
  }
  /* Disabled fields (per-kind n/a entries) fade visibly so users see
     they're not editable, without changing the row layout. */
  input:disabled,
  select:disabled {
    opacity: 0.4;
    background: transparent;
    color: var(--text-muted);
    cursor: not-allowed;
  }
  .del {
    background: transparent;
    color: var(--text-muted);
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0.05rem 0.4rem;
    cursor: pointer;
  }
  .del:disabled {
    opacity: 0.3;
    cursor: not-allowed;
  }
</style>
