// case_runner.js — shared V8↔boa differential case executor.
//
// Evaluated AFTER the prelude by BOTH the node harness and the boa
// differential test (tests/differential.rs), so the exact same code
// path produces both sides of the comparison. Inputs and outputs are
// JSON strings to keep the host bindings trivial.

// Format matrix: [{name, spec, values: [..]}, ...] → flat list of
// formatted strings, one per (case, value), tagged for diffability.
function __runFormatCases(casesJson) {
  var cases = JSON.parse(casesJson);
  var out = [];
  for (var i = 0; i < cases.length; ++i) {
    var c = cases[i];
    var spec = {};
    for (var key in c.spec) {
      if (Object.prototype.hasOwnProperty.call(c.spec, key)) {
        spec[key] = c.spec[key];
      }
    }
    // Symbolic scales that must be computed in-engine, not baked into
    // JSON (DEG is 180/π in the engine's own doubles).
    if (spec.scale === "DEG") {
      spec.scale = DEG;
    }
    var fmt = createFormat(spec);
    for (var j = 0; j < c.values.length; ++j) {
      out.push(c.name + "(" + c.values[j] + ")=" + fmt.format(c.values[j]));
    }
  }
  return JSON.stringify(out);
}

// Scenario list: [{name, body}] where body is a function body returning
// a string — covers variables/modals/words behavior end to end. Output
// state (emit sink, word separator, EOL) is reset before each scenario;
// the harness provides `__ivacTestReset`/`__ivacTestOutput` over its
// emit sink.
function __runScenarios(scenariosJson) {
  var scenarios = JSON.parse(scenariosJson);
  var out = [];
  for (var i = 0; i < scenarios.length; ++i) {
    var s = scenarios[i];
    __ivacTestReset();
    setWordSeparator(" ");
    setEOL("\n");
    /* eslint-disable no-new-func */
    var fn = new Function(s.body);
    out.push(s.name + "=" + String(fn()));
  }
  return JSON.stringify(out);
}
