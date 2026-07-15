/// Pure rendering helper for GUI-surfaced `PipelineWarning`s — kept rune-free
/// so it unit-tests under the logic-only vitest config. Takes the `translate`
/// function as a parameter (the component passes `t`), mirroring
/// `error-display.ts`.
///
/// The localization seam: a backend `PipelineWarning` carries a stable `kind`
/// (the language-agnostic code) + structured `params` (see
/// crates/ivac-core/src/pipeline.rs). We render `warn.<kind>` against `params`
/// when that key exists in the catalog; otherwise we fall back to the English
/// `message` the backend always supplies.
///
/// Unlike `WiacError.code` (an `Option`), `PipelineWarning.kind` is always
/// present, so there is no null signal telling us whether a template exists.
/// We therefore membership-test `warn.<kind>` against the generated `MSG_KEYS`
/// set and fall back to `message` for any kind that has no template yet — so a
/// not-yet-localized warning renders its English text rather than a raw key.
import type { PipelineWarning } from '../api/types';
import { MSG_KEYS, type MsgKey } from '../i18n/keys';

export type Translate = (key: MsgKey, params?: Record<string, string | number>) => string;

/// Every catalog key, for O(1) membership tests. Built once at module load.
const KNOWN_KEYS: ReadonlySet<string> = new Set(MSG_KEYS);

/// Does a `warn.<kind>` template exist in the catalog?
export function hasWarningTemplate(kind: string): boolean {
  return KNOWN_KEYS.has(`warn.${kind}`);
}

/// The localized (or English-fallback) text for a pipeline warning.
//
// The `t(`warn.${…}`)` call is written inline (not via a `key` variable) so
// the i18n dead-key scanner (`dynamicKeyPrefixes`) registers `warn.` as a live
// runtime prefix — otherwise every `warn.<kind>` catalog entry reads as dead.
// Mirrors `error-display.ts`'s inline `t(`error.code.${…}`)`.
export function warningMessage(w: PipelineWarning, t: Translate): string {
  if (!KNOWN_KEYS.has(`warn.${w.kind}`)) return w.message;
  return t(`warn.${w.kind}` as MsgKey, w.params ?? {});
}
