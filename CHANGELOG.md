# Changelog

All notable changes to ivaCAM are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
(plain `MAJOR.MINOR.PATCH`, git tags `vX.Y.Z`).

## [0.5.0] - 2026-08-18

The post-processor release: an Autodesk-(Fusion 360)-`.cps`-compatible
post-processor engine, so existing vendor posts run unmodified instead of
requiring a Rust dialect per machine. ~16 commits since v0.4.0.

### Added

- **Autodesk-`.cps`-compatible post-processor engine** (new `ivac-cps` crate
  on the pure-Rust boa JS runtime, feature-gated). A 16-module prelude
  implements the `.cps` runtime API — format/variable/modal factories, entry
  points from `onOpen` through `onClose`, canned cycles with
  `expandCyclePoint` re-entrancy, a 24-convention Euler engine with real
  `MachineConfiguration` rotary kinematics (AC/BC-table and AB-head verified),
  execution budgets, and a structured `PostError` taxonomy. Acceptance proof:
  the real Autodesk FANUC post runs unmodified and produces byte-stable,
  hand-verified NC; a V8↔boa differential (158 format values + 19 scenarios)
  is byte-identical; 100% of the 445 declared API symbols are present.
- **Post sources on every desktop transport.** A bundled post library
  (`grbl`) plus user-supplied `.cps` files: server `/posts` +
  `/posts/inspect` routes, Tauri commands, and CLI `--post cps` /
  `ivac posts`.
- **Frontend post picker with an auto-generated properties form** driven by
  the post's declared `properties`, persisting only user overrides.
- **IR-exact preview, sim, and timing for CPS posts.** The simulator carves
  the recorded intermediate representation, so preview geometry is exact no
  matter what dialect text the post renders; G-code line sync is recovered
  against the posted text where possible.

### Fixed

- `PostProfile` file extension and line ending were editable in the UI but
  ignored on export; both are now honored.
- Dependency advisory RUSTSEC-2026-0253: `lru` bumped 0.16.4 → 0.18.2
  (unsound `LruCache::pop()`; not reachable with ivaCAM's key types, but the
  release gate enforces a clean advisory scan).

### Known limitations

- CPS posts don't stream — `--stream` rejects them (same precedent as HPGL).
- CPS is compiled out of the wasm/browser build for size (≈1.5 MiB gzipped);
  the UI hides the picker there.
- The simulation of a CPS program reflects the recorded IR, not a re-parse of
  the final text — a post that transforms or drops moves in its own emission
  logic is not cross-checked at runtime (only bundled posts carry a geometry
  parity test).

## [0.4.0] - 2026-07-21

The 3D-machining release: two-sided (flip-stock) jobs, waterline roughing
from an STL, live deviation feedback, and a streaming G-code path that lifts
the old per-op size caps. ~185 commits since v0.3.0.

### Added

- **Two-sided (flip-stock) machining.** Model a front and a back side on one
  stock, flip about a chosen axis, and emit **two coordinated G-code programs**
  in one run. Includes a conflict guard that warns on overlapping front/back
  carves and refuses a front pass that would sever the workpiece, a flip
  transform carried through the schema and every transport, per-side G-code
  save, Front/Back tabs in the G-code panel, side grouping + filter chips in
  the operations list, a stock-flip fieldset with a per-op Side dropdown, and
  an unmistakable flip visual with a reflected back-side 3D preview.
- **Waterline roughing (`WaterlineRough`).** A constant-Z 3D roughing operation
  driven by an STL: a mesh slicer produces per-level contours, a per-level
  area-clearing pass nests loops and fills pockets, and a multi-level assembler
  stitches the levels into one toolpath.
- **Deviation overlay.** A live sim overlay that classifies the carved surface
  against a target relief per cell — gouge (red) / rest-stock (green) /
  on-target (neutral) — painted straight into the terrain mesh. Supports
  multiple relief targets (deepest-cut-wins) and recomputes only the dirty AABB
  each frame.
- **STL relief sources.** Rasterize an STL to a relief height grid as a
  selectable source in Relief Mill, with a real-Z heightgrid mode alongside the
  brightness mode. Rasterization routes through the active transport.
- **Streaming G-code path.** Emit G-code incrementally through an append-only
  sink and a `GcodeSink` streaming mode, with the op-cache preserved over the
  stream, a chunked-HTTP body on the server, a CLI transport, and a tee'd
  incremental interpreter for streaming preview. A per-op tee cap bounds a
  single oversized operation.
- **Streaming raster engrave.** Laser raster engraving now streams row-by-row
  (AlongX) and column-by-column (AlongY), lifting the previous pixel cap and
  keeping AlongY curves position-independent.
- **Watertight-solid STL export** end-to-end, including undercut voids meshed
  from the dexel sidecar (walls and floors), via a new voxel-solid export path.
- **German localization reaches the headless CLI** with an embedded i18n
  catalog (`de.json`) and localized pipeline warnings (English + German) driven
  by a structured-params seam on `PipelineWarning`.

### Changed

- **Arc-native simulation.** The live sim carves G2/G3 chords as analytic
  sub-arcs, envelope scans sample arc-chord bulge extrema, arc-tagged toolpath
  chords tessellate on read, and preview arc tessellation coarsens from 2° to
  15° for a lighter mesh.
- **Dexel-based sweep core.** The live sim runs on a hybrid dense-top +
  sparse-undercut `DexelField` with multi-span Z interval algebra and a generic
  `CarveTarget`, enabling undercut-aware sweeps and a finite `FormProfile`
  ceiling.
- Whole-program toolpath interpretation is memoized on a full cache hit.
- The HPGL post and streaming output now share a common `GcodeSink`
  write-through seam.
- Large frontend components (EntityCanvas2D, ToolLibraryDialog, App, the
  `ProjectState` slices) were decomposed into pure, testable modules, guarded
  by a component-size tripwire test.

### Fixed

- Add-Text now honors the selected text style (sets the op kind from the style
  instead of always engraving).
- Long German UI labels no longer overflow the tool table or op-properties
  panel.
- Cleared pedantic-clippy findings from the streaming/raster work so the
  release gate is green.

### Performance

- Deviation overlay recomputes only the dirty AABB each carve frame instead of
  the whole target grid.
- Allocation-free undercut void emitter (~6–7× faster).

## [0.3.0] - 2026-06-22

Localization release: full German (`de.json`) UI translation with enforceable
i18n coverage gates and a locale-invariance test, plus single-source
versioning. ~21 commits since v0.2.0.

## [0.2.0] - 2026-06-20

Touch and Android UX: system-back handling, tap-cycle through stacked objects
on the 2D canvas, swipeable screen panels, phone-oriented Stock/Layers/Text
panels, and clipboard/one-tap WCS-origin fixes for warnings. ~82 commits since
v0.0.1.

## [0.0.1] - 2026-06-18

First tagged release. (Android's manifest merger requires a versionName ≥
0.0.1, so the initial tag is v0.0.1 rather than v0.1.0.)

[0.5.0]: https://github.com/aalarchiv/ivacam/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/aalarchiv/ivacam/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/aalarchiv/ivacam/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/aalarchiv/ivacam/compare/v0.0.1...v0.2.0
[0.0.1]: https://github.com/aalarchiv/ivacam/releases/tag/v0.0.1
