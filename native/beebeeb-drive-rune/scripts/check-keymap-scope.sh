#!/usr/bin/env bash
# check-keymap-scope.sh [<frame-id> ...]
#
# A lane's additions to keymap.js must not change the golden of a frame that is another
# lane's. This recaptures each frame (default: DW-0401, the Files window) from the board with
# the keymap as it is now, light and dark, and compares the SET of exported keys with the
# committed golden's. Last line: `keymap scope: N frames checked, M differing`. Exit 1 when M > 0.
# Needs the network (the board's fonts), as every capture does.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE_DIR="$(dirname "$SCRIPT_DIR")"
[ $# -gt 0 ] || set -- DW-0401
OUT="$(mktemp -d)"
trap 'rm -r "$OUT"' EXIT
checked=0; differing=0
for frame in "$@"; do
    for dark in "" "--dark"; do
        stem="$frame"; [ -n "$dark" ] && stem="$frame-dark"
        [ -s "$CRATE_DIR/golden/$stem.json" ] || continue
        # shellcheck disable=SC2086
        # A capture that warns (Trash has a header cell with no words) still writes its export:
        # the export is what is compared; no export is a failure of this check.
        node "$SCRIPT_DIR/capture-golden.mjs" "$frame" $dark --out "$OUT" >/dev/null 2>&1 || true
        [ -s "$OUT/$stem.json" ] || { echo "$stem: the capture wrote no export"; checked=$((checked + 1)); differing=$((differing + 1)); continue; }
        checked=$((checked + 1))
        if ! python3 - "$CRATE_DIR/golden/$stem.json" "$OUT/$stem.json" "$stem" <<'PY'
import json, sys
keys = lambda p: set(json.load(open(p))["elements"])
was, now = keys(sys.argv[1]), keys(sys.argv[2])
if was != now:
    print(f"{sys.argv[3]}: {len(was)} keys committed, {len(now)} captured; new: {sorted(now - was)}; gone: {sorted(was - now)}")
    sys.exit(1)
print(f"{sys.argv[3]}: {len(now)} keys, the same set")
PY
        then differing=$((differing + 1)); fi
    done
done
echo "keymap scope: $checked frames checked, $differing differing"
[ "$checked" -gt 0 ] && [ "$differing" -eq 0 ]
