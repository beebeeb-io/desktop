#!/usr/bin/env bash
# compare.sh <frame-id> — compare the Rune render against the board golden.
#
# Captures the Rune headless render, then runs compare.py for the pixel diff.
# Exit code: 0 if below threshold, 1 if above, 2 on error.
#
# Usage: scripts/compare.sh DW-0401

set -euo pipefail

FRAME_ID="${1:?Usage: compare.sh <frame-id>}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CRATE_DIR="$(dirname "$SCRIPT_DIR")"
EVIDENCE_DIR="${EVIDENCE_DIR:-/home/guus/code/beebeeb.io/.claude/tasks/verification-evidence/2445/native}"
OUT_DIR="$EVIDENCE_DIR/$FRAME_ID"
GOLDEN_DIR="$CRATE_DIR/golden"

mkdir -p "$OUT_DIR" "$GOLDEN_DIR"

# ---------------------------------------------------------------------------
# 1. Golden image
# ---------------------------------------------------------------------------
GOLDEN="$GOLDEN_DIR/${FRAME_ID}-board.png"
if [[ ! -f "$GOLDEN" ]]; then
    echo "ERROR: golden image not found: $GOLDEN"
    echo "Capture it first with Playwright from the design board."
    exit 2
fi

# ---------------------------------------------------------------------------
# 2. Render the Rune frame
# ---------------------------------------------------------------------------
RUNE_PNG="$OUT_DIR/${FRAME_ID}-rune.png"
echo "Rendering Rune frame $FRAME_ID..."

source ~/code/bb-legion-native/lavapipe/env.sh 2>/dev/null || true
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/code/bb-worktrees/target-rune-spike}"
export CARGO_TARGET_DIR

cd "$CRATE_DIR"
/home/guus/code/beebeeb.io/scripts/coord/with-lock.sh cargo-build -- \
    cargo run --bin beebeeb-drive -- \
    --snap "$RUNE_PNG" --size 1280x800 --scale 2 --theme light 2>&1 | tail -3

if [[ ! -f "$RUNE_PNG" ]]; then
    echo "ERROR: Rune render failed, no output at $RUNE_PNG"
    exit 2
fi

# ---------------------------------------------------------------------------
# 3. Run the Python comparison
# ---------------------------------------------------------------------------
python3 "$SCRIPT_DIR/compare.py" "$GOLDEN" "$RUNE_PNG" "$OUT_DIR" "$FRAME_ID"
