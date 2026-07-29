// Pure export-shape decisions for the G-code save path. Rune-free so
// they unit-test under the logic-only vitest config; `file_ops.ts` binds
// them to the live project's machine settings.

export type DialectId = 'linuxcnc' | 'grbl' | 'hpgl' | 'cps';

/// Extension precedence: what the POST declared (`.cps` posts carry
/// their own `extension`) → the machine's PostProfile `file_extension`
/// → the per-dialect default.
///
/// The middle rung fixes a long-standing bug: the PostProcessorEditor
/// let users set `file_extension` and the exporter ignored it, always
/// writing `.ngc` / `.plt`.
export function resolveExportExtension(
  dialect: DialectId,
  declaredByPost?: string,
  profileExtension?: string,
): string {
  const declared = normalizeExtension(declaredByPost);
  if (declared) return declared;
  const profile = normalizeExtension(profileExtension);
  if (profile) return profile;
  return dialect === 'hpgl' ? 'plt' : 'ngc';
}

function normalizeExtension(raw?: string): string | null {
  const trimmed = raw?.trim().replace(/^\.+/, '');
  return trimmed ? trimmed : null;
}

/// Apply a PostProfile `line_ending` at WRITE time. The in-memory
/// buffer stays LF so the G-code panel's line numbers and the response
/// line index remain valid; only the bytes hitting disk change.
///
/// Accepts the literal (`"\r\n"`) and the friendly spellings
/// (`crlf`/`CRLF`, `lf`/`LF`) the profile editor may hold.
export function applyProfileLineEnding(gcode: string, lineEnding?: string): string {
  if (!lineEnding) return gcode;
  const eol = canonicalEol(lineEnding);
  if (eol === '\n') return gcode;
  return gcode.replace(/\r?\n/g, eol);
}

function canonicalEol(lineEnding: string): string {
  const key = lineEnding.toLowerCase();
  if (key === 'crlf' || lineEnding === '\r\n') return '\r\n';
  if (key === 'lf' || lineEnding === '\n') return '\n';
  if (key === 'cr' || lineEnding === '\r') return '\r';
  // An unrecognized spelling is used verbatim — the profile owner knows
  // their controller better than this table does.
  return lineEnding;
}
