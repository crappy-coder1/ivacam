# Material Model — Decision Record

**Status:** **ADOPTED & IMPLEMENTED** — Option A (multi-span Z-dexel, hybrid
dense-top + sparse-undercut sidecar). Phase 1 shipped; Phase 2 (two-sided) shipped
on top; Phase 3 (tri-dexel) deferred.
**Date:** 2026-07-21 (decision authored 2026-07-10; recorded here after Phase 1/2 landed)
**Tracker:** `ivac-58nl.6` under epic `ivac-58nl` (GrblGru comparison roadmap)
**Related:** two-sided machining `ivac-rt1.11` (unblocked by this, now **closed**);
kinematic sim `ivac-58nl.5` / [`kinematic-sim.md`](./kinematic-sim.md) (gates Phase 3);
undercut ops `OpKind::TSlot` / `OpKind::Dovetail`.
**Tracker IDs are ivaCAM beads issues — run `bd show <id>`.**

---

## TL;DR

The destructive-sim material model was a single `Z(x, y)` heightmap — one `f32`
per column, monotone-min. That is optimal for pure 3-axis top-down machining
(most of ivaCAM) but **structurally cannot** represent material *above a void*,
so it could not honestly simulate T-slot / dovetail undercut necks, ball-nose
steep-wall side cuts, two-sided flip parts, or any future 4/5-axis removal.

The decision: **do not introduce a voxel grid.** Adopt a **multi-span Z-dexel** —
each column is a sorted, disjoint list of solid `[lo, hi]` spans instead of a
single top surface — implemented as a **hybrid** of the *unchanged dense top
array* plus a *sparse sidecar* carrying full span lists only for the <1 % of
columns that actually grew an interior void. The current heightmap is a
degenerate **1-span dexel**, so this is a strict, back-compatible superset: the
3-axis hot path stays byte-for-byte identical and pays *zero* sidecar cost.

This shipped (Phase 1) and directly unblocked two-sided machining (Phase 2,
`ivac-rt1.11`) without voxels. Full tri-dexel (Phase 3) is deferred until — and
only if — multi-axis kinematics (`P5`) makes its cost pay.

## Why this came up

The single heightmap (`sim/heightmap.rs`, one `f32` per column, lowered
monotonically from `top_z`) is the **root limitation** of the destructive
terrain. Four concrete capabilities it *structurally* cannot deliver forced the
question — each already visible in the codebase or tracker:

| Capability | Why the single heightmap fails | Evidence |
|---|---|---|
| T-slot / dovetail **undercut** neck | one Z per column; the overhanging lip has material *above* a void | heightmap once downgraded a T-slot to "carve the widest disk" |
| **Two-sided / flip** parts | field is only ever *lowered from `top_z`* → single-sided; a from-below carve has nowhere to live | `ivac-rt1.11` scope; `rt1.11.1` gate |
| Ball-nose **steep-wall side cut** honesty | wall material removed by the flank isn't a top-down projection | implicit in the 2.5D limit |
| Future **4/5-axis / lathe** removal | tool axis not vertical → removed volume isn't a `Z(x, y)` graph | `P5`; GrblGru kinematic lead |

The reframing that made it tractable: **the heightmap is a degenerate 1-span
Z-dexel.** Every migration step below is a superset of the old model, so the
3-axis path never regresses.

## The representation spectrum

Memory is the deciding axis. For a *surface-oriented* process, **dexel memory
scales with surface area (~N²); dense-voxel memory scales with volume (~N³)** —
which is why every production CNC verifier (ModuleWorks CutSim, Vericut's core)
uses multi/tri-dexel, not dense voxels. Worked numbers for a representative
ivaCAM job — 200×200 mm stock, 0.2 mm cell (auto = `tool_dia/15`), i.e.
1000×1000 columns:

| Model | Structure | Carve op | Mem @ example | Unlocks | WASM fit |
|---|---|---|---|---|---|
| **Old: 1-span Z-map** | `Vec<f32>` `cols*rows` | `min()` per cell | **4 MB** | 3-axis top-down | ✅ zero-copy flat array |
| **A. Multi-span Z-dexel** ✅ chosen | per column: sorted `[lo,hi]` spans | 1-D interval subtract | ~6–12 MB (only undercut cols pay) | + undercuts, + **flip**, honest T-slot | ✅ hybrid dense + sidecar |
| **B. Tri-dexel** | 3 orthogonal dexel grids | ray vs swept-vol, ×3 | ~40–60 MB | + arbitrary orientation, **5-axis** | ⚠ heavier; remesh dirty bricks |
| **C. Dense voxel** | `N³` occupancy/SDF | per-voxel CSG | **≥125 MB–1 GB** | (same as B) | ❌ infeasible at fine cell |
| **C′. Sparse voxel (VDB/octree)** | narrow-band bricks | brick CSG | ~surface-proportional | (same as B) | ⚠ pointer-chasing, alloc churn — WASM-hostile |

Dense voxel (C) is out on arithmetic alone: 1000³ = 1e9 voxels; even 1 bit each is
125 MB and you must mesh the whole volume. Sparse voxel (C′) recovers the memory
but trades it for allocation/pointer patterns that fight WASM's single-thread,
single-linear-heap model and buys nothing over tri-dexel for a surface process.
**The real choice was A vs B, and the answer is "A now, B only if 4/5-axis
lands."**

## The decision in detail — Option A

### Why it's the right first (and, for now, only) step

- **Strict superset / back-compatible.** One span == the old model. The per-cell
  `min()` becomes "subtract an interval from the top span," which for a top-down
  cut *is* the previous behavior — provably identical (see the byte-identity note
  under `CarveTarget` below).
- **Unblocks flip machining cheaply.** A two-sided part needs *exactly two spans*
  per column (top carve descends from `top_z`; back carve ascends from
  `top_z − thickness`). One field → one watertight solid, which dissolves the
  `rt1.11.1` "two floating sheets vs one solid" risk *without* voxels.
- **Preserves every current win** (see the constraints table).

### The tool generalization that makes carving fall out

The tool profile gained a **removed-interval-per-radius** query alongside its
existing lower-surface query (`ToolProfile::eval_interval`, `sim/heightmap.rs`):

```
eval_interval(r) -> Option<(lo_dz, hi_dz)>   // material removed between tip+lo and tip+hi
```

- Endmill / ball / V / bull / drag / laser / engraver: `lo = eval(r)`,
  `hi = +∞` (no modeled ceiling) → subtracting `[lo, +∞]` from the top span is
  the old `min()`, unchanged.
- **T-slot / dovetail** (`FormProfile`): gains a finite upper surface
  (`form_upper`), so a disk removes only its `(disk_bottom(r), disk_top(r))` band
  at `r ∈ [neck_r, disk_r]` and **nothing** above the neck at `r < neck_r` → the
  overhang survives. Undercut, honestly. `FormProfile` is the *only* kind that
  returns a finite `hi`, so every other tool provably reduces to old behavior.

Carving one segment: for each swept cell, compute `(lo, hi)` at that cell's `r`
and cutter Z, then **interval-subtract** from the column's span list
(`subtract_interval` + `merge_adjacent`, `sim/dexel.rs`). Interval subtraction is
associative and idempotent, so the *partial-t bitwise-identity* property the
incremental carve relies on carries over from the `min()` case — arguably cleaner.

### Storage layout — the hybrid sidecar (the key idea)

The WASM contract is a flat `f32` array viewed directly by JS. Variable-length
spans would break a naive flat view, so the field is **hybrid**
(`DexelField`, `sim/dexel.rs`):

- a **dense `top: Vec<f32>`** — identical layout/semantics to the old
  `Heightmap::data` (highest solid Z per column). Unchanged hot path, unchanged
  zero-copy WASM/GL upload, unchanged LOD min-pool.
- a **sparse `undercut: HashMap<usize, Vec<Span>>` sidecar** — present only for
  columns with an interior void. An absent key means the implicit single span
  `[stock_bottom_z, top[idx]]`; when present, the span list is authoritative and
  `top[idx]` still mirrors its highest `hi` so every dense reader keeps working.

**Pay for generality only where geometry uses it.** A pure 3-axis job allocates
*zero* sidecar and runs at the old speed and footprint; only T-slot / flip
columns cost more.

### The `CarveTarget` seam

`Heightmap` and `DexelField` both implement `pub(super) trait CarveTarget`
(`sim/sweep.rs`), so the sweep carve core is generic over the field and
`DexelField` is a drop-in. The fast path — `carve_top_down` — is
`Heightmap::lower_at_unchecked` (monotone-min) for the heightmap and
`DexelField::carve_cell(.., +∞)`'s dense fast path for the dexel; these are
provably equal, so a pure 3-axis sweep lands a byte-identical top surface in
either target. This is what let the migration land without regressing the corpus.

### Meshing

The stepped-heightfield mesh extends per column: emit the top of each span and
the bottom of each span, plus **undercut side-walls/floors** where a span
boundary is exposed relative to neighbors, rendered from the sidecar
(frontend, `ivac-58nl.6.5.3`). Dirty-AABB partial upload survives — a column's
span count changing just re-emits that column's quads. No marching cubes needed
for Option A. The **STL export** meshes the undercut voids from the sidecar and
exposes a truly **watertight solid** through the transport + File menu
(`ivac-58nl.6.5.4/.5/.6/.8`).

## Preserving the current wins (hard constraints on any choice)

| Win | How Option A keeps it |
|---|---|
| Incremental partial-t carve @60 fps | interval subtract is associative → identical splitting |
| Dirty-AABB partial GPU upload | unchanged; a span-count change re-emits that column only |
| LOD mip-pyramid (MIN-pool) | dense top array pools exactly as before; undercuts are fine-LOD detail |
| Checkpoint / replay scrub | `DexelSnapshot` captures dense top + sidecar; carving stays monotone (material only removed), so snapshots remain orderable exactly as the `Vec<f32>` snapshots were |
| WASM zero-copy read-back | dense array untouched; sidecar is a second, small exposed buffer |
| Collision checks (rapid / holder / fixture) | operate on the top span; undercut spans add cases, not rewrites |

## Phasing (as executed)

- **Phase 0 — dual-surface spike** (`ivac-rt1.11.1`, ✅). Reframed from "two
  sheets or one solid" to "confirm a 2-span dexel renders as one watertight
  solid." De-risked Phase 2.
- **Phase 1 — multi-span Z-dexel core** (`ivac-58nl.6.1`–`.6.5`, ✅ **shipped**).
  `Span` + interval algebra + `eval_interval` → hybrid `DexelField` → generic
  `CarveTarget` sweep → finite `FormProfile` ceiling (first honest T-slot
  undercut) → live-sim flip to `DexelField` with sidecar buffers, undercut
  rendering, and undercut STL export. Fully back-compatible; 3-axis unchanged.
- **Phase 2 — dual-surface / flip** (`ivac-rt1.11`, ✅ **shipped & closed**).
  From-below carve as a second span; one watertight single-solid render.
  Two-sided machining is authorable, previewable, and documented.
- **Phase 3 — tri-dexel + dual contouring** (**deferred**). The literature is
  unanimous that tri-dexel is the correct model for *multi-axis* removal
  (raycast the swept volume against three orthogonal ray sets; extract the
  surface with dual contouring of Hermite data to preserve tool-mark sharp edges
  that marching cubes rounds off). But it is 3× the field, needs general
  ray/swept-volume intersection for a tilted tool of revolution, and must move
  meshing to dual contouring / surface nets over a Hermite grid — abandoning the
  cheap axis-aligned incremental upload. Nothing in the current 3-axis +
  drag-knife roadmap needs it. **Adopt B only when `P5` (kinematic 4/5-axis) is
  greenlit** — and the Z-dexel is not wasted then, it is one of the three stacks.
  The kinematic engine itself is currently deferred (see
  [`kinematic-sim.md`](./kinematic-sim.md)), so Phase 3 has no live trigger.

## Explicitly out of scope / deferred

- **Tri-dexel (Phase 3)** — gated on 4/5-axis kinematics; see above.
- **True per-column incremental undercut void meshing** (`ivac-58nl.6.5.7`,
  deferred) — a perf refinement gated on GPU profiling; the current allocation-free
  void emitter (`ivac-58nl.6.5.5`) rebuilds only touched columns and is fast
  enough without it.

## When to revisit

Reopen the representation question (Option A → B) only when a concrete,
greenlit feature needs a **non-vertical tool axis** or **arbitrary-orientation**
removal — i.e. `P5` 4/5-axis or lathe verification. Until then, the multi-span
Z-dexel is the material model, and new sim work should extend the span algebra
and the sidecar, not reach for voxels.

### Sources

- Multi-dexel material removal + cutting force, GPGPU — ScienceDirect S0965997811002262
- Tri-dexel cutter-workpiece engagement for 5-axis (validation) — ResearchGate 369299997
- Tri-dexel iso-surface / feature-preserving extraction — ResearchGate 382541364
- Surface reconstruction from three orthogonal ray (dexel) sets — ResearchGate 275383285
- Enhanced Dual Contouring for real-time CNC surface reconstruction (2024) — ResearchGate 379691947
- Ju, Losasso, Schaefer, Warren, "Dual Contouring of Hermite Data", SIGGRAPH 2002
