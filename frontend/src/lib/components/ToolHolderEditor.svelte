<script lang="ts">
  import type { HolderShape } from '../state/project.svelte';
  import { t } from '../i18n';

  // Holder-shape editor: the kind selector (none / cylinder / cone /
  // stepped) plus the geometry inputs for the chosen kind. Owns the
  // per-kind default dimensions used when switching kinds; the committed
  // shape lives on the tool and is round-tripped through `holder` /
  // `onChange`. `groupId` (the tool id) namespaces the radio group so
  // radios in sibling rows don't interfere.
  let {
    holder,
    groupId,
    onChange,
  }: {
    holder: HolderShape | undefined;
    groupId: number;
    onChange: (holder: HolderShape | undefined) => void;
  } = $props();

  type HolderKind = HolderShape['kind'] | 'none';
  const holderKindLabels: Record<HolderKind, () => string> = {
    none: () => t('tools.holder.kind.none'),
    cylinder: () => t('tools.holder.kind.cylinder'),
    cone: () => t('tools.holder.kind.cone'),
    stepped: () => t('tools.holder.kind.stepped'),
  };
  const holderKindOptions: HolderKind[] = ['none', 'cylinder', 'cone', 'stepped'];
  const currentKind = $derived<HolderKind>(holder?.kind ?? 'none');

  // Switching kind seeds a sensible default shape (kept if the current
  // shape already matches, so the user's edits survive a stray toggle).
  function setHolderKind(kind: HolderKind) {
    switch (kind) {
      case 'none':
        onChange(undefined);
        break;
      case 'cylinder':
        onChange(
          holder?.kind === 'cylinder'
            ? holder
            : { kind: 'cylinder', diameter_mm: 20, length_mm: 30 },
        );
        break;
      case 'cone':
        onChange(
          holder?.kind === 'cone'
            ? holder
            : { kind: 'cone', bottom_diameter_mm: 20, top_diameter_mm: 35, length_mm: 35 },
        );
        break;
      case 'stepped':
        onChange(
          holder?.kind === 'stepped'
            ? holder
            : {
                kind: 'stepped',
                cylinder_diameter_mm: 20,
                cylinder_length_mm: 12,
                cone_top_diameter_mm: 35,
                cone_length_mm: 25,
              },
        );
        break;
    }
  }

  function updateHolderField(key: string, value: number) {
    if (!holder) return;
    onChange({ ...holder, [key]: value } as HolderShape);
  }
</script>

<div class="holder-row">
  <span class="holder-label">{t('tools.holder.label')}</span>
  {#each holderKindOptions as k (k)}
    <label class="radio">
      <input
        type="radio"
        name="holder-kind-{groupId}"
        value={k}
        checked={currentKind === k}
        onchange={() => setHolderKind(k)}
      />
      <span>{holderKindLabels[k]()}</span>
    </label>
  {/each}
</div>
{#if holder?.kind === 'cylinder'}
  <div class="holder-row">
    <label>
      <span>{t('tools.holder.cyl.diameter')}</span>
      <input
        type="number"
        step="0.5"
        min="0"
        value={holder.diameter_mm}
        onchange={(e) =>
          updateHolderField(
            'diameter_mm',
            parseFloat((e.currentTarget as HTMLInputElement).value) || 0,
          )}
      />
    </label>
    <label>
      <span>{t('tools.holder.length')}</span>
      <input
        type="number"
        step="0.5"
        min="0"
        value={holder.length_mm}
        onchange={(e) =>
          updateHolderField(
            'length_mm',
            parseFloat((e.currentTarget as HTMLInputElement).value) || 0,
          )}
      />
    </label>
  </div>
{:else if holder?.kind === 'cone'}
  <div class="holder-row">
    <label>
      <span>{t('tools.holder.cone.bottom_diameter')}</span>
      <input
        type="number"
        step="0.5"
        min="0"
        value={holder.bottom_diameter_mm}
        onchange={(e) =>
          updateHolderField(
            'bottom_diameter_mm',
            parseFloat((e.currentTarget as HTMLInputElement).value) || 0,
          )}
      />
    </label>
    <label>
      <span>{t('tools.holder.cone.top_diameter')}</span>
      <input
        type="number"
        step="0.5"
        min="0"
        value={holder.top_diameter_mm}
        onchange={(e) =>
          updateHolderField(
            'top_diameter_mm',
            parseFloat((e.currentTarget as HTMLInputElement).value) || 0,
          )}
      />
    </label>
    <label>
      <span>{t('tools.holder.length')}</span>
      <input
        type="number"
        step="0.5"
        min="0"
        value={holder.length_mm}
        onchange={(e) =>
          updateHolderField(
            'length_mm',
            parseFloat((e.currentTarget as HTMLInputElement).value) || 0,
          )}
      />
    </label>
  </div>
{:else if holder?.kind === 'stepped'}
  <div class="holder-row">
    <label>
      <span>{t('tools.holder.stepped.cyl_diameter')}</span>
      <input
        type="number"
        step="0.5"
        min="0"
        value={holder.cylinder_diameter_mm}
        onchange={(e) =>
          updateHolderField(
            'cylinder_diameter_mm',
            parseFloat((e.currentTarget as HTMLInputElement).value) || 0,
          )}
      />
    </label>
    <label>
      <span>{t('tools.holder.stepped.cyl_length')}</span>
      <input
        type="number"
        step="0.5"
        min="0"
        value={holder.cylinder_length_mm}
        onchange={(e) =>
          updateHolderField(
            'cylinder_length_mm',
            parseFloat((e.currentTarget as HTMLInputElement).value) || 0,
          )}
      />
    </label>
    <label>
      <span>{t('tools.holder.stepped.cone_top_diameter')}</span>
      <input
        type="number"
        step="0.5"
        min="0"
        value={holder.cone_top_diameter_mm}
        onchange={(e) =>
          updateHolderField(
            'cone_top_diameter_mm',
            parseFloat((e.currentTarget as HTMLInputElement).value) || 0,
          )}
      />
    </label>
    <label>
      <span>{t('tools.holder.stepped.cone_length')}</span>
      <input
        type="number"
        step="0.5"
        min="0"
        value={holder.cone_length_mm}
        onchange={(e) =>
          updateHolderField(
            'cone_length_mm',
            parseFloat((e.currentTarget as HTMLInputElement).value) || 0,
          )}
      />
    </label>
  </div>
{/if}

<style>
  /* Scoped copies of the holder-panel primitives (Svelte styles don't
     cross the component boundary) — kept byte-identical to the parent's
     so the extracted rows sit flush with the surrounding panel. */
  .holder-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.6rem;
  }
  .holder-row label {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    font-size: 0.7rem;
    color: var(--text-muted);
    min-width: 7rem;
  }
  .holder-row label span {
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .holder-row label.radio {
    flex-direction: row;
    align-items: center;
    color: var(--text);
    text-transform: none;
    letter-spacing: normal;
    font-size: 0.78rem;
    min-width: auto;
  }
  .holder-row label.radio span {
    text-transform: none;
    letter-spacing: normal;
  }
  .holder-label {
    color: var(--text-muted);
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  input {
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
</style>
