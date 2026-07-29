#!/usr/bin/env bash
# wasm-size-guard.sh — measure what the .cps runtime costs the browser
# bundle, and fail when it exceeds the budget.
#
# Builds crates/ivac-wasm twice (without and with `--features cps`),
# reports raw + gzipped sizes and the delta, and exits non-zero when the
# gzipped delta is above WASM_CPS_BUDGET_KB.
#
#   scripts/wasm-size-guard.sh                 # gate at the default budget
#   WASM_CPS_BUDGET_KB=1200 scripts/wasm-size-guard.sh
#
# Gzipped is the number that matters: every real deployment serves the
# .wasm compressed, so that's what a user's first paint actually waits
# for.
set -uo pipefail

cd "$(dirname "$0")/.."

# Measured 2026-07-29 on this tree: 1542 KiB gzipped (baseline 1314 →
# 2856 with cps). The budget sits just above that so this script works
# as a REGRESSION detector — an accidental ICU4X/intl pull or a boa
# bump that drags in more surface blows past it immediately.
#
# SHIP DECISION (cps.10): wasm keeps `cps` OFF by default — 1.5 MiB
# gzipped more than doubles the browser bundle for a feature most web
# users won't touch. CPS ships on CLI / server / Tauri, and the web UI
# hides the option via the `post-cps` capability probe. Revisit if boa
# slims down or a browser-native executor replaces it (the prelude is
# engine-agnostic by design).
BUDGET_KB="${WASM_CPS_BUDGET_KB:-1700}"

if ! command -v wasm-pack >/dev/null 2>&1; then
  echo "SKIP: wasm-pack not on PATH (cargo install wasm-pack)"
  exit 0
fi

OUT_BASE="target/wasm-size-guard"
rm -rf "$OUT_BASE"
mkdir -p "$OUT_BASE"

build() {
  local label="$1"
  shift
  echo "building ivac-wasm ($label)…"
  if ! wasm-pack build crates/ivac-wasm --target web --release \
    --out-dir "../../$OUT_BASE/$label" "$@" >"$OUT_BASE/$label.log" 2>&1; then
    echo "FAIL: wasm-pack build ($label) — see $OUT_BASE/$label.log"
    return 1
  fi
  local wasm
  wasm="$(find "$OUT_BASE/$label" -name '*_bg.wasm' -print -quit)"
  if [[ -z "$wasm" ]]; then
    echo "FAIL: no .wasm produced for $label"
    return 1
  fi
  gzip -9 -c "$wasm" >"$wasm.gz"
  RAW=$(stat -c %s "$wasm")
  GZ=$(stat -c %s "$wasm.gz")
  return 0
}

build baseline || exit 1
BASE_RAW=$RAW
BASE_GZ=$GZ

build cps --features cps || exit 1
CPS_RAW=$RAW
CPS_GZ=$GZ

kib() { awk -v b="$1" 'BEGIN { printf "%.0f", b / 1024 }'; }

DELTA_RAW=$((CPS_RAW - BASE_RAW))
DELTA_GZ=$((CPS_GZ - BASE_GZ))
DELTA_GZ_KB=$(kib "$DELTA_GZ")

printf '\n%-24s %10s %10s\n' "build" "raw KiB" "gzip KiB"
printf '%-24s %10s %10s\n' "without cps" "$(kib $BASE_RAW)" "$(kib $BASE_GZ)"
printf '%-24s %10s %10s\n' "with cps" "$(kib $CPS_RAW)" "$(kib $CPS_GZ)"
printf '%-24s %10s %10s\n' "delta" "$(kib $DELTA_RAW)" "$DELTA_GZ_KB"
printf '%-24s %10s %10s\n\n' "budget (gzip)" "" "$BUDGET_KB"

if (( DELTA_GZ_KB > BUDGET_KB )); then
  echo "FAIL: cps adds ${DELTA_GZ_KB} KiB gzipped, over the ${BUDGET_KB} KiB budget."
  echo "      Either shrink the runtime or keep cps off by default for wasm"
  echo "      (the UI hides the option via the post-cps capability probe)."
  exit 1
fi

echo "OK: cps adds ${DELTA_GZ_KB} KiB gzipped (budget ${BUDGET_KB} KiB)."
