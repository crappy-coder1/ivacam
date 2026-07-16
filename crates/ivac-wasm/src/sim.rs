//! Dexel simulator bindings — wraps `ivac_core::sim::dexel::DexelField` and
//! the undercut-aware `sweep_*_dexel` family so the frontend can drive
//! incremental cutting preview at 60 fps, now with genuine undercuts for
//! form (T-slot / dovetail) tools. JS gets a `Float32Array` view directly
//! into WASM memory via `data_ptr()` (the dense top surface — byte-identical
//! to the old `Heightmap::data` for every non-form 3-axis job); each
//! `advance()` call mutates cells in place and reports the dirty AABB so the
//! renderer can re-upload only the touched sub-rectangle.
//!
//! Undercut sidecar: a form tool's carve grows interior voids the dense top
//! can't represent. Those columns are exposed to JS as three flat CSR
//! buffers (`undercut_col_index` / `undercut_span_offsets` / `undercut_spans`)
//! with their own ptr/len accessors, mirroring `data_ptr()`. They stay empty
//! for 3-axis work, so the fast path is unchanged.
//!
//! Wire shapes:
//! * `segments`: serde-serialized `Vec<ToolpathSegment>` (the same shape
//!   `PipelineResponse.toolpath` already carries).
//! * `tool`: serde-serialized `ToolEntry` (same shape as the project's
//!   tool library entries — `snake_case` fields).
//!
//! Perf: the toolpath is deserialized ONCE per Generate via
//! `set_toolpath(...)` and cached on the Simulator. `advance(from, to,
//! tool)` then indexes into the cached vec — no per-frame serde of the
//! full segment array. Tool stays as an `advance()` arg
//! because it's tiny and may change between ops.

// # CAM/sim pedantic-lint exemptions
// WASM-JS bridge passes cell counts as u32 (clamped at JS Number safe range);
// similar names (`row0`/`row1`, `col0`/`col1`) come from AABB-to-cell
// conversion.
#![allow(clippy::cast_possible_truncation, clippy::similar_names)]

use serde::Deserialize;
use wasm_bindgen::prelude::*;

use ivac_core::cam::surface::{deviation_union_into, deviation_union_of, SurfaceField};
use ivac_core::gcode::preview::ToolpathSegment;
use ivac_core::project::{Fixture, ToolEntry};
use ivac_core::sim::dexel::{DexelField, DexelSnapshot};
use ivac_core::sim::diagnostics::{SimDiagnostics, SimRunSummary};
use ivac_core::sim::heightmap::ToolProfile;
use ivac_core::sim::holder::HolderProfile;
use ivac_core::sim::sweep::{
    sweep_range_cached_dexel, sweep_segment_partial_dexel, SegmentWarningCache,
};

use crate::{into_js_error, panic_message, structured_error_to_js};

/// Owns a `DexelField` plus enough state to apply incremental sweeps.
/// Constructed with a world-space stock bbox + cell size + an explicit
/// stock-bottom Z (the span floor the old `Heightmap` left implicit); the
/// frontend then calls `advance()` with slices of `PipelineResponse.toolpath`
/// as the playhead moves.
#[wasm_bindgen]
#[derive(Debug)]
pub struct Simulator {
    field: DexelField,
    /// Warnings collected by the most recent `advance()` call. The JS
    /// driver pulls these via `take_diagnostics()` after each frame so
    /// the playbar / scene can mark offending segments. Reset on every
    /// `advance()` so each call's payload is self-contained.
    last_diagnostics: SimDiagnostics,
    /// Project-level fixtures threaded into every advance() so the
    /// fixture-collision check fires per segment. Set via
    /// `set_fixtures(...)`; default empty.
    fixtures: Vec<Fixture>,
    /// Toolpath cached at Generate time so subsequent `advance()`
    /// calls don't re-deserialize the whole array per frame
    /// Refreshed via `set_toolpath(...)` whenever a
    /// new toolpath replaces the previous one.
    toolpath: Vec<ToolpathSegment>,
    /// Sticky setup-time warnings (e.g. cell_size coarsening)
    /// that survive across `advance()` resets of `last_diagnostics`.
    /// Merged into `last_diagnostics` on every advance so the JS
    /// driver's `take_diagnostics()` keeps seeing them.
    sticky_warnings: Vec<ivac_core::sim::diagnostics::SimWarning>,
    /// Dexel snapshots for fast backward scrubbing, keyed by the
    /// segment boundary they represent (state = segments `[0, seg_idx)`
    /// carved). Kept sorted by `seg_idx`. The JS driver snapshots at
    /// clean segment boundaries during forward play and, on a backward
    /// scrub, restores the nearest snapshot ≤ the target so it only
    /// replays the tail instead of re-simulating the whole prefix. Each
    /// snapshot captures the dense top **and** the undercut sidecar, so a
    /// restore round-trips form-tool voids too. See
    /// [`Simulator::checkpoint`] / [`Simulator::restore_checkpoint`].
    checkpoints: Vec<DexelCheckpoint>,
    /// Per-segment fixture/holder/rapid diagnostics cache. A scrub-back
    /// re-sweep replays each already-swept segment's warnings instead of
    /// re-running the holder-footprint pass (the dominant re-sweep cost).
    /// Cleared whenever the toolpath or fixtures change.
    warning_cache: SegmentWarningCache,
    /// Undercut sidecar flattened to CSR for zero-copy JS reads, rebuilt at
    /// the end of every `advance()` / `partial_advance()` / restore. Empty
    /// for a pure 3-axis job. Column `i` lives at flat cell
    /// `undercut_col_index[i]`, its spans at
    /// `undercut_spans[2*undercut_span_offsets[i] .. 2*undercut_span_offsets[i+1]]`
    /// (`(lo, hi)` pairs). `undercut_span_offsets` is a CSR row-pointer with
    /// `undercut_col_index.len() + 1` entries. JS re-takes the views after
    /// every call (a growing WASM heap detaches them — same contract as
    /// `data_ptr()`).
    undercut_col_index: Vec<u32>,
    undercut_span_offsets: Vec<u32>,
    undercut_spans: Vec<f32>,
    /// Cached target surface for the red/green deviation overlay, set once
    /// per relief job via `set_deviation_target(...)` so `deviation()` can
    /// reclassify the carved field per frame without re-serializing the
    /// (potentially large) target grid each call — mirrors how `toolpath`
    /// is cached. `None` = overlay off.
    deviation_target: Option<DeviationTarget>,
    /// Persistent row-major `cols * rows` per-cell deviation-class buffer,
    /// index-aligned with `data_ptr()`. `deviation_recompute*` write into it
    /// and JS reads it zero-copy via `deviation_ptr()` (same contract as the
    /// dense top). Kept between frames so `deviation_recompute_in` can rewrite
    /// only the dirty AABB while every other cell keeps its still-correct
    /// class. Empty until the first recompute / cleared when the overlay is off.
    deviation_buf: Vec<u8>,
}

/// The cached deviation-overlay target: the relief surface(s) plus the
/// world-Z datum their `z = 0` maps to and the on-target tolerance band (mm).
/// `surfaces` holds every enabled relief op's target; a carved cell is
/// classified against their deepest-cut union (see
/// [`ivac_core::cam::surface::deviation_union_into`]), so a project milling
/// several distinct reliefs verifies all of them at once.
#[derive(Debug)]
struct DeviationTarget {
    surfaces: Vec<SurfaceField>,
    surface_z0: f32,
    tol: f32,
}

/// One backward-scrub dexel snapshot. `snapshot` captures the full carve
/// state (dense top + undercut sidecar) at the moment `[0, seg_idx)` had
/// been carved.
#[derive(Debug)]
struct DexelCheckpoint {
    seg_idx: u32,
    snapshot: DexelSnapshot,
}

#[wasm_bindgen]
impl Simulator {
    /// Build a fresh simulator covering the rectangle
    /// `[min_x, max_x] × [min_y, max_y]` with `cell_size`-mm cells. Every
    /// column starts as a single full-height solid span from `stock_bottom_z`
    /// up to `top_z` (the un-cut stock surface); the dense top reads `top_z`.
    ///
    /// `stock_bottom_z` is the span floor — the physical stock bottom
    /// (`top_z − thickness`). A bad value (`>= top_z`, or `NaN`) is guarded
    /// down to `top_z − 1.0` so the constructor never traps the wasm module;
    /// the frontend always passes a valid floor.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new(
        min_x: f64,
        min_y: f64,
        max_x: f64,
        max_y: f64,
        cell_size: f64,
        top_z: f32,
        stock_bottom_z: f32,
    ) -> Self {
        // Guard only invalid input (NaN compares false → fallback); a valid
        // floor passes through untouched so thin stock keeps its real bottom.
        let stock_bottom_z = if stock_bottom_z < top_z {
            stock_bottom_z
        } else {
            top_z - 1.0
        };
        Self {
            field: DexelField::from_bbox(
                min_x,
                min_y,
                max_x,
                max_y,
                cell_size,
                top_z,
                stock_bottom_z,
            ),
            last_diagnostics: SimDiagnostics::new(),
            fixtures: Vec::new(),
            toolpath: Vec::new(),
            sticky_warnings: Vec::new(),
            checkpoints: Vec::new(),
            warning_cache: SegmentWarningCache::new(),
            undercut_col_index: Vec::new(),
            undercut_span_offsets: vec![0],
            undercut_spans: Vec::new(),
            deviation_target: None,
            deviation_buf: Vec::new(),
        }
    }

    /// Reset every cell to `top_z` and clear the dirty AABB. Call this
    /// when a new Generate response replaces the toolpath the simulator
    /// was tracking.
    pub fn reset(&mut self) {
        self.field.reset();
        self.last_diagnostics = SimDiagnostics::new();
        self.refresh_undercut_csr();
    }

    /// Snapshot the current heightfield under `seg_idx` for fast backward
    /// scrubbing. The caller must invoke this only at a clean segment
    /// boundary — i.e. when segments `[0, seg_idx)` are exactly carved
    /// (no partial segment in progress) — so the snapshot is a valid
    /// replay base. A snapshot already stored at `seg_idx` is overwritten
    /// (idempotent re-snapshot of the same state). Snapshots stay sorted
    /// by `seg_idx`.
    pub fn checkpoint(&mut self, seg_idx: u32) {
        let snapshot = self.field.snapshot();
        match self
            .checkpoints
            .binary_search_by_key(&seg_idx, |c| c.seg_idx)
        {
            Ok(i) => self.checkpoints[i].snapshot = snapshot,
            Err(i) => self
                .checkpoints
                .insert(i, DexelCheckpoint { seg_idx, snapshot }),
        }
    }

    /// Restore the heightfield from the snapshot stored at exactly
    /// `seg_idx` (as returned by [`Simulator::nearest_checkpoint`]),
    /// marking the whole grid dirty so the renderer re-uploads. Returns
    /// `true` on a hit. The caller then forward-replays `[seg_idx, target]`
    /// to reach the scrub target. Diagnostics are NOT restored here — the
    /// JS driver keeps its own per-checkpoint diagnostics snapshot.
    pub fn restore_checkpoint(&mut self, seg_idx: u32) -> bool {
        let Ok(i) = self
            .checkpoints
            .binary_search_by_key(&seg_idx, |c| c.seg_idx)
        else {
            return false;
        };
        // `restore` copies back the dense top + undercut sidecar and marks
        // the whole grid dirty so the renderer re-uploads everything. Disjoint
        // field borrows (`self.field` vs `self.checkpoints`) let this skip a
        // snapshot clone.
        self.field.restore(&self.checkpoints[i].snapshot);
        self.last_diagnostics = SimDiagnostics::new();
        self.refresh_undercut_csr();
        true
    }

    /// The largest checkpoint `seg_idx` that is `≤ target`, or `-1` when
    /// none exists. The driver uses this to decide a backward scrub's
    /// replay start: restore there, then replay only `[result, target]`.
    #[must_use]
    pub fn nearest_checkpoint(&self, target: u32) -> i64 {
        self.checkpoints
            .iter()
            .rev()
            .find(|c| c.seg_idx <= target)
            .map_or(-1, |c| i64::from(c.seg_idx))
    }

    /// Drop every heightmap checkpoint. Call when the toolpath is
    /// replaced (a new Generate) so stale snapshots can't be restored
    /// against a different program.
    pub fn clear_checkpoints(&mut self) {
        self.checkpoints.clear();
    }

    /// Number of stored checkpoints (telemetry / tests).
    #[must_use]
    pub fn checkpoint_count(&self) -> u32 {
        self.checkpoints.len() as u32
    }

    /// Cache the full toolpath on the WASM side. Called once per
    /// Generate; subsequent `advance(...)` calls index into this vec
    /// without per-frame serde. Returns the cached segment count so the
    /// caller can assert the round-trip succeeded.
    pub fn set_toolpath(&mut self, segments: JsValue) -> Result<u32, JsValue> {
        let parsed: Vec<ToolpathSegment> =
            serde_wasm_bindgen::from_value(segments).map_err(into_js_error)?;
        let n = parsed.len() as u32;
        self.toolpath = parsed;
        // New program → cached per-segment diagnostics + heightmap
        // checkpoints are stale.
        self.warning_cache.clear();
        self.checkpoints.clear();
        Ok(n)
    }

    /// Drop the cached toolpath. Call when the project's toolpath is
    /// invalidated (e.g. the Generate response is cleared) to free
    /// WASM-side memory.
    pub fn clear_toolpath(&mut self) {
        self.toolpath = Vec::new();
        self.warning_cache.clear();
        self.checkpoints.clear();
    }

    /// Number of cached segments.
    #[must_use]
    pub fn toolpath_len(&self) -> u32 {
        self.toolpath.len() as u32
    }

    /// Record that the driver coarsened cell_size to fit the
    /// user's `maxSimulationCells` budget. The driver should call this
    /// once at `Simulator::new`-time when it coarsens, passing the
    /// originally-requested cell size and the coarsened one. The
    /// warning rides out via `take_diagnostics()` like any other sim
    /// warning. Stored on the simulator (NOT cleared by `advance()`)
    /// so the UI keeps seeing it across playhead changes.
    pub fn push_cell_size_coarsened(
        &mut self,
        original_cell_size_mm: f64,
        coarsened_cell_size_mm: f64,
        reason: String,
    ) {
        use ivac_core::sim::diagnostics::SimWarning;
        // Replace any existing sticky CellSizeCoarsened — only the most
        // recent coarsening matters (a rebuild with different cell
        // counts overrides the prior decision).
        self.sticky_warnings
            .retain(|w| !matches!(w, SimWarning::CellSizeCoarsened { .. }));
        let warn = SimWarning::CellSizeCoarsened {
            original_cell_size_mm,
            coarsened_cell_size_mm,
            reason,
        };
        self.last_diagnostics.push(warn.clone());
        self.sticky_warnings.push(warn);
    }

    /// Replace the simulator's fixture set. Pass the project's fixtures
    /// array (serialized as `Vec<Fixture>`) so subsequent `advance()`
    /// calls can emit `FixtureCollision` warnings. Pass an empty array
    /// to clear.
    pub fn set_fixtures(&mut self, fixtures: JsValue) -> Result<(), JsValue> {
        let parsed: Vec<Fixture> =
            serde_wasm_bindgen::from_value(fixtures).map_err(into_js_error)?;
        self.fixtures = parsed;
        // Fixture-collision warnings depend on the fixture set — drop the
        // cached per-segment diagnostics so they recompute against the new
        // fixtures.
        self.warning_cache.clear();
        Ok(())
    }

    /// Pull and clear the diagnostics collected by the most recent
    /// `advance()` call. Returns a JSON-shaped `SimDiagnostics`.
    /// Sticky warnings (cell-size coarsening) are merged in so
    /// the UI keeps seeing them across playhead movements.
    pub fn take_diagnostics(&mut self) -> Result<JsValue, JsValue> {
        let mut taken = std::mem::take(&mut self.last_diagnostics);
        for w in &self.sticky_warnings {
            taken.push(w.clone());
        }
        serde_wasm_bindgen::to_value(&taken).map_err(into_js_error)
    }

    /// Apply sweeps for toolpath segments `[from_idx, to_idx)` from the
    /// cached toolpath (set via `set_toolpath(...)`). Returns the
    /// resulting dirty AABB encoded as `[ix0, iy0, ix1, iy1]` so the
    /// JS renderer knows which mesh vertices to update; an empty `Vec`
    /// means no cells changed. The heightmap's internal dirty AABB is
    /// cleared first so the returned bounds reflect only this call.
    /// `tool` stays as an arg because it's tiny and may change between
    /// ops within a single toolpath.
    pub fn advance(
        &mut self,
        tool: JsValue,
        from_idx: u32,
        to_idx: u32,
    ) -> Result<Vec<u32>, JsValue> {
        let tool_entry: ToolEntry = from_tool_value(tool)?;
        // Guard the sweep with catch_unwind so a panic inside the
        // per-frame carve surfaces as a structured JS error rather than
        // trapping (aborting) the whole wasm instance mid-playback —
        // mirrors the pipeline `generate()` envelope.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // Inline the body that advance_inner provides for the test-only
            // path. We need disjoint borrows of `self.toolpath` (read) and
            // `self.field` / `self.last_diagnostics` (mutate), which
            // Rust's field-level split borrowing allows here.
            self.field.clear_dirty();
            self.last_diagnostics = SimDiagnostics::new();
            let profile = ToolProfile::from_tool(&tool_entry);
            let holder = HolderProfile::from_tool(&tool_entry);
            let touched = sweep_range_cached_dexel(
                &mut self.field,
                &self.toolpath,
                from_idx as usize,
                to_idx as usize,
                &profile,
                &self.fixtures,
                holder.as_ref(),
                &mut self.last_diagnostics,
                &mut self.warning_cache,
            );
            // Emit a single tracing::info line per advance so the
            // frontend (and post-mortem tooling) have a stable telemetry
            // record of cells_carved + per-kind warning
            // counts. `total_seconds` is left 0 here because advance()
            // doesn't wall-clock itself — the JS driver can pair this
            // with a Performance.now() delta when persisting.
            SimRunSummary::from_diagnostics(&self.last_diagnostics, u64::from(touched), 0.0).log();
            // Re-flatten the undercut sidecar so the JS driver can read the
            // fresh CSR (empty for 3-axis work; only form tools populate it).
            self.refresh_undercut_csr();
            match self.field.dirty_aabb() {
                Some((ix0, iy0, ix1, iy1)) => vec![ix0, iy0, ix1, iy1],
                None => Vec::new(),
            }
        }));
        result.map_err(|p| sweep_panic_to_js(&p))
    }

    /// Carve only the chunk `[t_start, t_end]` (parametric position) of
    /// segment `seg_idx` from the cached toolpath. Same wire shape as
    /// `advance(...)`: returns the dirty AABB as `[ix0, iy0, ix1, iy1]`,
    /// empty when no cells changed. Used by the per-frame driver so the
    /// 3D-sim destruction visually tracks the cutter inside long
    /// segments (drill plunges, long cuts) instead of popping in at
    /// segment-start. Fixture / holder / rapid warnings fire only
    /// on the first slice of the segment (`t_start ≈ 0`) so 60 fps
    /// driver frames don't duplicate diagnostics.
    pub fn partial_advance(
        &mut self,
        tool: JsValue,
        seg_idx: u32,
        t_start: f64,
        t_end: f64,
    ) -> Result<Vec<u32>, JsValue> {
        let tool_entry: ToolEntry = from_tool_value(tool)?;
        let idx = seg_idx as usize;
        if idx >= self.toolpath.len() {
            return Err(JsValue::from_str(
                "partial_advance: seg_idx out of range for cached toolpath",
            ));
        }
        // Same catch_unwind guard as advance() — a sweep panic in
        // the per-frame partial carve must not trap the wasm module.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.field.clear_dirty();
            self.last_diagnostics = SimDiagnostics::new();
            let profile = ToolProfile::from_tool(&tool_entry);
            let holder = HolderProfile::from_tool(&tool_entry);
            let _touched = sweep_segment_partial_dexel(
                &mut self.field,
                &self.toolpath[idx],
                &profile,
                idx,
                &self.fixtures,
                holder.as_ref(),
                &mut self.last_diagnostics,
                t_start,
                t_end,
            );
            self.refresh_undercut_csr();
            match self.field.dirty_aabb() {
                Some((ix0, iy0, ix1, iy1)) => vec![ix0, iy0, ix1, iy1],
                None => Vec::new(),
            }
        }));
        result.map_err(|p| sweep_panic_to_js(&p))
    }

    /// Number of grid columns (X cells).
    #[must_use]
    pub fn cols(&self) -> u32 {
        self.field.cols
    }

    /// Number of grid rows (Y cells).
    #[must_use]
    pub fn rows(&self) -> u32 {
        self.field.rows
    }

    /// Cell side length in world units (mm).
    #[must_use]
    pub fn cell_size(&self) -> f64 {
        self.field.cell
    }

    /// World X of the field origin (cell `(0, 0)`'s lower-left corner).
    #[must_use]
    pub fn origin_x(&self) -> f64 {
        self.field.origin.x
    }

    /// World Y of the field origin.
    #[must_use]
    pub fn origin_y(&self) -> f64 {
        self.field.origin.y
    }

    /// Stock-top Z. Cells the cutter has not reached still report this.
    #[must_use]
    pub fn top_z(&self) -> f32 {
        self.field.top_z
    }

    /// Serialize the carved stock as a binary STL. The perimeter skirt
    /// drops to `stock_bottom_z` at every edge sample so the result reads
    /// as a solid. Wired up via the File menu's "Export simulated stock as
    /// STL..." entry.
    ///
    /// Meshes the dense top surface **and** the undercut void cavities from
    /// the sidecar (a form-tool's T-slot / dovetail voids), so the STL
    /// matches what the 3-D preview shows rather than flattening the voids
    /// to the top surface. For a pure 3-axis job (empty sidecar) the output
    /// is byte-identical to the old dense-only heightfield mesh.
    #[must_use]
    pub fn export_stl(&self, stock_bottom_z: f32) -> Vec<u8> {
        ivac_core::sim::stl::dexel_to_stl_binary(&self.field, stock_bottom_z)
    }

    /// Pointer to the dense top-surface f32 buffer. JS wraps it as
    /// `new Float32Array(wasm.memory.buffer, sim.data_ptr(),
    /// sim.cols() * sim.rows())`.
    ///
    /// IMPORTANT: any operation that grows WASM linear memory invalidates
    /// the underlying `ArrayBuffer` of `WebAssembly.Memory.buffer`, which
    /// detaches every existing typed-array view. `advance()` allocates
    /// transiently while deserializing segments, so it MAY trigger
    /// growth — re-take the `Float32Array` view after every `advance()`
    /// call. The construction itself is O(1).
    #[must_use]
    pub fn data_ptr(&self) -> *const f32 {
        self.field.top_ptr()
    }

    /// Cache the target relief surface(s) for the red/green deviation overlay —
    /// the correctness view GrblGru can't offer (it never carves). `surfaces`
    /// is a serde-serialized array of [`SurfaceField`] (`snake_case` fields,
    /// same shape the STL rasterizer returns) — one per enabled relief op, so a
    /// multi-relief project verifies all of them via their deepest-cut union;
    /// `surface_z0` is the world Z their `z = 0` datum maps to (pass `top_z()`
    /// for a relief job); `tol` is the on-target band half-width in mm. An
    /// empty array clears the overlay (same as `clear_deviation_target`).
    /// Caching once (instead of passing every frame) keeps the per-frame
    /// recompute cheap for large targets. Replaces any previous target.
    pub fn set_deviation_target(
        &mut self,
        surfaces: JsValue,
        surface_z0: f32,
        tol: f32,
    ) -> Result<(), JsValue> {
        let surfaces: Vec<SurfaceField> =
            serde_wasm_bindgen::from_value(surfaces).map_err(into_js_error)?;
        if surfaces.is_empty() {
            self.clear_deviation_target();
            return Ok(());
        }
        self.deviation_target = Some(DeviationTarget {
            surfaces,
            surface_z0,
            tol,
        });
        // Force the next recompute to rebuild the whole class buffer against
        // the new target rather than partially patching stale classes.
        self.deviation_buf.clear();
        Ok(())
    }

    /// Drop the cached deviation target (overlay turned off).
    pub fn clear_deviation_target(&mut self) {
        self.deviation_target = None;
        self.deviation_buf = Vec::new();
    }

    /// Whether a deviation target is cached.
    #[must_use]
    pub fn has_deviation_target(&self) -> bool {
        self.deviation_target.is_some()
    }

    /// Classify the carved field against the cached deviation target (see
    /// [`SurfaceField::deviation_of`]). Returns a fresh row-major `cols * rows`
    /// `Vec` of [`ivac_core::cam::surface::Deviation`] codes (0 = on-target,
    /// 1 = gouge, 2 = rest stock), aligned index-for-index with `data_ptr()`.
    /// Empty when no target is cached.
    ///
    /// The zero-copy per-frame path is [`Simulator::deviation_recompute`] /
    /// [`Simulator::deviation_recompute_in`] plus [`Simulator::deviation_ptr`];
    /// this owned-`Vec` form stays for callers (and tests) that just want a
    /// one-shot snapshot without touching the persistent buffer.
    #[must_use]
    pub fn deviation(&self) -> Vec<u8> {
        match &self.deviation_target {
            Some(t) => deviation_union_of(&t.surfaces, &self.field, t.surface_z0, t.tol),
            None => Vec::new(),
        }
    }

    /// Reclassify the ENTIRE carved field into the persistent class buffer
    /// (`deviation_ptr()` / `deviation_len()`), resizing it to `cols * rows`.
    /// Use on the full-repaint paths — overlay turned on, a sim reset/backstep
    /// replay, or a mesh rebuild — after which JS re-takes the zero-copy view
    /// and uploads the whole grid. A no-op (buffer emptied) when no target is
    /// cached.
    pub fn deviation_recompute(&mut self) {
        let Some(t) = self.deviation_target.as_ref() else {
            self.deviation_buf.clear();
            return;
        };
        let n = (self.field.cols as usize) * (self.field.rows as usize);
        self.deviation_buf.clear();
        self.deviation_buf.resize(n, 0);
        deviation_union_into(
            &t.surfaces,
            &self.field,
            t.surface_z0,
            t.tol,
            &mut self.deviation_buf,
            0,
            0,
            self.field.cols,
            self.field.rows,
        );
    }

    /// Reclassify ONLY the half-open cell rectangle `[ix0, ix1) × [iy0, iy1)`
    /// in the persistent class buffer, leaving every other cell — whose height
    /// didn't change this frame — at its prior, still-correct class. Pass the
    /// carve's dirty AABB so a frame reclassifies just the cells the tool swept
    /// instead of re-sampling the whole target, mirroring the mesh's
    /// partial-AABB re-upload. JS then re-takes the view and uploads only that
    /// rectangle. A no-op when no target is cached; falls back to a full
    /// recompute if the buffer isn't yet sized to the current grid (e.g. the
    /// first partial after a resolution change slipped through without a full
    /// repaint).
    pub fn deviation_recompute_in(&mut self, ix0: u32, iy0: u32, ix1: u32, iy1: u32) {
        let n = (self.field.cols as usize) * (self.field.rows as usize);
        if self.deviation_buf.len() != n {
            self.deviation_recompute();
            return;
        }
        let Some(t) = self.deviation_target.as_ref() else {
            return;
        };
        deviation_union_into(
            &t.surfaces,
            &self.field,
            t.surface_z0,
            t.tol,
            &mut self.deviation_buf,
            ix0,
            iy0,
            ix1,
            iy1,
        );
    }

    /// Pointer to the persistent per-cell deviation-class buffer — one `u8`
    /// [`ivac_core::cam::surface::Deviation`] code per cell, row-major and
    /// index-aligned with `data_ptr()`. Zero-copy read the same way as the
    /// dense top: re-take the `Uint8Array` view after every `advance()` /
    /// recompute, since a growing WASM heap detaches it. Length is
    /// `deviation_len()` (0 until the first recompute or when the overlay is
    /// off).
    #[must_use]
    pub fn deviation_ptr(&self) -> *const u8 {
        self.deviation_buf.as_ptr()
    }

    /// Length of the persistent deviation-class buffer (`cols * rows` once a
    /// recompute has run, else `0`).
    #[must_use]
    pub fn deviation_len(&self) -> u32 {
        self.deviation_buf.len() as u32
    }

    /// Number of columns currently carrying an undercut sidecar entry (`0`
    /// for any pure 3-axis job). The JS driver checks this to skip the
    /// undercut upload entirely when there's nothing to draw.
    #[must_use]
    pub fn undercut_column_count(&self) -> u32 {
        self.undercut_col_index.len() as u32
    }

    /// Pointer to the flat cell-index buffer (one `u32` per undercut column).
    /// See the CSR contract on [`Simulator::undercut_spans_ptr`]. Re-take the
    /// `Uint32Array` view after every `advance()` — a growing heap detaches it.
    #[must_use]
    pub fn undercut_col_index_ptr(&self) -> *const u32 {
        self.undercut_col_index.as_ptr()
    }

    /// Length of the undercut cell-index buffer (== `undercut_column_count`).
    #[must_use]
    pub fn undercut_col_index_len(&self) -> u32 {
        self.undercut_col_index.len() as u32
    }

    /// Pointer to the CSR row-pointer buffer: `undercut_column_count + 1`
    /// `u32`s where column `i`'s spans occupy `undercut_spans[2*off[i] ..
    /// 2*off[i+1]]`. Re-take the `Uint32Array` view after every `advance()`.
    #[must_use]
    pub fn undercut_span_offsets_ptr(&self) -> *const u32 {
        self.undercut_span_offsets.as_ptr()
    }

    /// Length of the CSR row-pointer buffer (`undercut_column_count + 1`).
    #[must_use]
    pub fn undercut_span_offsets_len(&self) -> u32 {
        self.undercut_span_offsets.len() as u32
    }

    /// Pointer to the flat span buffer — consecutive `(lo, hi)` `f32` pairs,
    /// sliced per column by `undercut_span_offsets`. JS wraps it as
    /// `new Float32Array(wasm.memory.buffer, sim.undercut_spans_ptr(),
    /// sim.undercut_spans_len())`. Re-take the view after every `advance()`.
    #[must_use]
    pub fn undercut_spans_ptr(&self) -> *const f32 {
        self.undercut_spans.as_ptr()
    }

    /// Length of the flat span buffer (`2 × total span count`).
    #[must_use]
    pub fn undercut_spans_len(&self) -> u32 {
        self.undercut_spans.len() as u32
    }
}

impl Simulator {
    /// Re-flatten the [`DexelField`]'s undercut sidecar into the CSR buffers
    /// JS reads. Called at the end of every mutation entry point. O(total
    /// undercut spans) — effectively free for a pure 3-axis job (empty
    /// sidecar → `undercut_span_offsets == [0]`, the other two empty).
    fn refresh_undercut_csr(&mut self) {
        let (col_index, span_offsets, spans) = self.field.undercut_csr();
        self.undercut_col_index = col_index;
        self.undercut_span_offsets = span_offsets;
        self.undercut_spans = spans;
    }

    /// Pure-Rust core of `advance()` — used by tests that don't want to
    /// route through `JsValue`. Gated behind `#[cfg(test)]` to silence
    /// `dead_code` on the wasm production build.
    #[cfg(test)]
    pub(crate) fn advance_inner(
        &mut self,
        segments: &[ToolpathSegment],
        tool: &ToolEntry,
        from_idx: u32,
        to_idx: u32,
    ) -> Vec<u32> {
        self.field.clear_dirty();
        self.last_diagnostics = SimDiagnostics::new();
        let profile = ToolProfile::from_tool(tool);
        let holder = HolderProfile::from_tool(tool);
        let _touched = sweep_range_cached_dexel(
            &mut self.field,
            segments,
            from_idx as usize,
            to_idx as usize,
            &profile,
            &self.fixtures,
            holder.as_ref(),
            &mut self.last_diagnostics,
            &mut self.warning_cache,
        );
        self.refresh_undercut_csr();
        match self.field.dirty_aabb() {
            Some((ix0, iy0, ix1, iy1)) => vec![ix0, iy0, ix1, iy1],
            None => Vec::new(),
        }
    }

    /// Pure-Rust core of `partial_advance()` — same role as
    /// `advance_inner`, used by Rust-side tests that can't pass
    /// `JsValue`.
    #[cfg(test)]
    pub(crate) fn partial_advance_inner(
        &mut self,
        segments: &[ToolpathSegment],
        tool: &ToolEntry,
        seg_idx: u32,
        t_start: f64,
        t_end: f64,
    ) -> Vec<u32> {
        self.field.clear_dirty();
        self.last_diagnostics = SimDiagnostics::new();
        let profile = ToolProfile::from_tool(tool);
        let holder = HolderProfile::from_tool(tool);
        let idx = seg_idx as usize;
        if idx < segments.len() {
            let _touched = sweep_segment_partial_dexel(
                &mut self.field,
                &segments[idx],
                &profile,
                idx,
                &self.fixtures,
                holder.as_ref(),
                &mut self.last_diagnostics,
                t_start,
                t_end,
            );
        }
        self.refresh_undercut_csr();
        match self.field.dirty_aabb() {
            Some((ix0, iy0, ix1, iy1)) => vec![ix0, iy0, ix1, iy1],
            None => Vec::new(),
        }
    }

    /// Test-only handle on the inner dexel field. Lets the unit tests
    /// inspect cells without going through `data_ptr` (which would force
    /// `unsafe` to deref).
    #[cfg(test)]
    pub(crate) fn field(&self) -> &DexelField {
        &self.field
    }

    /// Test-only: cache deviation target surface(s) without the `JsValue`
    /// round-trip (`set_deviation_target` takes a `JsValue` the unit tests
    /// can't build).
    #[cfg(test)]
    pub(crate) fn set_deviation_target_inner(
        &mut self,
        surfaces: Vec<SurfaceField>,
        surface_z0: f32,
        tol: f32,
    ) {
        self.deviation_target = Some(DeviationTarget {
            surfaces,
            surface_z0,
            tol,
        });
        self.deviation_buf.clear();
    }

    /// Test-only: number of segments with cached diagnostics.
    #[cfg(test)]
    pub(crate) fn warning_cache_len(&self) -> usize {
        self.warning_cache.len()
    }

    /// Test-only view of the persistent deviation-class buffer that
    /// `deviation_ptr()` exposes to JS zero-copy (reading it via the raw
    /// pointer would force `unsafe`).
    #[cfg(test)]
    pub(crate) fn deviation_buf(&self) -> &[u8] {
        &self.deviation_buf
    }
}

/// Decode the JS-side tool spec. Goes through the long-form deserializer
/// path (rather than `from_value`) to keep us flexible if we later need
/// to relax unknown-field handling.
fn from_tool_value(value: JsValue) -> Result<ToolEntry, JsValue> {
    let de = serde_wasm_bindgen::Deserializer::from(value);
    ToolEntry::deserialize(de).map_err(into_js_error)
}

/// Convert a caught sweep panic into the same structured JS error
/// shape the pipeline `generate()` envelope produces, so the frontend's
/// `ErrorToast` renders it instead of the wasm instance trapping.
fn sweep_panic_to_js(panic: &Box<dyn std::any::Any + Send>) -> JsValue {
    structured_error_to_js(
        ivac_core::Error::internal(format!("sim sweep panic: {}", panic_message(panic)))
            .with_hint("Please report this bug — see the toast for details."),
    )
}

#[cfg(test)]
mod tests {
    // Test fixtures spread plunges across a grid via small-index `as f64`
    // arithmetic — the precision loss is irrelevant for these bounded
    // indices.
    #![allow(clippy::cast_precision_loss)]
    use super::*;
    use ivac_core::gcode::preview::{MoveKind, Pose3, ToolpathSegment};
    use ivac_core::project::{Coolant, FormProfileSample, SpindleDirection, ToolKind};

    /// Construct a `Simulator` with a deep stock floor so 3-axis carves never
    /// reach it (staying on the dense fast path). The dexel flip is otherwise
    /// transparent to the legacy tests, which only touch the top surface.
    fn new_sim(min_x: f64, min_y: f64, max_x: f64, max_y: f64, cell: f64, top_z: f32) -> Simulator {
        Simulator::new(min_x, min_y, max_x, max_y, cell, top_z, top_z - 1000.0)
    }

    /// A T-slot form tool: a wide disk (r=8) over the tip band z∈[0,4] and a
    /// narrow neck (r=2) above it (z∈[4,12]). Plunged into stock it leaves a
    /// genuine undercut — a void with a surviving overhang — the single-Z
    /// heightmap couldn't represent. Mirrors the core sweep test's profile.
    fn tslot_tool() -> ToolEntry {
        let mut tool = endmill(16.0);
        tool.kind = ToolKind::FormProfile;
        tool.form_profile_mm = vec![
            FormProfileSample {
                z_mm: 0.0,
                r_mm: 8.0,
            },
            FormProfileSample {
                z_mm: 4.0,
                r_mm: 8.0,
            },
            FormProfileSample {
                z_mm: 4.0,
                r_mm: 2.0,
            },
            FormProfileSample {
                z_mm: 12.0,
                r_mm: 2.0,
            },
        ];
        tool
    }

    fn endmill(diameter: f64) -> ToolEntry {
        ToolEntry {
            id: 1,
            name: "test endmill".into(),
            kind: ToolKind::Endmill,
            diameter,
            tip_diameter: None,
            tip_angle_deg: 60.0,
            dragoff: None,
            drag_knife_self_align_angle_deg: None,
            flutes: 2,
            speed: 18_000,
            plunge_rate: 100,
            feed_rate: 800,
            coolant: Coolant::Off,
            speed_finish: None,
            plunge_rate_finish: None,
            feed_rate_finish: None,
            speed_drill: None,
            plunge_rate_drill: None,
            feed_rate_drill: None,
            default_peck_step_mm: None,
            default_step: None,
            default_xy_overlap: None,
            comment: None,
            z_shift_mm: None,
            laser_pierce_sec: None,
            laser_lead_in_mm: None,
            kerf_mm: None,
            corner_radius_mm: None,
            form_profile_mm: Vec::new(),
            whirl: false,
            whirl_stepover_mm: None,
            whirl_extra_width_mm: None,
            whirl_osc_mm: None,
            pause: 1,
            flute_length_mm: None,
            length_mm: None,
            compression_transition_mm: None,
            thread_pitch_mm: None,
            shank_diameter_mm: None,
            stickout_length_mm: None,
            holder: None,
            // spindle_direction was added to ToolEntry — mirror the
            // core test fixture (sim/heightmap.rs) so WASM tests still
            // compile. Default is Cw, matches pre-spindle behavior.
            spindle_direction: SpindleDirection::default(),
            // Specialty fields — plasma pierce/cut heights +
            // vcarve lead-in. None = inactive, matches a plain endmill.
            pierce_height_mm: None,
            cut_height_mm: None,
            pierce_delay_sec: None,
            wear_offset_mm: 0.0,
            last_calibrated: None,
            vcarve_lead_in_angle_deg: None,
        }
    }

    fn plunge(x: f64, y: f64, top: f64, bottom: f64) -> ToolpathSegment {
        ToolpathSegment {
            from: Pose3 { x, y, z: top },
            to: Pose3 { x, y, z: bottom },
            kind: MoveKind::Plunge,
            gcode_line: 0,
            op_id: 0,
        }
    }

    #[test]
    fn new_initializes_heightmap_to_top_z() {
        let sim = new_sim(0.0, 0.0, 20.0, 20.0, 1.0, 0.0);
        // ceil(width / cell) + 1 grid lines — the +1 fencepost (see
        // Heightmap::from_bbox) so the bbox max-corner stays on-grid.
        // 20 mm / 1 mm = 20 cells → 21 nodes per axis.
        assert_eq!(sim.cols(), 21);
        assert_eq!(sim.rows(), 21);
        assert!((sim.cell_size() - 1.0).abs() < 1e-9);
        assert!((sim.top_z() - 0.0).abs() < 1e-6);
        assert!(sim.field().top().iter().all(|&z| (z - 0.0).abs() < 1e-6));
    }

    #[test]
    fn advance_endmill_plunge_lowers_cells_and_returns_dirty_aabb() {
        let mut sim = new_sim(0.0, 0.0, 40.0, 40.0, 1.0, 0.0);
        let segs = vec![plunge(20.0, 20.0, 0.0, -1.0)];
        let tool = endmill(4.0);
        let aabb = sim.advance_inner(&segs, &tool, 0, 1);
        assert_eq!(aabb.len(), 4, "non-empty dirty AABB expected");
        let (ix0, iy0, ix1, iy1) = (aabb[0], aabb[1], aabb[2], aabb[3]);
        assert!(ix0 < ix1 && iy0 < iy1, "AABB must be non-empty");
        // Cell directly under the plunge sits at the plunge depth.
        let hm = sim.field();
        let center = hm.top()[(20 * hm.cols + 20) as usize];
        assert!(
            (center - -1.0).abs() < 1e-5,
            "plunge center expected -1, got {center}"
        );
        // At least one cell is below top_z.
        assert!(hm.top().iter().any(|&z| z < hm.top_z));
    }

    #[test]
    fn reset_restores_top_z_and_no_dirty() {
        let mut sim = new_sim(0.0, 0.0, 20.0, 20.0, 1.0, 0.0);
        let _ = sim.advance_inner(&[plunge(10.0, 10.0, 0.0, -1.0)], &endmill(4.0), 0, 1);
        sim.reset();
        let hm = sim.field();
        assert!(hm.top().iter().all(|&z| (z - 0.0).abs() < 1e-6));
        assert!(hm.dirty_aabb().is_none());
    }

    #[test]
    fn advance_clears_previous_dirty_so_aabb_reflects_only_this_call() {
        let mut sim = new_sim(0.0, 0.0, 40.0, 40.0, 1.0, 0.0);
        let first = vec![plunge(5.0, 5.0, 0.0, -1.0)];
        let second = vec![plunge(30.0, 30.0, 0.0, -1.0)];
        let tool = endmill(2.0);
        let _ = sim.advance_inner(&first, &tool, 0, 1);
        let aabb = sim.advance_inner(&second, &tool, 0, 1);
        // Should report only the second plunge's region, not the union.
        assert!(
            aabb[0] >= 28 && aabb[2] <= 32,
            "second-plunge AABB drifted: {aabb:?}"
        );
    }

    #[test]
    fn advance_with_no_cuts_returns_empty_aabb() {
        let mut sim = new_sim(0.0, 0.0, 20.0, 20.0, 1.0, 0.0);
        let rapid = vec![ToolpathSegment {
            from: Pose3 {
                x: 0.0,
                y: 0.0,
                z: 5.0,
            },
            to: Pose3 {
                x: 10.0,
                y: 10.0,
                z: 5.0,
            },
            kind: MoveKind::Rapid,
            gcode_line: 0,
            op_id: 0,
        }];
        let aabb = sim.advance_inner(&rapid, &endmill(2.0), 0, 1);
        assert!(
            aabb.is_empty(),
            "rapid-only advance should report no dirty cells"
        );
    }

    #[test]
    fn data_ptr_and_len_consistent_with_cols_rows() {
        let sim = new_sim(0.0, 0.0, 10.0, 10.0, 0.5, 0.0);
        let len = (sim.cols() as usize) * (sim.rows() as usize);
        assert_eq!(len, sim.field().top_len());
        assert!(!sim.data_ptr().is_null());
    }

    /// `partial_advance(idx, 0, 0.5)` on a Plunge segment should carve
    /// the column down to the midpoint Z, not the full final depth.
    /// Calling `partial_advance(idx, 0.5, 1.0)` afterwards lowers the
    /// same column to the final depth.
    #[test]
    fn partial_advance_plunge_grows_as_t_advances() {
        let mut sim = new_sim(0.0, 0.0, 40.0, 40.0, 1.0, 0.0);
        let segs = vec![plunge(20.0, 20.0, 0.0, -2.0)];
        let tool = endmill(4.0);
        let aabb_half = sim.partial_advance_inner(&segs, &tool, 0, 0.0, 0.5);
        assert_eq!(aabb_half.len(), 4, "expected non-empty AABB at half-plunge");
        let center_after_half = sim.field().top()[(20 * sim.field().cols + 20) as usize];
        assert!(
            (center_after_half - -1.0).abs() < 1e-5,
            "plunge halfway should reach z=-1, got {center_after_half}"
        );
        let _ = sim.partial_advance_inner(&segs, &tool, 0, 0.5, 1.0);
        let center_after_full = sim.field().top()[(20 * sim.field().cols + 20) as usize];
        assert!(
            (center_after_full - -2.0).abs() < 1e-5,
            "plunge fully should reach z=-2, got {center_after_full}"
        );
    }

    /// A straight cut from x=5 to x=25 carved up to t=0.5 should only
    /// touch cells in the left half of the segment. The right half stays
    /// at `top_z`.
    #[test]
    fn partial_advance_cut_only_touches_swept_chunk() {
        let mut sim = new_sim(0.0, 0.0, 40.0, 40.0, 1.0, 0.0);
        let cut = ToolpathSegment {
            from: Pose3 {
                x: 5.0,
                y: 20.0,
                z: -1.0,
            },
            to: Pose3 {
                x: 25.0,
                y: 20.0,
                z: -1.0,
            },
            kind: MoveKind::Cut,
            gcode_line: 0,
            op_id: 0,
        };
        let segs = vec![cut];
        let tool = endmill(2.0);
        let _ = sim.partial_advance_inner(&segs, &tool, 0, 0.0, 0.5);
        let hm = sim.field();
        // Cell at x≈10 (within carved chunk [5..15]) is below top_z.
        let near = hm.top()[(20 * hm.cols + 10) as usize];
        assert!(near < hm.top_z, "cell in carved half should be lowered");
        // Cell at x≈22 (in uncarved chunk [15..25]) is still at top_z.
        let far = hm.top()[(20 * hm.cols + 22) as usize];
        assert!(
            (far - hm.top_z).abs() < 1e-6,
            "cell in un-carved half should still be at top_z, got {far}"
        );
        // After t goes 0.5→1, the right half also drops.
        let _ = sim.partial_advance_inner(&segs, &tool, 0, 0.5, 1.0);
        let far_after = sim.field().top()[(20 * sim.field().cols + 22) as usize];
        assert!(
            far_after < sim.field().top_z,
            "right half should be carved after full partial sweep, got {far_after}"
        );
    }

    /// Many distinct plunges so carving order is observable in the
    /// heightfield — used by the checkpoint equivalence tests below.
    fn staircase(n: usize) -> Vec<ToolpathSegment> {
        (0..n)
            .map(|i| {
                // Spread plunges across the grid; deepen with index so two
                // prefixes of different length produce different fields.
                let x = 2.0 + (i % 18) as f64 * 2.0;
                let y = 2.0 + (i / 18) as f64 * 2.0;
                plunge(x, y, 0.0, -(1.0 + i as f64 * 0.05))
            })
            .collect()
    }

    /// Restoring a checkpoint then replaying the tail must yield a
    /// heightfield byte-identical to a full replay from segment 0 — the
    /// whole point of H2 (cheap backward scrub without re-simulating the
    /// prefix).
    #[test]
    fn checkpoint_restore_then_replay_matches_full_replay() {
        let segs = staircase(40);
        let tool = endmill(3.0);
        let k = 17u32; // checkpoint boundary
        let n = segs.len() as u32;

        // Reference: carve the whole program in one go.
        let mut full = new_sim(0.0, 0.0, 40.0, 40.0, 1.0, 0.0);
        let _ = full.advance_inner(&segs, &tool, 0, n);

        // Carve [0, k), snapshot, carve [k, n): a normal forward play
        // that drops a checkpoint partway. Sanity: same field as `full`.
        let mut sim = new_sim(0.0, 0.0, 40.0, 40.0, 1.0, 0.0);
        let _ = sim.advance_inner(&segs, &tool, 0, k);
        sim.checkpoint(k);
        let _ = sim.advance_inner(&segs, &tool, k, n);
        assert_eq!(
            sim.field().top(),
            full.field().top(),
            "forward carve with a checkpoint must match a plain full carve"
        );

        // The checkpoint must hold exactly the [0, k) state.
        let mut prefix = new_sim(0.0, 0.0, 40.0, 40.0, 1.0, 0.0);
        let _ = prefix.advance_inner(&segs, &tool, 0, k);
        assert!(sim.restore_checkpoint(k), "checkpoint k must exist");
        assert_eq!(
            sim.field().top(),
            prefix.field().top(),
            "restore must reproduce the [0, k) heightfield exactly"
        );

        // Replay the tail from the restored base → back to the full field.
        let _ = sim.advance_inner(&segs, &tool, k, n);
        assert_eq!(
            sim.field().top(),
            full.field().top(),
            "restore + tail replay must equal a full replay"
        );
    }

    #[test]
    fn restore_checkpoint_marks_whole_grid_dirty() {
        let segs = staircase(10);
        let tool = endmill(3.0);
        let mut sim = new_sim(0.0, 0.0, 40.0, 40.0, 1.0, 0.0);
        let _ = sim.advance_inner(&segs, &tool, 0, 5);
        sim.checkpoint(5);
        let _ = sim.advance_inner(&segs, &tool, 5, 10);
        sim.field.clear_dirty();
        assert!(sim.restore_checkpoint(5));
        let aabb = sim.field().dirty_aabb().expect("full grid dirty");
        assert_eq!(aabb, (0, 0, sim.cols(), sim.rows()));
    }

    #[test]
    fn nearest_checkpoint_picks_largest_at_or_below_target() {
        let mut sim = new_sim(0.0, 0.0, 10.0, 10.0, 1.0, 0.0);
        assert_eq!(sim.nearest_checkpoint(100), -1, "no checkpoints yet");
        sim.checkpoint(10);
        sim.checkpoint(20);
        sim.checkpoint(30);
        assert_eq!(sim.checkpoint_count(), 3);
        assert_eq!(sim.nearest_checkpoint(25), 20);
        assert_eq!(sim.nearest_checkpoint(30), 30);
        assert_eq!(sim.nearest_checkpoint(31), 30);
        assert_eq!(sim.nearest_checkpoint(9), -1, "before the first checkpoint");
        assert!(!sim.restore_checkpoint(25), "no exact checkpoint at 25");
        sim.clear_checkpoints();
        assert_eq!(sim.checkpoint_count(), 0);
        assert_eq!(sim.nearest_checkpoint(100), -1);
    }

    /// Re-snapshotting the same boundary overwrites rather than
    /// duplicating, and snapshots stay sorted.
    #[test]
    fn checkpoint_same_index_overwrites() {
        let mut sim = new_sim(0.0, 0.0, 40.0, 40.0, 1.0, 0.0);
        let tool = endmill(3.0);
        let segs = staircase(20);
        let _ = sim.advance_inner(&segs, &tool, 0, 10);
        sim.checkpoint(10);
        // Carve more, then re-checkpoint the SAME index with the deeper
        // field (a degenerate but defensible call). Capture the deeper state
        // so we can prove the restore reflects the overwritten snapshot.
        let _ = sim.advance_inner(&segs, &tool, 10, 20);
        sim.checkpoint(10);
        let deeper = sim.field().top().to_vec();
        assert_eq!(sim.checkpoint_count(), 1, "no duplicate index");
        // Rewind so restore has to actually re-lay the snapshot.
        sim.reset();
        assert!(sim.restore_checkpoint(10));
        assert_eq!(
            sim.field().top(),
            deeper.as_slice(),
            "restore reflects the overwritten (deeper) snapshot"
        );
    }

    /// Re-sweeping the same range must replay cached per-segment
    /// diagnostics byte-identically to the first (computed) sweep — the
    /// M1 win: scrub-back skips the holder/fixture/rapid recompute but the
    /// warnings the user sees are unchanged.
    #[test]
    fn re_sweep_replays_cached_diagnostics_identically() {
        let mut sim = new_sim(0.0, 0.0, 40.0, 40.0, 1.0, 0.0);
        // seg 0 plunges a hole; seg 1 is a rapid dragging through stock at
        // z=-2 → a rapid_through_material collision warning.
        let segs = vec![
            plunge(10.0, 10.0, 0.0, -5.0),
            ToolpathSegment {
                from: Pose3 {
                    x: 0.0,
                    y: 20.0,
                    z: -2.0,
                },
                to: Pose3 {
                    x: 40.0,
                    y: 20.0,
                    z: -2.0,
                },
                kind: MoveKind::Rapid,
                gcode_line: 0,
                op_id: 0,
            },
        ];
        let tool = endmill(4.0);

        let _ = sim.advance_inner(&segs, &tool, 0, 2);
        let first: Vec<_> = sim.last_diagnostics.warnings.clone();
        assert!(
            first.iter().any(|w| matches!(
                w,
                ivac_core::sim::diagnostics::SimWarning::RapidThroughMaterial { .. }
            )),
            "first sweep should flag the rapid-through-material"
        );
        assert_eq!(sim.warning_cache_len(), 2, "both segments cached");

        // Rewind the heightfield (cache retained) and re-sweep — the
        // warnings must be replayed identically from the cache.
        sim.reset();
        let _ = sim.advance_inner(&segs, &tool, 0, 2);
        let second: Vec<_> = sim.last_diagnostics.warnings.clone();
        assert_eq!(
            format!("{first:?}"),
            format!("{second:?}"),
            "re-sweep diagnostics must match the first sweep exactly"
        );
    }

    /// Clearing the toolpath must drop the diagnostics cache so stale
    /// warnings can't be replayed against a changed program. (`set_toolpath`
    /// / `set_fixtures` route through the same `warning_cache.clear()`; they
    /// take a `JsValue` so they're exercised in the browser, not here.)
    #[test]
    fn clear_toolpath_drops_warning_cache() {
        let mut sim = new_sim(0.0, 0.0, 40.0, 40.0, 1.0, 0.0);
        let segs = vec![plunge(10.0, 10.0, 0.0, -1.0)];
        let _ = sim.advance_inner(&segs, &endmill(4.0), 0, 1);
        assert_eq!(sim.warning_cache_len(), 1);
        sim.clear_toolpath();
        assert_eq!(sim.warning_cache_len(), 0, "clear_toolpath drops the cache");
    }

    /// Partial slices with `t_start > 0` MUST NOT emit fixture / holder /
    /// rapid diagnostics — a 60 fps driver would otherwise spam the same
    /// warning each frame. The first slice (`t_start ≈ 0`) emits once.
    #[test]
    fn partial_advance_emits_rapid_warning_only_on_first_slice() {
        let mut sim = new_sim(0.0, 0.0, 40.0, 40.0, 1.0, 0.0);
        // Rapid through material: starts below top_z, so check_rapid_against_stock
        // reports a collision.
        let rapid = ToolpathSegment {
            from: Pose3 {
                x: 0.0,
                y: 20.0,
                z: -5.0,
            },
            to: Pose3 {
                x: 40.0,
                y: 20.0,
                z: -5.0,
            },
            kind: MoveKind::Rapid,
            gcode_line: 0,
            op_id: 0,
        };
        let segs = vec![rapid];
        let tool = endmill(2.0);
        let _ = sim.partial_advance_inner(&segs, &tool, 0, 0.0, 0.3);
        assert_eq!(
            sim.last_diagnostics.count("rapid_through_material"),
            1,
            "first partial slice should emit one warning"
        );
        let _ = sim.partial_advance_inner(&segs, &tool, 0, 0.3, 0.6);
        assert_eq!(
            sim.last_diagnostics.count("rapid_through_material"),
            0,
            "mid-segment partial slice must not re-emit the warning"
        );
    }

    /// Deviation overlay: with a target cached, `deviation()` reclassifies the
    /// carved field — over-cut cells become gouges, uncut cells rest stock.
    /// Exercises the cached-target path the `set_deviation_target` /
    /// `deviation` JS bindings wrap (the `JsValue` setter can't run in a plain
    /// unit test), against a real carved `Simulator` field.
    #[test]
    fn deviation_flags_gouge_under_plunge_and_reststock_around_it() {
        use ivac_core::cam::surface::{Deviation, SurfaceField};
        use ivac_core::geometry::Point2;

        let mut sim = new_sim(0.0, 0.0, 4.0, 4.0, 1.0, 0.0);
        assert!(sim.deviation().is_empty(), "no target cached yet → empty");

        let (cols, rows) = (sim.cols(), sim.rows());
        // Target wants a uniform 1mm cut everywhere over the same footprint.
        let target = SurfaceField::new(
            Point2::new(0.0, 0.0),
            1.0,
            cols,
            rows,
            vec![-1.0; (cols * rows) as usize],
        );
        sim.set_deviation_target_inner(vec![target], sim.top_z(), 0.25);
        assert!(sim.has_deviation_target());

        // 4mm endmill plunged 2mm deep at the grid center.
        let segs = vec![plunge(2.0, 2.0, 0.0, -2.0)];
        let _ = sim.advance_inner(&segs, &endmill(4.0), 0, 1);

        let dev = sim.deviation();
        assert_eq!(dev.len(), (cols * rows) as usize);
        // Cell under the plunge is carved to -2 (1mm past the -1 target) → gouge.
        assert_eq!(
            dev[(2 * cols + 2) as usize],
            Deviation::Gouge as u8,
            "over-cut center must be a gouge"
        );
        // A far corner is uncut (0), 1mm above the -1 target → rest stock.
        assert_eq!(
            dev[0],
            Deviation::RestStock as u8,
            "uncut corner must read as rest stock"
        );

        // Clearing the target returns to the empty (overlay-off) result.
        sim.clear_deviation_target();
        assert!(!sim.has_deviation_target());
        assert!(sim.deviation().is_empty());
    }

    /// The zero-copy per-frame path: `deviation_recompute_in` patches only the
    /// carve's dirty AABB into the persistent buffer, and the result matches a
    /// full `deviation_recompute` / `deviation()` — the whole point of item 4
    /// (reclassify the touched cells, not the whole grid, every frame).
    #[test]
    fn deviation_recompute_in_matches_full_over_the_dirty_aabb() {
        use ivac_core::cam::surface::SurfaceField;
        use ivac_core::geometry::Point2;

        let mut sim = new_sim(0.0, 0.0, 4.0, 4.0, 1.0, 0.0);
        // Buffer empty until a target + recompute.
        assert_eq!(sim.deviation_len(), 0);
        sim.deviation_recompute();
        assert_eq!(sim.deviation_len(), 0, "no target → buffer stays empty");

        let (cols, rows) = (sim.cols(), sim.rows());
        let n = (cols * rows) as usize;
        let target = SurfaceField::new(Point2::new(0.0, 0.0), 1.0, cols, rows, vec![-1.0; n]);
        sim.set_deviation_target_inner(vec![target], sim.top_z(), 0.25);
        // set_deviation_target invalidates the buffer until the next recompute.
        assert_eq!(sim.deviation_len(), 0);

        // Full recompute over the un-carved stock: every cell is 1mm of rest
        // stock above the -1 target.
        sim.deviation_recompute();
        assert_eq!(sim.deviation_len(), (cols * rows));
        assert_eq!(sim.deviation_buf(), sim.deviation().as_slice());

        // Carve a plunge, then reclassify ONLY the reported dirty AABB.
        let segs = vec![plunge(2.0, 2.0, 0.0, -2.0)];
        let aabb = sim.advance_inner(&segs, &endmill(4.0), 0, 1);
        assert_eq!(aabb.len(), 4, "carve reported a dirty AABB");
        sim.deviation_recompute_in(aabb[0], aabb[1], aabb[2], aabb[3]);

        // The partial buffer equals a from-scratch full classification: the
        // touched cells flipped to gouge, the untouched ones kept rest stock.
        assert_eq!(
            sim.deviation_buf(),
            sim.deviation().as_slice(),
            "partial-AABB recompute must equal the full classification"
        );

        // A partial call after a size change falls back to a full recompute
        // (defensive): shrink the buffer to force the mismatch branch.
        sim.deviation_recompute_in(0, 0, 1, 1);
        assert_eq!(sim.deviation_len(), (cols * rows));
    }

    /// Multi-relief overlay: two cached targets combine deepest-cut-wins, so a
    /// carve that overshoots the shallow relief but not the deep one reads as
    /// rest stock, not a gouge — the whole point of item 3.
    #[test]
    fn deviation_unions_multiple_targets_deepest_wins() {
        use ivac_core::cam::surface::{Deviation, SurfaceField};
        use ivac_core::geometry::Point2;

        let mut sim = new_sim(0.0, 0.0, 4.0, 4.0, 1.0, 0.0);
        let (cols, rows) = (sim.cols(), sim.rows());
        let n = (cols * rows) as usize;
        // Shallow relief wants -1 everywhere; deep relief wants -5 everywhere.
        let shallow = SurfaceField::new(Point2::new(0.0, 0.0), 1.0, cols, rows, vec![-1.0; n]);
        let deep = SurfaceField::new(Point2::new(0.0, 0.0), 1.0, cols, rows, vec![-5.0; n]);
        sim.set_deviation_target_inner(vec![shallow, deep], sim.top_z(), 0.25);

        // Plunge 3mm deep at the center: past the shallow (-1) target but 2mm
        // shy of the deep (-5) union target.
        let segs = vec![plunge(2.0, 2.0, 0.0, -3.0)];
        let _ = sim.advance_inner(&segs, &endmill(4.0), 0, 1);

        let dev = sim.deviation();
        assert_eq!(
            dev[(2 * cols + 2) as usize],
            Deviation::RestStock as u8,
            "3mm cut is rest stock against the -5 union target, not a gouge"
        );
        // Against the shallow relief ALONE the same cell would be a gouge —
        // proving the union (not per-surface class merge) drives the verdict.
        sim.set_deviation_target_inner(
            vec![SurfaceField::new(
                Point2::new(0.0, 0.0),
                1.0,
                cols,
                rows,
                vec![-1.0; n],
            )],
            sim.top_z(),
            0.25,
        );
        assert_eq!(
            sim.deviation()[(2 * cols + 2) as usize],
            Deviation::Gouge as u8,
            "shallow-only target reads the same cut as a gouge"
        );
    }

    /// The headline flip deliverable: plunging a T-slot (form) tool through an
    /// `advance()` grows a genuine undercut, so the simulator reports undercut
    /// columns and the CSR buffers are populated with a consistent shape.
    #[test]
    fn form_tool_advance_populates_undercut_sidecar() {
        let mut sim = new_sim(0.0, 0.0, 40.0, 40.0, 1.0, 0.0);
        let segs = vec![plunge(20.0, 20.0, 0.0, -5.0)];
        let _ = sim.advance_inner(&segs, &tslot_tool(), 0, 1);

        let uc = sim.undercut_column_count();
        assert!(
            uc > 0,
            "a form-tool advance must populate the undercut sidecar"
        );
        assert_eq!(
            u32::try_from(sim.field().undercut_columns()).unwrap(),
            uc,
            "the CSR column count must match the field's sidecar"
        );
        // CSR shape: one cell index per column, a row-pointer of length uc+1,
        // and 2 f32s (lo, hi) per span.
        assert_eq!(sim.undercut_col_index_len(), uc);
        assert_eq!(sim.undercut_span_offsets_len(), uc + 1);
        let total_spans = *sim.undercut_span_offsets.last().unwrap();
        assert_eq!(
            sim.undercut_spans_len(),
            2 * total_spans,
            "span buffer holds (lo, hi) pairs"
        );
        assert!(total_spans >= uc, "each undercut column carries ≥1 span");
        // Every span is well-formed (lo < hi) and the pointers are live.
        assert!(!sim.undercut_col_index_ptr().is_null());
        assert!(!sim.undercut_spans_ptr().is_null());
        for pair in sim.undercut_spans.chunks_exact(2) {
            assert!(pair[0] < pair[1], "span lo must be below hi");
        }
    }

    /// A pure 3-axis (endmill) advance must NOT touch the sidecar — the dense
    /// fast path is unchanged, so the CSR stays in its empty canonical form.
    #[test]
    fn three_axis_advance_keeps_sidecar_empty() {
        let mut sim = new_sim(0.0, 0.0, 40.0, 40.0, 1.0, 0.0);
        let segs = vec![plunge(20.0, 20.0, 0.0, -2.0)];
        let _ = sim.advance_inner(&segs, &endmill(4.0), 0, 1);
        assert_eq!(sim.undercut_column_count(), 0);
        assert_eq!(sim.field().undercut_columns(), 0);
        assert_eq!(
            sim.undercut_span_offsets_len(),
            1,
            "empty CSR keeps its leading 0 row-pointer"
        );
        assert_eq!(sim.undercut_spans_len(), 0);
        assert_eq!(sim.undercut_col_index_len(), 0);
    }

    /// Checkpoint/restore must round-trip the undercut sidecar, not just the
    /// dense top — the acceptance criterion for form-tool scrubbing. Restore
    /// after a full reset reproduces the exact CSR the checkpoint captured.
    #[test]
    fn checkpoint_restore_round_trips_undercut_sidecar() {
        let mut sim = new_sim(0.0, 0.0, 40.0, 40.0, 1.0, 0.0);
        let segs = vec![plunge(20.0, 20.0, 0.0, -5.0)];
        let _ = sim.advance_inner(&segs, &tslot_tool(), 0, 1);
        let uc = sim.field().undercut_columns();
        assert!(uc > 0, "precondition: the form plunge grew undercuts");

        // Capture the exact CSR the checkpoint should reproduce.
        let csr_before = (
            sim.undercut_col_index.clone(),
            sim.undercut_span_offsets.clone(),
            sim.undercut_spans.clone(),
        );
        sim.checkpoint(1);

        // Wipe the field (sidecar included) so restore has real work to do.
        sim.reset();
        assert_eq!(sim.field().undercut_columns(), 0);
        assert_eq!(sim.undercut_column_count(), 0);

        assert!(sim.restore_checkpoint(1), "checkpoint at seg 1 must exist");
        assert_eq!(
            sim.field().undercut_columns(),
            uc,
            "restore must bring the sidecar voids back"
        );
        let csr_after = (
            sim.undercut_col_index.clone(),
            sim.undercut_span_offsets.clone(),
            sim.undercut_spans.clone(),
        );
        assert_eq!(
            csr_before, csr_after,
            "checkpoint/restore must round-trip the undercut sidecar CSR exactly"
        );
        // The whole grid is marked dirty for a full re-upload after restore.
        assert_eq!(
            sim.field().dirty_aabb(),
            Some((0, 0, sim.cols(), sim.rows()))
        );
    }
}
