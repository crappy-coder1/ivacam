<script lang="ts">
  /// Presentational canvas context menu + per-tab popover, extracted from
  /// EntityCanvas2D.svelte (ivac-3xwn.2, Cluster B). Pure view: it owns no
  /// state and never touches the `project` singleton — the parent keeps the
  /// ctxMenu/tabPopover state, the open decision, and global dismiss
  /// (Escape / outside-click) wiring, and passes plain snapshots + callbacks.
  ///
  /// IMPORTANT (offsetParent): rendered inline where the parent's `{#if}`
  /// blocks used to be, inside `.canvas-host`. Svelte adds no wrapper element,
  /// so the popover's offsetParent stays `.canvas-host` and `use:clampPopup`
  /// keeps clamping against the same box.
  import { isContourOp, type OpEntry } from '../state/project.svelte';
  import OpKindPicker, { type PickerKind } from './OpKindPicker.svelte';
  import { clampPopup } from '../canvas/clamp-popup';
  import { t } from '../i18n';

  interface Props {
    ctxMenu: { x: number; y: number; dataX: number; dataY: number } | null;
    tabPopover: { x: number; y: number; opId: number; placementIdx: number } | null;
    operations: readonly OpEntry[];
    hasTextSelected: boolean;
    hasObjsSelected: boolean;
    onPatchTab: (
      opId: number,
      placementIdx: number,
      patch: { widthOverrideMm?: number | undefined; heightOverrideMm?: number | undefined },
    ) => void;
    onDeleteTab: (opId: number, placementIdx: number) => void;
    onCloseTabPopover: () => void;
    onSetTextOrigin: () => void;
    onCloseMenu: () => void;
    onPick: (kind: PickerKind) => void;
  }

  const {
    ctxMenu,
    tabPopover,
    operations,
    hasTextSelected,
    hasObjsSelected,
    onPatchTab,
    onDeleteTab,
    onCloseTabPopover,
    onSetTextOrigin,
    onCloseMenu,
    onPick,
  }: Props = $props();

  function parseOverride(raw: string): number | undefined {
    if (raw === '') return undefined;
    const v = parseFloat(raw);
    return isNaN(v) ? undefined : v;
  }
</script>

{#if tabPopover}
  {@const op = operations.find((o) => o.id === tabPopover.opId)}
  {@const placement = op && isContourOp(op) ? op.tabPlacements?.[tabPopover.placementIdx] : null}
  {#if op && isContourOp(op) && placement}
    <div
      class="tab-popover"
      style:left={`${tabPopover.x}px`}
      style:top={`${tabPopover.y}px`}
      role="dialog"
      use:clampPopup={tabPopover}
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
            onPatchTab(tabPopover.opId, tabPopover.placementIdx, {
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
            onPatchTab(tabPopover.opId, tabPopover.placementIdx, {
              heightOverrideMm: parseOverride((e.target as HTMLInputElement).value),
            })}
        />
        <span class="unit">mm</span>
      </label>
      <button
        type="button"
        class="tab-popover-delete"
        onclick={() => onDeleteTab(tabPopover.opId, tabPopover.placementIdx)}
        >{t('canvas.tab_popover.delete')}</button
      >
      <button
        type="button"
        class="tab-popover-close"
        aria-label={t('common.close')}
        onclick={onCloseTabPopover}>×</button
      >
    </div>
  {/if}
{/if}
{#if ctxMenu}
  {#if !hasTextSelected && !hasObjsSelected}
    <div
      class="ctx-menu empty"
      style:left={`${ctxMenu.x}px`}
      style:top={`${ctxMenu.y}px`}
      role="menu"
      use:clampPopup={ctxMenu}
    >
      <p class="ctx-hint">
        {t('canvas.ctx.empty_hint')}
      </p>
      <button type="button" onclick={onCloseMenu}>{t('canvas.ctx.dismiss')}</button>
    </div>
  {:else}
    <div
      class="ctx-menu"
      style:left={`${ctxMenu.x}px`}
      style:top={`${ctxMenu.y}px`}
      role="menu"
      use:clampPopup={ctxMenu}
    >
      {#if hasTextSelected}
        <div class="ctx-header">{t('canvas.ctx.text_layer')}</div>
        <button
          type="button"
          class="ctx-item"
          onclick={onSetTextOrigin}
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
        <OpKindPicker {onPick} />
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
