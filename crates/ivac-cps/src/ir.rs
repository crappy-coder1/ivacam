//! Program IR — the recorder ↔ runtime contract.
//!
//! `CpsRecorder` (ivac-core, behind its `cps` feature) records the
//! pipeline's `PostProcessor` calls into this structure; the JS driver
//! (prelude `15_driver.js`) receives it as one JSON value and dispatches
//! it through the post's entry points. The shape is therefore a wire
//! format between two halves of this codebase — versioned via
//! [`IR_VERSION`], serialized camelCase, and kept free of boa types so
//! the module works in any build.
//!
//! Unit invariants (enforced by the recorder, assumed by the driver):
//! all distances are mm, feeds are mm/min, angles are radians. The JS
//! driver scales to the post's active `unit` at dispatch time.
//!
//! Numeric code fields (`movement`, `coolant`, `command`, `tool_type`)
//! carry the Autodesk constant encodings (`MOVEMENT_*`, `COOLANT_*`,
//! `COMMAND_*`, `TOOL_*`). The value tables land with
//! `prelude/01_constants.js` (cps.4), which adds a Rust↔JS consistency
//! test so the two sides cannot drift.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Version stamp checked on deserialize — bump on any breaking shape
/// change so a stale recorder and a newer driver fail loudly instead of
/// dispatching garbage.
pub const IR_VERSION: u32 = 1;

/// XYZ triple for direction vectors and work-plane rows.
pub type Vec3 = [f64; 3];

/// A whole recorded program: what the pipeline ran, sectioned per
/// operation, ready for one JS post run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Program {
    #[serde(deserialize_with = "check_ir_version")]
    pub version: u32,
    pub header: Header,
    pub sections: Vec<Section>,
    /// Machine kinematics for `MachineConfiguration` wiring (cps.6).
    /// `None` for plain 3-axis programs — posts then run their own
    /// `defineMachine` path, exactly like Fusion without a machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<MachineSpec>,
}

fn check_ir_version<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let version = u32::deserialize(deserializer)?;
    if version == IR_VERSION {
        Ok(version)
    } else {
        Err(serde::de::Error::custom(format!(
            "program IR version {version} is not the supported {IR_VERSION}"
        )))
    }
}

/// Program-level metadata the driver exposes before the first section
/// (`programName`, `programComment`, kernel `tolerance`, and the global
/// parameter stream fired through `onParameter` after `onOpen`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Header {
    pub program_name: String,
    #[serde(default)]
    pub program_comment: String,
    pub tolerance_mm: f64,
    /// Global parameters, Autodesk names (e.g. `job-description`).
    /// A `Vec`, not a map: `onParameter` replay order is part of the
    /// observable contract and maps would re-sort it.
    #[serde(default)]
    pub parameters: Vec<Parameter>,
}

/// One `onParameter` name/value pair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Parameter {
    pub name: String,
    pub value: ParamValue,
}

/// Parameter payload — the three primitive shapes `.cps` posts read.
/// Untagged so the JSON is the bare JS value `getParameter` returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    Bool(bool),
    Number(f64),
    Text(String),
}

/// A point in WCS coordinates (mm).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// One operation = one section (Fusion's model; `begin_section` fires
/// per op even when the tool repeats).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    /// The pipeline op id — stamps warnings and preview attribution.
    pub id: u32,
    /// Strategy slug the post reads via `currentSection.strategy`
    /// (e.g. `"contour2d"`, `"drill"`).
    pub strategy: String,
    pub tool: ToolSpec,
    pub spindle_rpm: f64,
    pub spindle_clockwise: bool,
    /// Work offset number (1 → G54, …, 0 → post default).
    pub work_offset: u32,
    /// Row-major 3×3 orientation of the working plane — identity for
    /// every section ivacam records today (3-axis), but posts read it
    /// through `getWorkPlane()` even then.
    pub work_plane: [Vec3; 3],
    /// Section parameter stream (Autodesk names: `operation-comment`,
    /// `operation:tool_feedCutting`, …) replayed through `onParameter`
    /// BEFORE `onSection`, in order.
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    pub initial_position: Position,
    pub final_position: Position,
    pub records: Vec<Record>,
}

/// Tool description surfaced to posts as `tool.*`. Field names mirror
/// the Autodesk `Tool` members the wrappers (prelude `08`) expose;
/// distances mm, angles radians.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpec {
    pub number: u32,
    #[serde(default)]
    pub description: String,
    pub diameter: f64,
    /// 0 for flat end mills; radius for ball/bull noses.
    #[serde(default)]
    pub corner_radius: f64,
    /// Half the tip angle for V-bits / chamfer tools, 0 otherwise.
    #[serde(default)]
    pub taper_angle: f64,
    #[serde(default)]
    pub flutes: u32,
    /// `TOOL_*` code.
    pub tool_type: u32,
    /// `COOLANT_*` code the section starts with.
    #[serde(default)]
    pub coolant: u32,
}

/// One recorded post-processor call. Internally tagged so the JS
/// driver switches on `record.kind`.
///
/// Motion targets are CONCRETE coordinates: the recorder resolves the
/// trait's per-axis `Option`s against its tracked position, so the
/// driver never needs modal-state reconstruction. `Rapid5D`/`Linear5D`
/// are reserved — the driver dispatches them (cps.6) but the v1
/// recorder never emits them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Record {
    Rapid {
        x: f64,
        y: f64,
        z: f64,
    },
    /// Rapid in MACHINE coordinates (`G53`-style, tool-change staging).
    /// Axes are optional here — a machine-frame XY move deliberately
    /// leaves Z alone and there is no WCS position to resolve against.
    RapidMachine {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        x: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        y: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        z: Option<f64>,
    },
    Linear {
        x: f64,
        y: f64,
        z: f64,
        /// mm/min.
        feed: f64,
        /// `MOVEMENT_*` classification (cutting/lead-in/plunge/…).
        movement: u32,
    },
    Circular {
        clockwise: bool,
        /// Absolute arc center (the recorder resolves i/j offsets).
        center: Position,
        end: Position,
        /// mm/min.
        feed: f64,
        /// Plane normal — ±Z for everything ivacam records today.
        normal: Vec3,
    },
    /// Canned cycle: `cycle_type` uses Autodesk's names (`"drilling"`,
    /// `"counter-boring"`, `"deep-drilling"`, `"chip-breaking"`, …),
    /// `params` the `CycleParameters` bag (mm / mm/min / seconds), and
    /// each point fires one `onCyclePoint`.
    Cycle {
        cycle_type: String,
        params: BTreeMap<String, f64>,
        points: Vec<Position>,
    },
    /// Cancels the active cycle (G80 analog) → `onCycleEnd`.
    CycleEnd,
    Dwell {
        seconds: f64,
    },
    /// `COMMAND_*` code → `onCommand`.
    Command {
        command: u32,
    },
    /// Mid-section spindle change (section start state lives on the
    /// section itself; the recorder dedupes).
    SpindleSpeed {
        rpm: f64,
        clockwise: bool,
    },
    /// Mid-section coolant change, `COOLANT_*` code.
    Coolant {
        mode: u32,
    },
    Comment {
        text: String,
    },
    /// Raw line forwarded verbatim (GcodeInclude ops, `raw()` calls the
    /// recorder cannot classify).
    PassThrough {
        text: String,
    },
    #[serde(rename = "rapid5d")]
    Rapid5D {
        x: f64,
        y: f64,
        z: f64,
        dx: f64,
        dy: f64,
        dz: f64,
    },
    #[serde(rename = "linear5d")]
    Linear5D {
        x: f64,
        y: f64,
        z: f64,
        dx: f64,
        dy: f64,
        dz: f64,
        /// mm/min.
        feed: f64,
    },
}

/// Machine kinematics for `MachineConfiguration` (prelude `09`, cps.6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineSpec {
    #[serde(default)]
    pub vendor: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub description: String,
    pub axes: Vec<AxisSpec>,
}

/// One rotary axis of a [`MachineSpec`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AxisSpec {
    /// 0 = A, 1 = B, 2 = C.
    pub coordinate: u32,
    pub direction: Vec3,
    /// Table axis (rotates the part) vs head axis (rotates the tool).
    pub table: bool,
    /// Continuous rotation — `range` is ignored when set.
    #[serde(default)]
    pub cyclic: bool,
    /// [min, max] radians for ranged axes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<[f64; 2]>,
    /// Solution preference: -1 / 0 / +1 (Autodesk convention).
    #[serde(default)]
    pub preference: i32,
    #[serde(default)]
    pub tcp: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_program() -> Program {
        Program {
            version: IR_VERSION,
            header: Header {
                program_name: "1001".into(),
                program_comment: "spike part".into(),
                tolerance_mm: 0.01,
                parameters: vec![Parameter {
                    name: "job-description".into(),
                    value: ParamValue::Text("demo".into()),
                }],
            },
            sections: vec![Section {
                id: 7,
                strategy: "contour2d".into(),
                tool: ToolSpec {
                    number: 3,
                    description: "6mm flat".into(),
                    diameter: 6.0,
                    corner_radius: 0.0,
                    taper_angle: 0.0,
                    flutes: 2,
                    tool_type: 0,
                    coolant: 0,
                },
                spindle_rpm: 18000.0,
                spindle_clockwise: true,
                work_offset: 1,
                work_plane: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                parameters: vec![
                    Parameter {
                        name: "operation-comment".into(),
                        value: ParamValue::Text("Profile outer".into()),
                    },
                    Parameter {
                        name: "operation:tool_feedCutting".into(),
                        value: ParamValue::Number(800.0),
                    },
                ],
                initial_position: Position {
                    x: 0.0,
                    y: 0.0,
                    z: 15.0,
                },
                final_position: Position {
                    x: 42.0,
                    y: 0.0,
                    z: 15.0,
                },
                records: vec![
                    Record::Rapid {
                        x: 10.0,
                        y: 5.0,
                        z: 15.0,
                    },
                    Record::Linear {
                        x: 10.0,
                        y: 5.0,
                        z: -1.0,
                        feed: 300.0,
                        movement: 0,
                    },
                    Record::Circular {
                        clockwise: true,
                        center: Position {
                            x: 15.0,
                            y: 5.0,
                            z: -1.0,
                        },
                        end: Position {
                            x: 20.0,
                            y: 5.0,
                            z: -1.0,
                        },
                        feed: 800.0,
                        normal: [0.0, 0.0, 1.0],
                    },
                    Record::Cycle {
                        cycle_type: "deep-drilling".into(),
                        params: BTreeMap::from([
                            ("depth".into(), 12.0),
                            ("incrementalDepth".into(), 3.0),
                        ]),
                        points: vec![Position {
                            x: 30.0,
                            y: 8.0,
                            z: -12.0,
                        }],
                    },
                    Record::CycleEnd,
                    Record::Comment {
                        text: "OP 7".into(),
                    },
                ],
            }],
            machine: None,
        }
    }

    /// The full shape survives serialize → deserialize unchanged.
    #[test]
    fn round_trip() {
        let program = sample_program();
        let json = serde_json::to_string(&program).expect("serialize");
        let back: Program = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, program);
    }

    /// JSON casing and record tagging are the JS-facing contract —
    /// pin them so a refactor can't silently rename what the prelude
    /// dispatches on.
    #[test]
    fn wire_shape_is_camel_case_and_kind_tagged() {
        let json = serde_json::to_value(sample_program()).expect("serialize");
        assert_eq!(json["header"]["programName"], "1001");
        assert_eq!(json["header"]["toleranceMm"], 0.01);
        let section = &json["sections"][0];
        assert_eq!(section["workOffset"], 1);
        assert_eq!(section["tool"]["cornerRadius"], 0.0);
        assert_eq!(section["initialPosition"]["z"], 15.0);
        let records = section["records"].as_array().expect("records");
        assert_eq!(records[0]["kind"], "rapid");
        assert_eq!(records[1]["kind"], "linear");
        assert_eq!(records[2]["kind"], "circular");
        assert_eq!(records[3]["kind"], "cycle");
        assert_eq!(records[3]["cycleType"], "deep-drilling");
        assert_eq!(records[4]["kind"], "cycleEnd");
        // Untagged param values serialize as bare JS primitives.
        assert_eq!(section["parameters"][1]["value"], 800.0);
    }

    /// A version we don't support must fail deserialization loudly.
    #[test]
    fn version_gate_rejects_mismatch() {
        let mut json = serde_json::to_value(sample_program()).expect("serialize");
        json["version"] = serde_json::json!(999);
        let err = serde_json::from_value::<Program>(json).expect_err("must reject");
        assert!(
            err.to_string().contains("IR version 999"),
            "unexpected error: {err}"
        );
    }

    /// The reserved 5D variants keep their explicit lowercase tags.
    #[test]
    fn five_d_tags() {
        let rapid = serde_json::to_value(Record::Rapid5D {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            dx: 0.0,
            dy: 0.0,
            dz: 1.0,
        })
        .expect("serialize");
        assert_eq!(rapid["kind"], "rapid5d");
    }
}
