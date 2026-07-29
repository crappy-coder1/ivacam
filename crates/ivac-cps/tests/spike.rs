//! cps.0 spike checklist — one test per engine capability the `.cps`
//! runtime depends on. Each failure here is a polyfill obligation for
//! `prelude/00_polyfill.js` (cps.3), so keep the cases minimal and
//! self-describing.

use boa_engine::{JsString, JsValue, NativeFunction};
use ivac_cps::engine::Engine;

fn eval_str(engine: &mut Engine, src: &str) -> String {
    engine
        .eval_named("spike.js", src)
        .expect("eval should succeed")
        .to_string(engine.context_mut())
        .expect("to_string")
        .to_std_string_escaped()
}

/// Annex B: `String.prototype.substr` — used pervasively by vendor
/// posts (e.g. FANUC program-name handling).
#[test]
fn substr_annex_b() {
    let mut e = Engine::new();
    assert_eq!(eval_str(&mut e, r#""hello world".substr(6, 5)"#), "world");
    assert_eq!(eval_str(&mut e, r#""hello world".substr(-5)"#), "world");
    assert_eq!(eval_str(&mut e, r#""hello".substr(1)"#), "ello");
}

/// RegExp literals AND the `new RegExp` constructor (regress engine).
#[test]
fn regexp_literal_and_constructor() {
    let mut e = Engine::new();
    assert_eq!(eval_str(&mut e, r#"/G(\d+)/.exec("G01 X5")[1]"#), "01");
    assert_eq!(
        eval_str(
            &mut e,
            r#"String(new RegExp("^O[0-9]{1,4}$").test("O1234"))"#
        ),
        "true"
    );
    // Trailing-zero trim, the FormatNumber `trim` shape.
    assert_eq!(eval_str(&mut e, r#""1.500".replace(/\.?0+$/, "")"#), "1.5");
}

/// `JsValue::from_json` / `to_json` round-trip of a nested object —
/// the IR injection path.
#[test]
fn json_round_trip() {
    let mut e = Engine::new();
    let input = serde_json::json!({
        "version": 1,
        "header": { "programName": "1001", "toleranceMm": 0.01 },
        "sections": [
            { "id": 7, "records": [ { "kind": "linear", "x": 1.5, "y": -0.25, "z": null } ] }
        ],
        "flags": [true, false],
    });
    let js = JsValue::from_json(&input, e.context_mut()).expect("from_json");
    let back = js
        .to_json(e.context_mut())
        .expect("to_json")
        .expect("non-undefined");
    assert_eq!(back, input);
}

/// Named `Source` — syntax AND runtime errors must be attributable to
/// a source name so prelude bugs and user-post bugs are separable.
#[test]
fn named_source_errors_carry_line_info() {
    let mut e = Engine::new();
    // Syntax error on line 3.
    // Parse errors carry line+col but NOT the source name — the
    // caller knows which file it just evaluated, so diag mapping
    // (cps.4) attaches the name itself.
    let syn = e
        .eval_named("broken.cps", "var a = 1;\nvar b = 2;\nvar c = ;\n")
        .expect_err("must fail to parse");
    let msg = syn.to_string();
    assert!(msg.contains("line 3"), "expected 'line 3' in: {msg}");

    // Runtime errors DO carry the source name in the stack frame:
    //   "TypeError: not a callable function\n    at <main> (runtime.cps:2:12)"
    let rt = e
        .eval_named("runtime.cps", "var obj = {};\nobj.missing();\n")
        .expect_err("must throw");
    let msg = rt.to_string();
    assert!(
        msg.contains("runtime.cps:2"),
        "expected 'runtime.cps:2' stack frame in: {msg}"
    );
}

/// `Object.defineProperty` getters/setters — the Section/Tool lazy
/// wrappers (prelude 08) are built from these.
#[test]
fn define_property_accessors() {
    let mut e = Engine::new();
    let out = eval_str(
        &mut e,
        r"
        var store = { hits: 0 };
        var section = {};
        Object.defineProperty(section, 'workOffset', {
            get: function () { store.hits += 1; return 54; },
            set: function (v) { store.set = v; },
        });
        var a = section.workOffset;
        section.workOffset = 55;
        String(a) + ':' + String(store.hits) + ':' + String(store.set)
        ",
    );
    assert_eq!(out, "54:1:55");
}

/// `arguments` object semantics — `writeBlock` passes `arguments`
/// through to `formatWords`, which flattens array-likes recursively.
#[test]
fn arguments_object_semantics() {
    let mut e = Engine::new();
    let out = eval_str(
        &mut e,
        r"
        function flatten(args) {
            var parts = [];
            for (var i = 0; i < args.length; ++i) {
                parts.push(String(args[i]));
            }
            return parts.join(' ');
        }
        function writeBlock() {
            return arguments.length + '|' + arguments[1] + '|' + flatten(arguments);
        }
        writeBlock('G1', 'X10.5', 'F500')
        ",
    );
    assert_eq!(out, "3|X10.5|G1 X10.5 F500");
}

/// Closure-capturing NativeFunction on a global object — `__ivac.emit`
/// collects into a Rust-side sink.
#[test]
fn native_function_sink() {
    let mut e = Engine::new();
    e.eval_named(
        "emit.js",
        r#"__ivac.emit("line one"); __ivac.emit("line two");"#,
    )
    .expect("eval");
    assert_eq!(
        e.output(),
        vec!["line one".to_owned(), "line two".to_owned()]
    );
}

/// Posts monkey-patch host-installed globals (`setProperty` in the
/// FANUC post) — a script overwrite must win.
#[test]
fn script_can_overwrite_host_global() {
    let mut e = Engine::new();
    let host_fn = NativeFunction::from_copy_closure(|_this, _args, _ctx| {
        Ok(JsValue::from(JsString::from("host")))
    });
    let ctx = e.context_mut();
    let host_fn = host_fn.to_js_function(ctx.realm());
    ctx.register_global_property(
        JsString::from("setProperty"),
        host_fn,
        boa_engine::property::Attribute::all(),
    )
    .expect("register");

    assert_eq!(eval_str(&mut e, "setProperty()"), "host");
    let out = eval_str(
        &mut e,
        r"
        setProperty = function () { return 'patched'; };
        setProperty()
        ",
    );
    assert_eq!(out, "patched");
}

/// KNOWN boa 0.21.1 BUG (workaround pinned): a `new C()` executed
/// during the top-level run of the same script that assigned
/// `C.prototype.m = function () {…}` members yields the LAST assigned
/// method instead of the instance. The prelude works around it by
/// never instantiating during such a script's own top level
/// (09_machine.js declares, 15_driver.js instantiates). This test
/// documents the bug shape; when an engine upgrade makes the first
/// assertion fail, the bug is fixed upstream — drop the workaround
/// and this test together.
#[test]
fn boa_new_after_prototype_assignment_bug() {
    let mut e = Engine::new();
    e.eval_named(
        "bug.js",
        "function C() { this.v = 1; }\nC.prototype.m = function () { return 7; };\nvar sameScript = new C();",
    )
    .expect("eval");
    e.eval_named("later.js", "var laterScript = new C();")
        .expect("eval");
    let same = eval_str(&mut e, "typeof sameScript");
    let later = eval_str(&mut e, "typeof laterScript + ':' + laterScript.m()");
    assert_eq!(
        same, "function",
        "boa fixed the same-script new-after-prototype bug — remove the 09_machine.js workaround"
    );
    assert_eq!(later, "object:7", "the cross-script path must stay correct");
}

/// Work item 3: minimal end-to-end — a ~20-line inline post defines
/// `onOpen`/`onLinear`, Rust drives them through `call_global`, output
/// lands in the sink.
#[test]
fn minimal_post_end_to_end() {
    let mut e = Engine::new();
    e.eval_named(
        "mini.cps",
        r"
        var programName = 'SPIKE';
        function fmt(v) {
            var s = v.toFixed(3);
            return s;
        }
        function onOpen() {
            __ivac.emit('%');
            __ivac.emit('(' + programName + ')');
            __ivac.emit('G90 G21');
        }
        function onLinear(x, y, z, feed) {
            __ivac.emit('G1 X' + fmt(x) + ' Y' + fmt(y) + ' Z' + fmt(z) + ' F' + feed);
        }
        ",
    )
    .expect("post top level evals");

    e.call_global("onOpen", &[]).expect("onOpen");
    e.call_global(
        "onLinear",
        &[
            JsValue::from(10.5),
            JsValue::from(-0.25),
            JsValue::from(2.0),
            JsValue::from(500),
        ],
    )
    .expect("onLinear");

    assert_eq!(
        e.output(),
        vec![
            "%".to_owned(),
            "(SPIKE)".to_owned(),
            "G90 G21".to_owned(),
            "G1 X10.500 Y-0.250 Z2.000 F500".to_owned(),
        ]
    );
}
