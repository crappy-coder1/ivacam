//! `ivac` — headless converter and JSON-API surface for the Rust core.
//!
//! Subcommands:
//!   * `ivac import <file>`            — emit /import-shaped JSON to stdout
//!   * `ivac generate <file> [--post]` — emit /generate-shaped JSON
//!     (gcode + 3D preview toolpath) to stdout
//!
//! Mirrors the JSON contract in `schema/openapi.yaml`.

mod i18n;

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use ivac_core::cam::chaining::{classify_containment, segments_to_objects};
use ivac_core::cam::offsets::{
    apply_overcut_to_offsets, pocket_for_object, PocketEmit, PolylineOffset,
};
use ivac_core::cam::setup::Setup;
use ivac_core::cam::VcObject;
use ivac_core::gcode::{emit_polylines, grbl, hpgl, linuxcnc, preview};
use ivac_core::project::ToolOffset;
use ivac_core::{ImportOptions, ImportOutput};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct GenerateResponseJson<'a> {
    gcode: &'a str,
    toolpath: &'a [preview::ToolpathSegment],
    stats: GenerateStats,
}

#[derive(Serialize, Default)]
struct GenerateStats {
    object_count: usize,
    closed_object_count: usize,
    offset_count: usize,
    cut_distance: f64,
    travel_distance: f64,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::try_init().ok();
    // Resolve the UI language once, before anything user-facing is printed:
    // a `--lang` override (pulled out of argv here) beats the LANG/LC_ALL
    // environment. Everything below goes through `i18n::t`/`tp`.
    let (lang_override, rest) = i18n::extract_lang(std::env::args().skip(1).collect());
    i18n::set_locale(i18n::detect_locale(lang_override.as_deref(), |k| {
        std::env::var(k).ok()
    }));

    let mut args = rest.into_iter();
    let cmd = args.next().unwrap_or_default();
    match cmd.as_str() {
        "import" => cmd_import(args),
        "generate" => cmd_generate(args),
        "" | "-h" | "--help" => {
            print_help();
            Ok(())
        }
        other => {
            print_help();
            bail!(
                "{}",
                i18n::tp("cli.err.unknown_subcommand", &[("name", other)])
            );
        }
    }
}

fn print_help() {
    // The command skeletons (`ivac import <path>`, flag names) are literal and
    // language-agnostic; only the descriptions and labels go through the
    // catalog. Help prints to stderr so stdout stays clean JSON for pipes.
    eprintln!("{}\n", i18n::t("cli.help.tagline"));
    eprintln!("{}", i18n::t("cli.help.usage"));
    eprintln!("  ivac import <path>");
    eprintln!("      {}", i18n::t("cli.help.import"));
    eprintln!("  ivac generate <path> [--post linuxcnc|grbl|hpgl] [--diameter MM] [--depth MM]");
    eprintln!("                       [--inside|--outside|--on] [--overcut]");
    eprintln!("      {}", i18n::t("cli.help.generate"));
    eprintln!("  ivac --help");
    eprintln!("      {}", i18n::t("cli.help.help"));
    eprintln!("\n  --lang <en|de>   {}", i18n::t("cli.help.lang"));
}

fn cmd_import(mut args: impl Iterator<Item = String>) -> Result<()> {
    let path = args.next().context(i18n::t("cli.err.missing_path"))?;
    let path = PathBuf::from(path);
    let opts = ImportOptions::default();
    let out = ivac_core::input::import_path(&path, &opts).with_context(|| {
        i18n::tp(
            "cli.err.importing",
            &[("path", &path.display().to_string())],
        )
    })?;
    // Serialize the full ImportOutput — it already derives the /import
    // contract (snake_case field names matching the TS ImportResponse and
    // the wasm/server output), so emitting it directly keeps `objects`,
    // `object_meta`, and `text_entities`. The previous hand-rolled subset
    // dropped those, producing "degraded" sample fixtures the 2D canvas
    // couldn't select features from.
    serde_json::to_writer_pretty(std::io::stdout(), &out)?;
    println!();
    Ok(())
}

fn cmd_generate(args: impl Iterator<Item = String>) -> Result<()> {
    let mut path: Option<PathBuf> = None;
    let mut post_kind = "linuxcnc".to_string();
    let mut diameter = 3.0_f64;
    let mut depth = -2.0_f64;
    let mut step = -1.0_f64;
    let mut tool_offset = ToolOffset::Outside;

    let mut overcut = false;
    let mut iter = args.peekable();
    while let Some(arg) = iter.next() {
        let needs_value =
            |opt: &'static str| move || i18n::tp("cli.err.opt_needs_value", &[("opt", opt)]);
        match arg.as_str() {
            "--post" => post_kind = iter.next().with_context(needs_value("--post"))?,
            "--diameter" => {
                diameter = iter
                    .next()
                    .with_context(needs_value("--diameter"))?
                    .parse()?
            }
            "--depth" => depth = iter.next().with_context(needs_value("--depth"))?.parse()?,
            "--step" => step = iter.next().with_context(needs_value("--step"))?.parse()?,
            "--inside" => tool_offset = ToolOffset::Inside,
            "--outside" => tool_offset = ToolOffset::Outside,
            "--on" => tool_offset = ToolOffset::On,
            "--overcut" => overcut = true,
            other if path.is_none() => path = Some(PathBuf::from(other)),
            other => bail!("{}", i18n::tp("cli.err.unexpected_arg", &[("arg", other)])),
        }
    }
    let path = path.context(i18n::t("cli.err.missing_input_path"))?;

    let import =
        ivac_core::input::import_path(&path, &ImportOptions::default()).with_context(|| {
            i18n::tp(
                "cli.err.importing",
                &[("path", &path.display().to_string())],
            )
        })?;

    let (offsets, stats) = build_offsets(&import, diameter, depth, step, tool_offset, overcut);

    let mut setup = Setup::default();
    setup.tool.diameter = diameter;
    setup.mill.depth = depth;
    setup.mill.step = step;
    setup.mill.offset = tool_offset;
    setup.mill.overcut = overcut;
    setup.machine.comments = true;

    let gcode = match post_kind.as_str() {
        "linuxcnc" | "" => {
            let mut p = linuxcnc::Post::new();
            emit_polylines(&setup, &offsets, &mut p)
        }
        "grbl" => {
            let mut p = grbl::Post::new();
            emit_polylines(&setup, &offsets, &mut p)
        }
        "hpgl" => {
            let mut p = hpgl::Post::new();
            emit_polylines(&setup, &offsets, &mut p)
        }
        other => bail!("{}", i18n::tp("cli.err.unknown_post", &[("name", other)])),
    };

    let toolpath = preview::interpret(&gcode);

    let body = GenerateResponseJson {
        gcode: &gcode,
        toolpath: &toolpath,
        stats,
    };
    serde_json::to_writer_pretty(std::io::stdout(), &body)?;
    println!();
    Ok(())
}

/// Build per-object offsets from imported segments.
fn build_offsets(
    import: &ImportOutput,
    diameter: f64,
    _depth: f64,
    _step: f64,
    tool_offset: ToolOffset,
    overcut: bool,
) -> (Vec<PolylineOffset>, GenerateStats) {
    let mut objects = segments_to_objects(&import.segments);
    classify_containment(&mut objects);
    for obj in &mut objects {
        obj.tool_offset = tool_offset;
    }
    let radius = diameter * 0.5;
    let mut offsets = Vec::new();
    let mut closed = 0usize;
    for (idx, obj) in objects.iter().enumerate() {
        if obj.closed {
            closed += 1;
        }
        let pocket = obj.setup.pockets.active && obj.closed;
        if pocket {
            for mut o in pocket_for_object(
                obj,
                radius,
                false,
                6,
                PocketEmit::Cascade,
                &[],
                radius,
                0.0,
                None,
                ivac_core::project::tool::SpindleDirection::Cw,
            ) {
                o.source_object_idx = idx;
                offsets.push(o);
            }
            continue;
        }
        // Otherwise emit a single contour pass — the Rust core derives
        // direction from the user's tool_offset choice once the chain is
        // CCW-oriented (the importer ensures this for closed contours via
        // dxf-rs's CCW convention for ARC / CIRCLE).
        let delta = match tool_offset {
            ToolOffset::None | ToolOffset::On => 0.0,
            ToolOffset::Outside => -radius,
            ToolOffset::Inside => radius,
        };
        if delta.abs() < 1e-9 {
            offsets.push(PolylineOffset {
                segments: obj.segments.clone(),
                closed: obj.closed,
                level: 0,
                is_pocket: 0,
                layer: obj.layer.clone(),
                color: obj.color,
                source_object_idx: idx,
                tabs: Vec::new(),
                is_finish: false,
            });
        } else {
            for mut o in ivac_core::cam::offsets::parallel_offset_object(obj, delta) {
                o.source_object_idx = idx;
                offsets.push(o);
            }
        }
    }
    if overcut {
        apply_overcut_to_offsets(&mut offsets, &objects, radius);
    }

    let stats = GenerateStats {
        object_count: objects.len(),
        closed_object_count: closed,
        offset_count: offsets.len(),
        ..Default::default()
    };
    let _ = VcObject::new(Vec::new(), false); // exercise the constructor
    (offsets, stats)
}
