//! V8 ↔ boa differential: the SAME case tables and case-runner JS that
//! the node harness executes (prelude/tests/) run here under boa, and
//! the results must be byte-identical to the committed
//! `*_expected.json` files (which `node prelude/tests/run.mjs --update`
//! generates under V8). Any divergence is an engine-semantics gap that
//! would silently corrupt NC output — a polyfill obligation, not a
//! test to loosen.

use boa_engine::{JsString, JsValue, NativeFunction};
use ivac_cps::engine::Engine;

const CASE_RUNNER: &str = include_str!("../prelude/tests/case_runner.js");
const FORMAT_CASES: &str = include_str!("../prelude/tests/fixtures/format_cases.json");
const FORMAT_EXPECTED: &str = include_str!("../prelude/tests/fixtures/format_expected.json");
const SCENARIOS: &str = include_str!("../prelude/tests/fixtures/scenarios.json");
const SCENARIOS_EXPECTED: &str = include_str!("../prelude/tests/fixtures/scenarios_expected.json");

/// Prelude + case runner + the `__ivacTest*` helpers the node harness
/// provides (mirrored here over the engine's emit sink).
fn runtime() -> Engine {
    let mut engine = Engine::new();
    engine.eval_prelude().expect("prelude must eval");

    let sink = engine.sink();
    let output = NativeFunction::from_copy_closure_with_captures(
        |_this, _args, sink, _ctx| {
            Ok(JsValue::from(JsString::from(
                sink.borrow().concat().as_str(),
            )))
        },
        sink.clone(),
    );
    let reset = NativeFunction::from_copy_closure_with_captures(
        |_this, _args, sink, _ctx| {
            sink.borrow_mut().clear();
            Ok(JsValue::undefined())
        },
        sink,
    );
    let ctx = engine.context_mut();
    ctx.register_global_callable(JsString::from("__ivacTestOutput"), 0, output)
        .expect("register __ivacTestOutput");
    ctx.register_global_callable(JsString::from("__ivacTestReset"), 0, reset)
        .expect("register __ivacTestReset");

    engine
        .eval_named("case_runner.js", CASE_RUNNER)
        .expect("case runner must eval");
    engine
}

fn run_cases(engine: &mut Engine, function: &str, cases_json: &str) -> Vec<String> {
    let result = engine
        .call_global(function, &[JsValue::from(JsString::from(cases_json))])
        .expect("case run must succeed");
    let json = result
        .to_string(engine.context_mut())
        .expect("string result")
        .to_std_string_escaped();
    serde_json::from_str(&json).expect("valid JSON result")
}

fn assert_matches_expected(actual: &[String], expected_json: &str, what: &str) {
    let expected: Vec<String> = serde_json::from_str(expected_json).expect("valid expected JSON");
    assert_eq!(
        actual.len(),
        expected.len(),
        "{what}: case count drifted — regenerate fixtures"
    );
    let mut diffs = Vec::new();
    for (a, e) in actual.iter().zip(&expected) {
        if a != e {
            diffs.push(format!("  boa: {a}\n  v8:  {e}"));
        }
    }
    assert!(
        diffs.is_empty(),
        "{what}: {} divergence(s) between boa and V8:\n{}",
        diffs.len(),
        diffs.join("\n")
    );
}

/// Every formatted value in the matrix is byte-identical to V8's.
#[test]
fn format_matrix_matches_v8() {
    let mut engine = runtime();
    let actual = run_cases(&mut engine, "__runFormatCases", FORMAT_CASES);
    assert_matches_expected(&actual, FORMAT_EXPECTED, "format matrix");
}

/// Variables/modals/words scenarios are byte-identical to V8's.
#[test]
fn scenarios_match_v8() {
    let mut engine = runtime();
    let actual = run_cases(&mut engine, "__runScenarios", SCENARIOS);
    assert_matches_expected(&actual, SCENARIOS_EXPECTED, "scenarios");
}

/// When node is available, run the V8 harness itself (verifies the
/// committed expected files are FRESH, then runs the node suites).
/// Skips cleanly on machines without node.
#[test]
fn node_harness_agrees() {
    if std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("node not on PATH — skipping the V8-side harness run");
        return;
    }
    let status = std::process::Command::new("node")
        .arg("prelude/tests/run.mjs")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .expect("spawn node");
    assert!(
        status.success(),
        "node prelude harness failed — see output above"
    );
}
