#!/usr/bin/env bash
# compare.sh <frame-id> [--theme light|dark] [--no-build]
#
# The frame gate, one command: build, render the frame headless, export its element tree,
# and compare both against the board golden (scripts/gate.py). The last line is the truth line:
#   <frame> geometry: N checked, M failing · pixels: N checked, M failing · text: … · icons: …
#
# Exit 0 only when all four M are 0. Exit 1: the gate found differences. Exit 2: a step
# before the gate failed (no golden, build failed, no render): never a stale PASS, because
# the previous render and export are deleted before the build starts.
#
#   EVIDENCE_DIR   where renders, side-by-sides and fail crops go
#   CARGO_TARGET_DIR, BUILD_SLOT (scripts/coord/build-slot.sh: waits for a build slot)
set -euo pipefail

usage() { echo "usage: compare.sh <frame-id> [--theme light|dark] [--no-build]" >&2; exit 64; }
FRAME_ID=""; THEME="light"; BUILD=1
while [ $# -gt 0 ]; do
    case "$1" in
        --theme) THEME="${2:-}"; shift 2 ;;
        --no-build) BUILD=0; shift ;;
        -*) usage ;;
        *) [ -z "$FRAME_ID" ] || usage; FRAME_ID="$1"; shift ;;
    esac
done
[ -n "$FRAME_ID" ] || usage
case "$THEME" in light|dark) ;; *) usage ;; esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE_DIR="$(dirname "$SCRIPT_DIR")"
STEM="$FRAME_ID"; [ "$THEME" = dark ] && STEM="$FRAME_ID-dark"
EVIDENCE_DIR="${EVIDENCE_DIR:-/home/guus/code/beebeeb.io/.claude/tasks/verification-evidence/2445/native}"
OUT_DIR="$EVIDENCE_DIR/$STEM"
BOARD_PNG="$CRATE_DIR/golden/$STEM-board.png"
BOARD_JSON="$CRATE_DIR/golden/$STEM.json"
NATIVE_PNG="$OUT_DIR/$STEM-native.png"
NATIVE_JSON="$OUT_DIR/$STEM-native.json"
die() { echo "compare: $*" >&2; exit 2; }

for f in "$BOARD_PNG" "$BOARD_JSON"; do
    [ -s "$f" ] || die "no golden $f (make it: node scripts/capture-golden.mjs $FRAME_ID$([ "$THEME" = dark ] && echo ' --dark'))"
done
mkdir -p "$OUT_DIR"
# Nothing from an earlier run may answer for this one.
rm -f "$NATIVE_PNG" "$NATIVE_JSON" "$OUT_DIR/$STEM-gate.txt"

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/code/bb-worktrees/target-native-n2}"
BIN="$CARGO_TARGET_DIR/debug/beebeeb-drive"
cd "$CRATE_DIR"
if [ "$BUILD" = 1 ]; then
    BUILD_SLOT="${BUILD_SLOT:-/home/guus/code/beebeeb.io/scripts/coord/build-slot.sh}"
    [ -x "$BUILD_SLOT" ] || die "no build-slot script at $BUILD_SLOT (set BUILD_SLOT, or pass --no-build after your own build)"
    "$BUILD_SLOT" -- cargo build --locked --bin beebeeb-drive || die "the build failed"
fi
[ -x "$BIN" ] || die "no binary at $BIN"

# Software Vulkan for a headless render where there is no GPU (WSL).
LAVAPIPE_ENV="${LAVAPIPE_ENV:-$HOME/code/bb-legion-native/lavapipe/env.sh}"
# shellcheck source=/dev/null
[ -f "$LAVAPIPE_ENV" ] && source "$LAVAPIPE_ENV"

SIZE="$(python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(str(d["width"])+"x"+str(d["height"]))' "$BOARD_JSON")" || die "cannot read the size from $BOARD_JSON"
"$BIN" --snap "$NATIVE_PNG" --scene "$FRAME_ID" --size "$SIZE" --scale 2 --theme "$THEME" || die "the render failed"
[ -s "$NATIVE_PNG" ] || die "the render wrote nothing to $NATIVE_PNG"
"$BIN" --gate-export "$NATIVE_JSON" --scene "$FRAME_ID" --size "$SIZE" --theme "$THEME" || die "the element export failed"
[ -s "$NATIVE_JSON" ] || die "the element export wrote nothing to $NATIVE_JSON"

python3 "$SCRIPT_DIR/gate.py" "$BOARD_JSON" "$BOARD_PNG" "$NATIVE_JSON" "$NATIVE_PNG" "$OUT_DIR" "$STEM"
