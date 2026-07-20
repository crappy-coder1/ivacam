<script lang="ts">
  /// Expanded per-row editor for ToolLibraryDialog (ivac-bzpt). The
  /// `.holder-panel` that opens under a tool row: holder geometry + presets,
  /// per-pass overrides, comment, wear/calibration, and the laser / plasma /
  /// form-profile / thread / corner / drag-off attribute groups. Extracted
  /// verbatim from the dialog's {#each} body (it was ~680 lines of template
  /// inline) so the row detail form is a self-contained child with a narrow
  /// `tool` + callbacks seam.
  ///
  /// Stays a pure view: it never touches `project` or the draft directly —
  /// every edit routes back through a callback; the parent owns the draft,
  /// the expand toggle, and the calibration modal.
  import { t } from '../i18n';
  import type { ToolEntry, HolderShape } from '../state/project.svelte';
  import { attrApplies, MACHINE_MODE_NOUN, TOOL_COMPATIBLE_MODES } from '../state/tool_family';
  import { effectiveDiameterHint, isCalibrationStale } from '../state/tool_wear';
  import { HOLDER_PRESETS } from '../state/tool_presets';
  import ToolFormProfileEditor from './ToolFormProfileEditor.svelte';
  import ToolHolderEditor from './ToolHolderEditor.svelte';

  interface Props {
    tool: ToolEntry;
    /// Persist one field on this row (parent merges it into the draft).
    onUpdateField: <K extends keyof ToolEntry>(key: K, value: ToolEntry[K]) => void;
    /// Apply a holder preset by label (parent resolves + merges the patch).
    onApplyPreset: (label: string) => void;
    /// Persist a holder-shape change from the ToolHolderEditor child.
    onSetHolder: (holder: HolderShape | undefined) => void;
    /// Open the calibration dialog for this row (parent owns the modal).
    onCalibrate: () => void;
  }
  const { tool, onUpdateField, onApplyPreset, onSetHolder, onCalibrate }: Props = $props();
</script>

<div class="holder-panel">
  <div class="holder-row">
    <span class="holder-label" title={t('tools.holder.runs_on.title')}
      >{t('tools.holder.runs_on')}</span
    >
    {#each TOOL_COMPATIBLE_MODES[tool.kind] as m (m)}
      <span class="cap-chip">{MACHINE_MODE_NOUN[m]}</span>
    {/each}
  </div>
  <div class="holder-row">
    <label>
      <span>{t('tools.holder.flute_length')}</span>
      <input
        type="number"
        step="0.5"
        min="0"
        placeholder="—"
        value={tool.fluteLengthMm ?? ''}
        title={t('tools.holder.flute_length.title')}
        onchange={(e) => {
          const v = (e.currentTarget as HTMLInputElement).value;
          onUpdateField('fluteLengthMm', v === '' ? undefined : parseFloat(v));
        }}
      />
    </label>
    <label>
      <span>{t('tools.holder.overall_length')}</span>
      <input
        type="number"
        step="0.5"
        min="0"
        placeholder="—"
        value={tool.lengthMm ?? ''}
        title={t('tools.holder.overall_length.title')}
        onchange={(e) => {
          const v = (e.currentTarget as HTMLInputElement).value;
          onUpdateField('lengthMm', v === '' ? undefined : parseFloat(v));
        }}
      />
    </label>
    <label>
      <span>{t('tools.holder.shank_diameter')}</span>
      <input
        type="number"
        step="0.1"
        min="0"
        placeholder={t('tools.holder.shank_diameter.placeholder')}
        value={tool.shankDiameterMm ?? ''}
        title={t('tools.holder.shank_diameter.title')}
        onchange={(e) => {
          const v = (e.currentTarget as HTMLInputElement).value;
          onUpdateField('shankDiameterMm', v === '' ? undefined : parseFloat(v));
        }}
      />
    </label>
    <label>
      <span>{t('tools.holder.stickout')}</span>
      <input
        type="number"
        step="0.5"
        min="0"
        placeholder="—"
        value={tool.stickoutLengthMm ?? ''}
        title={t('tools.holder.stickout.title')}
        onchange={(e) => {
          const v = (e.currentTarget as HTMLInputElement).value;
          onUpdateField('stickoutLengthMm', v === '' ? undefined : parseFloat(v));
        }}
      />
    </label>
    <label>
      <span>{t('tools.holder.preset')}</span>
      <select
        title={t('tools.holder.preset.title')}
        onchange={(e) => {
          const sel = e.currentTarget as HTMLSelectElement;
          if (sel.value) {
            onApplyPreset(sel.value);
            sel.value = '';
          }
        }}
      >
        <option value="">{t('tools.holder.preset.apply')}</option>
        {#each HOLDER_PRESETS as p (p.label)}
          <option value={p.label}>{p.label}</option>
        {/each}
      </select>
    </label>
  </div>
  <ToolHolderEditor holder={tool.holder} groupId={tool.id} onChange={(h) => onSetHolder(h)} />
  <div class="holder-row pass-overrides">
    <span class="holder-label" title={t('tools.pass_overrides.title')}
      >{t('tools.pass_overrides')}</span
    >
  </div>
  <div class="holder-row">
    <label>
      <span>{t('tools.finish.rpm')}</span>
      <input
        type="number"
        step="500"
        min="0"
        placeholder={String(tool.speed)}
        value={tool.speedFinish ?? ''}
        onchange={(e) => {
          const v = (e.currentTarget as HTMLInputElement).value;
          onUpdateField('speedFinish', v === '' ? undefined : parseInt(v, 10));
        }}
      />
    </label>
    <label>
      <span>{t('tools.finish.feed')}</span>
      <input
        type="number"
        step="50"
        min="0"
        placeholder={String(tool.feedRate)}
        value={tool.feedRateFinish ?? ''}
        onchange={(e) => {
          const v = (e.currentTarget as HTMLInputElement).value;
          onUpdateField('feedRateFinish', v === '' ? undefined : parseInt(v, 10));
        }}
      />
    </label>
    <label>
      <span>{t('tools.finish.plunge')}</span>
      <input
        type="number"
        step="50"
        min="0"
        placeholder={String(tool.plungeRate)}
        value={tool.plungeRateFinish ?? ''}
        onchange={(e) => {
          const v = (e.currentTarget as HTMLInputElement).value;
          onUpdateField('plungeRateFinish', v === '' ? undefined : parseInt(v, 10));
        }}
      />
    </label>
  </div>
  <div class="holder-row">
    <label>
      <span>{t('tools.drill.rpm')}</span>
      <input
        type="number"
        step="500"
        min="0"
        placeholder={String(tool.speed)}
        value={tool.speedDrill ?? ''}
        onchange={(e) => {
          const v = (e.currentTarget as HTMLInputElement).value;
          onUpdateField('speedDrill', v === '' ? undefined : parseInt(v, 10));
        }}
      />
    </label>
    <label>
      <span>{t('tools.drill.feed')}</span>
      <input
        type="number"
        step="50"
        min="0"
        placeholder={String(tool.feedRate)}
        value={tool.feedRateDrill ?? ''}
        onchange={(e) => {
          const v = (e.currentTarget as HTMLInputElement).value;
          onUpdateField('feedRateDrill', v === '' ? undefined : parseInt(v, 10));
        }}
      />
    </label>
    <label>
      <span>{t('tools.drill.plunge')}</span>
      <input
        type="number"
        step="50"
        min="0"
        placeholder={String(tool.plungeRate)}
        value={tool.plungeRateDrill ?? ''}
        onchange={(e) => {
          const v = (e.currentTarget as HTMLInputElement).value;
          onUpdateField('plungeRateDrill', v === '' ? undefined : parseInt(v, 10));
        }}
      />
    </label>
    <label>
      <span>{t('tools.drill.peck')}</span>
      <input
        type="number"
        step="0.1"
        min="0"
        placeholder="—"
        value={tool.defaultPeckStepMm ?? ''}
        title={t('tools.drill.peck.title')}
        onchange={(e) => {
          const v = (e.currentTarget as HTMLInputElement).value;
          onUpdateField('defaultPeckStepMm', v === '' ? undefined : parseFloat(v));
        }}
      />
    </label>
    <label>
      <span>{t('tools.drill.xy_overlap')}</span>
      <input
        type="number"
        step="0.05"
        min="0.05"
        max="0.95"
        placeholder="0.5"
        value={tool.defaultXyOverlap ?? ''}
        title={t('tools.drill.xy_overlap.title')}
        onchange={(e) => {
          const v = (e.currentTarget as HTMLInputElement).value;
          if (v === '') {
            onUpdateField('defaultXyOverlap', undefined);
            return;
          }
          const n = parseFloat(v);
          onUpdateField('defaultXyOverlap', isNaN(n) ? undefined : n);
        }}
      />
    </label>
  </div>
  <div class="holder-row">
    <label class="comment-row">
      <span>{t('tools.comment')}</span>
      <textarea
        rows="2"
        value={tool.comment ?? ''}
        placeholder={t('tools.comment.placeholder')}
        title={t('tools.comment.title')}
        onchange={(e) => {
          const v = (e.currentTarget as HTMLTextAreaElement).value;
          onUpdateField('comment', v === '' ? undefined : v);
        }}
      ></textarea>
    </label>
  </div>
  <div class="holder-row">
    <label>
      <span>{t('tools.z_shift')}</span>
      <input
        type="number"
        step="0.01"
        placeholder="—"
        value={tool.zShiftMm ?? ''}
        title={t('tools.z_shift.title')}
        onchange={(e) => {
          const v = (e.currentTarget as HTMLInputElement).value;
          if (v === '') {
            onUpdateField('zShiftMm', undefined);
            return;
          }
          const n = parseFloat(v);
          onUpdateField('zShiftMm', isNaN(n) || n === 0 ? undefined : n);
        }}
      />
    </label>
    <label>
      <span>{t('tools.spindle_warmup')}</span>
      <input
        type="number"
        step="0.5"
        min="0"
        placeholder="1"
        value={tool.pause ?? ''}
        title={t('tools.spindle_warmup.title')}
        onchange={(e) => {
          const v = (e.currentTarget as HTMLInputElement).value;
          if (v === '') {
            onUpdateField('pause', undefined);
            return;
          }
          const n = parseFloat(v);
          onUpdateField('pause', isNaN(n) || n < 0 ? undefined : n);
        }}
      />
    </label>
    <fieldset
      class="spindle-dir"
      disabled={tool.kind === 'drag_knife' ||
        tool.kind === 'laser_beam' ||
        tool.kind === 'plasma_torch'}
      title={tool.kind === 'drag_knife'
        ? t('tools.spindle_dir.disabled.drag_knife')
        : tool.kind === 'laser_beam'
          ? t('tools.spindle_dir.disabled.laser')
          : tool.kind === 'plasma_torch'
            ? t('tools.spindle_dir.disabled.plasma')
            : t('tools.spindle_dir.title')}
    >
      <legend>{t('tools.spindle_dir')}</legend>
      <label class="radio">
        <input
          type="radio"
          name="spindle-dir-{tool.id}"
          value="cw"
          checked={(tool.spindleDirection ?? 'cw') === 'cw'}
          onchange={() => onUpdateField('spindleDirection', undefined)}
        />
        <span>{t('tools.spindle_dir.cw')}</span>
      </label>
      <label class="radio">
        <input
          type="radio"
          name="spindle-dir-{tool.id}"
          value="ccw"
          checked={tool.spindleDirection === 'ccw'}
          onchange={() => onUpdateField('spindleDirection', 'ccw')}
        />
        <span>{t('tools.spindle_dir.ccw')}</span>
      </label>
    </fieldset>
  </div>
  <div class="holder-row pass-overrides">
    <span class="holder-label" title={t('tools.whirl.title')}>{t('tools.whirl')}</span>
  </div>
  <div class="holder-row">
    <label class="radio">
      <input
        type="checkbox"
        checked={tool.whirl ?? false}
        onchange={(e) => onUpdateField('whirl', (e.currentTarget as HTMLInputElement).checked)}
      />
      <span>{t('tools.whirl.enable')}</span>
    </label>
    <label>
      <span>{t('tools.whirl.extra_width')}</span>
      <input
        type="number"
        step="0.1"
        min="0"
        placeholder="0"
        value={tool.whirlExtraWidthMm ?? ''}
        disabled={!tool.whirl}
        title={t('tools.whirl.extra_width.title')}
        onchange={(e) => {
          const v = (e.currentTarget as HTMLInputElement).value;
          onUpdateField('whirlExtraWidthMm', v === '' ? undefined : parseFloat(v));
        }}
      />
    </label>
  </div>
  <div class="holder-row">
    <label>
      <span>{t('tools.whirl.stride')}</span>
      <input
        type="number"
        step="0.1"
        min="0.05"
        placeholder={((tool.whirlExtraWidthMm ?? 0) * 0.5).toFixed(2)}
        value={tool.whirlStepoverMm ?? ''}
        disabled={!tool.whirl}
        title={t('tools.whirl.stride.title')}
        onchange={(e) => {
          const v = (e.currentTarget as HTMLInputElement).value;
          onUpdateField('whirlStepoverMm', v === '' ? undefined : parseFloat(v));
        }}
      />
    </label>
    <label>
      <span>{t('tools.whirl.z_wobble')}</span>
      <input
        type="number"
        step="0.05"
        min="0"
        placeholder="0"
        value={tool.whirlOscMm ?? ''}
        disabled={!tool.whirl}
        title={t('tools.whirl.z_wobble.title')}
        onchange={(e) => {
          const v = (e.currentTarget as HTMLInputElement).value;
          onUpdateField('whirlOscMm', v === '' ? undefined : parseFloat(v));
        }}
      />
    </label>
  </div>
  {#if attrApplies('dragoff', tool.kind)}
    <div class="holder-row pass-overrides">
      <span class="holder-label" title={t('tools.drag.title')}>{t('tools.drag')}</span>
    </div>
    <div class="holder-row">
      <label>
        <span>{t('tools.drag.offset')}</span>
        <input
          type="number"
          step="0.05"
          min="0"
          placeholder="—"
          value={tool.dragoff ?? ''}
          title={t('tools.drag.offset.title')}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLInputElement).value;
            onUpdateField('dragoff', v === '' ? undefined : parseFloat(v));
          }}
        />
      </label>
      <label>
        <span>{t('tools.drag.self_align')}</span>
        <input
          type="number"
          step="1"
          min="0"
          max="60"
          placeholder="30"
          value={tool.dragKnifeSelfAlignAngleDeg ?? ''}
          title={t('tools.drag.self_align.title')}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLInputElement).value;
            onUpdateField('dragKnifeSelfAlignAngleDeg', v === '' ? undefined : parseFloat(v));
          }}
        />
      </label>
    </div>
  {/if}
  {#if attrApplies('compressionTransition', tool.kind)}
    <div class="holder-row pass-overrides">
      <span class="holder-label" title={t('tools.compression.title')}>{t('tools.compression')}</span
      >
    </div>
    <div class="holder-row">
      <label>
        <span>{t('tools.compression.transition')}</span>
        <input
          type="number"
          step="0.5"
          min="0"
          placeholder={t('tools.compression.transition.placeholder')}
          value={tool.compressionTransitionMm ?? ''}
          title={t('tools.compression.transition.title')}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLInputElement).value;
            onUpdateField('compressionTransitionMm', v === '' ? undefined : parseFloat(v));
          }}
        />
      </label>
    </div>
  {/if}
  {#if attrApplies('threadPitch', tool.kind)}
    <div class="holder-row pass-overrides">
      <span class="holder-label" title={t('tools.thread.title')}>{t('tools.thread')}</span>
    </div>
    <div class="holder-row">
      <label>
        <span>{t('tools.thread.pitch')}</span>
        <input
          type="number"
          step="0.05"
          min="0"
          placeholder="—"
          value={tool.threadPitchMm ?? ''}
          title={t('tools.thread.pitch.title')}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLInputElement).value;
            onUpdateField('threadPitchMm', v === '' ? undefined : parseFloat(v));
          }}
        />
      </label>
    </div>
  {/if}
  {#if attrApplies('cornerRadius', tool.kind)}
    <div class="holder-row pass-overrides">
      <span class="holder-label" title={t('tools.bullnose.title')}>{t('tools.bullnose')}</span>
    </div>
    <div class="holder-row">
      <label>
        <span>{t('tools.bullnose.corner_radius')}</span>
        <input
          type="number"
          step="0.05"
          min="0"
          placeholder="—"
          value={tool.cornerRadiusMm ?? ''}
          title={t('tools.bullnose.corner_radius.title')}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLInputElement).value;
            onUpdateField('cornerRadiusMm', v === '' ? undefined : parseFloat(v));
          }}
        />
      </label>
    </div>
  {/if}
  {#if attrApplies('formProfile', tool.kind)}
    <ToolFormProfileEditor
      rows={tool.formProfileMm ?? []}
      diameterMm={tool.diameter}
      onChange={(next) => onUpdateField('formProfileMm', next)}
    />
  {/if}
  {#if attrApplies('wear', tool.kind)}
    <div class="holder-row pass-overrides">
      <span class="holder-label" title={t('tools.wear.title')}>{t('tools.wear')}</span>
    </div>
    <div class="holder-row">
      <label>
        <span>{t('tools.wear.offset')}</span>
        <input
          type="number"
          step="0.01"
          placeholder="0"
          value={tool.wearOffsetMm ?? ''}
          title={t('tools.wear.offset.title')}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLInputElement).value;
            if (v === '') {
              onUpdateField('wearOffsetMm', undefined);
              return;
            }
            const n = parseFloat(v);
            onUpdateField('wearOffsetMm', isNaN(n) || n === 0 ? undefined : n);
          }}
        />
      </label>
      <button
        type="button"
        class="profile-btn"
        onclick={() => onCalibrate()}
        title={t('tools.wear.calibrate.title')}>{t('tools.wear.calibrate')}</button
      >
      <span class="cal-status">
        {#if tool.lastCalibrated}
          {t('tools.wear.last_calibrated', { date: tool.lastCalibrated })}
          {#if isCalibrationStale(tool.lastCalibrated, new Date())}
            <span class="stale-chip" title={t('tools.wear.stale.title')}
              >{t('tools.wear.stale')}</span
            >
          {/if}
        {:else}
          {t('tools.wear.never')}
        {/if}
      </span>
      {#if (tool.wearOffsetMm ?? 0) !== 0}
        <span class="eff-hint"
          >{t('tools.wear.cuts_as', { diameter: effectiveDiameterHint(tool) })}</span
        >
      {/if}
    </div>
  {/if}
  {#if attrApplies('laser', tool.kind)}
    <div class="holder-row pass-overrides">
      <span class="holder-label" title={t('tools.laser.title')}>{t('tools.laser')}</span>
    </div>
    <div class="holder-row">
      <label>
        <span>{t('tools.laser.pierce_time')}</span>
        <input
          type="number"
          step="0.05"
          min="0"
          placeholder="—"
          value={tool.laserPierceSec ?? ''}
          title={t('tools.laser.pierce_time.title')}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLInputElement).value;
            onUpdateField('laserPierceSec', v === '' ? undefined : parseFloat(v));
          }}
        />
      </label>
      <label>
        <span>{t('tools.laser.lead_in')}</span>
        <input
          type="number"
          step="0.1"
          min="0"
          placeholder="—"
          value={tool.laserLeadInMm ?? ''}
          title={t('tools.laser.lead_in.title')}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLInputElement).value;
            onUpdateField('laserLeadInMm', v === '' ? undefined : parseFloat(v));
          }}
        />
      </label>
      <label>
        <span>{t('tools.laser.kerf')}</span>
        <input
          type="number"
          step="0.01"
          min="0"
          placeholder="0.15"
          value={tool.kerfMm ?? ''}
          title={t('tools.laser.kerf.title')}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLInputElement).value;
            if (v === '') {
              onUpdateField('kerfMm', undefined);
              return;
            }
            const n = parseFloat(v);
            onUpdateField('kerfMm', isNaN(n) || n <= 0 ? undefined : n);
          }}
        />
      </label>
    </div>
  {/if}
  {#if attrApplies('plasma', tool.kind)}
    <div class="holder-row pass-overrides">
      <span class="holder-label" title={t('tools.plasma.title')}>{t('tools.plasma')}</span>
    </div>
    <div class="holder-row">
      <label>
        <span>{t('tools.plasma.pierce_height')}</span>
        <input
          type="number"
          step="0.1"
          min="0"
          placeholder="3.8"
          value={tool.pierceHeightMm ?? ''}
          title={t('tools.plasma.pierce_height.title')}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLInputElement).value;
            onUpdateField('pierceHeightMm', v === '' ? undefined : parseFloat(v));
          }}
        />
      </label>
      <label>
        <span>{t('tools.plasma.cut_height')}</span>
        <input
          type="number"
          step="0.1"
          min="0"
          placeholder="1.5"
          value={tool.cutHeightMm ?? ''}
          title={t('tools.plasma.cut_height.title')}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLInputElement).value;
            onUpdateField('cutHeightMm', v === '' ? undefined : parseFloat(v));
          }}
        />
      </label>
      <label>
        <span>{t('tools.plasma.pierce_delay')}</span>
        <input
          type="number"
          step="0.1"
          min="0"
          placeholder="0.5"
          value={tool.pierceDelaySec ?? ''}
          title={t('tools.plasma.pierce_delay.title')}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLInputElement).value;
            onUpdateField('pierceDelaySec', v === '' ? undefined : parseFloat(v));
          }}
        />
      </label>
      <label>
        <span>{t('tools.plasma.kerf')}</span>
        <input
          type="number"
          step="0.1"
          min="0"
          placeholder="—"
          value={tool.kerfMm ?? ''}
          title={t('tools.plasma.kerf.title')}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLInputElement).value;
            if (v === '') {
              onUpdateField('kerfMm', undefined);
              return;
            }
            const n = parseFloat(v);
            onUpdateField('kerfMm', isNaN(n) || n <= 0 ? undefined : n);
          }}
        />
      </label>
    </div>
  {/if}
</div>

<style>
  .holder-panel {
    grid-column: 1 / -1;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0.45rem 0.6rem;
    margin: 0.25rem 0 0.5rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
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
  .holder-row .holder-label {
    color: var(--text-muted);
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .holder-row fieldset.spindle-dir {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0.1rem 0.4rem 0.15rem;
    margin: 0;
    min-width: 0;
  }
  .holder-row fieldset.spindle-dir legend {
    color: var(--text-muted);
    font-size: 0.62rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 0 0.2rem;
  }
  .holder-row fieldset.spindle-dir label.radio {
    min-width: auto;
    flex-direction: row;
  }
  .holder-row fieldset.spindle-dir[disabled] {
    opacity: 0.4;
  }
  .comment-row {
    flex: 1;
    display: flex;
    align-items: flex-start;
    gap: 0.5rem;
  }
  .comment-row textarea {
    flex: 1;
    resize: vertical;
  }
  .cap-chip {
    display: inline-block;
    padding: 0.05rem 0.45rem;
    border: 1px solid var(--border);
    border-radius: 9px;
    background: var(--bg-elevated);
    color: var(--text);
    font-size: 0.7rem;
    align-self: center;
  }
  .cal-status {
    color: var(--text-muted);
    font-size: 0.74rem;
    align-self: center;
  }
  .stale-chip {
    display: inline-block;
    margin-left: 0.35rem;
    padding: 0.05rem 0.4rem;
    border-radius: 3px;
    background: color-mix(in srgb, #e6a700 22%, var(--bg-elevated));
    color: var(--text-strong);
    font-size: 0.7rem;
  }
  .eff-hint {
    color: var(--text-muted);
    font-size: 0.74rem;
    font-style: italic;
    align-self: center;
  }
  .profile-btn {
    background: var(--bg-elevated);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0.2rem 0.5rem;
    font-size: 0.72rem;
    cursor: pointer;
    align-self: flex-end;
  }
</style>
