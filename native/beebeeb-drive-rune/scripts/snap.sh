#!/usr/bin/env bash
# snap.sh <frame-id> — render a headless PNG and element tree for a frame.
#
# Usage:
#   scripts/snap.sh DW-0401
#   scripts/snap.sh DW-0401 --scale 2
#
# Sources the lavapipe env (software Vulkan on Linux) so no display server
# is needed. Writes the PNG and tree to the verification evidence directory.
#
# This is the method every later slice will use:
#   1. Run snap.sh <frame-id> to render the frame headlessly.
#   2. Look at the PNG (Read tool or image viewer).
#   3. Compare with the board's drawing.
#   4. Fix code; re-run snap.sh; repeat until layout matches.
#   5. Write remaining differences to diff.md.

set -euo pipefail

FRAME_ID="${1:?Usage: snap.sh <frame-id> [--scale N]}"
SCALE="${2:-1}"
if [ "$SCALE" = "--scale" ]; then
    SCALE="${3:-1}"
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE_DIR="$(dirname "$SCRIPT_DIR")"
REPO_ROOT="$(cd "$CRATE_DIR/../.." && pwd)"
EVIDENCE_DIR="/home/guus/code/beebeeb.io/.claude/tasks/verification-evidence/2445/rune"

# Source lavapipe (software Vulkan) if available.
LAVAPIPE_ENV="$HOME/code/bb-legion-native/lavapipe/env.sh"
if [ -f "$LAVAPIPE_ENV" ]; then
    # shellcheck source=/dev/null
    source "$LAVAPIPE_ENV"
fi

# Build release if not already built.
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/code/bb-worktrees/target-rune-spike}"
export CARGO_TARGET_DIR
BIN="$CARGO_TARGET_DIR/release/beebeeb-drive"
if [ ! -f "$BIN" ]; then
    echo "Building release..."
    (cd "$CRATE_DIR" && cargo build --release)
fi

mkdir -p "$EVIDENCE_DIR"

echo "Rendering ${FRAME_ID} dark..."
"$BIN" --snap "$EVIDENCE_DIR/${FRAME_ID}-rune-dark.png" --size 1280x800 --scale "$SCALE" --theme dark

echo "Rendering ${FRAME_ID} light..."
"$BIN" --snap "$EVIDENCE_DIR/${FRAME_ID}-rune-light.png" --size 1280x800 --scale "$SCALE" --theme light

echo "Dumping element tree..."
"$BIN" --tree "$EVIDENCE_DIR/tree.json"

echo ""
echo "Output:"
echo "  Dark:  $EVIDENCE_DIR/${FRAME_ID}-rune-dark.png"
echo "  Light: $EVIDENCE_DIR/${FRAME_ID}-rune-light.png"
echo "  Tree:  $EVIDENCE_DIR/tree.json"
echo ""
echo "Next: Read the PNGs, compare with the board, fix code, re-run."
