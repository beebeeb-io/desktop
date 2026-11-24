#!/bin/sh
# The engine crate's boundary rules (and the updater's test doubles), from cargo's own dependency graph (no build):
#   1. `crates/beebeeb-engine` has no `tauri` crate anywhere in its dependency tree.
#   2. its `test-support` feature (test helpers, injected failures) is on only through dev-dependencies: it is in
#      no normal or build edge of the native app.
# Each count is taken from a tree that is first shown to be non-empty, and rule 2's instrument is shown to see the
# feature where it IS allowed (the native app's dev edges), so neither check can pass by reading nothing.
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

native="$root/native/beebeeb-drive-rune"
needle='beebeeb-engine feature "test-support"'
dev_graph=$(cd "$native" && cargo tree $locked -e normal,build,dev,features -p beebeeb-drive-rune)
dev=$(printf '%s\n' "$dev_graph" | grep -c "$needle" || true)
[ "$dev" -ge 1 ] || fail "native/beebeeb-drive-rune's dev edges do not show test-support, so this check cannot see the feature"
graph=$(cd "$native" && cargo tree $locked -e normal,build,features -p beebeeb-drive-rune)
printf '%s\n' "$graph" | grep -q 'beebeeb-engine' || fail "native/beebeeb-drive-rune does not depend on the engine crate (read the wrong thing?)"
on=$(printf '%s\n' "$graph" | grep -c "$needle" || true)
[ "$on" -eq 0 ] || fail "native/beebeeb-drive-rune turns on the engine's test-support feature outside dev-dependencies ($on edge(s))"
# Rule 3 (task 2488): the engine's `direct-state-db` feature makes the state database's raw constructors `pub`. The
# native app must not have it on any normal or build edge: it reaches the local state through the engine's
# account-bound handle, and a direct open does not compile there. The instrument is shown to see the feature where it
# IS on (the native app's dev edges: `test-support` implies `direct-state-db`).
direct='beebeeb-engine feature "direct-state-db"'
dev_direct=$(printf '%s\n' "$dev_graph" | grep -c "$direct" || true)
[ "$dev_direct" -ge 1 ] || fail "native/beebeeb-drive-rune's dev edges do not show direct-state-db, so this check cannot see the feature"
native_direct=$(printf '%s\n' "$graph" | grep -c "$direct" || true)
[ "$native_direct" -eq 0 ] || fail "native/beebeeb-drive-rune turns on the engine's direct-state-db feature ($native_direct edge(s)): its readers must use the account-bound handle"
# Rule 4 (#437 review): the updater's `testkit` feature (test doubles and the `minisign` signer) is on only through
# the native app's dev-dependencies, on every target (`--target all`: the Windows-only `net` edge included). The
# instrument is shown to see the feature where it IS on (the native app's dev edges).
testkit='beebeeb-updater feature "testkit"'
testkit_dev=$(cd "$root/native/beebeeb-drive-rune" && cargo tree $locked --target all -e normal,build,dev,features -p beebeeb-drive-rune | grep -c "$testkit" || true)
[ "$testkit_dev" -ge 1 ] || fail "native/beebeeb-drive-rune's dev edges do not show the updater's testkit, so this check cannot see the feature"
testkit_on=$(cd "$root/native/beebeeb-drive-rune" && cargo tree $locked --target all -e normal,build,features -p beebeeb-drive-rune | grep -c "$testkit" || true)
[ "$testkit_on" -eq 0 ] || fail "native/beebeeb-drive-rune turns on the updater's testkit feature outside dev-dependencies ($testkit_on edge(s))"
echo "engine boundary: ok (updater testkit on $testkit_dev dev edge(s) of native/beebeeb-drive-rune, 0 normal or build edges)"
echo "engine boundary: ok (direct-state-db on $dev_direct dev edge(s) of native/beebeeb-drive-rune, 0 normal or build edges)"
echo "engine boundary: ok ($lines tree lines, 0 name tauri; test-support on $dev dev edge(s) of native/beebeeb-drive-rune, 0 normal or build edges)"
