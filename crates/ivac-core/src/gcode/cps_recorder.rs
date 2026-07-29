//! `CpsRecorder` — a [`PostProcessor`] that emits no text at all.
//!
//! It records the pipeline's emit calls into [`ivac_cps::ir::Program`],
//! the IR one JS post run consumes (cps.7 wires that run). Because it
//! declares `owns_scaffold`, the pipeline routes program/section/tool
//! boundaries through the structured hooks instead of dialect
//! scaffolding, so the recording is dialect-free: absolute mm
//! positions, modal feed stamped onto motion records, canned drill
//! cycles kept as cycle records (the headline: `.cps` posts emit real
//! G81/G83 from them).
//!
//! Position semantics: the trait's per-axis `Option`s are resolved
//! against tracked state here, so IR motion targets are always
//! concrete. `reset_state()` is deliberately a no-op — the tracked
//! position is physical truth and the IR carries absolute coordinates,
//! so the dialect posts' delta-encoding determinism dance does not
//! apply.

use ivac_cps::ir::{self, codes};

use super::{CapturedPostState, CoolantState, PostProcessor, UnitSystem};
use crate::gcode::preview::{ArcXY, MoveKind, Pose3, ToolpathSegment};
use crate::pipeline::cps_ctx::{
    CpsToolInfo, PostCaps, ProgramCtx, ProgramEventCtx, SectionCtx, SectionToolCtx,
};
use crate::pipeline::CpsParamValue;
use crate::project::{Coolant, ToolKind};

/// Chip-break retract used by the trait-default expansion
/// (`PostProcessor::drill_chip_break`) — recorded into the cycle params
/// so the JS expansion and the preview mirror the same distance.
const CHIP_BREAK_DISTANCE_MM: f64 = 0.5;
/// Re-entry clearance of the trait-default peck expansion, mirrored by
/// [`ir_to_toolpath`]'s deep-drilling preview.
const RE_ENTRY_CLEARANCE_MM: f64 = 0.5;

#[derive(Debug)]
pub struct CpsRecorder {
    header: ir::Header,
    sections: Vec<ir::Section>,
    /// Records arriving before the first section (program events of a
    /// leading Pause/GcodeInclude op) — flushed into the first section.
    pending: Vec<ir::Record>,
    /// Tracked WCS position (mm) for resolving partial moves.
    x: f64,
    y: f64,
    z: f64,
    /// Modal feed (mm/min).
    feed: u32,
    /// Live spindle/coolant for mid-section dedupe.
    cur_rpm: f64,
    cur_cw: bool,
    cur_coolant: u32,
    /// The active section's plunge rate — the movement-classification
    /// heuristic compares against it.
    section_rate_v: u32,
}

impl Default for CpsRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl CpsRecorder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            header: ir::Header {
                program_name: String::new(),
                program_comment: String::new(),
                // Fusion-typical kernel tolerance; a real knob can land
                // later without touching the IR shape.
                tolerance_mm: 0.01,
                parameters: Vec::new(),
            },
            sections: Vec::new(),
            pending: Vec::new(),
            x: 0.0,
            y: 0.0,
            z: 0.0,
            feed: 0,
            cur_rpm: 0.0,
            cur_cw: true,
            cur_coolant: codes::COOLANT_DISABLED,
            section_rate_v: 0,
        }
    }

    /// Consume the recorder, yielding the recorded program.
    ///
    /// Pending records of a program that never opened a section (only
    /// program-only ops) are dropped — there is no section a post could
    /// attach them to, and the pipeline already warned about the ops it
    /// skipped.
    #[must_use]
    pub fn into_program(self) -> ir::Program {
        ir::Program {
            version: ir::IR_VERSION,
            header: self.header,
            sections: self.sections,
            machine: None,
        }
    }

    fn position(&self) -> ir::Position {
        ir::Position {
            x: self.x,
            y: self.y,
            z: self.z,
        }
    }

    fn resolve(&mut self, x: Option<f64>, y: Option<f64>, z: Option<f64>) -> (f64, f64, f64) {
        if let Some(v) = x {
            self.x = v;
        }
        if let Some(v) = y {
            self.y = v;
        }
        if let Some(v) = z {
            self.z = v;
        }
        (self.x, self.y, self.z)
    }

    /// Append to the open (or last) section; buffer when none exists
    /// yet so a leading program event still lands in the first section.
    fn push(&mut self, record: ir::Record) {
        match self.sections.last_mut() {
            Some(section) => section.records.push(record),
            None => self.pending.push(record),
        }
    }

    /// Coalesce consecutive same-parameter drill calls into ONE cycle
    /// record with many points — Fusion's model (one `onCycle`, many
    /// `onCyclePoint`).
    fn push_cycle_point(
        &mut self,
        cycle_type: &str,
        params: std::collections::BTreeMap<String, f64>,
        point: ir::Position,
    ) {
        if let Some(ir::Record::Cycle {
            cycle_type: prev_type,
            params: prev_params,
            points,
        }) = self.sections.last_mut().and_then(|s| s.records.last_mut())
        {
            if prev_type == cycle_type && *prev_params == params {
                points.push(point);
                return;
            }
        }
        self.push(ir::Record::Cycle {
            cycle_type: cycle_type.to_owned(),
            params,
            points: vec![point],
        });
    }

    fn set_spindle(&mut self, rpm: f64, clockwise: bool) {
        #[allow(clippy::float_cmp)] // exact: both sides assigned from the same integers
        if self.cur_rpm == rpm && (self.cur_cw == clockwise || rpm == 0.0) {
            return;
        }
        self.cur_rpm = rpm;
        if rpm > 0.0 {
            self.cur_cw = clockwise;
        }
        self.push(ir::Record::SpindleSpeed {
            rpm,
            clockwise: self.cur_cw,
        });
    }

    fn set_coolant(&mut self, mode: u32) {
        if self.cur_coolant == mode {
            return;
        }
        self.cur_coolant = mode;
        self.push(ir::Record::Coolant { mode });
    }

    fn open_section(
        &mut self,
        id: u32,
        strategy: String,
        tool: &CpsToolInfo,
        work_offset: u32,
        parameters: Vec<ir::Parameter>,
    ) {
        let mut section = ir::Section {
            id,
            strategy,
            tool: tool_spec(tool),
            spindle_rpm: f64::from(tool.spindle_rpm),
            spindle_clockwise: tool.spindle_clockwise,
            work_offset,
            work_plane: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            parameters,
            initial_position: self.position(),
            final_position: self.position(),
            records: Vec::new(),
        };
        section.records.append(&mut self.pending);
        self.sections.push(section);
        // Section start state — the emit shells re-assert spindle and
        // coolant per cut block; only genuine mid-section CHANGES
        // should surface as records.
        self.cur_rpm = f64::from(tool.spindle_rpm);
        self.cur_cw = tool.spindle_clockwise;
        self.cur_coolant = coolant_code(tool.coolant);
    }

    fn close_section(&mut self) {
        let final_position = self.position();
        if let Some(section) = self.sections.last_mut() {
            section.final_position = final_position;
        }
    }
}

fn coolant_code(coolant: Coolant) -> u32 {
    match coolant {
        Coolant::Off => codes::COOLANT_DISABLED,
        Coolant::Flood => codes::COOLANT_FLOOD,
        Coolant::Mist => codes::COOLANT_MIST,
    }
}

fn tool_type_code(kind: ToolKind) -> u32 {
    match kind {
        ToolKind::Endmill | ToolKind::Compression => codes::TOOL_MILLING_END_FLAT,
        ToolKind::BallNose => codes::TOOL_MILLING_END_BALL,
        ToolKind::BullNose => codes::TOOL_MILLING_END_BULLNOSE,
        ToolKind::VBit | ToolKind::Engraver => codes::TOOL_MILLING_CHAMFER,
        ToolKind::Drill => codes::TOOL_DRILL,
        ToolKind::LaserBeam => codes::TOOL_LASER_CUTTER,
        ToolKind::DragKnife => codes::TOOL_MARKER,
        ToolKind::FormProfile => codes::TOOL_MILLING_FORM,
        ToolKind::Kegel => codes::TOOL_MILLING_TAPERED,
        ToolKind::ThreadMill => codes::TOOL_MILLING_THREAD,
        ToolKind::PlasmaTorch => codes::TOOL_PLASMA_CUTTER,
    }
}

fn tool_spec(tool: &CpsToolInfo) -> ir::ToolSpec {
    ir::ToolSpec {
        number: tool.number,
        description: tool.description.clone(),
        diameter: tool.diameter,
        corner_radius: tool.corner_radius,
        taper_angle: tool.taper_angle,
        flutes: tool.flutes,
        tool_type: tool_type_code(tool.kind),
        coolant: coolant_code(tool.coolant),
    }
}

fn param_value(value: &CpsParamValue) -> ir::ParamValue {
    match value {
        CpsParamValue::Bool(b) => ir::ParamValue::Bool(*b),
        CpsParamValue::Number(n) => ir::ParamValue::Number(*n),
        CpsParamValue::Text(s) => ir::ParamValue::Text(s.clone()),
    }
}

impl PostProcessor for CpsRecorder {
    fn capabilities(&self) -> PostCaps {
        PostCaps {
            owns_scaffold: true,
        }
    }

    fn begin_program(&mut self, ctx: &ProgramCtx<'_>) {
        self.header.program_name.clone_from(&ctx.program_name);
        self.header.program_comment.clone_from(&ctx.program_comment);
        // The physical program-start pose: XY at part zero, Z at the
        // program-wide clearance — what program_begin's first rapid
        // would establish for a dialect post.
        self.x = 0.0;
        self.y = 0.0;
        self.z = ctx.fast_move_z;
    }

    fn end_program(&mut self) {
        self.close_section();
    }

    fn begin_section(&mut self, ctx: &SectionCtx) {
        let mut parameters: Vec<ir::Parameter> = Vec::with_capacity(ctx.params.len() + 1);
        if let Some(group) = &ctx.group {
            parameters.push(ir::Parameter {
                name: "operation-group".to_owned(),
                value: ir::ParamValue::Text(group.clone()),
            });
        }
        for (name, value) in &ctx.params {
            parameters.push(ir::Parameter {
                name: name.clone(),
                value: param_value(value),
            });
        }
        self.section_rate_v = ctx
            .params
            .iter()
            .find_map(|(name, value)| match value {
                CpsParamValue::Number(n) if name == "operation:tool_feedPlunge" =>
                {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    Some(n.round() as u32)
                }
                _ => None,
            })
            .unwrap_or(0);
        self.open_section(
            ctx.op_id,
            ctx.strategy.to_owned(),
            &ctx.tool,
            ctx.wcs.p_number(),
            parameters,
        );
    }

    fn end_section(&mut self) {
        self.close_section();
    }

    fn mid_section_toolchange(&mut self, ctx: &SectionToolCtx) {
        let Some(prev) = self.sections.last() else {
            // No open section (nothing recorded yet) — nothing to split.
            return;
        };
        let id = prev.id;
        let strategy = prev.strategy.clone();
        let work_offset = prev.work_offset;
        let parameters = prev.parameters.clone();
        self.close_section();
        self.open_section(id, strategy, &ctx.tool, work_offset, parameters);
    }

    fn program_event(&mut self, ev: &ProgramEventCtx) {
        match ev {
            ProgramEventCtx::Stop { message, optional } => {
                if !message.is_empty() {
                    self.push(ir::Record::Comment {
                        text: message.clone(),
                    });
                }
                self.push(ir::Record::Command {
                    command: if *optional {
                        codes::COMMAND_OPTIONAL_STOP
                    } else {
                        codes::COMMAND_STOP
                    },
                });
            }
            ProgramEventCtx::PassThrough { lines } => {
                for line in lines {
                    self.push(ir::Record::PassThrough { text: line.clone() });
                }
            }
        }
    }

    fn unit(&mut self, _unit: UnitSystem) {
        // The IR is always mm; the JS driver scales to the post's unit.
    }

    fn feedrate(&mut self, rate: u32) {
        self.feed = rate;
    }

    fn spindle_cw(&mut self, speed: u32, pause_seconds: u32) {
        self.set_spindle(f64::from(speed), true);
        if pause_seconds > 0 {
            self.push(ir::Record::Dwell {
                seconds: f64::from(pause_seconds),
            });
        }
    }

    fn spindle_ccw(&mut self, speed: u32, pause_seconds: u32) {
        self.set_spindle(f64::from(speed), false);
        if pause_seconds > 0 {
            self.push(ir::Record::Dwell {
                seconds: f64::from(pause_seconds),
            });
        }
    }

    fn spindle_off(&mut self) {
        self.set_spindle(0.0, self.cur_cw);
    }

    // Laser power rides the S-word by convention (GRBL semantics, and
    // what .cps laser posts read back from spindle speed) — record it
    // as spindle speed so power survives into the IR.
    fn laser_on(&mut self, power: u32) {
        self.set_spindle(f64::from(power), true);
    }

    fn laser_arm(&mut self) {
        self.set_spindle(0.0, true);
    }

    fn laser_off(&mut self) {
        self.set_spindle(0.0, self.cur_cw);
    }

    fn coolant_mist(&mut self) {
        self.set_coolant(codes::COOLANT_MIST);
    }

    fn coolant_flood(&mut self) {
        self.set_coolant(codes::COOLANT_FLOOD);
    }

    fn coolant_off(&mut self) {
        self.set_coolant(codes::COOLANT_DISABLED);
    }

    fn move_to(&mut self, x: Option<f64>, y: Option<f64>, z: Option<f64>) {
        let before = self.position();
        let (x, y, z) = self.resolve(x, y, z);
        if (before.x, before.y, before.z) == (x, y, z) {
            return; // dialect posts delta-suppress this too
        }
        self.push(ir::Record::Rapid { x, y, z });
    }

    fn rapid_machine_xy(&mut self, x_mm: f64, y_mm: f64) {
        // Machine-frame move: WCS position becomes unknown, but every
        // driver re-establishes XY explicitly afterwards, so keeping
        // the stale WCS values is harmless for resolution.
        self.push(ir::Record::RapidMachine {
            x: Some(x_mm),
            y: Some(y_mm),
            z: None,
        });
    }

    fn rapid_machine_z(&mut self, z_mm: f64) {
        self.push(ir::Record::RapidMachine {
            x: None,
            y: None,
            z: Some(z_mm),
        });
    }

    fn linear(&mut self, x: Option<f64>, y: Option<f64>, z: Option<f64>) {
        let before = self.position();
        let (x, y, z) = self.resolve(x, y, z);
        if (before.x, before.y, before.z) == (x, y, z) {
            return;
        }
        #[allow(clippy::float_cmp)] // copied-through coordinates, exact compare intended
        let z_only = before.x == x && before.y == y;
        let movement = if z_only && z < before.z && self.feed == self.section_rate_v {
            codes::MOVEMENT_PLUNGE
        } else {
            codes::MOVEMENT_CUTTING
        };
        self.push(ir::Record::Linear {
            x,
            y,
            z,
            feed: f64::from(self.feed),
            movement,
        });
    }

    fn arc_cw(
        &mut self,
        x: Option<f64>,
        y: Option<f64>,
        z: Option<f64>,
        i: Option<f64>,
        j: Option<f64>,
    ) {
        self.record_arc(true, x, y, z, i, j);
    }

    fn arc_ccw(
        &mut self,
        x: Option<f64>,
        y: Option<f64>,
        z: Option<f64>,
        i: Option<f64>,
        j: Option<f64>,
    ) {
        self.record_arc(false, x, y, z, i, j);
    }

    fn dwell(&mut self, seconds: f64) {
        self.push(ir::Record::Dwell { seconds });
    }

    fn comment(&mut self, text: &str) {
        self.push(ir::Record::Comment {
            text: text.to_owned(),
        });
    }

    fn raw(&mut self, cmd: &str) {
        // `; …` raw lines are comments in disguise (the `; OP <id>`
        // marker, driver annotations) — keep them as comments so posts
        // can render them via writeComment. Everything else passes
        // through verbatim.
        let trimmed = cmd.trim_start();
        if let Some(text) = trimmed.strip_prefix(';') {
            self.push(ir::Record::Comment {
                text: text.trim().to_owned(),
            });
        } else {
            self.push(ir::Record::PassThrough {
                text: cmd.to_owned(),
            });
        }
    }

    // Canned drill cycles stay cycles — the headline CPS feature. The
    // params carry the trait-default expansion's knobs so the JS
    // fallback expansion and the preview replay the same motion.
    fn drill_simple(&mut self, x: f64, y: f64, z: f64, r: f64, rate_v: u32, dwell_sec: f64) {
        let cycle_type = if dwell_sec > 0.0 {
            "counter-boring"
        } else {
            "drilling"
        };
        let mut params = cycle_params(z, r, rate_v);
        if dwell_sec > 0.0 {
            params.insert("dwell".to_owned(), dwell_sec);
        }
        self.push_cycle_point(cycle_type, params, ir::Position { x, y, z });
        self.resolve(Some(x), Some(y), Some(r));
    }

    fn drill_peck(&mut self, x: f64, y: f64, z: f64, r: f64, q: f64, rate_v: u32, dwell_sec: f64) {
        let q = q.abs();
        if q < 1e-9 {
            self.drill_simple(x, y, z, r, rate_v, dwell_sec);
            return;
        }
        let mut params = cycle_params(z, r, rate_v);
        params.insert("incrementalDepth".to_owned(), q);
        if dwell_sec > 0.0 {
            params.insert("dwell".to_owned(), dwell_sec);
        }
        self.push_cycle_point("deep-drilling", params, ir::Position { x, y, z });
        self.resolve(Some(x), Some(y), Some(r));
    }

    fn drill_chip_break(
        &mut self,
        x: f64,
        y: f64,
        z: f64,
        r: f64,
        q: f64,
        rate_v: u32,
        dwell_sec: f64,
    ) {
        let q = q.abs();
        if q < 1e-9 {
            self.drill_simple(x, y, z, r, rate_v, dwell_sec);
            return;
        }
        let mut params = cycle_params(z, r, rate_v);
        params.insert("incrementalDepth".to_owned(), q);
        params.insert("chipBreakDistance".to_owned(), CHIP_BREAK_DISTANCE_MM);
        // Full depth per entry — no forced re-expansion in the post.
        params.insert("accumulatedDepth".to_owned(), (r - z).abs());
        if dwell_sec > 0.0 {
            params.insert("dwell".to_owned(), dwell_sec);
        }
        self.push_cycle_point("chip-breaking", params, ir::Position { x, y, z });
        self.resolve(Some(x), Some(y), Some(r));
    }

    fn cancel_canned_cycle(&mut self) {
        // Only meaningful after a cycle actually ran.
        if matches!(
            self.sections.last().and_then(|s| s.records.last()),
            Some(ir::Record::Cycle { .. })
        ) {
            self.push(ir::Record::CycleEnd);
        }
    }

    fn capture_state(&self) -> CapturedPostState {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        CapturedPostState {
            last_x: Some(self.x),
            last_y: Some(self.y),
            last_z: Some(self.z),
            last_rate: (self.feed > 0).then_some(self.feed),
            last_speed: (self.cur_rpm > 0.0).then_some(self.cur_rpm.round() as u32),
            last_coolant: match self.cur_coolant {
                codes::COOLANT_FLOOD => CoolantState::Flood,
                codes::COOLANT_MIST => CoolantState::Mist,
                _ => CoolantState::Off,
            },
            ..CapturedPostState::default()
        }
    }

    fn finish(&self) -> String {
        // The program is the IR (`into_program`); there is no text.
        String::new()
    }
}

impl CpsRecorder {
    fn record_arc(
        &mut self,
        clockwise: bool,
        x: Option<f64>,
        y: Option<f64>,
        z: Option<f64>,
        i: Option<f64>,
        j: Option<f64>,
    ) {
        let start = self.position();
        let center = ir::Position {
            x: start.x + i.unwrap_or(0.0),
            y: start.y + j.unwrap_or(0.0),
            z: start.z,
        };
        let (x, y, z) = self.resolve(x, y, z);
        self.push(ir::Record::Circular {
            clockwise,
            center,
            end: ir::Position { x, y, z },
            feed: f64::from(self.feed),
            normal: [0.0, 0.0, 1.0],
        });
    }
}

fn cycle_params(z: f64, r: f64, rate_v: u32) -> std::collections::BTreeMap<String, f64> {
    std::collections::BTreeMap::from([
        ("clearance".to_owned(), r),
        ("retract".to_owned(), r),
        ("bottom".to_owned(), z),
        ("depth".to_owned(), (r - z).abs()),
        ("feedrate".to_owned(), f64::from(rate_v)),
    ])
}

/// Preview toolpath straight from the IR — geometry is exact regardless
/// of what dialect text the JS post renders (fixes the HPGL-style
/// silent-degradation class where preview quality hinged on text
/// re-parseability). `gcode_line` stays 0 (unknown); cps.7's line-sync
/// recovery zips real line numbers on when the rendered text matches.
#[must_use]
pub fn ir_to_toolpath(program: &ir::Program) -> Vec<ToolpathSegment> {
    let mut out = Vec::new();
    for section in &program.sections {
        let mut pos = pose(section.initial_position);
        let op_id = section.id;
        let seg = |from: Pose3, to: Pose3, kind: MoveKind, arc: Option<ArcXY>| ToolpathSegment {
            from,
            to,
            kind,
            gcode_line: 0,
            op_id,
            arc,
        };
        for record in &section.records {
            match record {
                ir::Record::Rapid { x, y, z } => {
                    let to = Pose3 {
                        x: *x,
                        y: *y,
                        z: *z,
                    };
                    if to != pos {
                        out.push(seg(pos, to, MoveKind::Rapid, None));
                        pos = to;
                    }
                }
                ir::Record::Linear { x, y, z, .. } => {
                    let to = Pose3 {
                        x: *x,
                        y: *y,
                        z: *z,
                    };
                    if to != pos {
                        out.push(seg(pos, to, classify_linear(pos, to), None));
                        pos = to;
                    }
                }
                ir::Record::Circular {
                    clockwise,
                    center,
                    end,
                    ..
                } => {
                    let to = Pose3 {
                        x: end.x,
                        y: end.y,
                        z: end.z,
                    };
                    push_arc_chords(&mut out, &mut pos, center, to, !clockwise, op_id);
                }
                ir::Record::Cycle {
                    cycle_type,
                    params,
                    points,
                } => {
                    expand_cycle_segments(&mut out, &mut pos, cycle_type, params, points, op_id);
                }
                ir::Record::RapidMachine { .. }
                | ir::Record::CycleEnd
                | ir::Record::Dwell { .. }
                | ir::Record::Command { .. }
                | ir::Record::SpindleSpeed { .. }
                | ir::Record::Coolant { .. }
                | ir::Record::Comment { .. }
                | ir::Record::PassThrough { .. }
                | ir::Record::Rapid5D { .. }
                | ir::Record::Linear5D { .. } => {}
            }
        }
    }
    out
}

fn pose(p: ir::Position) -> Pose3 {
    Pose3 {
        x: p.x,
        y: p.y,
        z: p.z,
    }
}

fn classify_linear(from: Pose3, to: Pose3) -> MoveKind {
    #[allow(clippy::float_cmp)] // coordinates copied verbatim; exact compare intended
    let xy_same = from.x == to.x && from.y == to.y;
    if xy_same {
        if to.z > from.z {
            MoveKind::Retract
        } else {
            MoveKind::Plunge
        }
    } else {
        MoveKind::Cut
    }
}

/// Tessellate one circular record into ≤15° chords (min 4), each tagged
/// with the parent arc — the same policy `preview::interpret` applies
/// to G2/G3 lines, so sim/renderer/envelope consumers see the shape
/// they already handle.
fn push_arc_chords(
    out: &mut Vec<ToolpathSegment>,
    pos: &mut Pose3,
    center: &ir::Position,
    to: Pose3,
    ccw: bool,
    op_id: u32,
) {
    const ARC_CHORD_STEP_DEG: f64 = 15.0;
    let from = *pos;
    let (cx, cy) = (center.x, center.y);
    let r = ((from.x - cx).powi(2) + (from.y - cy).powi(2)).sqrt();
    if r < 1e-9 {
        return;
    }
    let theta_start = (from.y - cy).atan2(from.x - cx);
    let theta_end = (to.y - cy).atan2(to.x - cx);
    let mut sweep = theta_end - theta_start;
    const TAU: f64 = std::f64::consts::TAU;
    if ccw {
        if sweep <= 1e-9 {
            sweep += TAU; // start==end → full circle in the arc's direction
        }
    } else if sweep >= -1e-9 {
        sweep -= TAU;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let n = (sweep.abs() / ARC_CHORD_STEP_DEG.to_radians())
        .ceil()
        .max(4.0) as usize;
    let dtheta = sweep / (n as f64);
    let dz = to.z - from.z;
    let mut prev = from;
    for k in 1..=n {
        let theta = theta_start + dtheta * (k as f64);
        let chord_to = if k == n {
            to
        } else {
            Pose3 {
                x: cx + r * theta.cos(),
                y: cy + r * theta.sin(),
                z: from.z + dz * (k as f64) / (n as f64),
            }
        };
        out.push(ToolpathSegment {
            from: prev,
            to: chord_to,
            kind: MoveKind::Arc,
            gcode_line: 0,
            op_id,
            arc: Some(ArcXY { cx, cy, ccw }),
        });
        prev = chord_to;
    }
    *pos = to;
}

/// Cycle records → the G0/G1-equivalent segments of the trait-default
/// expansions (`drill_simple`/`drill_peck`/`drill_chip_break`), so the
/// preview and time estimate see the same motion a non-canned dialect
/// would cut.
fn expand_cycle_segments(
    out: &mut Vec<ToolpathSegment>,
    pos: &mut Pose3,
    cycle_type: &str,
    params: &std::collections::BTreeMap<String, f64>,
    points: &[ir::Position],
    op_id: u32,
) {
    let get = |k: &str| params.get(k).copied();
    for point in points {
        let bottom = point.z;
        let retract = get("retract").unwrap_or(bottom);
        let entry = Pose3 {
            x: point.x,
            y: point.y,
            z: retract,
        };
        let mut push = |from: Pose3, to: Pose3, kind: MoveKind| {
            if from != to {
                out.push(ToolpathSegment {
                    from,
                    to,
                    kind,
                    gcode_line: 0,
                    op_id,
                    arc: None,
                });
            }
        };
        push(*pos, entry, MoveKind::Rapid);
        let at = |z: f64| Pose3 {
            x: point.x,
            y: point.y,
            z,
        };
        match cycle_type {
            "deep-drilling" | "chip-breaking" => {
                let q = get("incrementalDepth").unwrap_or((retract - bottom).abs());
                let q = if q < 1e-9 {
                    (retract - bottom).abs().max(1e-9)
                } else {
                    q
                };
                let full_retract = cycle_type == "deep-drilling";
                let mut current = retract;
                loop {
                    let next = (current - q).max(bottom);
                    push(at(current), at(next), MoveKind::Plunge);
                    current = next;
                    if current <= bottom + 1e-9 {
                        break;
                    }
                    if full_retract {
                        push(at(current), at(retract), MoveKind::Retract);
                        let re_entry = current + RE_ENTRY_CLEARANCE_MM;
                        push(at(retract), at(re_entry.min(retract)), MoveKind::Rapid);
                        push(at(re_entry.min(retract)), at(current), MoveKind::Plunge);
                    } else {
                        let break_z = (current
                            + get("chipBreakDistance").unwrap_or(CHIP_BREAK_DISTANCE_MM))
                        .min(retract);
                        push(at(current), at(break_z), MoveKind::Retract);
                        push(at(break_z), at(current), MoveKind::Plunge);
                    }
                }
                push(at(bottom), at(retract), MoveKind::Retract);
            }
            // drilling / counter-boring / anything unrecognized:
            // straight plunge + retract (dwells carry no geometry).
            _ => {
                push(entry, at(bottom), MoveKind::Plunge);
                push(at(bottom), entry, MoveKind::Retract);
            }
        }
        *pos = entry;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Wcs;

    fn tool(number: u32, kind: ToolKind, rpm: u32) -> CpsToolInfo {
        CpsToolInfo {
            number,
            description: format!("tool {number}"),
            kind,
            diameter: 6.0,
            corner_radius: 0.0,
            taper_angle: 0.0,
            flutes: 2,
            spindle_rpm: rpm,
            spindle_clockwise: true,
            coolant: Coolant::Off,
        }
    }

    fn section_ctx(op_id: u32, tool: CpsToolInfo) -> SectionCtx {
        SectionCtx {
            op_id,
            op_name: format!("Op {op_id}"),
            strategy: "contour2d",
            group: None,
            tool,
            wcs: Wcs::G54,
            params: vec![
                (
                    "operation-comment".to_owned(),
                    CpsParamValue::Text(format!("Op {op_id}")),
                ),
                (
                    "operation:tool_feedCutting".to_owned(),
                    CpsParamValue::Number(800.0),
                ),
                (
                    "operation:tool_feedPlunge".to_owned(),
                    CpsParamValue::Number(300.0),
                ),
            ],
        }
    }

    fn begin(rec: &mut CpsRecorder) {
        let machine = crate::project::MachineConfig::default();
        rec.begin_program(&ProgramCtx {
            unit: UnitSystem::Mm,
            wcs: Wcs::G54,
            fast_move_z: 15.0,
            machine: &machine,
            program_name: "1001".to_owned(),
            program_comment: "test".to_owned(),
        });
    }

    /// One op = one section with metadata; motion resolves partial
    /// moves; plunge classification keys on Z-only descent at the
    /// section's plunge feed.
    #[test]
    fn records_section_and_classifies_movement() {
        let mut rec = CpsRecorder::new();
        begin(&mut rec);
        rec.begin_section(&section_ctx(7, tool(3, ToolKind::Endmill, 18000)));
        rec.move_to(Some(10.0), Some(5.0), None); // z stays at fast_move_z
        rec.feedrate(300);
        rec.linear(None, None, Some(-1.0)); // plunge at rate_v
        rec.feedrate(800);
        rec.linear(Some(20.0), None, None); // cut
        rec.linear(None, None, Some(-2.0)); // z-only, but at cut feed
        rec.end_section();
        rec.end_program();
        let program = rec.into_program();

        assert_eq!(program.sections.len(), 1);
        let s = &program.sections[0];
        assert_eq!(s.id, 7);
        assert_eq!(s.strategy, "contour2d");
        assert_eq!(s.tool.number, 3);
        assert_eq!(s.tool.tool_type, codes::TOOL_MILLING_END_FLAT);
        assert_eq!(s.work_offset, 1);
        assert_eq!(s.initial_position.z, 15.0);
        assert_eq!(
            s.final_position,
            ir::Position {
                x: 20.0,
                y: 5.0,
                z: -2.0
            }
        );
        match &s.records[..] {
            [ir::Record::Rapid {
                x: 10.0,
                y: 5.0,
                z: 15.0,
            }, ir::Record::Linear {
                z: -1.0,
                feed: f1,
                movement: m1,
                ..
            }, ir::Record::Linear {
                x: 20.0,
                movement: m2,
                ..
            }, ir::Record::Linear {
                z: -2.0,
                movement: m3,
                ..
            }] => {
                assert_eq!(*f1, 300.0);
                assert_eq!(*m1, codes::MOVEMENT_PLUNGE);
                assert_eq!(*m2, codes::MOVEMENT_CUTTING);
                // Z-only descent at CUT feed is not a plunge.
                assert_eq!(*m3, codes::MOVEMENT_CUTTING);
            }
            other => panic!("unexpected records: {other:?}"),
        }
    }

    /// Drill calls coalesce into ONE cycle record with many points;
    /// cancel closes it; dwell selects counter-boring.
    #[test]
    fn drill_cycles_coalesce_points() {
        let mut rec = CpsRecorder::new();
        begin(&mut rec);
        rec.begin_section(&section_ctx(1, tool(2, ToolKind::Drill, 9000)));
        rec.drill_peck(10.0, 0.0, -12.0, 2.0, 3.0, 120, 0.0);
        rec.drill_peck(20.0, 0.0, -12.0, 2.0, 3.0, 120, 0.0);
        rec.drill_peck(30.0, 0.0, -12.0, 2.0, 3.0, 120, 0.0);
        rec.cancel_canned_cycle();
        rec.drill_simple(40.0, 0.0, -3.0, 2.0, 120, 0.5);
        rec.cancel_canned_cycle();
        rec.end_section();
        let program = rec.into_program();

        let records = &program.sections[0].records;
        match &records[..] {
            [ir::Record::Cycle {
                cycle_type: peck,
                params,
                points,
            }, ir::Record::CycleEnd, ir::Record::Cycle {
                cycle_type: cbore,
                points: p2,
                ..
            }, ir::Record::CycleEnd] => {
                assert_eq!(peck, "deep-drilling");
                assert_eq!(points.len(), 3);
                assert_eq!(params["incrementalDepth"], 3.0);
                assert_eq!(params["retract"], 2.0);
                assert_eq!(params["feedrate"], 120.0);
                assert_eq!(cbore, "counter-boring");
                assert_eq!(p2.len(), 1);
            }
            other => panic!("unexpected records: {other:?}"),
        }
    }

    /// A mid-section tool change splits the section: same op id and
    /// strategy, the new tool on the sibling.
    #[test]
    fn mid_section_toolchange_splits() {
        let mut rec = CpsRecorder::new();
        begin(&mut rec);
        rec.begin_section(&section_ctx(4, tool(1, ToolKind::Endmill, 18000)));
        rec.linear(Some(5.0), Some(5.0), Some(-1.0));
        rec.mid_section_toolchange(&SectionToolCtx {
            tool: tool(9, ToolKind::VBit, 24000),
        });
        rec.linear(Some(6.0), Some(6.0), Some(-0.5));
        rec.end_section();
        let program = rec.into_program();

        assert_eq!(program.sections.len(), 2);
        assert_eq!(program.sections[0].id, 4);
        assert_eq!(program.sections[1].id, 4);
        assert_eq!(program.sections[0].tool.number, 1);
        assert_eq!(program.sections[1].tool.number, 9);
        assert_eq!(
            program.sections[1].tool.tool_type,
            codes::TOOL_MILLING_CHAMFER
        );
        // Split point continuity: sibling starts where the first ended.
        assert_eq!(
            program.sections[0].final_position,
            program.sections[1].initial_position
        );
    }

    /// Spindle/coolant records only surface on genuine mid-section
    /// change — section-start state comes from the section itself.
    #[test]
    fn spindle_and_coolant_dedupe() {
        let mut rec = CpsRecorder::new();
        begin(&mut rec);
        let mut t = tool(1, ToolKind::Endmill, 18000);
        t.coolant = Coolant::Flood;
        rec.begin_section(&section_ctx(1, t));
        rec.spindle_cw(18000, 0); // same as section start → no record
        rec.coolant_flood(); // same → no record
        rec.spindle_cw(12000, 0); // change → record
        rec.coolant_off(); // change → record
        rec.end_section();
        let program = rec.into_program();

        let records = &program.sections[0].records;
        assert_eq!(records.len(), 2, "unexpected: {records:?}");
        assert!(
            matches!(records[0], ir::Record::SpindleSpeed { rpm, .. } if rpm == 12000.0),
            "unexpected: {records:?}"
        );
        assert!(matches!(
            records[1],
            ir::Record::Coolant {
                mode: codes::COOLANT_DISABLED
            }
        ));
    }

    /// Program events between sections attach to the PREVIOUS section
    /// (emission order); a leading event lands in the first section.
    #[test]
    fn program_events_attach_in_order() {
        let mut rec = CpsRecorder::new();
        begin(&mut rec);
        rec.program_event(&ProgramEventCtx::PassThrough {
            lines: vec!["G4 P1".to_owned()],
        });
        rec.begin_section(&section_ctx(1, tool(1, ToolKind::Endmill, 18000)));
        rec.linear(Some(1.0), Some(1.0), Some(-1.0));
        rec.end_section();
        rec.program_event(&ProgramEventCtx::Stop {
            message: "swap fixture".to_owned(),
            optional: false,
        });
        rec.begin_section(&section_ctx(2, tool(1, ToolKind::Endmill, 18000)));
        rec.end_section();
        rec.end_program();
        let program = rec.into_program();

        let first = &program.sections[0].records;
        assert!(
            matches!(&first[0], ir::Record::PassThrough { text } if text == "G4 P1"),
            "leading event must flush into the first section: {first:?}"
        );
        let tail: Vec<_> = first.iter().rev().take(2).collect();
        assert!(
            matches!(
                tail[0],
                ir::Record::Command {
                    command: codes::COMMAND_STOP
                }
            ),
            "stop lands at the end of section 1: {first:?}"
        );
        assert!(matches!(&tail[1], ir::Record::Comment { text } if text == "swap fixture"));
        assert!(program.sections[1].records.is_empty());
    }

    /// The raw `; OP <id>` marker arrives as a comment record.
    #[test]
    fn raw_semicolon_lines_become_comments() {
        let mut rec = CpsRecorder::new();
        begin(&mut rec);
        rec.begin_section(&section_ctx(9, tool(1, ToolKind::Endmill, 18000)));
        rec.raw("; OP 9");
        rec.raw("G4 P0.5");
        rec.end_section();
        let program = rec.into_program();
        let records = &program.sections[0].records;
        assert!(matches!(&records[0], ir::Record::Comment { text } if text == "OP 9"));
        assert!(matches!(&records[1], ir::Record::PassThrough { text } if text == "G4 P0.5"));
    }

    /// ir_to_toolpath: rapids/cuts/arcs/cycles come out in the preview
    /// segment vocabulary, op-stamped, arc-tagged.
    #[test]
    fn ir_to_toolpath_shapes() {
        let mut rec = CpsRecorder::new();
        begin(&mut rec);
        rec.begin_section(&section_ctx(3, tool(1, ToolKind::Endmill, 18000)));
        rec.move_to(Some(10.0), Some(0.0), Some(15.0));
        rec.feedrate(300);
        rec.linear(None, None, Some(-1.0));
        rec.feedrate(800);
        // Quarter arc ccw around (10,5): (10,0) → (15,5).
        rec.arc_ccw(Some(15.0), Some(5.0), Some(-1.0), Some(0.0), Some(5.0));
        rec.drill_simple(30.0, 0.0, -5.0, 2.0, 120, 0.0);
        rec.end_section();
        let program = rec.into_program();
        let segments = ir_to_toolpath(&program);

        assert!(segments.iter().all(|s| s.op_id == 3));
        assert!(matches!(segments[0].kind, MoveKind::Rapid));
        assert!(matches!(segments[1].kind, MoveKind::Plunge));
        // Arc: ≥4 chords, all tagged with the same center.
        let arcs: Vec<_> = segments
            .iter()
            .filter(|s| matches!(s.kind, MoveKind::Arc))
            .collect();
        assert!(arcs.len() >= 4, "got {} arc chords", arcs.len());
        for chord in &arcs {
            let arc = chord.arc.expect("chord tagged");
            assert!((arc.cx - 10.0).abs() < 1e-9 && (arc.cy - 5.0).abs() < 1e-9);
            assert!(arc.ccw);
        }
        let last_arc = arcs.last().unwrap();
        assert!((last_arc.to.x - 15.0).abs() < 1e-9 && (last_arc.to.y - 5.0).abs() < 1e-9);
        // Cycle: rapid to entry, plunge to bottom, retract out.
        let n = segments.len();
        assert!(matches!(segments[n - 3].kind, MoveKind::Rapid));
        assert!(matches!(segments[n - 2].kind, MoveKind::Plunge));
        assert!((segments[n - 2].to.z - -5.0).abs() < 1e-9);
        assert!(matches!(segments[n - 1].kind, MoveKind::Retract));
        assert!((segments[n - 1].to.z - 2.0).abs() < 1e-9);
    }

    /// Peck cycles expand with full retract + re-entry (deep-drilling)
    /// vs small chip-break retract, mirroring the trait defaults.
    #[test]
    fn cycle_expansion_matches_trait_shapes() {
        let mut rec = CpsRecorder::new();
        begin(&mut rec);
        rec.begin_section(&section_ctx(1, tool(1, ToolKind::Drill, 9000)));
        rec.drill_peck(0.0, 0.0, -6.0, 2.0, 4.0, 120, 0.0);
        rec.end_section();
        let deep = ir_to_toolpath(&rec.into_program());
        // 2 pecks: rapid-in, plunge(2→-2), retract(→2), rapid(→-1.5),
        // plunge(→-2), plunge(-2→-6), retract(→2).
        let kinds: Vec<MoveKind> = deep.iter().map(|s| s.kind).collect();
        assert_eq!(
            kinds,
            vec![
                MoveKind::Rapid,
                MoveKind::Plunge,
                MoveKind::Retract,
                MoveKind::Rapid,
                MoveKind::Plunge,
                MoveKind::Plunge,
                MoveKind::Retract,
            ],
            "deep-drilling expansion: {deep:#?}"
        );
        assert!((deep[1].to.z - -2.0).abs() < 1e-9);
        assert!((deep[2].to.z - 2.0).abs() < 1e-9, "full retract");
        assert!((deep[3].to.z - -1.5).abs() < 1e-9, "re-entry clearance");
        assert!((deep[5].to.z - -6.0).abs() < 1e-9);
    }
}
