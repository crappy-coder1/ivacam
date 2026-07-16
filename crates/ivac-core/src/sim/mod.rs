//! 2.5D cutting simulator primitives. The heightmap stores Z(x,y) over
//! the stock footprint and tool Z-profiles describe what radius of the
//! cutter surface reaches how far down at a given radial offset.
//!
//! # Arcs: analytic swept-arc footprint (no chord-error floor)
//!
//! `preview::interpret_with_index` still tessellates each G2/G3 into ~2°
//! chord [`gcode::preview::ToolpathSegment`]s (the wireframe renderer,
//! envelope scans, and the interactive per-segment sim all rely on that dense
//! stream and its indexing), but every arc chord now carries its parent arc's
//! center + direction in [`gcode::preview::ArcXY`]. The sweep loop
//! ([`sweep::sweep_segment`] and its dexel / partial siblings) dispatches an
//! arc-tagged chord to the analytic sub-arc footprint
//! ([`sweep::for_each_swept_cell_arc_windowed`]) instead of the straight
//! chord, so the union of a G2/G3's chords is the exact swept-arc tube.
//!
//! The upshot: the old chord-error floor `r · (1 − cos(1°))` is gone from the
//! carve — a finishing scallop at any `cell_size` matches the analytic arc,
//! not the tessellation step (bd ivac-58nl.4). The remaining tessellation is
//! purely a rendering / envelope-scan detail, not a sim-accuracy limit.

pub mod dexel;
pub mod diagnostics;
pub mod fixture_check;
pub mod heightmap;
pub mod holder;
pub mod holder_check;
pub mod rapid_check;
pub mod stl;
pub mod sweep;
pub mod timing;

pub use diagnostics::{kind_str, severity, Severity, SimDiagnostics, SimWarning};
pub use fixture_check::{check_segment_against_fixtures, FixtureCheck};
pub use holder::HolderProfile;
pub use holder_check::{check_segment_holder_against_walls, HolderCheck};

/// Register this module's wire types in the OpenAPI components map.
/// Co-located with the type definitions so adding a wire type is
/// a same-file edit; `crate::schema::components_schemas` composes these.
pub(crate) fn register_schemas(map: &mut crate::schema::SchemaMap) {
    crate::schema::insert::<diagnostics::SimWarning>(map, "SimWarning");
    crate::schema::insert::<diagnostics::SimDiagnostics>(map, "SimDiagnostics");
    crate::schema::insert::<diagnostics::Severity>(map, "SimSeverity");
    crate::schema::insert::<timing::TimeEstimate>(map, "TimeEstimate");
}
