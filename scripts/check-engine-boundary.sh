#!/bin/sh
# The engine crate's two boundary rules, from cargo's own dependency graph (no build):
#   1. `crates/beebeeb-engine` has no `tauri` crate anywhere in its dependency tree.
#   2. its `test-support` feature (test helpers, injected failures) is on only through dev-dependencies: it is in
#      no normal or build edge of the desktop app or of the native app.
# Each count is taken from a tree that is first shown to be non-empty, and rule 2's instrument is shown to see the
# feature where it IS allowed (src-tauri's dev edges), so neither check can pass by reading nothing.
#   scripts/check-engine-boundary.sh         check this checkout (cargo --locked)
#   BB_BOUNDARY_UNLOCKED=1 ...               the same without --locked (to try a local mutant)
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
locked=--locked
[ "${BB_BOUNDARY_UNLOCKED:-}" = 1 ] && locked=
fail() { echo "engine boundary: FAILED: $1"; exit 1; }

tree=$(cd "$root/crates/beebeeb-engine" && cargo tree $locked -p beebeeb-engine --prefix none)
lines=$(printf '%s\n' "$tree" | grep -c . || true)
[ "$lines" -ge 20 ] || fail "the engine's dependency tree has only $lines lines (read the wrong thing?)"
tauri=$(printf '%s\n' "$tree" | grep -ci 'tauri' || true)
[ "$tauri" -eq 0 ] || fail "$tauri line(s) of the engine's dependency tree name tauri: $(printf '%s\n' "$tree" | grep -i tauri | sort -u | head -3 | tr '\n' ' ')"

needle='beebeeb-engine feature "test-support"'
dev=$(cd "$root/src-tauri" && cargo tree $locked -e normal,build,dev,features -p beebeeb-desktop | grep -c "$needle" || true)
[ "$dev" -ge 1 ] || fail "src-tauri's dev edges do not show test-support, so this check cannot see the feature"
for crate in src-tauri:beebeeb-desktop native/beebeeb-drive-rune:beebeeb-drive-rune; do
  dir=${crate%%:*}; package=${crate##*:}
  graph=$(cd "$root/$dir" && cargo tree $locked -e normal,build,features -p "$package")
  printf '%s\n' "$graph" | grep -q 'beebeeb-engine' || fail "$dir does not depend on the engine crate (read the wrong thing?)"
  on=$(printf '%s\n' "$graph" | grep -c "$needle" || true)
  [ "$on" -eq 0 ] || fail "$dir turns on the engine's test-support feature outside dev-dependencies ($on edge(s))"
done
echo "engine boundary: ok ($lines tree lines, 0 name tauri; test-support on $dev dev edge(s) of src-tauri, 0 normal or build edges of src-tauri and native/beebeeb-drive-rune)"
