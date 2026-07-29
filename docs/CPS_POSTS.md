# Writing and testing a `.cps` post for ivaCAM

ivaCAM runs Autodesk-`.cps`-compatible post-processors: ES5 JavaScript
files that declare config globals, user properties, and entry-point
callbacks (`onOpen`, `onSection`, `onLinear`, `onCircular`,
`onCyclePoint`, `onClose`, …). If you have written or tuned a Fusion 360
post, the same file shape works here.

The runtime lives in [`crates/ivac-cps`](../crates/ivac-cps): a JS
engine ([boa], pure Rust, so every transport can run it) plus a runtime
prelude implementing the kernel API the posts call.

## Using a post

- **Bundled**: pick one in *Machine settings → G-code dialect → CPS
  post*. `ivac posts` lists them from the CLI.
- **Your own file**: *Open .cps file…* in the same place. The script is
  **embedded into the project**, so a saved `.ivac-project` stays
  self-contained and reproduces byte-identical output on another
  machine.
- Whatever the post declares in `properties` shows up as a form
  (checkbox / number / dropdown / text). Only values you actually change
  are stored, so a post updating its own defaults takes effect.

From the CLI:

```sh
ivac posts                                 # list bundled posts
ivac posts inspect my-post.cps             # identity + property sheet as JSON
ivac generate part.dxf --post cps --post-id grbl
ivac generate part.dxf --post cps --post-file my-post.cps \
  --post-prop useM30=false --post-prop spindleWarmupSeconds=2
```

## Writing one

Start from [`crates/ivac-cps/posts/grbl.cps`](../crates/ivac-cps/posts/grbl.cps)
— it is deliberately small and covers the whole shape: config globals,
properties, format/variable/modal factories, the entry points, and
cycle expansion for a controller without canned cycles.

The contract in brief:

```js
description = "My controller";
extension = "nc";                     // drives the export file extension
capabilities = CAPABILITY_MILLING;
tolerance = spatial(0.01, MM);
allowedCircularPlanes = 1 << PLANE_XY; // omit entirely to allow any plane

properties = {
  useM30: { title: "End with M30", type: "boolean", value: true, scope: "post" },
};

var xyzFormat = createFormat({ decimals: 3, forceDecimal: true });
var xOutput = createVariable({ prefix: "X" }, xyzFormat);

function onOpen() { /* header */ }
function onSection() { /* per operation: tool, spindle, WCS, first move */ }
function onRapid(x, y, z) { /* G0 */ }
function onLinear(x, y, z, feed) { /* G1 */ }
function onCircular(cw, cx, cy, cz, x, y, z, feed) { /* G2/G3 */ }
function onCyclePoint(x, y, z) { /* canned cycle, or expandCyclePoint(x,y,z) */ }
function onClose() { /* footer */ }
```

Things worth knowing about this runtime specifically:

- **Kernel arc policy runs before your `onCircular`.** Arcs outside your
  declared radius / sweep / chord / plane limits, or helical arcs when
  `allowHelicalMoves` is false, are auto-linearized through `onLinear`;
  arcs sweeping past `maximumCircularSweep` are split. Declare the
  limits honestly and your callback only sees arcs it can emit.
- **`expandCyclePoint(x, y, z)`** replays the cycle as plain
  rapid/feed/dwell moves through *your* callbacks. Controllers without
  canned cycles (GRBL) simply call it for every point.
- **Diagnostics**: `warning(msg)` surfaces in the UI warnings panel;
  `error(msg)` halts the run with your message (partial output is kept
  for inspection). Unsupported kernel corners emit one named diagnostic
  rather than failing silently.
- **Sandbox**: no filesystem, network, or process access — the only host
  API is NC text out plus diagnostics. `redirectToFile` captures into
  memory and warns; execution budgets stop runaway loops.
- **What is not supported in v1**: probing macros, subprograms written
  to separate files, turning and additive. Those symbols exist and emit
  a named diagnostic, so a post touching them tells you what it wanted.

## Testing a post

```sh
# Does it parse, and what properties does it expose?
ivac posts inspect my-post.cps

# Real output over real geometry:
ivac generate part.dxf --post cps --post-file my-post.cps | jq -r .gcode
```

Errors name the file and line — `my-post.cps:143` — so a stack trace
points at your source, not at runtime internals.

For runtime development, the crate carries its own harnesses:

```sh
cargo test -p ivac-cps            # engine, IR, goldens, symbol audit
node crates/ivac-cps/prelude/tests/run.mjs   # prelude suites under node/V8
cargo bench -p ivac-cps           # post-execution throughput
```

The prelude is checked twice over: node runs it under V8, and
`tests/differential.rs` replays the identical case tables under boa and
byte-compares. Any divergence between the two engines is a bug in the
prelude, not a test to relax.

## Licensing

Posts bundled with ivaCAM are ivaCAM-authored and GPL-3.0-or-later. The
runtime reimplements a published API surface from Autodesk's public
`globals.d.ts` declarations; no Autodesk kernel code is included, and the
Autodesk-copyrighted material under `refs/` is a test oracle only —
tests using it skip when it is absent and it is never embedded in a
build artifact. A vendor post you supply stays yours; it is embedded in
your project file, not redistributed by ivaCAM.

[boa]: https://github.com/boa-dev/boa
