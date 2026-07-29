<script lang="ts">
  // `.cps` post picker + auto-generated properties form. Mounted by the
  // machine dialog when the dialect is `cps`; edits flow back through
  // the dialog's draft (`onchange`) so undo/cancel behave like every
  // other machine setting.
  //
  // A bundled post is referenced by id (its script ships in the
  // binary); an opened file embeds its SCRIPT so a saved project stays
  // self-contained. `properties` stays SPARSE — only values differing
  // from the post's own defaults are stored, so a post updating its
  // defaults doesn't leave stale values behind.

  import { t } from '../i18n/i18n.svelte';
  import { defaultClient } from '../api/http';
  import type { CpsPostConfig } from '../state/project-types';
  import type { PostListEntry, PostMeta } from '../api/types';
  import { formControls, sparseProperties, type PropertyControl } from './cps-post-form';

  let {
    value,
    onchange,
  }: {
    value: CpsPostConfig | undefined;
    onchange: (next: CpsPostConfig | undefined) => void;
  } = $props();

  const client = defaultClient();

  let bundled = $state<PostListEntry[]>([]);
  let meta = $state<PostMeta | null>(null);
  let loadError = $state<string | null>(null);
  let busy = $state(false);

  // Load the bundled library once; a transport without CPS support
  // simply reports the error (the option is hidden upstream anyway).
  $effect(() => {
    let cancelled = false;
    void (async () => {
      if (!client.listPosts) {
        loadError = t('machine.cps.unsupported');
        return;
      }
      try {
        const list = await client.listPosts();
        if (!cancelled) bundled = list;
      } catch (e) {
        if (!cancelled) loadError = e instanceof Error ? e.message : String(e);
      }
    })();
    return () => {
      cancelled = true;
    };
  });

  /// Resolve the metadata for the current selection: a bundled post's
  /// metadata rides the listing; a file's comes from inspecting its
  /// embedded script.
  $effect(() => {
    const selection = value;
    let cancelled = false;
    void (async () => {
      if (!selection) {
        meta = null;
        return;
      }
      if (selection.source === 'bundled') {
        meta = bundled.find((e) => e.id === selection.bundledId)?.meta ?? null;
        return;
      }
      if (!selection.script || !client.inspectPost) {
        meta = null;
        return;
      }
      try {
        const inspected = await client.inspectPost(selection.script, selection.filename);
        if (!cancelled) meta = inspected;
      } catch (e) {
        if (!cancelled) {
          meta = null;
          loadError = e instanceof Error ? e.message : String(e);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  });

  const controls = $derived<PropertyControl[]>(
    meta ? formControls(meta, value?.properties ?? {}) : [],
  );

  function selectBundled(id: string) {
    loadError = null;
    onchange(id ? { source: 'bundled', bundledId: id, properties: {} } : undefined);
  }

  async function openFile() {
    if (!client.inspectPost) return;
    loadError = null;
    busy = true;
    try {
      const opened = await pickCpsFile();
      if (!opened) return;
      // Inspect before storing so a broken script surfaces its parse
      // error here instead of at generate time.
      const inspected = await client.inspectPost(opened.script, opened.filename);
      meta = inspected;
      onchange({
        source: 'file',
        filename: opened.filename,
        script: opened.script,
        properties: {},
      });
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  /// Tauri gets a native dialog; the browser falls back to a hidden
  /// `<input type=file>`.
  async function pickCpsFile(): Promise<{ filename: string; script: string } | null> {
    const { isTauri } = await import('../api/env');
    if (isTauri()) {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const path = await open({
        multiple: false,
        filters: [{ name: 'CPS post', extensions: ['cps'] }],
      });
      if (typeof path !== 'string') return null;
      const { readTextFile } = await import('@tauri-apps/plugin-fs');
      const script = await readTextFile(path);
      return { filename: path.split(/[\\/]/).pop() ?? 'post.cps', script };
    }
    return new Promise((resolve) => {
      const input = document.createElement('input');
      input.type = 'file';
      input.accept = '.cps,text/plain';
      input.onchange = () => {
        const file = input.files?.[0];
        if (!file) {
          resolve(null);
          return;
        }
        void file.text().then((script) => resolve({ filename: file.name, script }));
      };
      input.click();
    });
  }

  function setProperty(name: string, next: boolean | number | string) {
    if (!value || !meta) return;
    onchange({
      ...value,
      properties: sparseProperties(meta, { ...value.properties, [name]: next }),
    });
  }
</script>

<div class="cps">
  <label title={t('machine.cps.post.title')}>
    {t('machine.cps.post')}
    <span class="field">
      <select
        value={value?.source === 'bundled' ? (value.bundledId ?? '') : ''}
        onchange={(e) => selectBundled((e.currentTarget as HTMLSelectElement).value)}
      >
        <option value="">{t('machine.cps.post.none')}</option>
        {#each bundled as entry (entry.id)}
          <option value={entry.id}>{entry.meta.description || entry.id}</option>
        {/each}
      </select>
    </span>
  </label>

  <div class="row">
    <button type="button" onclick={openFile} disabled={busy}>
      {t('machine.cps.open_file')}
    </button>
    {#if value?.source === 'file'}
      <span class="filename" title={value.filename}>{value.filename}</span>
    {/if}
  </div>

  {#if loadError}
    <p class="err">{loadError}</p>
  {/if}

  {#if meta}
    <p class="meta">
      {meta.description}{meta.vendor ? ` — ${meta.vendor}` : ''} · .{meta.extension}
    </p>
    {#if controls.length > 0}
      <div class="section-title">{t('machine.cps.properties')}</div>
      {#each controls as control (control.name)}
        <label title={control.description}>
          {control.title}
          <span class="field">
            {#if control.kind === 'bool'}
              <input
                type="checkbox"
                checked={control.value as boolean}
                onchange={(e) =>
                  setProperty(control.name, (e.currentTarget as HTMLInputElement).checked)}
              />
            {:else if control.kind === 'enum'}
              <select
                value={String(control.value)}
                onchange={(e) =>
                  setProperty(control.name, (e.currentTarget as HTMLSelectElement).value)}
              >
                {#each control.values ?? [] as option (option.id)}
                  <option value={option.id}>{option.title}</option>
                {/each}
              </select>
            {:else if control.kind === 'number' || control.kind === 'integer'}
              <input
                type="number"
                step={control.kind === 'integer' ? 1 : 'any'}
                value={control.value as number}
                onchange={(e) => {
                  const raw = (e.currentTarget as HTMLInputElement).value;
                  const parsed = control.kind === 'integer' ? parseInt(raw, 10) : parseFloat(raw);
                  if (Number.isFinite(parsed)) setProperty(control.name, parsed);
                }}
              />
            {:else}
              <input
                type="text"
                value={String(control.value)}
                onchange={(e) =>
                  setProperty(control.name, (e.currentTarget as HTMLInputElement).value)}
              />
            {/if}
          </span>
        </label>
      {/each}
    {/if}
  {/if}
</div>

<style>
  .cps {
    display: contents;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .filename {
    font-size: 0.85em;
    opacity: 0.8;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    margin: 0;
    font-size: 0.85em;
    opacity: 0.8;
  }
  .err {
    margin: 0;
    color: var(--color-danger, #c33);
    font-size: 0.85em;
  }
</style>
