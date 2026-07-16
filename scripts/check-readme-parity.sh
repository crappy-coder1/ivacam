#!/usr/bin/env bash
#
# README parity guard (os2k.9).
#
# Keeps README.md and its German mirror README_de.md structurally in
# lock-step: both must expose the same *sequence of heading levels* so a
# section added to one without a counterpart in the other fails the build.
# Heading TEXT is not compared — it's translated (## Use it vs ## Verwendung),
# so only the structure (how many headings, at which depth, in what order)
# is checked. Also asserts the reciprocal language-switcher links exist.
#
# Run standalone, from scripts/pre-release.sh, and from .github/workflows/ci.yml.
# Exit 0 when parity holds, 1 (with a diff) otherwise.
set -euo pipefail

cd "$(dirname "$0")/.."

en=README.md
de=README_de.md

for f in "$en" "$de"; do
  [[ -f $f ]] || { echo "readme-parity: $f is missing" >&2; exit 1; }
done

# Emit one line per ATX heading: its level (count of leading '#'). Lines
# inside ``` fenced code blocks are skipped so shell comments like
# `# core + CLI + server` are never mistaken for headings.
levels() {
  awk '
    /^```/            { fence = !fence; next }
    !fence && /^#+[[:space:]]/ { match($0, /^#+/); print RLENGTH }
  ' "$1"
}

en_levels=$(levels "$en")
de_levels=$(levels "$de")

if [[ "$en_levels" != "$de_levels" ]]; then
  echo "readme-parity: heading structure differs between $en and $de" >&2
  echo "--- $en headings ---" >&2
  grep -nE '^#+[[:space:]]' "$en" >&2 || true
  echo "--- $de headings ---" >&2
  grep -nE '^#+[[:space:]]' "$de" >&2 || true
  exit 1
fi

# The language-switcher bar must cross-link both ways.
grep -q 'README_de.md' "$en" || { echo "readme-parity: $en must link $de (language bar)" >&2; exit 1; }
grep -q './README.md'  "$de" || { echo "readme-parity: $de must link $en (language bar)" >&2; exit 1; }

count=$(grep -cE '^#+[[:space:]]' "$en" || true)
echo "readme-parity: ok ($en and $de share $count headings)"
