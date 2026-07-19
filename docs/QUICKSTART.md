# ivaCAM — quickstart

A 5-minute tour for people who already know CNC milling, laser cutting,
or plasma cutting and want to turn a 2D drawing into G-code. If you're
trying to _build_ the app or contribute code, see [`README.md`](../README.md)
and [`BUILDING.md`](./BUILDING.md) instead.

## 1. Install

ivaCAM ships as a single-binary desktop app via **Tauri**. The
desktop bundle includes the rendering engine and CAM math; nothing else
needs to be installed.

- **Linux**: download the `.AppImage` for your release, `chmod +x` it,
  and run it. (No prebuilt binaries are published yet — the repo has no
  remote — so follow [`BUILDING.md`](./BUILDING.md) to produce one
  locally with `cargo tauri build --bundles appimage`.)
- **macOS / Windows**: same pattern with the platform-native bundle.
- **Browser-only**: the WASM build runs entirely client-side; everything
  in this guide applies, just hosted from a static site.

## 2. First run

The window opens to an empty workspace. The four areas you'll use:

| Area                 | What it's for                               |
| -------------------- | ------------------------------------------- |
| **Top menu bar**     | File / Edit / View / Tools / Help           |
| **2D canvas (left)** | Your drawing, with layer + object selection |
| **3D scene (right)** | Toolpath preview + the cut simulation       |
| **Sidebar / panels** | Operations, tools, machine, stock, settings |

The bottom strip is the **Generate bar** — the button that turns the
project into G-code.

## 3. Open a drawing

`File ▸ Open` accepts:

- **DXF** — the most common CAD interchange (R12 and later).
- **SVG** — any vector editor's output (paths, polylines, basic text).

Drag-and-drop also works: drop a file onto the window.

After import you should see your geometry in the 2D canvas. Use the
**Layers** sidebar panel to toggle layers on / off — anything hidden is
also excluded from operations.

> If the drawing came in at the wrong scale, units, or rotation, open
> **File ▸ File transform** (or the per-import gear icon in the Layers
> panel). Common case: DXF saved in mm but interpreted as inches.

## 4. Tell ivaCAM about your machine

Two configurations matter before you generate code:

### Machine

`Tools ▸ Machine setup` — set the **post-processor** (LinuxCNC, GRBL,
HPGL), **work-area limits**, **rapid speed**, and whether the controller
supports an automatic **tool changer** (M6). The defaults are LinuxCNC,
no toolchanger, 500×400×100 mm work area; change what doesn't match
your machine.

If it's a laser, plasma, or drag-knife setup, switch the **mode**
dropdown — that hides the irrelevant fields (RPM for laser, plunge for
drag-knife, etc.) so you can't accidentally drive the cutter the wrong
way.

### Stock

`Stock` panel in the sidebar — width, depth, thickness, and where the
work-coordinate-system **origin** sits relative to the stock. The 3D
scene draws the stock as a translucent block; this is what the
simulation carves into.

### Tools

`Tools ▸ Tool library` opens the per-project tool list. Each tool has a
**kind** (endmill, ball-nose, V-bit, drill, drag-knife, laser, …), a
diameter, RPM, feed / plunge rates, and a `tipAngleDeg` for conical
bits. Bad values (zero RPM, zero feed, sub-0.01 mm diameter) get a red
border and disable **OK** until fixed — same for any open operation.

## 5. Add operations

The **Operations** sidebar panel is the heart of the project. Click
**+ Add op**, pick a kind:

- **Profile** — cut along a contour (outside, inside, on-line).
- **Pocket** — clear an area inside a closed contour.
- **Drill** — peck or spot-drill at point geometry.
- **VCarve / Engrave** — V-bit depth follows local clearance.
- **Chamfer** — bevel an edge.
- **Thread mill** — internal or external threads.
- **Drag-knife / Laser** — non-cutting kinds (no plunge, no RPM).

Each operation needs a **source** (which geometry it consumes), a
**tool**, and a few kind-specific parameters (depth-per-pass, finish
allowance, …). Inapplicable fields show a tooltip explaining why
they're disabled (e.g. "Laser cuts at constant Z").

> **Tabs**: for profile / pocket ops with `tabMode: manual` or `mixed`,
> click in the 2D canvas to place a tab on the nearest source contour.
> The ghost preview snaps to vertices / midpoints / existing tabs as you
> hover.

## 6. Generate G-code

Hit **Generate** in the bottom bar. Two things happen:

1. The Rust pipeline plans the toolpath for every enabled op, in order.
2. A voxel simulation carves the stock with the planned toolpath and
   raises **warnings** for anything dangerous.

The 3D scene updates live: cut moves coloured by op, the cutter cone
tracking the active segment, the heightmap deforming with the
simulation.

## 7. Read the warnings before you cut

The Warnings panel (auto-opens when warnings exist) is the safety
gate. The four critical kinds:

| Warning                  | What it means                                                      |
| ------------------------ | ------------------------------------------------------------------ |
| `rapid_through_material` | A rapid (G0) moves through stock — broken depth or missing retract |
| `fixture_collision`      | The tool or shank passes through a fixture                         |
| `holder_collision`       | The tool-holder hits a sidewall or pocket floor                    |
| `cell_size_coarsened`    | Sim cell size was raised to fit the budget — informational         |

Click a warning row to jump the 3D scene to the warning location. The
generated G-code is still emitted — ivaCAM doesn't block you
from running unsafe code — but the **Settings** panel has a _"Block
G-code save on critical warnings"_ toggle that turns the warnings into
hard gates if you want belt-and-braces.

## 8. Save the G-code

`File ▸ Save G-code…` writes the result to disk, using the file
extension your post-processor's profile prefers (`.ngc` / `.nc` /
`.gcode`). The status bar shows the program length, total time
estimate, and the tools used.

For **multi-tool jobs**, the toolchange envelope is automatic: a safe-Z
retract → M5 + dwell → M6 (or a manual `M0` pause if your machine has
no changer) → tool Z-shift → M3 + dwell at the new tool's commanded
RPM. The Warnings panel surfaces a row per toolchange so you can review
them before running.

## 9. Save the project

`File ▸ Save project as…` writes a `.ivac-project.json` containing your
imports, ops, tools, machine, stock, and fixtures. Reopening it brings
you back exactly where you left off, including which layers were
visible and which op was selected.

Tool libraries can also be saved separately as `.ivac-toolset.json`
(`Tools ▸ Save toolset` / `Load toolset`), so the same library can be
reused across projects.

## Two-sided (flip-stock) machining

Some parts need cutting from both faces: two-sided signs, joinery,
pockets deeper than the stock is thick, or 3D-effect carving with a
front and a back. ivaCAM handles this as **one project that emits two
G-code programs** — you cut the front, flip the stock over once, and cut
the back. The alignment between the two setups is the whole game, so the
workflow is opinionated and automated around **dowel-pin registration**.

### 1. Enable it and pick the flip axis

In the **Stock** panel, turn on **Enable two-sided machining**. That
seeds a sensible default registration: flip about **X**, two **6 mm**
dowel holes inset **10 mm** from the edges.

The **flip axis** is the single most error-prone choice in the whole
workflow — get it wrong and the back cuts land mirrored the wrong way.
Read the two options literally:

| Choice                     | You turn the stock over about… | What mirrors        |
| -------------------------- | ------------------------------- | ------------------- |
| **Flip about X (mirror Y)** | a left–right line (parallel to X) | Y (front↔back edge) |
| **Flip about Y (mirror X)** | a front–back line (parallel to Y) | X (left↔right edge) |

The **flip direction preview** badge next to the setting shows which way
the stock rolls, so you can match it to how you'll physically turn the
material on the table.

### 2. Dowel registration (automatic)

Two dowel holes are the minimum for an unambiguous flip — one hole would
let the stock pivot. ivaCAM auto-places them **on the flip-axis
centre-line** (the line the mirror leaves untouched, so the same holes
register both setups) and spreads them toward the two ends, inset by the
**dowel inset** you set. You never type hole coordinates.

- **Dowel diameter** — match your pins (6 mm is the hobby-CNC standard).
- **Dowel holes** — count (2 is plenty for most parts; 3+ for large or
  asymmetric stock).
- **Dowel inset** — distance from the stock edge, kept in waste material
  clear of the part envelope.

The **front** program drills these holes (through the stock into your
spoilboard, so a pin passes fully); the **back** program only references
their positions in its header — it does not re-drill them.

### 3. Tag each operation's side

With two-sided mode on, every operation's properties panel gains a
**Side** dropdown: **Front** (default) or **Back**. Back operations are
mirrored about the flip axis and machined after you turn the stock over.

Author a **back** op's cut depths **from the back face** — 0 is the back
face, negative goes into the stock — exactly like a front op reads from
the top face. ivaCAM emits those depths verbatim and tells the operator
to re-zero Z after the flip, because real stock thickness varies by a
few tenths and a physical re-zero is more reliable than a computed one.

### 4. Preview and conflict checks

Generate as usual. The 3D scene shows **one solid**: the front carve as
the top surface and the mirrored back carve as the underside, stitched
watertight. Two safety checks run automatically:

- A front op that would cut **clean through** the stock is **refused** —
  you can't flip and re-register a part you've already severed. Reduce
  its depth, add holding tabs, or move the through-cut to the back
  (last) side.
- Where a front cut and a back cut **overlap** (they meet past the stock
  thickness), red **conflict markers** appear at the collision and a
  warning rides on the front program. Verify the through-feature is
  intended before cutting.

### 5. Two programs out

A two-sided generate produces a **front** and a **back** program:

- The **G-code panel** grows **Front / Back** tabs. (The back tab is a
  read-only listing — the live 3D playhead tracks the front toolpath.)
- The Generate bar shows **two** download buttons — **Front** and
  **Back**, each in your post-processor's file extension — and `File ▸
  Save G-code` gains matching *front* / *back* entries. Save both.

Each program carries a header block that repeats the workflow so the
operator at the machine doesn't need the app open.

### 6. At the machine

This is the physical routine the two programs are written for:

1. **Run the front program first.** It drills the dowel holes, then cuts
   the front. Do **not** unclamp the stock yet.
2. **Drop the dowel pins in** the freshly drilled holes.
3. **Flip the stock** about the axis named in the back program's header,
   and seat it on the pins at the coordinates the header lists.
4. **Re-zero Z** to the new top (the back face) — the header reminds you,
   because the exact thickness matters here.
5. **Run the back program.**

### Worked example (desk-verified)

A 60 × 40 × 10 mm two-sided sign — a 3 mm front pocket and a 2 mm back
pocket, flip about X, two 6 mm dowels inset 2 mm — emits a front program
that opens with:

```gcode
; ===== TWO-SIDED JOB: FRONT PROGRAM =====
; Run this program FIRST.
; Drills 2 dowel registration hole(s) — leave the pins in for the back program.
; Do NOT unclamp the stock until the back program is set up.
```

drills the two dowels on the y = 20 centre-line (`G83 X2 Y20 …` /
`G83 X58 …`, deep enough for the drill tip to break through), then cuts
the front pocket. The back program opens with:

```gcode
; ===== TWO-SIDED JOB: BACK PROGRAM =====
; 1. FLIP the stock about the X axis.
; 2. Seat the stock on the dowel pins at: (2.00, 20.00), (58.00, 20.00).
; 3. RE-ZERO Z to the new top (back) face before running — stock thickness varies.
```

and cuts the back pocket mirrored into the far half of the stock
(Y 23–37 where the front pocket sat at Y 3–17), at depths measured from
the back face. Both programs' preambles are the standard LinuxCNC / GRBL
`G21 G90 G54 G17 G40 G94` block; the dowel peck cycle closes with `G80`
before the tool change — safe to run on either controller.

## Troubleshooting

| Symptom                                      | First thing to check                                                                                           |
| -------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| Generate button does nothing                 | At least one operation must be enabled.                                                                        |
| Op has a red status badge                    | Hover the badge — the warning message says which field is off (zero feed, missing depth, wrong tool kind, …).  |
| 2D canvas is empty after import              | Layer might be hidden — check the Layers panel.                                                                |
| Toolpath previews but G-code is empty        | The op's source filter doesn't match any chained object; switch source mode or pick objects manually.          |
| Sim shows nothing carving                    | Stock thickness might be 0 / wrong sign. The stock origin is _measured from_ the WCS origin; positive Z is up. |
| `(open)` next to a contour in tabs / engrave | The contour isn't closed — close it in the source DXF/SVG, or use a different op that accepts open paths.      |

## Where to next

- The 3D scene's **right-click** menu has a quick "make this an op
  source" picker — handy for engrave / drag-knife flows.
- **Settings ▸ OSnap** controls vertex / midpoint / intersection /
  centre snapping in the 2D canvas — turn off the kinds that get in
  your way.
- For per-operation tabs, lead-ins, dual-tool finishing passes, and
  every post-processor knob, the operation's properties panel on the
  right has everything; hover any field for a tooltip.
- Architecture and contributor patterns: [`ARCHITECTURE.md`](./ARCHITECTURE.md).
- Build and packaging: [`BUILDING.md`](./BUILDING.md).
- Open issues + planned work: run `bd ready` in the repo, or browse the
  [`.beads/`](../.beads/) directory.

Found a bug or a missing feature? File it: `bd create --title="..."
--description="..." --type=bug` from the project root.
