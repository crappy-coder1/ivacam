# Kinematic Machine Simulation — Decision Record

**Status:** **DEFERRED** (engine), **SCHEMA ADOPTED as a future import target**
**Date:** 2026-07-17
**Tracker:** `ivac-58nl.5` under epic `ivac-58nl` (GrblGru comparison roadmap)
**Related:** material-model track `ivac-58nl.6`; two-sided machining `ivac-rt1.11` / spike `ivac-rt1.11.1`
**Tracker IDs are ivaCAM beads issues — run `bd show <id>`.**
**Source assessment:** `~/grblgru/GRBLGRU_ASSESS_01.md` (external, untracked — see the correction restated in the last section)

---

## TL;DR

GrblGru's crown jewel is a **kinematic machine model** — full forward/inverse
kinematics for 3-axis gantries, 4/5-axis mills, lathes, foam cutters, and
5/6-DOF robot arms, driven by a data-only machine library (139 configs / 120+
wireframe meshes). ivaCAM has nothing comparable: its `MachineConfig` is a
post-processor config (units, tool-change strategy, axis limits, Z policy) with
**no knowledge of machine geometry, joint positions, or rotation axes**.

The decision:

1. **Defer the FK/IK engine.** Do not build forward/inverse kinematics now. It is
   orthogonal to the material-removal roadmap (P1–P4) and only pays off after the
   material model can represent non-top-down removal (P3, `ivac-58nl.6`).
2. **Treat kinematic simulation as a separate strategic track**, not a child of
   the material-model work. It answers a different question (does the *machine*
   physically reach this pose without collision) than the destructive terrain
   (did the *cut* remove the right material).
3. **Adopt the `.tco` + `Machine.dat` schema as a documented data-only import
   target now** (this document). Capturing the schema is cheap and de-risks the
   hardest architectural decision — the machine-model data shape — before any
   engine work begins.

**Do not let this block P1–P3.**

---

## Why this came up

A decompiled-source comparison of GrblGru V6.38.0 vs ivaCAM (epic `ivac-58nl`)
produced two findings that bear on scope:

- **ivaCAM is *ahead* on material-removal simulation.** GrblGru has no
  volumetric/voxel carve at all — `DoJob3DRoughingSimple` / `DoJob3DFinishing`
  are empty stubs, and the linked `geometry3Sharp` voxel/implicit types are never
  called. Its "simulation" is a kinematic replay drawing a tool-tip polyline
  trail; the stock mesh is never carved. ivaCAM's destructive 2.5D heightmap
  already removes real material. (This retracts the source assessment's original
  "port their voxel sim" recommendation — see the last section.)
- **GrblGru is genuinely ahead on kinematics.** The one component ivaCAM would
  take longest to build independently is exactly the 4/5-axis / lathe / robot
  kinematic chain. That is real, and worth capturing — but it is a *verification*
  capability that only becomes meaningful once ivaCAM targets those machine
  classes and can model their (non-top-down) material removal.

So the value is real but **sequenced behind** the material model, not alongside
it.

---

## The decision in detail

### Defer the engine

The kinematic math is a chain of 4×4 homogeneous transforms (one rotation about
each joint axis at its pivot, accumulated in `OrderOfAxes`) plus, for 5/6-DOF
machines, an inverse solve (closed-form for robot arms, numeric for general
5-axis). The source assessment estimates ~500–1000 lines of linear algebra for
forward kinematics and a substantially harder IK piece. That is mechanical but
non-trivial work with **no payoff until**:

- ivaCAM supports 4/5-axis, lathe, or robot **toolpath generation** (none exist
  today — the pipeline is 3-axis), **and**
- the **material model can represent removal at an arbitrary cutter orientation**
  — i.e. the tri-dexel / voxel evolution in `ivac-58nl.6`. The current single
  `Z(x,y)` heightmap is top-down-only by construction; a tilted 5-axis pass has
  nowhere to write.

Building kinematics before either exists would produce a posed wireframe that
can only replay motion (exactly GrblGru's tool-tip-trail level) without verifying
any cut — the capability ivaCAM already exceeds.

### Separate strategic track

Kinematic simulation is **orthogonal** to the destructive-terrain material model:

| Question | Answered by |
|---|---|
| Did the cut remove the correct material (gouge / rest-stock)? | Destructive terrain — `sim/heightmap.rs` today, `ivac-58nl.6` next |
| Can the *machine* physically reach this tool pose (reach, joint limits, self-collision)? | Kinematic model — this track |

They share almost nothing in code or data. Keeping them as separate tracks avoids
entangling the material-model design (hot path, 60 fps, WASM memory budget) with a
machine-geometry model that is mostly cold-path posing and rendering.

### Adopt the schema now (data-only)

Capturing the machine-model data shape is the "get it right once" architectural
decision the source assessment flags. It costs nothing to model as data and it
lets a future importer target a proven, scalable schema. The schema is documented
below. **No engine, no importer code lands now** — only this specification.

---

## Data schema for future machine-library import

GrblGru defines every machine with two INI-format files. Both are simple enough to
parse with an INI reader; the schema — joint origin + rotation-axis unit vector,
one section per body — scales unchanged from a 3-axis Shapeoko to a 6-DOF robot.

### `.tco` — per-axis wireframe mesh

INI, one section per moving body (`[base]`, `[X]`, `[Y]`, `[Z]`, `[A]`, `[B]`, …).
Each section lists vertices and edges of that body's wireframe:

```ini
[X]
v0001=12.0,0.0,0.0     ; vertex: x,y,z  (mm)
v0002=12.0,80.0,0.0
l0001=1,2              ; edge: vertex index v1,v2 (1-based)
```

The wireframe is **presentation-only** — it is what gets re-posed and drawn per
frame. It is not required for a kinematics *engine*; it is required for the
*visual*. An importer can ingest `.tco` independently of any FK/IK code.

### `Machine.dat` — kinematic config

INI, one section per machine. Each rotary joint (labelled `A`–`H`, plus `Q`) is a
pivot point + a rotation axis:

```ini
[Haas_UMC750_BC]
MachineType=HAAS_UMC750
BposX=0  BposY=151.78  BposZ=11.43  BdirX=0 BdirY=1 BdirZ=0   ; B trunnion, about +Y
CposX=0  CposY=0       CposZ=0      CdirX=0 CdirY=0 CdirZ=1   ; C table,    about +Z
r1=A r2=B r3=X r4=Y r5=Z r6=H r7=Q   ; rotary-axis slot map
t1=X t2=Y t3=Z                       ; translational-axis slot map
OrderOfAxes=xyzab                    ; chain multiplication order
ToolX=-0.987 ToolY=-287.084 ToolZ=424.0  ; tool tip offset in the final frame
```

Per-joint fields:

| Field | Meaning |
|---|---|
| `[J]posX/Y/Z` | Joint pivot point in world space at the zero configuration (mm) |
| `[J]dirX/Y/Z` | Rotation axis as a unit vector |
| `MachineType` | One of the `EnumMachineTyp` kinds (drives the IK code path) |
| `r1..r7` | Which axis occupies each rotary slot |
| `t1..t3` | Which axis occupies each translational slot |
| `OrderOfAxes` | Order in which per-axis transforms are chained |
| `ToolX/Y/Z` | Tool-tip offset applied in the accumulated final frame |

`MachineType` values observed (`EnumMachineTyp`): `FIVE_AXES_1/2/3`,
`FOAM_CUTTER`, `LASER`, `PLASMA_CUTTER`, `MILLINGLATHE`, `MILLY`,
`ROBOT_ARM_5AXES`, `ROBOT_ARM_6AXES`, `SHAPEOKO`, `SPRITE`, `FAGNER_LATHE`,
`FAGNER_ROBOT`, plus `LATHE`, `XYZBC`, `XYZAC`, `X_TABLE_YZBC`, `HAAS_UMC750`.

### Proposed ivaCAM Rust shape (import target — not to be built now)

A future importer would deserialize into a machine-geometry model kept **separate
from `MachineConfig`** (which stays a pure post-processor config). Sketch:

```rust
/// Data-only kinematic machine model. No FK/IK engine implied — this is the
/// import target the .tco + Machine.dat schema deserializes into.
pub struct KinematicMachine {
    pub name: String,
    pub machine_type: MachineType,        // maps EnumMachineTyp
    pub joints: Vec<Joint>,               // one per moving body, in chain order
    pub order_of_axes: Vec<AxisId>,       // OrderOfAxes, resolved to joints
    pub tool_offset: [f64; 3],            // ToolX/Y/Z in the final frame
    pub wireframe: Vec<AxisMesh>,         // from the .tco, presentation-only
}

pub struct Joint {
    pub id: AxisId,                       // A..H, Q, or a linear X/Y/Z
    pub kind: JointKind,                  // Rotary | Linear
    pub origin: [f64; 3],                 // [J]posX/Y/Z — pivot at zero config
    pub axis: [f64; 3],                   // [J]dirX/Y/Z — unit rotation/travel axis
}

pub struct AxisMesh {
    pub id: AxisId,                       // which body ([X], [base], …)
    pub verts: Vec<[f64; 3]>,
    pub edges: Vec<(u32, u32)>,           // 1-based indices in the .tco, 0-based here
}
```

This attaches to a project as an **optional** `kinematic: Option<KinematicMachine>`
alongside the existing `MachineConfig` — it never replaces or reshapes the
post-processor config, and its absence is the default (3-axis, no kinematic
model), so the schema contract for existing projects is untouched.

### Licensing note (important)

The **schema** (the file structure and field semantics above) is not
copyrightable and can be documented and implemented freely — that is what this
record does.

The **machine-library data itself** (the 120+ `.tco` meshes and the `Machine.dat`
configs shipped with GrblGru) is the author's material, distributed only inside a
closed binary. ivaCAM **must not bundle or redistribute those files** without the
author's permission / a compatible relicense. A future importer should instead
either (a) read files the user already has from their own GrblGru install, or
(b) ship an ivaCAM-authored library built from public machine specs. Do not
conflate "adopt the schema" (free) with "import their library" (needs
permission).

---

## Explicitly out of scope (deferred with the engine)

- Forward-kinematics matrix chain (`CalcProduktMatrix` / `EulerTrans` /
  `GetResultingMatrix` equivalents).
- Inverse kinematics (closed-form robot-arm IK; numeric general 5-axis).
- The posed-wireframe renderer.
- GrblGru's macro scripting language (`.dat` parameterized G-code) — a separate,
  independent question if ever wanted.

## When to revisit

Pick this track up when **both** hold:

1. `ivac-58nl.6` has landed a material model that can represent removal at an
   arbitrary cutter orientation (tri-dexel or voxel), so a kinematically-posed
   cut can actually carve, **and**
2. there is a concrete user need for 4/5-axis, lathe, or robot **verification**
   (not just posing) — i.e. ivaCAM has begun generating toolpaths for those
   machine classes.

Until then, the schema above is the artifact; the engine stays deferred.

---

## Correction restated (durability)

The source assessment `~/grblgru/GRBLGRU_ASSESS_01.md` originally recommended
porting GrblGru's "3D voxel sim" (its §4 and verdict recommendation #5). **That
recommendation is retracted:** GrblGru performs no voxel material removal — it
draws a tool-tip trail and never carves the stock. ivaCAM's destructive heightmap
is already ahead on material-removal simulation, and the real upgrade path is
ivaCAM's own multi-dexel/voxel evolution (`ivac-58nl.6`), not a port from
GrblGru. The external assessment has been annotated in place, but that file is not
tracked in this repository — this paragraph is the durable in-repo record of the
correction.

