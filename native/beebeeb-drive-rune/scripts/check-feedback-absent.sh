#!/usr/bin/env bash
# check-feedback-absent.sh [--release] — proves a build WITHOUT the `feedback` feature has none of the
# feedback mode in it, and that the check can fail: the same greps on a build WITH the feature must hit.
# Truth line: "feedback-absent (<profile>): 3 markers, 0 in the plain binary, 3 in the feedback binary; default features: none"
set -euo pipefail
cd "$(dirname "$0")/.."
PROFILE=debug; FLAG=()
if [ "${1:-}" = "--release" ]; then PROFILE=release; FLAG=(--release); fi
TARGET="${CARGO_TARGET_DIR:-target}"
MARKERS=("beebeeb-feedback-note/1" "fb-overlay" "BEEBEEB_FEEDBACK_DIR")

default=$(cargo metadata --locked --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "beebeeb-drive-rune") | (.features.default // []) | join(",")')
[ -z "$default" ] || { echo "feedback-absent: FAIL, default features are '$default' (must be none)"; exit 1; }

count() { local n=0 m; for m in "${MARKERS[@]}"; do if grep -qaF -- "$m" "$1"; then n=$((n + 1)); fi; done; echo "$n"; }

cargo build --locked "${FLAG[@]}" >/dev/null 2>&1
plain=$(count "$TARGET/$PROFILE/beebeeb-drive")
cargo build --locked "${FLAG[@]}" --features feedback >/dev/null 2>&1
with=$(count "$TARGET/$PROFILE/beebeeb-drive")
echo "feedback-absent ($PROFILE): ${#MARKERS[@]} markers, $plain in the plain binary, $with in the feedback binary; default features: none"
[ "$plain" -eq 0 ] && [ "$with" -eq "${#MARKERS[@]}" ]
