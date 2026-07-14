<script lang="ts">
  /// ReliefMill op-properties fieldset. Shown when op.kind ===
  /// 'relief_mill'. Owns the relief-source picker + image loader, the depth
  /// range, scallop/stepover, scan direction, and the physical width (which
  /// sets the source cell size). Styles inherited from OpPropertiesPanel's
  /// :global(.props ...) rules.
  import {
    project,
    type OpField,
    type OpFieldValue,
    type ReliefMillOp,
  } from '../../state/project.svelte';
  import { t } from '../../i18n';
  import { decodeImageFile } from '../../state/relief_image';
  import { rasterizeStlFile } from '../../state/relief_stl';
  import { isHeightgrid } from '../../state/relief';

  interface Props {
    op: ReliefMillOp;
    patch: <K extends OpField>(field: K, value: OpFieldValue<K>) => void;
  }
  let { op, patch }: Props = $props();

  let loading = $state(false);
  let loadError = $state<string | null>(null);
  let fileInput: HTMLInputElement | null = $state(null);
  let stlInput: HTMLInputElement | null = $state(null);

  const source = $derived(project.data.reliefSources.find((s) => s.id === op.sourceId) ?? null);
  /// STL (real-geometry) source vs. a grayscale image relief. Drives which
  /// controls apply: an STL carries real mm dimensions + real Z, so the
  /// Width rescale and brightness→depth remap don't apply to it.
  const isStl = $derived(source ? isHeightgrid(source) : false);
  /// Physical width (mm) of the loaded relief = cols * cell. Editing it
  /// rescales an IMAGE source's cell; an STL's cell is real geometry.
  const widthMm = $derived(source ? source.cols * source.cell : 0);
  const heightMm = $derived(source ? source.rows * source.cell : 0);

  async function onImagePicked(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = ''; // allow re-picking the same file
    if (!file) return;
    loading = true;
    loadError = null;
    try {
      const grid = await decodeImageFile(file, 256);
      if (grid.cols === 0 || grid.rows === 0) throw new Error('empty image');
      // Default to a 100 mm-wide relief at (0,0); the user can rescale via
      // the Width field. An image carries no real size, so we pick one.
      const targetWidthMm = !isStl && widthMm > 0 ? widthMm : 100;
      const cell = targetWidthMm / grid.cols;
      const added = project.addReliefSource({
        name: file.name,
        origin: { x: 0, y: 0 },
        cell,
        cols: grid.cols,
        rows: grid.rows,
        grid: { kind: 'grayscale', brightness: grid.brightness },
      });
      patch('sourceId', added.id);
    } catch (err) {
      loadError = err instanceof Error ? err.message : String(err);
    } finally {
      loading = false;
    }
  }

  async function onStlPicked(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = ''; // allow re-picking the same file
    if (!file) return;
    loading = true;
    loadError = null;
    try {
      // The rasterizer sizes the grid + cell from the mesh's real mm bbox,
      // and shifts the model top to z = 0 — origin/cell/cols/rows all come
      // back resolved, so no width rescale is needed (or wanted).
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

  function setWidthMm(v: number) {
    if (!source || isStl || !(v > 0)) return;
    project.updateReliefSource(source.id, { cell: v / source.cols });
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
    accept="image/*"
    style="display:none"
    bind:this={fileInput}
    onchange={onImagePicked}
  />
  <input
    type="file"
    accept=".stl,model/stl,application/sla,application/vnd.ms-pki.stl"
    style="display:none"
    bind:this={stlInput}
    onchange={onStlPicked}
  />
  <div class="load-row">
    <button type="button" onclick={() => fileInput?.click()} disabled={loading}>
      {loading ? t('ops.relief_mill.decoding') : t('ops.relief_mill.load_image')}
    </button>
    <button type="button" onclick={() => stlInput?.click()} disabled={loading}>
      {loading ? t('ops.relief_mill.decoding') : t('ops.relief_mill.load_stl')}
    </button>
  </div>
  {#if loadError}
    <p class="err" role="alert">{t('ops.relief_mill.load_error.hint', { error: loadError })}</p>
  {/if}
  {#if source}
    <label class="row" title={t('ops.relief_mill.width.help')}>
      <span>{t('ops.relief_mill.width.label')}</span>
      <div class="num-cell">
        <input
          type="number"
          step="1"
          min="1"
          value={widthMm.toFixed(2)}
          disabled={isStl}
          title={isStl ? t('ops.relief_mill.width.stl_locked') : undefined}
          onchange={(e) => setWidthMm(numFromEvent(e))}
        />
        <span class="unit">mm</span>
      </div>
    </label>
    <p class="hint">
      {t('ops.relief_mill.dimensions.hint', {
        cols: source.cols,
        rows: source.rows,
        widthMm: widthMm.toFixed(0),
        heightMm: heightMm.toFixed(0),
      })}
    </p>
    {#if isStl}
      <p class="hint">{t('ops.relief_mill.stl.dimensions.hint')}</p>
    {/if}
  {/if}
</fieldset>

<fieldset>
  <legend>{t('ops.relief.depth.legend')}</legend>
  {#if isStl}
    <!-- An STL carries real Z; depth is a floor CLAMP (tool reach), not a
         brightness remap. z_max / invert don't apply. -->
    <label class="row" title={t('ops.relief_mill.depth_limit.help')}>
      <span>{t('ops.relief_mill.depth_limit.label')}</span>
      <div class="num-cell">
        <input
          type="number"
          step="0.5"
          max="0"
          placeholder={t('ops.relief_mill.depth_limit.placeholder')}
          value={op.zMinMm}
          onchange={(e) => {
            const v = numFromEvent(e);
            if (!isNaN(v)) patch('zMinMm', v);
          }}
        />
        <span class="unit">mm</span>
      </div>
    </label>
    <p class="hint">{t('ops.relief_mill.depth_limit.hint')}</p>
  {:else}
    <label class="row" title={t('ops.relief_mill.z_min.help')}>
      <span>{t('ops.relief_mill.z_min.label')}</span>
      <div class="num-cell">
        <input
          type="number"
          step="0.5"
          max="0"
          value={op.zMinMm}
          onchange={(e) => {
            const v = numFromEvent(e);
            if (!isNaN(v)) patch('zMinMm', v);
          }}
        />
        <span class="unit">mm</span>
      </div>
    </label>
    <label class="row" title={t('ops.relief_mill.z_max.help')}>
      <span>{t('ops.relief_mill.z_max.label')}</span>
      <div class="num-cell">
        <input
          type="number"
          step="0.5"
          max="0"
          value={op.zMaxMm}
          onchange={(e) => {
            const v = numFromEvent(e);
            if (!isNaN(v)) patch('zMaxMm', v);
          }}
        />
        <span class="unit">mm</span>
      </div>
    </label>
    <label class="row" title={t('ops.relief_mill.invert.help')}>
      <span>{t('ops.relief_mill.invert.label')}</span>
      <input
        type="checkbox"
        checked={op.invert}
        onchange={(e) => patch('invert', (e.currentTarget as HTMLInputElement).checked)}
      />
    </label>
  {/if}
</fieldset>

<fieldset>
  <legend>{t('ops.relief.finish.legend')}</legend>
  <label class="row" title={t('ops.relief_mill.scallop.help')}>
    <span>{t('ops.relief_mill.scallop.label')}</span>
    <div class="num-cell">
      <input
        type="number"
        step="0.01"
        min="0.005"
        value={op.scallopHeightMm}
        onchange={(e) => {
          const v = numFromEvent(e);
          if (!isNaN(v) && v > 0) patch('scallopHeightMm', v);
        }}
      />
      <span class="unit">mm</span>
    </div>
  </label>
  <label class="row" title={t('ops.relief_mill.stepover.help')}>
    <span>{t('ops.relief_mill.stepover.label')}</span>
    <div class="num-cell">
      <input
        type="number"
        step="0.1"
        min="0"
        placeholder={t('ops.relief_mill.stepover.placeholder')}
        value={op.stepoverMm ?? ''}
        onchange={(e) => {
          const v = numFromEvent(e);
          patch('stepoverMm', isNaN(v) || v <= 0 ? null : v);
        }}
      />
      <span class="unit">mm</span>
    </div>
  </label>
  <label class="row">
    <span>{t('ops.relief_mill.scan.label')}</span>
    <div class="num-cell">
      <select
        value={op.scanDirection}
        onchange={(e) =>
          patch(
            'scanDirection',
            (e.currentTarget as HTMLSelectElement).value as 'along_x' | 'along_y',
          )}
      >
        <option value="along_x">{t('ops.relief.scan.along_x')}</option>
        <option value="along_y">{t('ops.relief.scan.along_y')}</option>
      </select>
    </div>
  </label>
  <label class="row" title={t('ops.relief_mill.step.help')}>
    <span>{t('ops.relief_mill.step.label')}</span>
    <div class="num-cell">
      <input
        type="number"
        step="0.1"
        min="0.05"
        value={op.alongStepMm}
        onchange={(e) => {
          const v = numFromEvent(e);
          if (!isNaN(v) && v > 0) patch('alongStepMm', v);
        }}
      />
      <span class="unit">mm</span>
    </div>
  </label>
  <p class="hint">
    {t('ops.relief_mill.surfacing.hint')}
  </p>
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
