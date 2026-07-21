<script lang="ts">
  /// Tool library dialog. Project-scoped table of every tool the user
  /// has configured; ops will reference an entry by id once UX-7 lands.
  /// Each row is editable in place; the modal commits/cancels as a
  /// single unit so the user can revert without touching project.data.tools.
  import {
    project,
    type ToolEntry,
    type ToolKind,
    type HolderShape,
  } from '../state/project.svelte';
  import { untrack } from 'svelte';
  import { t } from '../i18n';
  import Modal from './Modal.svelte';
  import { DialogDraft } from './dialog-draft.svelte';
  import * as fileOps from '../services/file_ops';
  import {
    effectiveModes,
    KIND_DISPLAY_LABELS,
    machineModesLabel,
    toolCompatibleWithAnyMode,
  } from '../state/tool_family';
  import { workspace } from '../state/workspace.svelte';
  import { seedInventoryFromProject, syncStockedFromInventory } from '../state/tool_inventory';
  import { isAutoToolName, suggestToolName } from '../state/tool_naming';
  import ToolRowExpandedEditor from './ToolRowExpandedEditor.svelte';
  import ToolRowSummary from './ToolRowSummary.svelte';
  import {
    applyToolTableView,
    EMPTY_TOOL_VIEW,
    nextSortState,
    pageOfTool,
    paginateToolRows,
    type ToolSortKey,
    type ToolTableView,
  } from '../state/tool_table';
  import { defaultToolForMode } from '../state/tool_mode_defaults';
  import { applyPresetPatch } from '../state/tool_presets';
  import ToolCalibrationDialog from './ToolCalibrationDialog.svelte';
  import { rowInvalid, kindNeedsExpansion } from '../state/tool_validation';

  interface Props {
    open: boolean;
    onClose: () => void;
    /// Render as a first-class tab panel instead of a modal: no Modal
    /// wrapper, no × / Cancel (the component stays mounted across tab
    /// switches, so an in-progress draft survives), footer becomes
    /// Apply / Revert.
    embedded?: boolean;
    /// Backing store: 'project' edits the working tool set of the
    /// current project/machine (the legacy modal behavior); 'inventory'
    /// edits the workspace-level SHOP inventory — every tool the user
    /// owns. Inventory commits propagate into same-id stocked copies in
    /// the project, so "the 6 mm endmill" stays one tool everywhere.
    source?: 'project' | 'inventory';
  }
  let { open, onClose, embedded = false, source = 'project' }: Props = $props();
  const active = $derived(open || embedded);
  const isInventory = $derived(source === 'inventory');
  /// The list this editor seeds from / commits to.
  const backingTools = $derived.by(() => {
    if (source === 'inventory') {
      void workspace.version;
      return workspace.get().tool_inventory;
    }
    return project.data.tools;
  });

  /// Draft / pristine / dirty / discard lifecycle lives in DialogDraft
  /// so X / Esc / click-outside can prompt before silently discarding
  /// edits. The `draft` alias keeps the table markup terse — row
  /// rebuilds always reassign `dd.draft`, never the alias.
  const dd = new DialogDraft<ToolEntry[]>();
  const draft = $derived(dd.draft ?? []);
  /// Per-row UI flag — Holder sub-panel collapsed by default to keep the
  /// table compact. Stored as a Set of row ids so reorders / additions
  /// don't accidentally move the toggle to a different tool.
  let expanded = $state<Set<number>>(new Set());
  /// Tool id that flashes briefly when the dialog is opened with a focus
  /// request (the "edit this tool" link in OpPropertiesPanel).
  let highlightedId = $state<number | null>(null);
  let bodyEl = $state<HTMLDivElement | null>(null);
  /// Mode filter: the default view shows only tools the machine's
  /// EFFECTIVE mode set (primary mode + capabilities — a combo
  /// mill+plasma machine keeps both halves visible) can run; the
  /// "N hidden — Show all" row reveals the rest. View-only — a mode
  /// switch never mutates the library.
  let showIncompatible = $state(false);
  /// Row index whose wear calibration dialog is open, or null.
  let calibratingIdx = $state<number | null>(null);
  function applyCalibration(idx: number, wearOffsetMm: number, dateIso: string) {
    dd.draft = draft.map((t, i) =>
      i === idx
        ? {
            ...t,
            wearOffsetMm: wearOffsetMm === 0 ? undefined : wearOffsetMm,
            lastCalibrated: dateIso,
          }
        : t,
    );
  }
  const machineModes = $derived(effectiveModes(project.data.machine));
  const incompatibleCount = $derived(
    isInventory ? 0 : draft.filter((t) => !toolCompatibleWithAnyMode(t.kind, machineModes)).length,
  );
  function rowVisible(tool: ToolEntry): boolean {
    if (isInventory) return true; // the shop inventory is machine-agnostic
    return showIncompatible || toolCompatibleWithAnyMode(tool.kind, machineModes);
  }

  // ── table view: sort / filter / pagination ─────────────────────────
  // View-only — never reorders the stored library. Rows are wrapped as
  // { tool, i } with i = the ORIGINAL draft index, so every edit
  // handler keeps mutating the right entry regardless of sort order.
  let view = $state<ToolTableView>({ ...EMPTY_TOOL_VIEW });
  let page = $state(0);
  const viewedRows = $derived(
    applyToolTableView(
      draft.map((tool, i) => ({ tool, i })).filter((r) => rowVisible(r.tool)),
      view,
    ),
  );
  const paged = $derived(paginateToolRows(viewedRows, page));
  const filtersActive = $derived(view.query.trim() !== '' || view.kind !== '' || view.mode !== '');
  function setSort(key: ToolSortKey) {
    const next = nextSortState(view, key);
    view.sortKey = next.sortKey;
    view.sortDir = next.sortDir;
  }
  function sortArrow(key: ToolSortKey): string {
    if (view.sortKey !== key) return '';
    return view.sortDir === 'asc' ? ' ▲' : ' ▼';
  }
  function clearFilters() {
    view.query = '';
    view.kind = '';
    view.mode = '';
    page = 0;
  }
  /// Make a specific tool visible: drop filters that would hide it and
  /// jump to its page under the current sort.
  function revealTool(id: number) {
    clearFilters();
    const rows = applyToolTableView(
      draft.map((tool, i) => ({ tool, i })).filter((r) => rowVisible(r.tool)),
      { ...view, query: '', kind: '', mode: '' },
    );
    page = pageOfTool(rows, id) ?? 0;
  }

  $effect(() => {
    if (!active) return;
    // Tracked deps: ONLY the backing store (deep snapshot) + the
    // project tools the inventory seeds from. Everything below runs
    // untracked — the previous version read dd.isDirty (which
    // deep-reads dd.draft) and then WROTE dd.draft via dd.open(), so
    // every clone write re-invalidated the effect: an infinite loop
    // that froze the whole app the moment the tab mounted.
    const backing = $state.snapshot(backingTools) as ToolEntry[];
    const projectTools = $state.snapshot(project.data.tools) as ToolEntry[];
    untrack(() => {
      // Embedded panels stay mounted, so external tool changes (undo,
      // stocking from the Machine tab) re-run this — refresh a CLEAN
      // draft to stay in sync, but never clobber in-progress edits.
      if (embedded && dd.isDirty) return;
      let tools = backing;
      if (isInventory && tools.length === 0 && projectTools.length > 0) {
        // First use of the shop inventory on an installation that
        // predates it: seed from the current project's tools so the
        // user starts from what they already configured. Deferred —
        // workspace.version is $state and must not bump synchronously
        // inside an effect body.
        const seeded = seedInventoryFromProject(projectTools);
        queueMicrotask(() => workspace.setToolInventory(seeded));
        tools = seeded;
      }
      dd.open(tools);
      showIncompatible = false;
      calibratingIdx = null;
      // Tools whose kind has a REQUIRED kind-specific field open by
      // default so the user sees `dragoff` / `cornerRadiusMm` / T-slot
      // neck dims without hunting for them. Other kinds start collapsed.
      expanded = new Set(tools.filter((t) => kindNeedsExpansion(t.kind)).map((t) => t.id));
    });
  });

  // Numeric-field validation, fieldApplies, and the per-kind
  // disabled-reason tooltips all live in lib/state/tool_validation.ts;
  // the dialog wires them in via the imports up top.
  let hasInvalidRow = $derived(draft.some(rowInvalid));

  /// Close protocol (dd.requestClose): the first attempt on a dirty
  /// draft arms the inline "Discard / Keep editing" footer pair; the
  /// second confirms. The inline bar replaces the prior `window.confirm`
  /// prompt, which silently returns false in some Tauri / WebKitGTK
  /// builds (audit-C10).
  function close() {
    if (dd.requestClose()) onClose();
  }

  $effect(() => {
    const focusId = project.sel.toolsDialogFocusId;
    if (!active || focusId == null) return;
    // The focus target may be mode-filtered out (an op still
    // referencing a mill tool on a plasma machine) — reveal it.
    const target = draft.find((t) => t.id === focusId);
    if (target && !rowVisible(target)) showIncompatible = true;
    if (target) revealTool(focusId);
    queueMicrotask(() => {
      const host = bodyEl;
      if (!host) return;
      const row = host.querySelector(`[data-tool-id="${focusId}"]`) as HTMLElement | null;
      if (row) row.scrollIntoView({ block: 'center', behavior: 'smooth' });
      highlightedId = focusId;
      window.setTimeout(() => {
        if (highlightedId === focusId) highlightedId = null;
      }, 1400);
    });
  });

  function commit() {
    // Refuse to commit while any row has an invalid numeric field. The
    // OK button is also disabled in that state — belt-and-braces so a
    // keyboard / programmatic invocation can't smuggle a zero-rate tool
    // through.
    if (draft.some(rowInvalid)) return;
    // Deep-snapshot so the command system receives plain objects —
    // Svelte 5 `$state` proxies inside `draft[i]` can trip up the
    // `structuredClone` call inside replaceToolsCommand on some
    // production builds, which would silently abort and leave the
    // dialog open.
    if (draft.length > 0) {
      const snap = JSON.parse(JSON.stringify(draft)) as typeof draft;
      try {
        if (isInventory) {
          workspace.setToolInventory(snap);
          // Propagate the edits into same-id stocked copies so the
          // machine's loadout (and its profile, via the mirror)
          // follows the inventory definition. One undoable step.
          const synced = syncStockedFromInventory(
            snap,
            JSON.parse(JSON.stringify(project.data.tools)) as typeof draft,
          );
          if (synced) project.replaceTools(synced);
        } else {
          project.replaceTools(snap);
        }
      } catch (e) {
        console.error('ToolLibraryDialog.commit: apply failed', e);
      }
    }
    // Embedded (tab) mode: Apply commits and stays — re-baseline the
    // draft instead of closing.
    if (embedded) dd.markClean();
    else onClose();
  }

  /// Embedded-mode Revert: drop the draft back to the committed tools.
  function revert() {
    dd.open(backingTools);
  }

  function addTool() {
    const nextId = (draft.reduce((m, t) => Math.max(m, t.id), 0) || 0) + 1;
    // Seed the PRIMARY mode's signature kind — a new tool on a plasma
    // machine starts as a torch, not an endmill the mode filter would
    // immediately hide.
    dd.draft = [...draft, defaultToolForMode(project.data.machine.mode, nextId)];
    revealTool(nextId);
  }

  function removeAt(idx: number) {
    if (draft.length <= 1) return;
    dd.draft = draft.filter((_, i) => i !== idx);
  }

  function updateField<K extends keyof ToolEntry>(idx: number, key: K, value: ToolEntry[K]) {
    dd.draft = draft.map((t, i) => {
      if (i !== idx) return t;
      // Auto-naming, editor-autocomplete style: while a row's name is
      // empty or still equals its own suggestion, setting edits keep
      // the name in sync ("3mm endmill" follows the diameter). A name
      // the user typed is never rewritten.
      const wasAuto = key !== 'name' && isAutoToolName(t);
      const next = { ...t, [key]: value };
      if (wasAuto) next.name = suggestToolName(next);
      return next;
    });
  }

  /// Per-kind default fill-in on `kind` change. Pre-populates the
  /// fields that newly APPLY to the target kind so the user doesn't
  /// see blank inputs when flipping endmill → drill (twist drills
  /// usually have 2 flutes and a 118° tip). Existing user-set values
  /// are preserved.
  function onKindChange(idx: number, kind: ToolKind) {
    let touchedId: number | null = null;
    dd.draft = draft.map((t, i) => {
      if (i !== idx) return t;
      const wasAuto = isAutoToolName(t);
      const next: ToolEntry = { ...t, kind };
      if (wasAuto) next.name = suggestToolName(next);
      if (kind === 'drill') {
        if (next.flutes === 0 || next.flutes === undefined) next.flutes = 2;
        if (next.tipAngleDeg === undefined) next.tipAngleDeg = 118;
      }
      if (
        (kind === 'v_bit' || kind === 'engraver' || kind === 'cone') &&
        next.tipAngleDeg === undefined
      ) {
        // Cone bits are commonly steeper than engraving V-bits; 30° is a
        // sensible cone default vs 60° for V/engrave.
        next.tipAngleDeg = kind === 'cone' ? 30 : 60;
      }
      if (kind === 'thread_mill') {
        // Thread mill: tipAngleDeg is the thread flank angle (60° metric
        // / 55° Whitworth); seed a 1 mm pitch and the metric flank.
        if (next.tipAngleDeg === undefined) next.tipAngleDeg = 60;
        if (next.threadPitchMm === undefined) next.threadPitchMm = 1.0;
      }
      touchedId = next.id;
      return next;
    });
    // Open the expanded section so the new kind's required kind-specific
    // field is in view (e.g. dragoff for drag_knife).
    if (touchedId != null && kindNeedsExpansion(kind)) {
      const next = new Set(expanded);
      next.add(touchedId);
      expanded = next;
    }
  }

  function toggleExpanded(id: number) {
    const next = new Set(expanded);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    expanded = next;
  }

  /// Apply a holder preset (by label) to draft row `idx`, merging its patch.
  /// The preset table + the patch math live in the pure, unit-tested
  /// tool_presets module; the dropdown reads HOLDER_PRESETS from there too.
  function applyPreset(idx: number, label: string) {
    const patch = applyPresetPatch(draft[idx], label);
    if (!patch) return;
    dd.draft = draft.map((t, i) => (i === idx ? { ...t, ...patch } : t));
  }

  /// Persist a holder-shape change from the ToolHolderEditor child. The
  /// child owns the kind selector, per-kind default dimensions, and the
  /// geometry inputs; the parent just writes the resulting shape onto the
  /// row. Bypasses updateField's auto-naming (a holder edit never changes
  /// the suggested name), matching the previous direct-map behavior.
  function setHolder(idx: number, holder: HolderShape | undefined) {
    dd.draft = draft.map((t, i) => (i === idx ? { ...t, holder } : t));
  }

  // Display labels for the kind dropdown live in tool_family.ts so the
  // dialog, the disabled-reason tooltips, and any other UI surface that
  // names a tool kind read from the same source.
  const kindLabels = KIND_DISPLAY_LABELS;
  const kindOptions = Object.keys(kindLabels) as ToolKind[];
</script>

{#snippet shell()}
  {#if !embedded}
    <header>
      <h2 id="tools-title">{t('tools.title')}</h2>
      <button class="dlg-close" onclick={close} aria-label={t('common.close')}>×</button>
    </header>
  {/if}
  <!-- Header-attached filters: text search + kind + machine
       capability. View-only — filtering never touches the library. -->
  <div class="table-actions">
    <!-- File actions left-aligned, matching the Project toolbar. In
         inventory mode they import/export the SHOP INVENTORY (via the
         draft — Apply persists); in project mode the working tool set. -->
    <button
      type="button"
      class="tc-file"
      onclick={async () => {
        if (isInventory) await fileOps.exportToolset(JSON.parse(JSON.stringify(draft)));
        else await fileOps.saveToolset();
      }}
      title={isInventory
        ? t('tools.file.save.inventory.title')
        : t('tools.file.save.project.title')}>{t('common.save_ellipsis')}</button
    >
    <button
      type="button"
      class="tc-file"
      onclick={async () => {
        if (isInventory) {
          const merged = await fileOps.importToolset('replace', draft);
          if (merged) dd.draft = merged;
        } else {
          await fileOps.loadToolset('replace');
          dd.draft = project.data.tools.map((t) => ({ ...t }));
        }
      }}
      title={isInventory
        ? t('tools.file.load_replace.inventory.title')
        : t('tools.file.load_replace.project.title')}>{t('tools.file.load_replace')}</button
    >
    <button
      type="button"
      class="tc-file"
      onclick={async () => {
        if (isInventory) {
          const merged = await fileOps.importToolset('add', draft);
          if (merged) dd.draft = merged;
        } else {
          await fileOps.loadToolset('add');
          dd.draft = project.data.tools.map((t) => ({ ...t }));
        }
      }}
      title={t('tools.file.load_add.title')}>{t('tools.file.load_add')}</button
    >
  </div>
  <div class="table-filters">
    <input
      type="text"
      class="tc-search"
      placeholder={t('tools.search.placeholder')}
      value={view.query}
      oninput={(e) => {
        view.query = (e.currentTarget as HTMLInputElement).value;
        page = 0;
      }}
      title={t('tools.search.title')}
    />
    <label class="tc-filter">
      <span>{t('tools.filter.kind')}</span>
      <select
        value={view.kind}
        onchange={(e) => {
          view.kind = (e.currentTarget as HTMLSelectElement).value as typeof view.kind;
          page = 0;
        }}
      >
        <option value="">{t('tools.filter.all')}</option>
        {#each kindOptions as k (k)}
          <option value={k}>{kindLabels[k]}</option>
        {/each}
      </select>
    </label>
    <label class="tc-filter" title={t('tools.filter.runs_on.title')}>
      <span>{t('tools.filter.runs_on')}</span>
      <select
        value={view.mode}
        onchange={(e) => {
          view.mode = (e.currentTarget as HTMLSelectElement).value as typeof view.mode;
          page = 0;
        }}
      >
        <option value="">{t('tools.filter.runs_on.any')}</option>
        <option value="mill">{t('tools.filter.runs_on.mill')}</option>
        <option value="laser">{t('tools.filter.runs_on.laser')}</option>
        <option value="drag">{t('tools.filter.runs_on.drag')}</option>
        <option value="plasma">{t('tools.filter.runs_on.plasma')}</option>
      </select>
    </label>
    {#if filtersActive}
      <button type="button" class="tc-clear" onclick={clearFilters}
        >{t('tools.filter.clear')}</button
      >
    {/if}
    <span class="tc-count"
      >{paged.total === draft.length
        ? draft.length === 1
          ? t('tools.count.one', { count: draft.length })
          : t('tools.count.many', { count: draft.length })
        : t('tools.count.filtered', { shown: paged.total, total: draft.length })}</span
    >
  </div>
  <div class="body" bind:this={bodyEl}>
    <div class="table">
      <div class="row head">
        <button
          class="sort-h"
          type="button"
          onclick={() => setSort('id')}
          title={t('tools.col.id.title')}>#{sortArrow('id')}</button
        >
        <button
          class="sort-h"
          type="button"
          onclick={() => setSort('name')}
          title={t('tools.col.name.title')}>{t('tools.col.name')}{sortArrow('name')}</button
        >
        <button
          class="sort-h"
          type="button"
          onclick={() => setSort('kind')}
          title={t('tools.col.kind.title')}>{t('tools.col.kind')}{sortArrow('kind')}</button
        >
        <button
          class="sort-h"
          type="button"
          onclick={() => setSort('diameter')}
          title={t('tools.col.diameter.title')}
          >⌀ <span class="unit-hdr">mm</span>{sortArrow('diameter')}</button
        >
        <span>{t('tools.col.tip_diameter')} <span class="unit-hdr">mm</span></span>
        <span title={t('tools.col.tip_angle.title')}
          >{t('tools.col.tip_angle')} <span class="unit-hdr">°</span></span
        >
        <button
          class="sort-h"
          type="button"
          onclick={() => setSort('flutes')}
          title={t('tools.col.flutes.title')}>{t('tools.col.flutes')}{sortArrow('flutes')}</button
        >
        <button
          class="sort-h"
          type="button"
          onclick={() => setSort('speed')}
          title={t('tools.col.speed.title')}
          >{t('tools.col.speed')} <span class="unit-hdr">RPM</span>{sortArrow('speed')}</button
        >
        <button
          class="sort-h"
          type="button"
          onclick={() => setSort('feedRate')}
          title={t('tools.col.feed.title')}
          >{t('tools.col.feed')} <span class="unit-hdr">mm/min</span>{sortArrow('feedRate')}</button
        >
        <button
          class="sort-h"
          type="button"
          onclick={() => setSort('plungeRate')}
          title={t('tools.col.plunge.title')}
          >{t('tools.col.plunge')} <span class="unit-hdr">mm/min</span>{sortArrow(
            'plungeRate',
          )}</button
        >
        <span title={t('tools.col.dflt_step.title')}
          >{t('tools.col.dflt_step')} <span class="unit-hdr">mm</span></span
        >
        <span>{t('tools.col.coolant')}</span>
        <span></span>
      </div>
      {#each paged.rows as { tool, i } (tool.id)}
        <div class="row" class:highlight={highlightedId === tool.id} data-tool-id={tool.id}>
          <ToolRowSummary
            {tool}
            expanded={expanded.has(tool.id)}
            canDelete={draft.length > 1}
            onUpdateField={(key, value) => updateField(i, key, value)}
            onKindChange={(kind) => onKindChange(i, kind)}
            onToggleExpanded={() => toggleExpanded(tool.id)}
            onRemove={() => removeAt(i)}
          />
        </div>
        {#if expanded.has(tool.id)}
          <ToolRowExpandedEditor
            {tool}
            onUpdateField={(key, value) => updateField(i, key, value)}
            onApplyPreset={(label) => applyPreset(i, label)}
            onSetHolder={(h) => setHolder(i, h)}
            onCalibrate={() => (calibratingIdx = i)}
          />
        {/if}
      {/each}
      {#if paged.total === 0 && filtersActive}
        <div class="mode-filter-row">
          <span>{t('tools.no_match')}</span>
          <button type="button" class="btn-secondary" onclick={clearFilters}
            >{t('tools.clear_filters')}</button
          >
        </div>
      {/if}
      {#if !showIncompatible && incompatibleCount > 0}
        <div class="mode-filter-row">
          <span
            >{incompatibleCount === 1
              ? t('tools.hidden.one', {
                  count: incompatibleCount,
                  machine: machineModesLabel(machineModes),
                })
              : t('tools.hidden.many', {
                  count: incompatibleCount,
                  machine: machineModesLabel(machineModes),
                })}</span
          >
          <button
            type="button"
            class="btn-secondary"
            onclick={() => (showIncompatible = true)}
            title={t('tools.show_all.title')}>{t('tools.show_all')}</button
          >
        </div>
      {:else if showIncompatible && incompatibleCount > 0}
        <div class="mode-filter-row">
          <span
            >{t('tools.showing_all', {
              count: incompatibleCount,
              machine: machineModesLabel(machineModes),
            })}</span
          >
          <button type="button" class="btn-secondary" onclick={() => (showIncompatible = false)}
            >{t('tools.hide_incompatible')}</button
          >
        </div>
      {/if}
    </div>
    <button class="add" onclick={addTool}>{t('tools.add')}</button>
  </div>
  {#if paged.pageCount > 1}
    <div class="table-pager">
      <button type="button" disabled={paged.page === 0} onclick={() => (page = paged.page - 1)}
        >{t('tools.pager.prev')}</button
      >
      <span>{t('tools.pager.status', { page: paged.page + 1, total: paged.pageCount })}</span>
      <button
        type="button"
        disabled={paged.page >= paged.pageCount - 1}
        onclick={() => (page = paged.page + 1)}>{t('tools.pager.next')}</button
      >
    </div>
  {/if}
  <footer>
    {#if dd.confirmingDiscard}
      <span class="discard-prompt">{t('common.discard_unsaved')}</span>
      <button class="btn-secondary" onclick={() => dd.cancelDiscard()}
        >{t('common.keep_editing')}</button
      >
      <button class="btn-danger" onclick={close}>{t('common.discard')}</button>
    {:else}
      <span class="sep"></span>
      {#if hasInvalidRow}
        <!-- Surface why OK is greyed out so the user knows which inputs need fixing. -->
        <span class="validation-msg" role="status">{t('tools.validation_msg')}</span>
      {/if}
      {#if embedded}
        <button class="btn-secondary" onclick={revert} disabled={!dd.isDirty}
          >{t('common.revert')}</button
        >
        <button
          class="btn-primary"
          onclick={commit}
          disabled={hasInvalidRow || !dd.isDirty}
          title={hasInvalidRow ? t('tools.apply.invalid.title') : ''}>{t('common.apply')}</button
        >
      {:else}
        <button class="btn-secondary" onclick={close}>{t('common.cancel')}</button>
        <button
          class="btn-primary"
          onclick={commit}
          disabled={hasInvalidRow}
          title={hasInvalidRow ? t('tools.save.invalid.title') : ''}>{t('common.ok')}</button
        >
      {/if}
    {/if}
  </footer>
{/snippet}

{#if embedded}
  <section class="embedded-shell">{@render shell()}</section>
{:else if open}
  <Modal
    onClose={close}
    persistKey="tool-library"
    width="min(960px, 96vw)"
    draggable
    resizable
    ariaLabelledBy="tools-title"
  >
    {@render shell()}
  </Modal>
{/if}
{#if active && calibratingIdx != null && draft[calibratingIdx]}
  {@const calTool = draft[calibratingIdx]}
  <ToolCalibrationDialog
    open
    toolName={calTool.name}
    nominalDiameterMm={calTool.diameter}
    currentWearOffsetMm={calTool.wearOffsetMm ?? 0}
    onApply={(wear, date) => {
      if (calibratingIdx != null) applyCalibration(calibratingIdx, wear, date);
    }}
    onClose={() => (calibratingIdx = null)}
  />
{/if}

<style>
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.5rem 0.7rem;
    border-bottom: 1px solid var(--border);
    background: var(--bg-elevated);
  }
  h2 {
    font-size: 0.95rem;
    margin: 0;
    color: var(--text-strong);
  }
  .body {
    padding: 0.6rem 0.7rem;
    overflow: auto;
    min-width: 0;
    /* Bound the scroll container's height so its OWN horizontal
       scrollbar always sits at the fixed bottom edge of the visible
       body — reachable without first scrolling the (potentially long)
       tool list vertically. `flex: 1` claims the space between the
       sticky header/filters and the footer; the modal shell and the
       embedded shell are both flex columns, so this works in both. The
       wide `.table` (min-width: min-content) overflows horizontally
       here and `overflow-x: auto` turns it into the pinned scroller. */
    flex: 1;
    min-height: 0;
    /* Touch: allow horizontal panning of the wide rows (and vertical
       scroll of the list) without the browser claiming the gesture. */
    touch-action: pan-x pan-y;
    /* Firefox: a thicker-than-thin scrollbar with themed colours. */
    scrollbar-width: auto;
    scrollbar-color: var(--text-muted) var(--bg-elevated);
  }
  /* WebKit/Blink: give the horizontal scrollbar a clearly grabbable
     height with a contrasting thumb, so it reads as an interactive
     control on both pointer and touch. Theme-driven, so it tracks
     light/dark via the CSS vars. */
  .body::-webkit-scrollbar {
    height: 13px;
    width: 13px;
  }
  .body::-webkit-scrollbar-track {
    background: var(--bg-elevated);
    border-radius: 7px;
  }
  .body::-webkit-scrollbar-thumb {
    background: var(--text-muted);
    border: 3px solid var(--bg-elevated);
    border-radius: 7px;
  }
  .body::-webkit-scrollbar-thumb:hover {
    background: var(--text);
  }
  .body::-webkit-scrollbar-corner {
    background: var(--bg-elevated);
  }
  /* Tab-panel (embedded) shell — fills the main area; the body scrolls
     between the sticky header and footer. */
  .embedded-shell {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    background: var(--bg-panel);
  }
  .table {
    display: grid;
    gap: 0.2rem;
    /* Below ~720 px the 13-column tools grid (id, name, kind, diameter,
       reach, flutes, ∠, speed, feed, plunge, warmup, notes, trash) used
       to squash numeric inputs to unreadable widths because every column
       was fr-based. min-content forces the cells to their intrinsic
       width and the body's `overflow: auto` kicks in as a horizontal
       scroller — a clearly worse-than-fitting outcome only on tiny
       windows, but never an unreadable squash on the common 900-1200 px
       laptop sizes. */
    min-width: min-content;
  }
  .row {
    display: grid;
    /* Numeric columns are widened past their English-label minimum so
       longer localized headers (de: "Geschw", "Eintauchen",
       "Std.-Zustellung") don't spill into their neighbours. Units render
       on their own line under the label (see `.row.head .unit-hdr`), so a
       column only has to fit the label word, not "label + unit". */
    grid-template-columns:
      2.5rem minmax(8rem, 1.6fr) minmax(6rem, 1fr)
      4.5rem 4.5rem 4rem 3.5rem 5rem 5.5rem 5.5rem 6rem minmax(6rem, 1fr) 2rem;
    gap: 0.3rem;
    align-items: center;
    font-size: 0.78rem;
  }
  .row.head {
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    font-size: 0.68rem;
    padding-bottom: 0.2rem;
    border-bottom: 1px solid var(--border);
    /* Sticky so unit headers (mm / ° / RPM / mm/min) stay visible while
       scrolling through a long tool library; without it they scroll
       off and leave rows context-free. `.body` is the scroll container. */
    position: sticky;
    top: 0;
    background: var(--bg-panel);
    z-index: var(--z-anchor);
  }
  /* Header cells must never let a long localized label overflow into the
     next column. Allow shrink-to-fit and break over-long single words
     (e.g. de "Std.-Zustellung") at hyphenation points instead of spilling. */
  .row.head > * {
    min-width: 0;
  }
  .row.head button,
  .row.head > span {
    overflow-wrap: anywhere;
    hyphens: auto;
    line-height: 1.15;
  }
  .row.head .unit-hdr {
    /* Unit drops onto its own line under the label so the column width is
       governed by the (translated) label alone, not "label + unit". */
    display: block;
    color: var(--text-faint);
    font-size: 0.62rem;
    text-transform: none;
    letter-spacing: 0;
    margin-left: 0;
    line-height: 1.1;
  }
  @keyframes ivac-tool-flash {
    0%,
    100% {
      background: transparent;
    }
    25%,
    75% {
      background: color-mix(in srgb, var(--accent) 22%, transparent);
    }
  }
  .row.highlight {
    border-radius: 3px;
    animation: ivac-tool-flash 1.2s ease-in-out;
  }
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
  /* Two header rows: file actions on the activated-tab surface, then
     the filter row in a lighter tone acting as a divider before the
     table. */
  .table-actions {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.45rem 0.7rem;
    background: var(--bg-panel);
    font-size: 0.78rem;
    flex-wrap: wrap;
  }
  .table-filters {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.35rem 0.7rem;
    background: var(--bg-elevated);
    border-top: 1px solid var(--border);
    border-bottom: 1px solid var(--border);
    font-size: 0.78rem;
    flex-wrap: wrap;
  }
  .tc-file {
    background: var(--bg-panel);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0.2rem 0.55rem;
    font-size: 0.74rem;
    cursor: pointer;
    white-space: nowrap;
  }
  .tc-search {
    width: 14rem;
    max-width: 40vw;
  }
  .tc-filter {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    color: var(--text-muted);
  }
  .tc-clear {
    background: var(--bg-panel);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0.15rem 0.5rem;
    font-size: 0.72rem;
    cursor: pointer;
  }
  .tc-count {
    margin-left: auto;
    color: var(--text-muted);
  }
  /* Sortable column headers — tri-state (natural → ▲ → ▼). */
  .sort-h {
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    font-weight: inherit;
    color: inherit;
    text-align: left;
    cursor: pointer;
    white-space: nowrap;
  }
  .sort-h:hover {
    color: var(--text-strong);
    text-decoration: underline;
  }
  /* Pager — appears only when the filtered set exceeds one page. */
  .table-pager {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.7rem;
    padding: 0.35rem 0.7rem;
    border-top: 1px solid var(--border);
    background: var(--bg-elevated);
    font-size: 0.78rem;
  }
  .table-pager button {
    background: var(--bg-panel);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0.15rem 0.55rem;
    font-size: 0.74rem;
    cursor: pointer;
  }
  .table-pager button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  /* Machine-mode filter banner — the "N tools hidden — Show all" /
     "Hide incompatible" row under the table. Muted: it's a view
     control, not a warning (the library itself is untouched). */
  .mode-filter-row {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.35rem 0.5rem;
    font-size: 0.78rem;
    color: var(--text-muted);
    border-top: 1px dashed var(--border);
  }
  .mode-filter-row button {
    background: var(--bg-elevated);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0.15rem 0.5rem;
    font-size: 0.72rem;
    cursor: pointer;
  }
  .add {
    margin-top: 0.5rem;
    background: var(--bg-elevated);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0.25rem 0.6rem;
    font-size: 0.78rem;
    cursor: pointer;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 0.4rem;
    padding: 0.5rem 0.7rem;
    border-top: 1px solid var(--border);
    background: var(--bg-elevated);
  }
  /* Footer-side validation hint shown when an OK-disabling row is
     present. Same red palette as `.discard-prompt`, but keeps the
     action buttons aligned to the right by NOT setting
     `margin-right: auto` — we want this slot inline with the buttons,
     not pushed to the start. */
  .validation-msg {
    color: var(--danger);
    font-size: 0.78rem;
    align-self: center;
  }
</style>
