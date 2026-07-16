<script lang="ts">
  /// WaterlineRough op-properties fieldset. Shown when op.kind ===
  /// 'waterline_rough'. Owns the STL relief-source picker + loader and the
  /// roughing parameters (per-level Z step, lateral stepover, floor clamp).
  /// Waterline slices real mesh geometry, so only STL (height-grid) sources
  /// apply — a grayscale image relief can't be sliced. Styles inherited from
  /// OpPropertiesPanel's :global(.props ...) rules.
  import {
    project,
    type OpField,
    type OpFieldValue,
    type WaterlineRoughOp,
  } from '../../state/project.svelte';
  import { t } from '../../i18n';
  import { rasterizeStlFile } from '../../state/relief_stl';
  import { isHeightgrid } from '../../state/relief';

  interface Props {
    op: WaterlineRoughOp;
    patch: <K extends OpField>(field: K, value: OpFieldValue<K>) => void;
  }
  let { op, patch }: Props = $props();

  let loading = $state(false);
  let loadError = $state<string | null>(null);
  let stlInput: HTMLInputElement | null = $state(null);

  const source = $derived(project.data.reliefSources.find((s) => s.id === op.sourceId) ?? null);
  /// Waterline can only slice a real mesh (an STL height grid); flag a
  /// grayscale source so the user knows to load an STL instead.
  const isStl = $derived(source ? isHeightgrid(source) : false);
  const widthMm = $derived(source ? source.cols * source.cell : 0);
  const heightMm = $derived(source ? source.rows * source.cell : 0);

  async function onStlPicked(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = ''; // allow re-picking the same file
    if (!file) return;
    loading = true;
    loadError = null;
    try {
      // The rasterizer sizes the grid + cell from the mesh's real mm bbox and
      // shifts the model top to z = 0 — origin/cell/cols/rows come back
      // resolved.
      const grid = await rasterizeStlFile(file, 256);
      if (!grid) throw new Error(t('ops.relief_mill.stl.no_footprint'));
      const added = project.addReliefSource({
        name: file.name,
        origin: { x: grid.origin.x, y: grid.origin.y },
        cell: grid.cell,
        cols: grid.cols,
        rows: grid.rows,
        grid: { kind: 'heightgrid', z: grid.z },
      });
      patch('sourceId', added.id);
    } catch (err) {
      loadError = err instanceof Error ? err.message : String(err);
    } finally {
      loading = false;
    }
  }

  function numFromEvent(e: Event): number {
    return parseFloat((e.currentTarget as HTMLInputElement).value);
  }
</script>

<fieldset>
  <legend>{t('ops.relief.source.legend')}</legend>
  <label class="row">
    <span>{t('ops.relief_mill.image.label')}</span>
    <div class="num-cell">
      <select
        value={op.sourceId}
        onchange={(e) =>
          patch('sourceId', parseInt((e.currentTarget as HTMLSelectElement).value, 10))}
      >
        {#if project.data.reliefSources.length === 0}
          <option value={0}>{t('ops.image.none_loaded')}</option>
        {/if}
        {#each project.data.reliefSources as s (s.id)}
          <option value={s.id}>{s.name} ({s.cols}×{s.rows})</option>
        {/each}
      </select>
    </div>
  </label>
  <input
    type="file"
    accept=".stl,model/stl,application/sla,application/vnd.ms-pki.stl"
    style="display:none"
    bind:this={stlInput}
    onchange={onStlPicked}
  />
  <div class="load-row">
    <button type="button" onclick={() => stlInput?.click()} disabled={loading}>
      {loading ? t('ops.relief_mill.decoding') : t('ops.waterline.load_stl')}
    </button>
  </div>
  {#if loadError}
    <p class="err" role="alert">{t('ops.relief_mill.load_error.hint', { error: loadError })}</p>
  {/if}
  {#if source && !isStl}
    <p class="err" role="alert">{t('ops.waterline.needs_stl.hint')}</p>
  {/if}
  {#if source && isStl}
    <p class="hint">
      {t('ops.relief_mill.dimensions.hint', {
        cols: source.cols,
        rows: source.rows,
        widthMm: widthMm.toFixed(0),
        heightMm: heightMm.toFixed(0),
      })}
    </p>
    <p class="hint">{t('ops.relief_mill.stl.dimensions.hint')}</p>
  {/if}
</fieldset>

<fieldset>
  <legend>{t('ops.waterline.levels.legend')}</legend>
  <label class="row" title={t('ops.waterline.z_step.help')}>
    <span>{t('ops.waterline.z_step.label')}</span>
    <div class="num-cell">
      <input
        type="number"
        step="0.5"
        min="0.05"
        value={op.zStepMm}
        onchange={(e) => {
          const v = numFromEvent(e);
          if (!isNaN(v) && v > 0) patch('zStepMm', v);
        }}
      />
      <span class="unit">mm</span>
    </div>
  </label>
  <label class="row" title={t('ops.waterline.stepover.help')}>
    <span>{t('ops.waterline.stepover.label')}</span>
    <div class="num-cell">
      <input
        type="number"
        step="0.1"
        min="0"
        value={op.stepoverMm}
        onchange={(e) => {
          const v = numFromEvent(e);
          if (!isNaN(v) && v >= 0) patch('stepoverMm', v);
        }}
      />
      <span class="unit">mm</span>
    </div>
  </label>
  <label class="row" title={t('ops.waterline.floor.help')}>
    <span>{t('ops.waterline.floor.label')}</span>
    <div class="num-cell">
      <input
        type="number"
        step="0.5"
        max="0"
        placeholder={t('ops.waterline.floor.placeholder')}
        value={op.floorZMm}
        onchange={(e) => {
          const v = numFromEvent(e);
          if (!isNaN(v)) patch('floorZMm', v);
        }}
      />
      <span class="unit">mm</span>
    </div>
  </label>
  <p class="hint">{t('ops.waterline.roughing.hint')}</p>
</fieldset>

<style>
  .load-row {
    display: flex;
    gap: 0.4em;
  }
  .load-row button {
    flex: 1;
  }
  .err {
    color: var(--danger, #c0392b);
    font-size: 0.8em;
    margin: 0.25em 0 0;
  }
  .hint {
    font-size: 0.78em;
    opacity: 0.7;
    margin: 0.35em 0 0;
  }
</style>
