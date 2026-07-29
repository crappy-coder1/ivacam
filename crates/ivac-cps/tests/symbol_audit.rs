//! API symbol audit: every symbol Autodesk's `globals.d.ts` declares
//! must EXIST in the runtime — as a real implementation or as a
//! warn-once stub.
//!
//! Why this matters: a third-party post touching an unimplemented
//! corner should get a named diagnostic ("probing is not supported by
//! ivacam CPS v1") rather than a bare `TypeError: not a callable
//! function` that no field report can act on.
//!
//! The universe lives in `tests/fixtures/api_symbols.json`, generated
//! by `tools/extract-symbols.mjs` from the published declarations.
//! Refresh it after a `refs/` update and let this test name what's
//! newly missing.

use ivac_cps::engine::Engine;

#[derive(serde::Deserialize)]
struct SymbolUniverse {
    functions: Vec<String>,
    constants: Vec<String>,
    classes: Vec<String>,
    variables: Vec<String>,
}

const UNIVERSE: &str = include_str!("fixtures/api_symbols.json");

/// Entry points are the POST's to define — the kernel deliberately
/// leaves them undefined so `__ivacCallOptional`'s `typeof === "function"`
/// probe distinguishes "post handles this" from "post doesn't". Defining
/// no-op kernel versions would make a post's missing handler silently
/// swallow records instead of being skippable.
const ENTRY_POINTS: &[&str] = &[
    "onBedTemp",
    "onCircular",
    "onCircularExtrude",
    "onClose",
    "onCommand",
    "onComment",
    "onCyclePathEnd",
    "onCyclePoint",
    "onDwell",
    "onExpandedLinear",
    "onExtruderChange",
    "onJerk",
    "onLayerEnd",
    "onLinear",
    "onLinear5D",
    "onLiveAlignment",
    "onMachine",
    "onMachineCommand",
    "onManualNC",
    "onMaxAcceleration",
    "onMovement",
    "onOpen",
    "onOrientateSpindle",
    "onParameter",
    "onPassThrough",
    "onRadiusCompensation",
    "onRapid",
    "onRapid5D",
    "onReturnFromSafeRetractPosition",
    "onRewindMachineEntry",
    "onRotateAxes",
    "onSection",
    "onSectionEnd",
    "onSectionEndSpecialCycle",
    "onSpindleSpeed",
    "onTerminate",
    "onToolCompensation",
    "onCycle",
    "onCycleEnd",
    "onMoveToSafeRetractPosition",
];

/// Every declared symbol resolves to something other than `undefined`
/// after the prelude loads.
#[test]
fn every_declared_symbol_is_defined() {
    let universe: SymbolUniverse = serde_json::from_str(UNIVERSE).expect("valid fixture");
    let mut engine = Engine::new();
    engine.eval_prelude().expect("prelude");

    let mut missing: Vec<String> = Vec::new();
    let groups = [
        ("function", &universe.functions),
        ("constant", &universe.constants),
        ("class", &universe.classes),
        ("variable", &universe.variables),
    ];
    for (kind, names) in groups {
        for name in names {
            if ENTRY_POINTS.contains(&name.as_str()) {
                continue;
            }
            // Presence, not definedness: `var x;` (a declared global
            // awaiting its per-run value — `currentSection`, `tool`,
            // `cycle`, …) is PRESENT even though `typeof` says
            // "undefined", so fall back to an `in globalThis` probe.
            // `typeof` first because it never throws on an undeclared
            // identifier.
            let probe =
                format!("(typeof {name} !== \"undefined\") ? \"ok\" : ((\"{name}\" in globalThis) ? \"ok\" : \"undefined\")");
            let value = engine
                .eval_named("audit.js", &probe)
                .expect("typeof cannot throw");
            let text = value
                .to_string(engine.context_mut())
                .expect("string")
                .to_std_string_escaped();
            if text == "undefined" {
                missing.push(format!("{kind} {name}"));
            }
        }
    }

    assert!(
        missing.is_empty(),
        "{} of {} declared symbols are undefined in the prelude — implement them or add warn-once stubs in 14_stubs.js:\n{}",
        missing.len(),
        universe.functions.len()
            + universe.constants.len()
            + universe.classes.len()
            + universe.variables.len(),
        missing.join("\n")
    );
}

/// Symbols the FANUC oracle uses that the published declarations omit
/// (kernel API that exists in practice but isn't documented). These
/// must be present too — the goldens would break otherwise, but naming
/// them here documents the gap.
#[test]
fn undeclared_but_used_symbols_are_defined() {
    const UNDECLARED: &[&str] = &[
        "isProbeOperation",
        "getNextTool",
        "getCommandStringId",
        "onImpliedCommand",
        "repositionToCycleClearance",
        "isFirstCyclePoint",
        "isLastCyclePoint",
        "getGlobalParameter",
        "hasGlobalParameter",
        "invokeOnRapid",
        "invokeOnLinear",
        "invokeOnRapid5D",
        "invokeOnLinear5D",
        "isInspectionOperation",
        "writeSectionNotes",
        "getToolTypeName",
        "onUnsupportedCommand",
        "setCodePage",
        "isDrillingCycle",
        "isWellKnownCycle",
        "isExpanding",
    ];
    let mut engine = Engine::new();
    engine.eval_prelude().expect("prelude");
    let mut missing = Vec::new();
    for name in UNDECLARED {
        let value = engine
            .eval_named("audit.js", &format!("typeof {name}"))
            .expect("typeof cannot throw");
        if value
            .to_string(engine.context_mut())
            .expect("string")
            .to_std_string_escaped()
            == "undefined"
        {
            missing.push((*name).to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "undeclared-but-used symbols missing: {}",
        missing.join(", ")
    );
}
