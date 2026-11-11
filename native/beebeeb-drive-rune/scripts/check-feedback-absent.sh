#!/usr/bin/env bash
# check-feedback-absent.sh — proves a binary has none of the feedback mode in it.
#
#   check-feedback-absent.sh --artifact <exe>   check THIS file (packaging: the exact file it ships)
#   check-feedback-absent.sh [--release]        build both variants and check them (the gate)
#
# Gate mode builds the plain binary in the normal target dir and the feedback binary in a
# SEPARATE one (<target>-feedback-probe), so the gate never leaves a feedback build where a
# release step would pick it up. It also proves the check can fail: the feedback binary must
# contain every marker. Truth lines:
#   feedback-absent (<profile>): 3 markers, 0 in the plain binary, 3 in the feedback binary; default features: none
#   feedback-absent (artifact): 3 markers, 0 in <exe>
set -euo pipefail
cd "$(dirname "$0")/.."
# Long strings only: an optimised build inlines short ones ("fb-overlay") as immediates, where
# grep cannot see them (seen: the first --release run found 2 of 3).
MARKERS=("beebeeb-feedback-note/1" "beebeeb-feedback-tree/1" "BEEBEEB_FEEDBACK_DIR")

count() { local n=0 m; for m in "${MARKERS[@]}"; do if grep -qaF -- "$m" "$1"; then n=$((n + 1)); fi; done; echo "$n"; }

if [ "${1:-}" = "--artifact" ]; then
  exe="${2:-}"
  [ -n "$exe" ] && [ -f "$exe" ] && [ -s "$exe" ] || { echo "feedback-absent (artifact): FAIL, '$exe' is not a non-empty file"; exit 2; }
  n=$(count "$exe")
  echo "feedback-absent (artifact): ${#MARKERS[@]} markers, $n in $exe"
  [ "$n" -eq 0 ]
  exit
fi

PROFILE=debug; FLAG=()
if [ "${1:-}" = "--release" ]; then PROFILE=release; FLAG=(--release); fi
TARGET="${CARGO_TARGET_DIR:-$PWD/target}"
PROBE="${TARGET%/}-feedback-probe"

default=$(cargo metadata --locked --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "beebeeb-drive-rune") | (.features.default // []) | join(",")')
[ -z "$default" ] || { echo "feedback-absent: FAIL, default features are '$default' (must be none)"; exit 1; }

cargo build --locked "${FLAG[@]}" >/dev/null 2>&1
cargo build --locked "${FLAG[@]}" --features feedback --target-dir "$PROBE" >/dev/null 2>&1
plain=$(count "$TARGET/$PROFILE/beebeeb-drive"); with=$(count "$PROBE/$PROFILE/beebeeb-drive")
echo "feedback-absent ($PROFILE): ${#MARKERS[@]} markers, $plain in the plain binary, $with in the feedback binary; default features: none"
[ "$plain" -eq 0 ] && [ "$with" -eq "${#MARKERS[@]}" ]
