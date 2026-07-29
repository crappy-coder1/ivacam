// run.mjs — prelude test entry point.
//
//   node prelude/tests/run.mjs            verify committed differential
//                                         fixtures + run the suites
//   node prelude/tests/run.mjs --update   regenerate the *_expected.json
//                                         differential fixtures
//
// The *_expected.json files are the V8 half of the V8↔boa differential:
// tests/differential.rs replays the same case tables in boa and
// byte-compares. Regenerate them (and re-commit) whenever the case
// tables or the prelude semantics deliberately change.

import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname } from "node:path";
import { computeExpected, fixturesDir } from "./harness.mjs";

const testsDir = dirname(fileURLToPath(import.meta.url));
const update = process.argv.includes("--update");

const expected = computeExpected();
const files = {
  "format_expected.json": JSON.stringify(expected.formats, null, 1) + "\n",
  "scenarios_expected.json": JSON.stringify(expected.scenarios, null, 1) + "\n",
};

let ok = true;
for (const [name, text] of Object.entries(files)) {
  const path = join(fixturesDir, name);
  if (update) {
    writeFileSync(path, text);
    console.log(`updated ${name}`);
  } else {
    let committed = null;
    try {
      committed = readFileSync(path, "utf8");
    } catch {
      // missing counts as stale
    }
    if (committed !== text) {
      console.error(
        `STALE ${name} — run \`node prelude/tests/run.mjs --update\` and commit`
      );
      ok = false;
    } else {
      console.log(`fresh ${name}`);
    }
  }
}

if (!update) {
  const result = spawnSync(
    process.execPath,
    ["--test", join(testsDir, "*.test.mjs")],
    { stdio: "inherit" }
  );
  if (result.status !== 0) {
    ok = false;
  }
}

process.exit(ok ? 0 : 1);
