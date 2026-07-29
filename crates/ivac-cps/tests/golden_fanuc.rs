//! FANUC golden fixtures — the acceptance oracle for the program model
//! + dispatch driver (cps.4): the REAL Autodesk FANUC post from refs/
//! runs over hand-built IR programs and must produce byte-stable,
//! hand-sanity-checked NC output.
//!
//! The refs directory is Autodesk-copyrighted test material: these
//! tests SKIP (with a notice) when it is absent, and nothing from it
//! is embedded in this repository.
//!
//! **Updating baselines:** `IVAC_UPDATE_SNAPSHOTS=1 cargo test -p
//! ivac-cps --test golden_fanuc` prints paste-ready literals (same
//! convention as ivac-core's snapshot suites).

use ivac_cps::ir::{
    codes, Header, ParamValue, Parameter, Position, Program, Record, Section, ToolSpec, IR_VERSION,
};
use ivac_cps::{inspect_post, run_post, PostError};

const FANUC_RELATIVE: &str = "../../refs/cam-posteditor/src/post-parser/test/test.cps";

fn fanuc_source() -> Option<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(FANUC_RELATIVE);
    match std::fs::read_to_string(&path) {
        Ok(source) => Some(source),
        Err(_) => {
            eprintln!("SKIP: FANUC oracle not present at {}", path.display());
            None
        }
    }
}

fn assert_snapshot(name: &str, actual: &str, expected: &str) {
    if std::env::var("IVAC_UPDATE_SNAPSHOTS").is_ok() {
        eprintln!("=== UPDATE SNAPSHOT [{name}] ===");
        eprintln!("let expected = \"\\");
        for line in actual.lines() {
            let escaped = line.replace('\\', "\\\\").replace('"', "\\\"");
            eprintln!("{escaped}\\n\\");
        }
        eprintln!("\";");
        panic!("IVAC_UPDATE_SNAPSHOTS set — paste the new baseline and rerun without it.");
    }
    assert_eq!(
        actual, expected,
        "[{name}] snapshot drift; rerun with IVAC_UPDATE_SNAPSHOTS=1 to refresh"
    );
}

fn flat_endmill(number: u32) -> ToolSpec {
    ToolSpec {
        number,
        description: format!("{number}mm flat"),
        diameter: f64::from(number),
        corner_radius: 0.0,
        taper_angle: 0.0,
        flutes: 2,
        tool_type: codes::TOOL_MILLING_END_FLAT,
        coolant: codes::COOLANT_DISABLED,
    }
}

fn section(id: u32, name: &str, tool: ToolSpec, rpm: f64, records: Vec<Record>) -> Section {
    let initial = Position {
        x: 0.0,
        y: 0.0,
        z: 15.0,
    };
    Section {
        id,
        strategy: "contour2d".into(),
        tool,
        spindle_rpm: rpm,
        spindle_clockwise: true,
        work_offset: 1,
        work_plane: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        parameters: vec![
            Parameter {
                name: "operation-comment".into(),
                value: ParamValue::Text(name.into()),
            },
            Parameter {
                name: "operation:tool_feedCutting".into(),
                value: ParamValue::Number(800.0),
            },
            Parameter {
                name: "operation:tool_feedPlunge".into(),
                value: ParamValue::Number(300.0),
            },
        ],
        initial_position: initial,
        final_position: initial,
        records,
    }
}

fn program(sections: Vec<Section>) -> Program {
    Program {
        version: IR_VERSION,
        header: Header {
            program_name: "1001".into(),
            program_comment: "ivac golden".into(),
            tolerance_mm: 0.01,
            parameters: vec![],
        },
        sections,
        machine: None,
    }
}

/// Fixture (a): two sections, linear + arc motion, tool change between.
fn fixture_a() -> Program {
    let cut = |x: f64, y: f64| Record::Linear {
        x,
        y,
        z: -1.0,
        feed: 800.0,
        movement: codes::MOVEMENT_CUTTING,
    };
    let first = vec![
        Record::Rapid {
            x: 10.0,
            y: 5.0,
            z: 15.0,
        },
        Record::Rapid {
            x: 10.0,
            y: 5.0,
            z: 2.0,
        },
        Record::Linear {
            x: 10.0,
            y: 5.0,
            z: -1.0,
            feed: 300.0,
            movement: codes::MOVEMENT_PLUNGE,
        },
        cut(30.0, 5.0),
        // Quarter arc CCW around (30, 10): (30,5) → (35,10).
        Record::Circular {
            clockwise: false,
            center: Position {
                x: 30.0,
                y: 10.0,
                z: -1.0,
            },
            end: Position {
                x: 35.0,
                y: 10.0,
                z: -1.0,
            },
            feed: 800.0,
            normal: [0.0, 0.0, 1.0],
        },
        cut(35.0, 25.0),
        Record::Rapid {
            x: 35.0,
            y: 25.0,
            z: 15.0,
        },
    ];
    let second = vec![
        Record::Rapid {
            x: -4.0,
            y: 0.0,
            z: 15.0,
        },
        Record::Rapid {
            x: -4.0,
            y: 0.0,
            z: 2.0,
        },
        Record::Linear {
            x: -4.0,
            y: 0.0,
            z: -0.5,
            feed: 300.0,
            movement: codes::MOVEMENT_PLUNGE,
        },
        // Clockwise half circle around (0,0): (-4,0) → (4,0).
        Record::Circular {
            clockwise: true,
            center: Position {
                x: 0.0,
                y: 0.0,
                z: -0.5,
            },
            end: Position {
                x: 4.0,
                y: 0.0,
                z: -0.5,
            },
            feed: 640.0,
            normal: [0.0, 0.0, 1.0],
        },
        Record::Rapid {
            x: 4.0,
            y: 0.0,
            z: 15.0,
        },
    ];
    let mut s1 = section(1, "Profile outer", flat_endmill(6), 18000.0, first);
    s1.final_position = Position {
        x: 35.0,
        y: 25.0,
        z: 15.0,
    };
    let mut s2 = section(2, "Slot arc", flat_endmill(3), 24000.0, second);
    s2.initial_position = Position {
        x: 35.0,
        y: 25.0,
        z: 15.0,
    };
    s2.final_position = Position {
        x: 4.0,
        y: 0.0,
        z: 15.0,
    };
    program(vec![s1, s2])
}

/// Fixture (c): full circle + helical arc (one tool).
fn fixture_c() -> Program {
    let records = vec![
        Record::Rapid {
            x: 10.0,
            y: 0.0,
            z: 15.0,
        },
        Record::Rapid {
            x: 10.0,
            y: 0.0,
            z: 2.0,
        },
        // Helical entry: one CCW full turn around (0,0) descending 2 → -2
        // (start == end in XY ⇒ full-circle sweep; the kernel splits it
        // per the post's maximumCircularSweep).
        Record::Circular {
            clockwise: false,
            center: Position {
                x: 0.0,
                y: 0.0,
                z: 2.0,
            },
            end: Position {
                x: 10.0,
                y: 0.0,
                z: -2.0,
            },
            feed: 400.0,
            normal: [0.0, 0.0, 1.0],
        },
        // Flat full circle at depth.
        Record::Circular {
            clockwise: false,
            center: Position {
                x: 0.0,
                y: 0.0,
                z: -2.0,
            },
            end: Position {
                x: 10.0,
                y: 0.0,
                z: -2.0,
            },
            feed: 800.0,
            normal: [0.0, 0.0, 1.0],
        },
        Record::Rapid {
            x: 10.0,
            y: 0.0,
            z: 15.0,
        },
    ];
    let mut s = section(1, "Helix bore", flat_endmill(6), 18000.0, records);
    s.initial_position = Position {
        x: 0.0,
        y: 0.0,
        z: 15.0,
    };
    s.final_position = Position {
        x: 10.0,
        y: 0.0,
        z: 15.0,
    };
    program(vec![s])
}

fn no_overrides() -> serde_json::Value {
    serde_json::json!({})
}

#[test]
fn fixture_a_two_sections_toolchange() {
    let Some(source) = fanuc_source() else { return };
    let out = run_post(&source, "fanuc.cps", &fixture_a(), &no_overrides())
        .expect("FANUC post must run fixture (a)");
    assert_eq!(out.extension, "nc");
    assert_eq!(out.unit, "mm");
    let expected = "\
%\n\
O1001 (IVAC GOLDEN)\n\
(T6 D=6. CR=0. - ZMIN=-1. - FLAT END MILL)\n\
(T3 D=3. CR=0. - ZMIN=-0.5 - FLAT END MILL)\n\
N10 G90 G94 G17 G49 G40 G80\n\
N15 G21\n\
N20 G28 G91 Z0.\n\
N25 G90\n\
\n\
(PROFILE OUTER)\n\
N30 T6 M06\n\
N35 T3\n\
N40 S18000 M03\n\
N45 G54\n\
N50 G00 X0. Y0.\n\
N55 G43 Z15. H06\n\
N60 G00 X10. Y5.\n\
N65 Z2.\n\
N70 G01 Z-1. F300.\n\
N75 X30. F800.\n\
N80 G03 X35. Y10. J5.\n\
N85 G01 Y25.\n\
N90 G00 Z15.\n\
N95 M05\n\
N100 G28 G91 Z0.\n\
N105 G90\n\
N110 G49\n\
\n\
(SLOT ARC)\n\
N115 M01\n\
N120 T3 M06\n\
N125 T6\n\
N130 S24000 M03\n\
N135 G54\n\
N140 G00 X35. Y25.\n\
N145 G43 Z15. H03\n\
N150 G00 X-4. Y0.\n\
N155 Z2.\n\
N160 G01 Z-0.5 F300.\n\
N165 G02 X4. I4. F640.\n\
N170 G00 Z15.\n\
\n\
N175 G28 G91 Z0.\n\
N180 G90\n\
N185 G49\n\
N190 G28 G91 X0. Y0.\n\
N195 G90\n\
N200 M30\n\
%\n\
";
    assert_snapshot("fanuc_fixture_a", &out.text, expected);
}

#[test]
fn fixture_c_full_circle_and_helix() {
    let Some(source) = fanuc_source() else { return };
    let out = run_post(&source, "fanuc.cps", &fixture_c(), &no_overrides())
        .expect("FANUC post must run fixture (c)");
    let expected = "\
%\n\
O1001 (IVAC GOLDEN)\n\
(T6 D=6. CR=0. - ZMIN=2. - FLAT END MILL)\n\
N10 G90 G94 G17 G49 G40 G80\n\
N15 G21\n\
N20 G28 G91 Z0.\n\
N25 G90\n\
\n\
(HELIX BORE)\n\
N30 T6 M06\n\
N35 S18000 M03\n\
N40 G54\n\
N45 G00 X0. Y0.\n\
N50 G43 Z15. H06\n\
N55 G00 X10.\n\
N60 Z2.\n\
N65 G03 X-10. Z0. I-10. F400.\n\
N70 X10. Z-2. I10.\n\
N75 X-10. I-10. F800.\n\
N80 X10. I10.\n\
N85 G00 Z15.\n\
\n\
N90 G28 G91 Z0.\n\
N95 G90\n\
N100 G49\n\
N105 G28 G91 X0. Y0.\n\
N110 G90\n\
N115 M30\n\
%\n\
";
    assert_snapshot("fanuc_fixture_c", &out.text, expected);
}

/// Property override changes output: useRadius → R-word arcs instead of
/// IJK (and full circles get linearized per the post's own logic).
#[test]
fn fixture_c_use_radius_override() {
    let Some(source) = fanuc_source() else { return };
    let out = run_post(
        &source,
        "fanuc.cps",
        &fixture_c(),
        &serde_json::json!({"useRadius": true}),
    )
    .expect("FANUC post must run fixture (c) with useRadius");
    assert!(
        !out.text.contains(" I") || out.text.contains("R"),
        "useRadius must switch arcs away from IJK-only output:\n{}",
        out.text
    );
    let expected = "\
%\n\
O1001 (IVAC GOLDEN)\n\
(T6 D=6. CR=0. - ZMIN=2. - FLAT END MILL)\n\
N10 G90 G94 G17 G49 G40 G80\n\
N15 G21\n\
N20 G28 G91 Z0.\n\
N25 G90\n\
\n\
(HELIX BORE)\n\
N30 T6 M06\n\
N35 S18000 M03\n\
N40 G54\n\
N45 G00 X0. Y0.\n\
N50 G43 Z15. H06\n\
N55 G00 X10.\n\
N60 Z2.\n\
N65 G03 X0. Y10. Z1. R10. F400.\n\
N70 X-10. Y0. Z0. R10.\n\
N75 X0. Y-10. Z-1. R10.\n\
N80 X10. Y0. Z-2. R10.\n\
N85 X0. Y10. R10. F800.\n\
N90 X-10. Y0. R10.\n\
N95 X0. Y-10. R10.\n\
N100 X10. Y0. R10.\n\
N105 G00 Z15.\n\
\n\
N110 G28 G91 Z0.\n\
N115 G90\n\
N120 G49\n\
N125 G28 G91 X0. Y0.\n\
N130 G90\n\
N135 M30\n\
%\n\
";
    assert_snapshot("fanuc_fixture_c_radius", &out.text, expected);
}

/// Fixture (b): drill section exercising G81/G82/G83/G73 plus one
/// forced expansion (chip-breaking with dwell — the FANUC post expands
/// that combination itself).
fn fixture_b() -> Program {
    let drill = ToolSpec {
        number: 5,
        description: "5mm drill".into(),
        diameter: 5.0,
        corner_radius: 0.0,
        taper_angle: 0.0,
        flutes: 2,
        tool_type: codes::TOOL_DRILL,
        coolant: codes::COOLANT_DISABLED,
    };
    let params = |extra: &[(&str, f64)]| {
        let mut m = std::collections::BTreeMap::from([
            ("clearance".to_string(), 2.0),
            ("retract".to_string(), 2.0),
            ("feedrate".to_string(), 120.0),
        ]);
        for (k, v) in extra {
            m.insert((*k).to_string(), *v);
        }
        m
    };
    let at = |x: f64, z: f64| Position { x, y: 0.0, z };
    let records = vec![
        Record::Cycle {
            cycle_type: "drilling".into(),
            params: params(&[("bottom", -5.0), ("depth", 7.0)]),
            points: vec![at(10.0, -5.0), at(20.0, -5.0), at(30.0, -5.0)],
        },
        Record::CycleEnd,
        Record::Cycle {
            cycle_type: "counter-boring".into(),
            params: params(&[("bottom", -3.0), ("depth", 5.0), ("dwell", 0.5)]),
            points: vec![at(40.0, -3.0)],
        },
        Record::CycleEnd,
        Record::Cycle {
            cycle_type: "deep-drilling".into(),
            params: params(&[
                ("bottom", -12.0),
                ("depth", 14.0),
                ("incrementalDepth", 3.0),
            ]),
            points: vec![at(50.0, -12.0), at(60.0, -12.0)],
        },
        Record::CycleEnd,
        Record::Cycle {
            cycle_type: "chip-breaking".into(),
            params: params(&[
                ("bottom", -12.0),
                ("depth", 14.0),
                ("incrementalDepth", 3.0),
                ("chipBreakDistance", 0.5),
                ("accumulatedDepth", 14.0),
            ]),
            points: vec![at(70.0, -12.0)],
        },
        Record::CycleEnd,
        // Forced expansion: chip-breaking WITH dwell — FANUC calls
        // expandCyclePoint for it.
        Record::Cycle {
            cycle_type: "chip-breaking".into(),
            params: params(&[
                ("bottom", -6.0),
                ("depth", 8.0),
                ("incrementalDepth", 3.0),
                ("chipBreakDistance", 0.5),
                ("accumulatedDepth", 8.0),
                ("dwell", 0.3),
            ]),
            points: vec![at(80.0, -6.0)],
        },
        Record::CycleEnd,
    ];
    let mut s = section(1, "Drill pattern", drill, 9000.0, records);
    s.strategy = "drill".into();
    s.final_position = Position {
        x: 80.0,
        y: 0.0,
        z: 2.0,
    };
    program(vec![s])
}

#[test]
fn fixture_b_canned_cycles() {
    let Some(source) = fanuc_source() else { return };
    let out = run_post(&source, "fanuc.cps", &fixture_b(), &no_overrides())
        .expect("FANUC post must run fixture (b)");
    let expected = "\
%\n\
O1001 (IVAC GOLDEN)\n\
(T5 D=5. CR=0. - ZMIN=-12. - DRILL)\n\
N10 G90 G94 G17 G49 G40 G80\n\
N15 G21\n\
N20 G28 G91 Z0.\n\
N25 G90\n\
\n\
(DRILL PATTERN)\n\
N30 T5 M06\n\
N35 S9000 M03\n\
N40 G54\n\
N45 G00 X0. Y0.\n\
N50 G43 Z15. H05\n\
N55 G98 G81 X10. Y0. Z-5. R2. F120.\n\
N60 X20.\n\
N65 X30.\n\
N70 G80\n\
N75 G82 X40. Y0. Z-3. R2. P500\n\
N80 G80\n\
N85 G83 X50. Y0. Z-12. R2. Q3.\n\
N90 X60.\n\
N95 G80\n\
N100 G73 X70. Y0. Z-12. R2. Q3.\n\
N105 G80\n\
N110 G00 X80. Z2.\n\
N115 G01 Z-1. F120.\n\
N120 G04 P300\n\
N125 G00 Z-0.5\n\
N130 G01 Z-1. F120.\n\
N135 Z-4.\n\
N140 G04 P300\n\
N145 G00 Z-3.5\n\
N150 G01 Z-4. F120.\n\
N155 Z-6.\n\
N160 G04 P300\n\
N165 G00 Z2.\n\
\n\
N170 G28 G91 Z0.\n\
N175 G90\n\
N180 G49\n\
N185 G28 G91 X0. Y0.\n\
N190 G90\n\
N195 M30\n\
%\n\
";
    assert_snapshot("fanuc_fixture_b", &out.text, expected);
}

#[test]
fn inspect_post_returns_fanuc_property_sheet() {
    let Some(source) = fanuc_source() else { return };
    let meta = inspect_post(&source, "fanuc.cps").expect("inspect");
    assert_eq!(meta.description, "FANUC");
    assert_eq!(meta.vendor, "Fanuc");
    assert_eq!(meta.extension, "nc");
    assert!(meta.capabilities & 1 != 0, "CAPABILITY_MILLING bit");
    let names: Vec<&str> = meta.properties.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"useRadius"), "got: {names:?}");
    assert!(names.contains(&"sequenceNumberStart"));
    let use_radius = meta
        .properties
        .iter()
        .find(|p| p.name == "useRadius")
        .unwrap();
    assert_eq!(use_radius.title, "Radius arcs");
    assert!(matches!(
        use_radius.kind,
        ivac_cps::meta::PropertyKind::Bool
    ));
}

// ---- error paths ----

/// A spin loop hits the loop-iteration budget instead of hanging the
/// worker (server-grade protection on by default).
#[test]
fn runaway_script_hits_budget() {
    let script = "function onOpen() { while (true) {} }";
    let err = ivac_cps::run_post_with_limits(
        script,
        "spin.cps",
        &fixture_c(),
        &no_overrides(),
        ivac_cps::RunLimits {
            loop_iterations: 10_000,
            recursion: 512,
        },
    )
    .expect_err("must exceed the budget");
    assert!(matches!(err, PostError::BudgetExceeded(_)), "got: {err:?}");
}

#[test]
fn syntax_error_maps_to_parse_with_line() {
    let program = fixture_c();
    let err =
        run_post("var a = ;\n", "broken.cps", &program, &no_overrides()).expect_err("must fail");
    match err {
        PostError::Parse {
            source_name,
            message,
        } => {
            assert_eq!(source_name, "broken.cps");
            assert!(message.contains("line 1"), "got: {message}");
        }
        other => panic!("expected Parse, got {other:?}"),
    }
}

#[test]
fn error_call_halts_with_partial_output() {
    let script = r#"
description = "halter";
function onOpen() {
  writeln("HEADER");
}
function onSection() {
  error("deliberate halt");
}
function onRapid() { writeln("RAPID"); }
"#;
    let err = run_post(script, "halter.cps", &fixture_c(), &no_overrides()).expect_err("halted");
    match err {
        PostError::PostErrorCall {
            messages,
            partial_output,
            ..
        } => {
            assert_eq!(messages, vec!["deliberate halt".to_string()]);
            assert!(partial_output.contains("HEADER"));
            assert!(
                !partial_output.contains("RAPID"),
                "dispatch must halt after error()"
            );
        }
        other => panic!("expected PostErrorCall, got {other:?}"),
    }
}

#[test]
fn validate_throw_maps_to_runtime_error() {
    let script = r#"
function onOpen() {
  validate(false, "must be true");
}
"#;
    let err = run_post(script, "thrower.cps", &fixture_c(), &no_overrides()).expect_err("throws");
    match err {
        PostError::PostRuntime { message } => {
            assert!(message.contains("must be true"), "got: {message}");
        }
        other => panic!("expected PostRuntime, got {other:?}"),
    }
}

#[test]
fn ir_version_gate() {
    let mut bad = fixture_c();
    bad.version = 999;
    let err = run_post("function onOpen(){}", "x.cps", &bad, &no_overrides()).expect_err("gate");
    assert!(matches!(err, PostError::IrVersionMismatch(_)));
}
