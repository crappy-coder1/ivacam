/// Component-size regrowth guard (ivac-3xwn).
///
/// The ifx1 epic decomposed the frontend god-components; months later two of
/// them (App.svelte, EntityCanvas2D.svelte) had silently REGROWN past their
/// post-decomposition size because new interaction/layout logic had no owning
/// module and piled back onto the template. The scene3d/* builder classes held
/// precisely because growth had somewhere else to go. This guard is the
/// tripwire the ifx1 re-decomposition (ivac-3xwn) added "so it stops
/// recurring": it fails the build when a `.svelte` component or `.svelte.ts`
/// runes module grows past its budget, forcing a conscious choice — extract to
/// a module (preferred), or, if the growth is genuinely warranted, bump the
/// number here in a reviewable one-line diff.
///
/// Runs under the standard `pnpm run test` (vitest) step, so it guards every
/// CI run and `scripts/pre-release.sh` alongside the i18n + codegen guards.
///
/// Scope is deliberately the REACTIVE layer — `.svelte` templates and
/// `.svelte.ts` rune modules — where "logic with no home" accretes. Pure `.ts`
/// modules are the extraction TARGET, not the problem, so they're not capped.
import { describe, it, expect } from 'vitest';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, resolve, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const componentsDir = resolve(fileURLToPath(import.meta.url), '..'); // …/lib/components
const srcDir = resolve(componentsDir, '..', '..'); // …/frontend/src

/// Any capped file with no explicit budget below must stay under this. It sits
/// well above every healthy component today, so it only trips when a genuinely
/// NEW god-component emerges — the exact failure mode this guard exists for.
const DEFAULT_MAX_LINES = 1000;

/// Per-file budgets (line count ceiling), keyed by path relative to
/// `frontend/src`, POSIX-separated. Each is the file's current size plus a
/// small headroom — TIGHT on purpose: the epic wants these to shrink, not to
/// license 20% regrowth. When you legitimately need more room, raise the number
/// here in the same commit; when a decomposition shrinks a file, ratchet it
/// DOWN so the win can't quietly erode. A stale entry (file gone, or now far
/// under budget) is itself flagged below.
const BUDGETS: Record<string, number> = {
  // Epic-tracked god-components under active decomposition (ivac-3xwn).
  'lib/components/EntityCanvas2D.svelte': 2290, // 2273 — ivac-1hxn: PointerDragController slice
  'App.svelte': 2200, // 2155 — ivac-3xwn.1
  'lib/state/project.svelte.ts': 1005, // 991 — machine + selection ops extracted to project-{machine,selection}-ops.ts
  // Large but stable / separately tracked components.
  'lib/components/ToolLibraryDialog.svelte': 1010, // 969 — u9iy: collapsed-row cells → ToolRowSummary
  'lib/components/Scene3D.svelte': 1450, // 1404 — held via scene3d/* builders
  'lib/components/MachineDialog.svelte': 1300, // 1239
  'lib/components/OpPropertiesPanel.svelte': 1250, // 1200
  'lib/components/GenerateBar.svelte': 1100, // 1060
};

/// How far under budget a file may sit before the entry is considered stale and
/// should be ratcheted down. Generous so a normal edit doesn't trip it — only a
/// real decomposition win (which SHOULD lower the ceiling) does.
const STALE_SLACK = 200;

/// Recursively collect capped files (`.svelte`, `.svelte.ts`) under `dir`,
/// returned as `{ rel, lines }` with POSIX-relative paths.
function cappedFiles(dir: string): { rel: string; lines: number }[] {
  const out: { rel: string; lines: number }[] = [];
  for (const name of readdirSync(dir)) {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) {
      out.push(...cappedFiles(full));
      continue;
    }
    if (!name.endsWith('.svelte') && !name.endsWith('.svelte.ts')) continue;
    const lines = readFileSync(full, 'utf8').split('\n').length;
    out.push({ rel: relative(srcDir, full).split(sep).join('/'), lines });
  }
  return out;
}

const files = cappedFiles(srcDir);

describe('component size regrowth guard (ivac-3xwn)', () => {
  it('finds the reactive files it is meant to guard', () => {
    // Sanity: the walk actually reaches src/ (guards against a path-resolution
    // regression silently making this test vacuous).
    expect(files.length).toBeGreaterThan(20);
    expect(files.some((f) => f.rel === 'lib/components/EntityCanvas2D.svelte')).toBe(true);
  });

  it('keeps every .svelte / .svelte.ts file within its line budget', () => {
    const over = files
      .map((f) => ({ ...f, cap: BUDGETS[f.rel] ?? DEFAULT_MAX_LINES }))
      .filter((f) => f.lines > f.cap)
      .map((f) => `${f.rel}: ${f.lines} > ${f.cap}`);
    expect(
      over,
      `\nComponent(s) over budget. Extract logic into a module (preferred — ` +
        `see lib/canvas/*, lib/scene3d/*, lib/state/*), or if the growth is ` +
        `warranted raise the budget in component-size-guard.test.ts:\n  ` +
        over.join('\n  '),
    ).toEqual([]);
  });

  it('has no stale budget entries (file gone, or now far under budget)', () => {
    const byRel = new Map(files.map((f) => [f.rel, f.lines]));
    const stale = Object.entries(BUDGETS).flatMap(([rel, cap]) => {
      const lines = byRel.get(rel);
      if (lines === undefined) return [`${rel}: budgeted but no longer exists — remove the entry`];
      if (cap - lines > STALE_SLACK)
        return [
          `${rel}: ${lines} lines but budget ${cap} — ratchet the budget down toward current`,
        ];
      return [];
    });
    expect(stale, `\nStale budget entries:\n  ${stale.join('\n  ')}`).toEqual([]);
  });
});
