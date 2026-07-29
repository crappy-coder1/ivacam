# ivac-cps

Autodesk-`.cps`-compatible post-processor runtime: Fusion-style posts
(ES5 JavaScript) executed on the pure-Rust [boa] engine, so every
ivacam transport — CLI, server, Tauri, wasm — can run vendor posts
without a C toolchain.

Architecture, IR contract, and milestone plan live in the tracking epic
(`bd show ivac-yhdf`).

## Layout

- `src/engine.rs` — boa context lifecycle: `__ivac` host object,
  named-source eval, prelude loader, entry-point calls.
- `src/ir.rs` — the recorder↔runtime program IR (`IR_VERSION`-gated,
  camelCase JSON) plus the `codes` constant table the JS prelude must
  mirror.
- `src/meta.rs` — `PostMeta` property sheets for UI forms.
- `prelude/*.js` — the runtime API surface, evaluated in numeric order.
  Plain ES5: the same files run under node (tests) and boa
  (production).
- `prelude/tests/` — node-side test harness (node ≥ 20, zero npm
  dependencies) and the V8↔boa differential fixtures.

## Testing

```sh
# Rust side: engine spike, IR round-trips, boa half of the differential
cargo test -p ivac-cps

# Node side: prelude suites + freshness check of the differential
# fixtures (also runs inside `cargo test` when node is on PATH)
node prelude/tests/run.mjs
```

### V8 ↔ boa differential

`prelude/tests/fixtures/format_cases.json` and `scenarios.json` are
case tables; `*_expected.json` are their outputs computed under node/V8
by `node prelude/tests/run.mjs --update` (committed). The Rust test
`tests/differential.rs` replays the same tables through the same
`case_runner.js` under boa and byte-compares. A divergence means an
engine-semantics gap that would corrupt NC output — fix the prelude (or
add a `00_polyfill.js` shim); never loosen the comparison.

## Security model

User `.cps` scripts are UNTRUSTED input. The runtime gives them no
filesystem, network, or process host bindings — the entire host API is
`__ivac`: an in-memory NC-text sink, diagnostics, a fixed clock, and a
cancellation probe. Execution budgets (`RunLimits`: loop-iteration +
recursion caps) terminate runaway scripts instead of wedging a worker;
the server additionally caps inspected scripts at 1 MiB and rejects
filesystem-path post selections outright (scripts travel inline or by
bundled id — the server never reads server-side paths on a client's
behalf).

## License note

`refs/cam-posteditor` (Autodesk post-editor sources, FANUC test post,
`globals.d.ts`) is a TEST ORACLE only — tests that use it skip when the
directory is absent, and nothing from it ships in any artifact. Bundled
posts under `posts/` are ivacam-authored.

[boa]: https://github.com/boa-dev/boa
