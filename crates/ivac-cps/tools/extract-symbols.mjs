// Extract the `.cps` runtime's public symbol universe from Autodesk's
// published `globals.d.ts` declaration file.
//
//   node tools/extract-symbols.mjs [path/to/globals.d.ts] > symbols.json
//
// Output: `{ functions: [...], constants: [...], classes: [...],
// variables: [...] }`, each sorted. The committed
// `tests/fixtures/api_symbols.json` is this output; `tests/symbol_audit.rs`
// asserts the prelude DEFINES every entry (real implementation or
// warn-once stub), so a third-party post hitting an unimplemented
// corner gets a named diagnostic instead of a mystery TypeError.
//
// Refresh after a `refs/` update: re-run and commit the fixture, then
// let the audit tell you what's newly missing.

import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const DEFAULT_PATH = join(
  here,
  '../../../refs/cam-posteditor/vs-code-extension/res/language files/globals.d.ts',
);

const path = process.argv[2] ?? DEFAULT_PATH;
const source = readFileSync(path, 'utf8');

const out = { functions: [], constants: [], classes: [], variables: [] };
const patterns = [
  [/^declare function\s+([A-Za-z_$][\w$]*)/, 'functions'],
  [/^declare const\s+([A-Za-z_$][\w$]*)/, 'constants'],
  [/^declare class\s+([A-Za-z_$][\w$]*)/, 'classes'],
  [/^declare (?:var|let)\s+([A-Za-z_$][\w$]*)/, 'variables'],
];

for (const line of source.split('\n')) {
  for (const [pattern, bucket] of patterns) {
    const m = pattern.exec(line);
    if (m) {
      out[bucket].push(m[1]);
      break;
    }
  }
}

for (const bucket of Object.keys(out)) {
  out[bucket] = [...new Set(out[bucket])].sort();
}

process.stdout.write(`${JSON.stringify(out, null, 1)}\n`);
