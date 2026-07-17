//! Project = geometry + machine + tool library + ordered list of
//! Operations. The Op is the unit of CAM work — each one carries a
//! tool reference and per-kind parameters and produces a gcode block in
//! the final program.
//!
//! Modeled after mainstream CAM tools (Carbide Create, Fusion 360 CAM,
//! Estlcum, `FreeCAD` Path Workbench) so the user's mental model translates
//! without surprises.
//!
//! This module is a thin hub: the actual types live in per-domain
//! submodules and are re-exported here so callers continue to use
//! `crate::project::X` unchanged.

// # CAM/sim pedantic-lint exemptions
// Default-impl test helpers use parallel names (`tool_a`/`tool_b`,
// `op_with`/`op_without`) that enumerate distinct test cases. Serde
// `skip_serializing_if = "is_default_…"` helpers take `&T` because that's
// the signature serde requires. `OpParams` is the user-facing
// per-op config bag — one bool per UI checkbox, so the JSON contract
// flattens the bool fields by design (see the planned move-to-OpKind-variants refactor).
#![allow(
    clippy::similar_names,
    clippy::trivially_copy_pass_by_ref,
    clippy::struct_excessive_bools
)]

pub mod config;
pub mod fixture;
pub mod machine;
pub mod op;
pub mod params;
pub mod text;
pub mod tool;

pub use config::*;
pub use fixture::*;
pub use machine::*;
pub use op::*;
pub use params::*;
pub use text::*;
pub use tool::*;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::geometry::Segment;

// ─── top level ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct Project {
    /// Imported geometry — the same `segments` the existing pipeline
    /// consumes. We keep it inline rather than referencing it by id so the
    /// project file is self-contained.
    pub segments: Vec<Segment>,

    pub machine: MachineConfig,
    pub tools: Vec<ToolEntry>,
    pub operations: Vec<Op>,

    /// Fixtures (clamps, dogs, vise jaws, hold-downs) the cutter must
    /// avoid throughout the entire program — including rapids. The sim
    /// pass tests every toolpath segment against this set and emits
    /// `SimWarning::FixtureCollision` on overlap. Default empty: a
    /// project with no fixtures behaves exactly as before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fixtures: Vec<Fixture>,

    /// First-class editable text entities — content / font / size /
    /// position / rotation / spacing. The pipeline pre-pass renders each
    /// `TextLayer` to segments before any op runs so the existing
    /// `Engrave` (and friends) op can target the rendered geometry by
    /// layer name `__text_<id>`. Edits to a `TextLayer` re-run the
    /// pipeline; cache keys include `text_layers` content.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_layers: Vec<TextLayer>,

    /// Explicit work-offset (MVP) between the geometry frame
    /// (where the DXF / SVG was drawn) and the gcode WCS origin
    /// (where the user zeros the spindle on the real machine). All
    /// zeros (default) means "geometry origin = WCS origin". Full
    /// G54..G59 + per-fixture origins are a future feature; this
    /// field gives a single offset the sim and the WCS warning
    /// consult. Persisted into project files; legacy files lacking
    /// the field default to zeros and behave exactly as before.
    #[serde(default, skip_serializing_if = "WorkOffset::is_default")]
    pub work_offset: WorkOffset,

    /// Physical stock envelope, resolved to an axis-aligned box in
    /// the geometry frame. The frontend derives this from its auto/manual
    /// stock UI (margin / custom dims / offset) via `computeFootprint`
    /// and sends the resolved box; a CLI / server consumer sets the
    /// dimensions directly. `None` (default) skips the `out_of_stock`
    /// scan, so a transport that doesn't model stock simply gets no
    /// out-of-stock checks. The stock
    /// top sits at z = 0 (the WCS / geometry origin plane); the body
    /// extends downward by `thickness_mm`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stock: Option<StockConfig>,

    /// Relief / 3-axis surfacing sources — the target Z(x,y) surfaces
    /// that [`OpKind::ReliefMill`] ops finish. Stored at project level (like
    /// `text_layers`) and referenced by `source_id`, not embedded in the op,
    /// because a surface grid is large and ops get cloned + hashed. Each
    /// carries a normalized-brightness grid; the op maps it to Z at planning
    /// time. Default empty: projects with no relief ops are unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relief_sources: Vec<ReliefSource>,

    /// When `true`, the pipeline runs an optional tool-change-order
    /// optimization that groups consecutive same-tool work so a
    /// `T1 / T2 / T1` program emits `T1, T1, T2` with ONE tool change
    /// instead of two. Matters most on manual machines, where every swap
    /// is minutes + a re-probe + operator-error risk. The reorder is
    /// barrier-aware: program-only ops (Pause / Homing / …) and any op
    /// with [`Op::pin_order`] stay put and nothing moves across them, so
    /// a deliberate cut order (tabs, thin walls) is preserved. `false`
    /// (default) keeps the declared op order unchanged. See
    /// `order_ops_by_tool` in the pipeline.
    #[serde(default, skip_serializing_if = "crate::project::op::is_false")]
    pub group_ops_by_tool: bool,
}

/// A target surface source for relief / ball-nose surfacing. Holds a
/// row-major grid (see [`ReliefGrid`]) plus its world placement. Two
/// producers feed the same type: a grayscale image decoded frontend-side
/// ([`ReliefGrid::Grayscale`]) and an STL rasterized to real geometry Z
/// ([`ReliefGrid::Heightgrid`], via `SurfaceField::from_stl`). The
/// [`OpKind::ReliefMill`] driver turns either kind into a
/// [`crate::cam::surface::SurfaceField`]; the placement (`origin` / `cell`
/// / `cols` / `rows`) is shared, only the per-cell payload differs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReliefSource {
    /// Stable id referenced by [`OpKind::ReliefMill::source_id`].
    pub id: u32,
    /// Human-readable label (e.g. the source filename). Optional.
    #[serde(default)]
    pub name: String,
    /// World XY of the grid's min corner (the (0,0) cell's lower-left).
    pub origin: crate::geometry::Point2,
    /// Cell size in mm (square cells / pixel pitch in world units).
    pub cell: f64,
    pub cols: u32,
    pub rows: u32,
    /// The per-cell surface data — a normalized-brightness grid (image
    /// relief) or a real target-Z height grid (STL). Length must be
    /// `cols * rows`. See [`ReliefGrid`].
    pub grid: ReliefGrid,
}

impl ReliefSource {
    /// The brightness grid, if this is a [`ReliefGrid::Grayscale`] source;
    /// `None` for a height grid (which the raster-engrave driver can't use —
    /// there is no brightness to modulate laser power from).
    #[must_use]
    pub fn brightness(&self) -> Option<&[f32]> {
        match &self.grid {
            ReliefGrid::Grayscale { brightness } => Some(brightness),
            ReliefGrid::Heightgrid { .. } => None,
        }
    }
}

/// The per-cell payload of a [`ReliefSource`], tagged by `kind`. Decoupled
/// from the shared placement so both producers reuse the same footprint
/// plumbing. A [`OpKind::ReliefMill`] op accepts either kind; a
/// [`OpKind::RasterEngrave`] op only accepts [`ReliefGrid::Grayscale`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReliefGrid {
    /// Row-major normalized brightness in `[0, 1]`. The op remaps it to Z
    /// through `z_min_mm` / `z_max_mm` at planning time
    /// (`SurfaceField::from_grayscale`), so depth is a cheap op-level knob
    /// that retunes without re-decoding the image.
    Grayscale { brightness: Vec<f32> },
    /// Row-major real target Z per cell (mm), stock top at 0 and relief
    /// carved downward — the geometry an STL rasterizes to
    /// (`SurfaceField::from_stl` / `from_mesh`, which already shift the
    /// model top to 0). The op cuts this Z directly (clamped to tool
    /// reach), NOT through a brightness → Z remap.
    Heightgrid { z: Vec<f32> },
}

impl ReliefGrid {
    /// Number of cells in the grid (brightness or Z length). Must equal
    /// `cols * rows` on the owning [`ReliefSource`].
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            ReliefGrid::Grayscale { brightness } => brightness.len(),
            ReliefGrid::Heightgrid { z } => z.len(),
        }
    }

    /// True when the grid carries no cells.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Resolved stock box. See [`Project::stock`]. Kept deliberately
/// thin — the auto/manual/margin derivation lives frontend-side (it's a
/// UI convenience for sizing the box to imported geometry); the core
/// only needs the final axis-aligned envelope for the `out_of_stock`
/// scan (and, in future, stock-aware sim / rapid / holder checks).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StockConfig {
    /// Min corner (x, y) of the stock box in the geometry frame (mm).
    #[serde(default)]
    pub origin: [f64; 2],
    /// X extent of the stock box (mm).
    pub width_mm: f64,
    /// Y extent of the stock box (mm).
    pub height_mm: f64,
    /// Material thickness (mm). The stock body spans
    /// z ∈ [`top_z_mm` − thickness, `top_z_mm`].
    pub thickness_mm: f64,
    /// Z of the stock TOP plane (mm) in the WCS frame. Default 0 ⇒
    /// the top sits at the WCS origin plane (the legacy assumption), body
    /// extending down to `-thickness_mm`. A non-zero value models zeroing
    /// the machine somewhere other than the stock top (e.g. on the bed,
    /// `top_z_mm = +thickness`); the `out_of_stock` scan and the sim
    /// heightmap shift with it. Distinct from `WorkOffset::z_mm` (which
    /// moves the WCS origin relative to the geometry) — this moves the
    /// stock material relative to that origin.
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub top_z_mm: f64,
    /// Two-sided (flip-stock) registration. `None` (the default) ⇒ a
    /// single-sided job — no flip axis, no dowel pins, and the field is
    /// omitted from the wire so existing projects are untouched. `Some`
    /// enables the two-sided workflow: the axis the stock is turned about
    /// between the front and back programs, plus optional dowel-pin
    /// registration. The flip transform ([`crate::cam::flip`]) reads the
    /// axis; auto-placement of the dowel holes is a later phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip: Option<FlipRegistration>,
}

/// Two-sided machining registration: how the stock is turned over between
/// the front and back programs. See [`StockConfig::flip`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FlipRegistration {
    /// The axis the physical stock is rotated 180° about to expose the back
    /// face. See [`FlipAxis`] for the exact XY/Z consequence — getting this
    /// wrong is the classic two-sided error, so the UI must show it
    /// unmistakably.
    #[serde(default)]
    pub axis: FlipAxis,
    /// Optional dowel-pin registration. `None` ⇒ the operator aligns the
    /// flip by fence/eye. `Some` ⇒ the front program drills dowel holes and
    /// the back program references their mirrored positions, so the flip is
    /// repeatable to the pin fit. Hole positions are auto-derived in a
    /// later phase; this carries only the operator-facing knobs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dowels: Option<DowelPinConfig>,
}

/// Which axis the stock is flipped about between the two sides of a
/// two-sided job. Named for the axis the flip line runs **parallel to**:
///
/// - `X` — turn the stock about a line parallel to the X-axis (like
///   flipping a page whose spine runs left–right). X is preserved, Y
///   mirrors about the stock's Y centre-line, Z inverts. On a 50 mm-tall
///   stock a point `(10, 5)` lands at `(10, 45)`.
/// - `Y` — turn about a line parallel to the Y-axis (spine runs
///   front–back). Y is preserved, X mirrors about the stock's X
///   centre-line, Z inverts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FlipAxis {
    /// Flip about a line parallel to X — mirrors Y, preserves X.
    #[default]
    X,
    /// Flip about a line parallel to Y — mirrors X, preserves Y.
    Y,
}

/// Auto-placed dowel-pin registration for a two-sided job. The front
/// program drills `count` holes of `diameter_mm`, inset `margin_mm` from
/// the stock edge and placed symmetrically about the flip axis so the same
/// holes line up once the stock is flipped. Exact positions are derived in
/// a later phase; this struct is only the operator-facing knobs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DowelPinConfig {
    /// Dowel hole diameter (mm) — matches the physical dowel pin.
    pub diameter_mm: f64,
    /// Number of registration holes. Two is the minimum for an
    /// unambiguous flip alignment (one leaves the part free to pivot).
    #[serde(default = "default_dowel_count")]
    pub count: u32,
    /// Inset (mm) from the stock edge to the hole centres, keeping the
    /// holes in waste stock clear of the part envelope.
    #[serde(default)]
    pub margin_mm: f64,
}

fn default_dowel_count() -> u32 {
    2
}

/// Program-level work-coordinate offset. Defaults to all
/// zeros — geometry origin == WCS origin. When the user zeros the
/// machine somewhere different from the geometry origin, set this
/// so the sim can align the heightmap to the WCS frame. The full
/// per-fixture / G54..G59 selector is a follow-up feature.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkOffset {
    /// X offset (mm) from geometry origin to WCS origin.
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub x_mm: f64,
    /// Y offset (mm) from geometry origin to WCS origin.
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub y_mm: f64,
    /// Z offset (mm) from geometry origin to WCS origin.
    /// Positive means the WCS Z=0 is ABOVE the geometry's z=0.
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub z_mm: f64,
    /// Which work coordinate system this offset applies to. The
    /// gcode emitter doesn't (yet) flip between G54..G59 — this is
    /// a labelling field for the UI + future expansion.
    #[serde(default, skip_serializing_if = "Wcs::is_default")]
    pub wcs: Wcs,
}

impl WorkOffset {
    fn is_default(v: &Self) -> bool {
        is_zero_f64(&v.x_mm)
            && is_zero_f64(&v.y_mm)
            && is_zero_f64(&v.z_mm)
            && Wcs::is_default(&v.wcs)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum Wcs {
    #[default]
    G54,
    G55,
    G56,
    G57,
    G58,
    G59,
}

impl Wcs {
    fn is_default(v: &Self) -> bool {
        matches!(v, Self::G54)
    }

    /// The gcode word that activates this WCS (`G54`..`G59`).
    /// Consumed by the post-processor prologue so the controller's
    /// active WCS matches `Project.work_offset.wcs` even when the
    /// boot-default isn't G54.
    #[must_use]
    pub fn gcode_word(self) -> &'static str {
        match self {
            Self::G54 => "G54",
            Self::G55 => "G55",
            Self::G56 => "G56",
            Self::G57 => "G57",
            Self::G58 => "G58",
            Self::G59 => "G59",
        }
    }

    /// The `P<n>` operand for `G10 L20 P<n>` that targets this WCS.
    /// `G54 = P1`, `G55 = P2`, …, `G59 = P6` per RS-274 / Mach3 / GRBL
    /// 1.1+ / LinuxCNC convention. GRBL's `tool_z_shift` must use this
    /// mapping so a user-active G55 writes its z-shift into the correct
    /// WCS rather than `P1`.
    #[must_use]
    pub fn p_number(self) -> u32 {
        match self {
            Self::G54 => 1,
            Self::G55 => 2,
            Self::G56 => 3,
            Self::G57 => 4,
            Self::G58 => 5,
            Self::G59 => 6,
        }
    }
}

/// Register this module's wire types in the OpenAPI components map.
/// Co-located with the type definitions so adding a wire type is
/// a same-file edit; `crate::schema::components_schemas` composes these.
pub(crate) fn register_schemas(map: &mut crate::schema::SchemaMap) {
    crate::schema::insert::<Project>(map, "Project");
    crate::schema::insert::<Op>(map, "Op");
    crate::schema::insert::<OpKind>(map, "OpKind");
    crate::schema::insert::<DrillCycle>(map, "DrillCycle");
    crate::schema::insert::<OpParams>(map, "OpParams");
    crate::schema::insert::<OpSource>(map, "OpSource");
    crate::schema::insert::<SourceCombine>(map, "SourceCombine");
    crate::schema::insert::<CutDirection>(map, "CutDirection");
    crate::schema::insert::<PlungeStrategy>(map, "PlungeStrategy");
    crate::schema::insert::<PocketStrategy>(map, "PocketStrategy");
    crate::schema::insert::<PatternConfig>(map, "PatternConfig");
    crate::schema::insert::<ToolEntry>(map, "ToolEntry");
    crate::schema::insert::<ToolKind>(map, "ToolKind");
    crate::schema::insert::<Coolant>(map, "Coolant");
    crate::schema::insert::<Fixture>(map, "Fixture");
    crate::schema::insert::<FixtureKind>(map, "FixtureKind");
    crate::schema::insert::<TextLayer>(map, "TextLayer");
    crate::schema::insert::<TextLayerKind>(map, "TextLayerKind");
    crate::schema::insert::<TextAlignment>(map, "TextAlignment");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_default_is_empty_but_well_typed() {
        let p = Project::default();
        assert!(p.segments.is_empty());
        assert!(p.tools.is_empty());
        assert!(p.operations.is_empty());
        assert!(p.fixtures.is_empty());
    }

    #[test]
    fn fixtures_round_trip() {
        let p = Project {
            fixtures: vec![
                Fixture {
                    id: 1,
                    name: "front clamp".into(),
                    kind: FixtureKind::Box {
                        width: 30.0,
                        depth: 50.0,
                    },
                    origin: (15.0, -25.0),
                    z_bottom: 0.0,
                    z_top: 12.0,
                    color: 0xFFA0_50C0,
                },
                Fixture {
                    id: 2,
                    name: "dog".into(),
                    kind: FixtureKind::Cylinder { radius: 6.0 },
                    origin: (-10.0, 40.0),
                    z_bottom: -1.0,
                    z_top: 8.0,
                    color: 0xFFA0_50C0,
                },
                Fixture {
                    id: 3,
                    name: "L-bracket".into(),
                    kind: FixtureKind::Polygon {
                        vertices: vec![
                            (0.0, 0.0),
                            (20.0, 0.0),
                            (20.0, 5.0),
                            (5.0, 5.0),
                            (5.0, 25.0),
                            (0.0, 25.0),
                        ],
                    },
                    origin: (60.0, 60.0),
                    z_bottom: 0.0,
                    z_top: 6.0,
                    color: 0x8080_8080,
                },
            ],
            ..Project::default()
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(back.fixtures.len(), 3);
        assert!(matches!(
            back.fixtures[0].kind,
            FixtureKind::Box { width, depth }
                if (width - 30.0).abs() < 1e-9 && (depth - 50.0).abs() < 1e-9
        ));
        assert!(matches!(
            back.fixtures[1].kind,
            FixtureKind::Cylinder { radius } if (radius - 6.0).abs() < 1e-9
        ));
        match &back.fixtures[2].kind {
            FixtureKind::Polygon { vertices } => assert_eq!(vertices.len(), 6),
            _ => panic!("expected Polygon"),
        }
    }

    #[test]
    fn project_with_no_fixtures_skips_field_on_serialize() {
        let p = Project::default();
        let json = serde_json::to_string(&p).unwrap();
        assert!(
            !json.contains("\"fixtures\""),
            "empty fixtures should be skipped: {json}"
        );
    }

    /// Project files must stay **language-agnostic**: they store enum keys
    /// (`"kind":"pocket"`) and numbers, never localized labels, so a file
    /// saved under German loads byte-identically under English (i18n epic
    /// ivac-os2k, locale-invariance requirement). The core has no locale in
    /// its serde path at all, so the guard here is twofold:
    ///   1. The on-disk fixture carries no non-ASCII text — a translated
    ///      label leaking into saved data would trip this.
    ///   2. Serialization is deterministic (round-trip byte-stable), the
    ///      prerequisite for "same project → same bytes" regardless of which
    ///      locale the GUI/CLI was in when it saved.
    ///
    /// The CLI catalog parity half of the coverage issue lands with the CLI
    /// i18n work (ivac-os2k.7), which introduces the catalog it would check.
    #[test]
    fn project_file_is_locale_invariant() {
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest.parent().unwrap().parent().unwrap();
        let path = root.join("tests/fixtures/test.vc-project.json");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));

        assert!(
            text.is_ascii(),
            "project fixture {path:?} contains non-ASCII text — a localized \
             label may have leaked into saved data; project files must store \
             language-agnostic enum keys, not translated strings"
        );

        // Round-trip through serde twice; the canonical bytes must not drift,
        // so a save never depends on ambient state such as the active locale.
        let value: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {path:?}: {e}"));
        let once = serde_json::to_string(&value).unwrap();
        let twice =
            serde_json::to_string(&serde_json::from_str::<serde_json::Value>(&once).unwrap())
                .unwrap();
        assert_eq!(once, twice, "project serialization must be deterministic");
    }
}
