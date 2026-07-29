//! Rust ↔ JS constant-table consistency: every encoding the recorder
//! stamps into IR code fields (`ivac_cps::ir::codes`) must equal the
//! prelude's constant of the same name — posts compare by NAME in JS,
//! the recorder by value in Rust, and this test is the weld between
//! them. The full-table symbol audit (every `globals.d.ts` name
//! defined) lands with cps.10.

use ivac_cps::engine::Engine;
use ivac_cps::ir::codes;

macro_rules! table {
    ($($name:ident),* $(,)?) => {
        &[$((stringify!($name), codes::$name)),*]
    };
}

#[test]
fn recorder_codes_match_prelude_constants() {
    let expected: &[(&str, u32)] = table![
        MOVEMENT_RAPID,
        MOVEMENT_LEAD_IN,
        MOVEMENT_CUTTING,
        MOVEMENT_LEAD_OUT,
        MOVEMENT_LINK_TRANSITION,
        MOVEMENT_LINK_DIRECT,
        MOVEMENT_RAMP_HELIX,
        MOVEMENT_RAMP_PROFILE,
        MOVEMENT_RAMP_ZIG_ZAG,
        MOVEMENT_RAMP,
        MOVEMENT_PLUNGE,
        MOVEMENT_PREDRILL,
        COOLANT_DISABLED,
        COOLANT_FLOOD,
        COOLANT_MIST,
        COMMAND_STOP,
        COMMAND_OPTIONAL_STOP,
        COMMAND_END,
        COMMAND_SPINDLE_CLOCKWISE,
        COMMAND_SPINDLE_COUNTERCLOCKWISE,
        COMMAND_START_SPINDLE,
        COMMAND_STOP_SPINDLE,
        COMMAND_COOLANT_ON,
        COMMAND_COOLANT_OFF,
        TOOL_UNSPECIFIED,
        TOOL_DRILL,
        TOOL_MILLING_END_FLAT,
        TOOL_MILLING_END_BALL,
        TOOL_MILLING_END_BULLNOSE,
        TOOL_MILLING_CHAMFER,
        TOOL_MILLING_TAPERED,
        TOOL_MILLING_FORM,
        TOOL_MILLING_THREAD,
        TOOL_LASER_CUTTER,
        TOOL_PLASMA_CUTTER,
        TOOL_MARKER,
    ];

    let mut engine = Engine::new();
    engine.eval_prelude().expect("prelude");
    for (name, rust_value) in expected {
        let js = engine
            .eval_named("probe.js", name)
            .unwrap_or_else(|e| panic!("prelude must define {name}: {e}"));
        let number = js
            .as_number()
            .unwrap_or_else(|| panic!("{name} is not numeric in JS: {js:?}"));
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let js_value = number as u32;
        assert_eq!(
            js_value, *rust_value,
            "{name}: prelude has {js_value}, ir::codes has {rust_value}"
        );
    }
}
