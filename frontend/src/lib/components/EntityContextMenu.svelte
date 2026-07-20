<script lang="ts">
  /// Canvas context menu + per-tab popover for EntityCanvas2D (ivac-3xwn.2,
  /// Cluster B). Owns its OWN ephemeral open/close state (ctxMenu, tabPopover)
  /// and its dismissal (outside-click, Escape); the parent drives it through
  /// imperative handles (`open` / `handleDocClick` / `handleEscape` /
  /// `closeAll`) via `bind:this` and never sees the state.
  ///
  /// It still never touches the `project` singleton: the parent resolves the
  /// open ENV (tab hit-test, selection booleans, transform, the lazy hint) and
  /// passes plain data + callbacks in; project mutations route back out through
  /// onPatchTab / onDeleteTab / onSetTextOrigin / onPick.
  ///
  /// IMPORTANT (offsetParent): rendered inline where the parent's `{#if}`
  /// blocks used to be, inside `.canvas-host`. Svelte adds no wrapper element,
  /// so the popover's offsetParent stays `.canvas-host` and `use:clampPopup`
  /// keeps clamping against the same box.
  import { isContourOp, type OpEntry } from '../state/project.svelte';
  import OpKindPicker, { type PickerKind } from './OpKindPicker.svelte';
  import { clampPopup } from '../canvas/clamp-popup';
  import {
    reduceContextMenuOpen,
    type CtxMenuState,
    type TabPopoverState,
    type CtxOpenEnv,
  } from '../canvas/context-menu';
  import { t } from '../i18n';

  interface Props {
    operations: readonly OpEntry[];
    hasTextSelected: boolean;
    hasObjsSelected: boolean;
    onPatchTab: (
      opId: number,
      placementIdx: number,
      patch: { widthOverrideMm?: number | undefined; heightOverrideMm?: number | undefined },
    ) => void;
    onDeleteTab: (opId: number, placementIdx: number) => void;
    /// Plant the selected text layer's origin at the data-space position the
    /// user right-clicked (the menu carries dataX/dataY). Parent no-ops when no
    /// text layer is selected.
    onSetTextOrigin: (dataX: number, dataY: number) => void;
    onPick: (kind: PickerKind) => void;
  }

  const {
    operations,
    hasTextSelected,
    hasObjsSelected,
    onPatchTab,
    onDeleteTab,
    onSetTextOrigin,
    onPick,
  }: Props = $props();

  /// Right-click context menu. `null` = closed. Lists the same op kinds as the
  /// Add-operation picker; clicking an entry creates an op from the current
  /// selection. Carries the cursor's data-space position for "set text origin".
  let ctxMenu = $state<CtxMenuState | null>(null);

  /// Per-tab popover. Opens on right-click over an existing tab; carries the
  /// canvas-space anchor + the (opId, placementIdx) it edits. Clamped to canvas
  /// bounds at render time so a tab near the edge doesn't open off-screen.
  let tabPopover = $state<TabPopoverState | null>(null);

  /// Open at a canvas-relative pixel position. The parent resolves the env (it
  /// owns the transform + hit-tests); this runs the pure decision and applies
  /// it — each result fully sets both bits of state (the unnamed one clears).
  export function open(cx: number, cy: number, env: CtxOpenEnv) {
    const r = reduceContextMenuOpen(cx, cy, env);
    if (r.kind === 'tab') {
      tabPopover = r.tabPopover;
      ctxMenu = null;
    } else if (r.kind === 'menu') {
      ctxMenu = r.ctxMenu;
      tabPopover = null;
    } else {
      ctxMenu = null;
      tabPopover = null;
    }
  }

  export function closeAll() {
    ctxMenu = null;
    tabPopover = null;
  }

  /// Outside-click dismissal — wired to the parent's `<svelte:window onclick>`.
  /// Self-contained: bails cheaply when nothing is open, else walks the DOM so
  /// a click INSIDE the popover / menu doesn't dismiss it.
  export function handleDocClick(e: MouseEvent) {
    if (!ctxMenu && !tabPopover) return;
    const target = e.target as HTMLElement | null;
    if (tabPopover && !(target && target.closest('.tab-popover'))) {
      tabPopover = null;
    }
    if (!ctxMenu) return;
    if (target && target.closest('.ctx-menu')) return;
    ctxMenu = null;
  }

  /// Escape dismissal. Returns what it closed so the parent's multi-purpose
  /// keydown can mirror the original fall-through: closing the popover CONSUMES
  /// the key (parent returns), closing the menu only preventDefaults (parent
  /// lets lower Escape handlers — approach-picker, box-select — still run).
  export function handleEscape(): 'popover' | 'menu' | null {
    if (tabPopover) {
      tabPopover = null;
      return 'popover';
    }
    if (ctxMenu) {
      ctxMenu = null;
      return 'menu';
    }
    return null;
  }

  function parseOverride(raw: string): number | undefined {
    if (raw === '') return undefined;
    const v = parseFloat(raw);
    return isNaN(v) ? undefined : v;
  }

  /// Menu "set text origin here": hand the data-space cursor to the parent,
  /// then close (the parent no-ops if no text layer is selected).
  function setTextOrigin() {
    if (!ctxMenu) return;
    onSetTextOrigin(ctxMenu.dataX, ctxMenu.dataY);
    ctxMenu = null;
  }

  /// Op-picker click: create the op from selection (parent), then close.
  function pick(kind: PickerKind) {
    onPick(kind);
    ctxMenu = null;
  }

  /// Delete this tab placement (parent), then close the popover.
  function deleteTab() {
    if (!tabPopover) return;
    onDeleteTab(tabPopover.opId, tabPopover.placementIdx);
    tabPopover = null;
  }
</script>

{#if tabPopover}
  {@const tp = tabPopover}
  {@const op = operations.find((o) => o.id === tp.opId)}
  {@const placement = op && isContourOp(op) ? op.tabPlacements?.[tp.placementIdx] : null}
  {#if op && isContourOp(op) && placement}
    <div
      class="tab-popover"
      style:left={`${tp.x}px`}
      style:top={`${tp.y}px`}
      role="dialog"
      use:clampPopup={tp}
    >
      <div class="tab-popover-header">{t('canvas.tab_popover.header', { id: op.id })}</div>
      <label class="tab-popover-row">
        <span>{t('canvas.tab_popover.width')}</span>
        <input
          type="number"
          step="0.5"
          min="0.1"
          placeholder={String(op.tabWidth ?? 10)}
          value={placement.widthOverrideMm ?? ''}
          oninput={(e) =>
            onPatchTab(tp.opId, tp.placementIdx, {
              widthOverrideMm: parseOverride((e.target as HTMLInputElement).value),
            })}
        />
        <span class="unit">mm</span>
      </label>
      <label class="tab-popover-row">
        <span>{t('canvas.tab_popover.height')}</span>
        <input
          type="number"
          step="0.1"
          min="0.1"
          placeholder={String(op.tabHeight ?? 1)}
          value={placement.heightOverrideMm ?? ''}
          oninput={(e) =>
            onPatchTab(tp.opId, tp.placementIdx, {
              heightOverrideMm: parseOverride((e.target as HTMLInputElement).value),
            })}
        />
        <span class="unit">mm</span>
      </label>
      <button type="button" class="tab-popover-delete" onclick={deleteTab}
        >{t('canvas.tab_popover.delete')}</button
      >
      <button
        type="button"
        class="tab-popover-close"
        aria-label={t('common.close')}
        onclick={() => (tabPopover = null)}>×</button
      >
    </div>
  {/if}
{/if}
{#if ctxMenu}
  {@const cm = ctxMenu}
  {#if !hasTextSelected && !hasObjsSelected}
    <div
      class="ctx-menu empty"
      style:left={`${cm.x}px`}
      style:top={`${cm.y}px`}
      role="menu"
      use:clampPopup={cm}
    >
      <p class="ctx-hint">
        {t('canvas.ctx.empty_hint')}
      </p>
      <button type="button" onclick={() => (ctxMenu = null)}>{t('canvas.ctx.dismiss')}</button>
    </div>
  {:else}
    <div
      class="ctx-menu"
      style:left={`${cm.x}px`}
      style:top={`${cm.y}px`}
      role="menu"
      use:clampPopup={cm}
    >
      {#if hasTextSelected}
        <div class="ctx-header">{t('canvas.ctx.text_layer')}</div>
        <button
          type="button"
          class="ctx-item"
          onclick={setTextOrigin}
          title={t('canvas.ctx.set_text_origin.title')}
        >
          {t('canvas.ctx.set_text_origin')}
        </button>
        {#if hasObjsSelected}
          <div class="ctx-divider"></div>
        {/if}
      {/if}
      {#if hasObjsSelected}
        <div class="ctx-header">{t('canvas.ctx.new_op_from_selection')}</div>
        <OpKindPicker onPick={pick} />
      {/if}
    </div>
  {/if}
{/if}

<style>
  .ctx-menu {
    position: absolute;
    min-width: 16rem;
    max-width: 22rem;
    background: var(--bg-panel);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 4px;
    box-shadow: 0 6px 18px var(--shadow-modal);
    z-index: var(--z-floating);
    padding: 0.25rem;
  }
  .tab-popover {
    position: absolute;
    min-width: 11rem;
    max-width: 14rem;
    background: var(--bg-panel);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 4px;
    box-shadow: 0 6px 18px var(--shadow-modal);
    z-index: var(--z-floating);
    padding: 0.55rem 0.6rem 0.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    font-size: 0.78rem;
  }
  .tab-popover-header {
    font-size: 0.7rem;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin-bottom: 0.2rem;
  }
  .tab-popover-row {
    display: grid;
    grid-template-columns: 3.5rem 1fr auto;
    gap: 0.35rem;
    align-items: center;
  }
  .tab-popover-row input {
    width: 100%;
    padding: 0.15rem 0.3rem;
  }
  .tab-popover-row .unit {
    color: var(--text-muted);
    font-size: 0.7rem;
  }
  .tab-popover-delete {
    margin-top: 0.3rem;
    background: transparent;
    color: var(--danger);
    border: 1px solid var(--danger);
    border-radius: 3px;
    padding: 0.25rem 0.5rem;
    font-size: 0.72rem;
    cursor: pointer;
  }
  .tab-popover-delete:hover {
    background: color-mix(in srgb, var(--danger) 15%, transparent);
  }
  .tab-popover-close {
    position: absolute;
    top: 0.25rem;
    right: 0.3rem;
    background: transparent;
    color: var(--text-muted);
    border: 0;
    font-size: 1rem;
    cursor: pointer;
    line-height: 1;
    padding: 0 0.3rem;
  }
  .ctx-header {
    font-size: 0.68rem;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 0.25rem 0.45rem 0.3rem;
  }
  .ctx-item {
    background: transparent;
    color: var(--text);
    border: 0;
    padding: 0.3rem 0.55rem;
    font-size: 0.78rem;
    text-align: left;
    cursor: pointer;
    border-radius: 3px;
    margin: 0 0.2rem;
  }
  .ctx-item:hover {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .ctx-divider {
    height: 1px;
    background: var(--border);
    margin: 0.2rem 0.1rem;
  }
  .ctx-menu.empty {
    padding: 0.4rem 0.55rem;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    min-width: 14rem;
  }
  .ctx-hint {
    margin: 0;
    font-size: 0.78rem;
    color: var(--text-muted);
  }
  .ctx-menu.empty button {
    align-self: flex-end;
    background: var(--bg-elevated);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0.15rem 0.6rem;
    font-size: 0.74rem;
    cursor: pointer;
  }
</style>
