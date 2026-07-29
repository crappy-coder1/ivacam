// 00_polyfill.js — engine-gap shims.
//
// The cps.0 spike (crates/ivac-cps/tests/spike.rs) verified boa 0.21.1
// natively covers everything vendor posts lean on: annex-b
// String.prototype.substr, RegExp literals + constructor, arguments
// semantics, Object.defineProperty accessors, host-global overwrite.
// This file stays as the landing pad for any gap a future engine bump
// or field report uncovers — keep shims tiny and documented.
//
// Runs under both boa (production) and node (prelude test harness);
// everything here must be plain ES5.

// (intentionally empty)
