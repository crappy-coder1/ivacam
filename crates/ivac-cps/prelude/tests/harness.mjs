// harness.mjs — node-side prelude runtime for the test suites and the
// V8↔boa differential fixtures. Zero npm dependencies (node >= 20).
//
// Mirrors the boa engine setup (crates/ivac-cps/src/engine.rs): a bare
// context with the `__ivac` host object, prelude files evaluated in
// numeric order with their file names as source names.

import { readFileSync, readdirSync } from "node:fs";
import { createContext, runInContext } from "node:vm";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const testsDir = dirname(fileURLToPath(import.meta.url));
const preludeDir = join(testsDir, "..");
export const fixturesDir = join(testsDir, "fixtures");

export function preludeFiles() {
  return readdirSync(preludeDir)
    .filter((f) => /^\d\d_.*\.js$/.test(f))
    .sort();
}

export function createRuntime() {
  const chunks = [];
  const sandbox = {
    __ivac: {
      emit: (text) => chunks.push(String(text)),
      log: () => {},
      localize: (s) => String(s),
    },
    __ivacTestOutput: () => chunks.join(""),
    __ivacTestReset: () => {
      chunks.length = 0;
    },
  };
  const ctx = createContext(sandbox);
  for (const file of preludeFiles()) {
    runInContext(readFileSync(join(preludeDir, file), "utf8"), ctx, {
      filename: file,
    });
  }
  runInContext(readFileSync(join(testsDir, "case_runner.js"), "utf8"), ctx, {
    filename: "case_runner.js",
  });
  return {
    eval: (src, name = "inline.js") => runInContext(src, ctx, { filename: name }),
    output: () => chunks.join(""),
    reset: () => {
      chunks.length = 0;
    },
  };
}

export function readFixture(name) {
  return readFileSync(join(fixturesDir, name), "utf8");
}

/** Run the differential case tables exactly like tests/differential.rs. */
export function computeExpected() {
  const rt = createRuntime();
  const formatCases = readFixture("format_cases.json");
  const scenarios = readFixture("scenarios.json");
  const formats = JSON.parse(
    rt.eval(`__runFormatCases(${JSON.stringify(formatCases)})`)
  );
  const scenarioResults = JSON.parse(
    rt.eval(`__runScenarios(${JSON.stringify(scenarios)})`)
  );
  return { formats, scenarios: scenarioResults };
}
