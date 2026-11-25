#!/bin/sh
# D-P3-1: every crate that reaches core (crates/beebeeb-engine, and the native Rune app through its lock) pins the
# SAME core rev, and no other one.
# Core 9b3219c (folder digest v1 #129); was a5176f9 (kept-key rule #127, local seal #120); was 7d5b5f9 (0.3.0 unreleased: chunk format v2, the two-level lock and its app lock, adopted on v2-native; a
# release waits for task 2376); was abf4e5d (the two-level lock before round 3 and app lock), 56cd54f (core main
# before the lock),
# v0.2.8 = f76ccfc (desktop v2, batch V028), v0.2.5 = f9257d7 (LD-0139), v0.2.3 = df93011,
# v0.2.2 = 2d89adc, v0.2.1 = a5413d2, v0.2.0 = 3db0cc2.
#
#   scripts/check-core-pin.sh                 check this checkout
#   scripts/check-core-pin.sh --root <dir>    check another tree (the self-test's)
#   scripts/check-core-pin.sh --self-test     prove it accepts the pin and refuses each way it can break,
#                                             each case run as its own process, as CI runs it
# Every refusal prints one line and exits 1; a pass prints one line and exits 0.
set -eu
want=9b3219cf415c553d50cfb8a93f886c2bd0d06332

refuse() { echo "$1"; exit 1; }

# Where core is pinned. A crate with a core dependency of its own pins it in its manifest and its lock; a crate that
# reaches core only through another crate (the native Rune app, through the engine crate) pins it in its lock alone.
manifest_crates="crates/beebeeb-engine"
lock_only_crates="native/beebeeb-drive-rune"

check_lock() { # <root> <crate>
  lock="$1/$2/Cargo.lock"
  [ -f "$lock" ] || refuse "$2/Cargo.lock is missing"
  grep -q "rev=$want#" "$lock" || refuse "$2/Cargo.lock is not pinned at $want"
  stray=$(grep -o 'beebeeb-io/core?rev=[0-9a-f]\{40\}' "$lock" | sed 's/.*rev=//' | sort -u | grep -v "^$want\$" || true)
  [ -z "$stray" ] || refuse "$2/Cargo.lock also pins core at $stray"
}

check() {
  root=$1
  for crate in $manifest_crates; do
    manifest="$root/$crate/Cargo.toml"
    [ -f "$manifest" ] || refuse "$crate/Cargo.toml is missing"
    core_lines=$(grep -c 'github.com/beebeeb-io/core' "$manifest" || true)
    [ "$core_lines" -ge 1 ] || refuse "$crate/Cargo.toml has no core dependency"
    pinned=$(grep 'github.com/beebeeb-io/core' "$manifest" | grep -c "rev = \"$want\"" || true)
    [ "$pinned" = "$core_lines" ] || refuse "$crate/Cargo.toml: $pinned of $core_lines core dependencies pin $want"
    check_lock "$root" "$crate"
  done
  for crate in $lock_only_crates; do
    [ -f "$root/$crate/Cargo.toml" ] || refuse "$crate/Cargo.toml is missing"
    check_lock "$root" "$crate"
  done
  echo "core pinned at $want ($manifest_crates: manifests and locks; $lock_only_crates: lock)"
}

self_test() {
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  other=f9257d7362376e77b9a04fc0a014d0d2b8a84987
  dep() { printf 'beebeeb-core = { git = "https://github.com/beebeeb-io/core", rev = "%s" }' "$1"; }
  lockline() { printf 'source = "git+https://github.com/beebeeb-io/core?rev=%s#%s"\n' "$1" "$1"; }
  mk() { # <case>: a tree where every pin is right; a case then breaks one thing
    for crate in $manifest_crates $lock_only_crates; do
      mkdir -p "$tmp/$1/$crate"
      lockline "$want" > "$tmp/$1/$crate/Cargo.lock"
    done
    for crate in $manifest_crates; do dep "$want" > "$tmp/$1/$crate/Cargo.toml"; done
    for crate in $lock_only_crates; do echo 'beebeeb-engine = { path = "../../crates/beebeeb-engine" }' > "$tmp/$1/$crate/Cargo.toml"; done
  }
  cases="ok split two-deps-one-split no-rev no-core stale-lock stray missing no-lock native-stale-lock native-stray native-no-lock"
  for case in $cases; do mk "$case"; done
  engine="crates/beebeeb-engine"
  dep "$other" > "$tmp/split/$engine/Cargo.toml"
  { dep "$want"; echo; dep "$other"; echo; } > "$tmp/two-deps-one-split/$engine/Cargo.toml"
  echo 'beebeeb-core = { git = "https://github.com/beebeeb-io/core", branch = "main" }' > "$tmp/no-rev/$engine/Cargo.toml"
  echo 'tokio = "1"' > "$tmp/no-core/$engine/Cargo.toml"
  lockline "$other" > "$tmp/stale-lock/$engine/Cargo.lock"
  lockline "$other" >> "$tmp/stray/$engine/Cargo.lock"
  rm "$tmp/missing/$engine/Cargo.toml"
  rm "$tmp/no-lock/$engine/Cargo.lock"
  lockline "$other" > "$tmp/native-stale-lock/native/beebeeb-drive-rune/Cargo.lock"
  lockline "$other" >> "$tmp/native-stray/native/beebeeb-drive-rune/Cargo.lock"
  rm "$tmp/native-no-lock/native/beebeeb-drive-rune/Cargo.lock"
  behaved=0
  total=0
  for case in $cases; do
    total=$((total + 1))
    set +e
    out=$(sh "$0" --root "$tmp/$case" 2>&1)
    rc=$?
    set -e
    if [ "$case" = ok ]; then
      [ "$rc" -eq 0 ] && behaved=$((behaved + 1))
    elif [ "$rc" -eq 1 ] && [ -n "$out" ]; then
      behaved=$((behaved + 1))
      echo "  refuses $case: $out"
    else
      echo "self-test: case $case exited $rc with '${out}'"
    fi
  done
  [ "$behaved" -eq "$total" ] || { echo "self-test: $behaved of $total cases behaved"; exit 1; }
  echo "self-test: $behaved of $total cases behaved (accepts the pin; refuses in the engine crate a split pin, one of two core dependencies split, a branch pin, a missing core dependency, a stale lock, a stray lock rev, a missing manifest and a missing lock; refuses a stale, stray or missing lock in the native crate; each with a message)"
}

case "${1:-}" in
  --self-test) self_test ;;
  --root) check "$2" ;;
  "") check "$(cd "$(dirname "$0")/.." && pwd)" ;;
  *) echo "usage: $0 [--self-test | --root <dir>]"; exit 2 ;;
esac
