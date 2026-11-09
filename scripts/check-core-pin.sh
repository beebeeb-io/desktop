#!/bin/sh
# D-P3-1: both crates (src-tauri, windows) and both locks pin the SAME core rev, and no other one.
# Core v0.2.8 = f76ccfc (desktop v2, batch V028); was v0.2.5 = f9257d7 (LD-0139), v0.2.3 = df93011,
# v0.2.2 = 2d89adc, v0.2.1 = a5413d2, v0.2.0 = 3db0cc2.
#
#   scripts/check-core-pin.sh                 check this checkout
#   scripts/check-core-pin.sh --root <dir>    check another tree (the self-test's)
#   scripts/check-core-pin.sh --self-test     prove it accepts the pin and refuses each way it can break,
#                                             each case run as its own process, as CI runs it
# Every refusal prints one line and exits 1; a pass prints one line and exits 0.
set -eu
want=f76ccfc32429d80b6c66ba02c44e9959c9afa93d

refuse() { echo "$1"; exit 1; }

check() {
  root=$1
  for crate in src-tauri windows; do
    manifest="$root/$crate/Cargo.toml"
    lock="$root/$crate/Cargo.lock"
    [ -f "$manifest" ] || refuse "$crate/Cargo.toml is missing"
    [ -f "$lock" ] || refuse "$crate/Cargo.lock is missing"
    core_lines=$(grep -c 'github.com/beebeeb-io/core' "$manifest" || true)
    [ "$core_lines" -ge 1 ] || refuse "$crate/Cargo.toml has no core dependency"
    pinned=$(grep 'github.com/beebeeb-io/core' "$manifest" | grep -c "rev = \"$want\"" || true)
    [ "$pinned" = "$core_lines" ] || refuse "$crate/Cargo.toml: $pinned of $core_lines core dependencies pin $want"
    grep -q "rev=$want#" "$lock" || refuse "$crate/Cargo.lock is not pinned at $want"
    stray=$(grep -o 'beebeeb-io/core?rev=[0-9a-f]\{40\}' "$lock" | sed 's/.*rev=//' | sort -u | grep -v "^$want\$" || true)
    [ -z "$stray" ] || refuse "$crate/Cargo.lock also pins core at $stray"
  done
  echo "core pinned at $want (src-tauri and windows, manifests and locks)"
}

self_test() {
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  other=f9257d7362376e77b9a04fc0a014d0d2b8a84987
  mk() { # <case> <src-tauri dep line> <windows dep line> <src-tauri lock rev> <windows lock rev>
    mkdir -p "$tmp/$1/src-tauri" "$tmp/$1/windows"
    printf '%s\n' "$2" > "$tmp/$1/src-tauri/Cargo.toml"
    printf '%s\n' "$3" > "$tmp/$1/windows/Cargo.toml"
    printf 'source = "git+https://github.com/beebeeb-io/core?rev=%s#%s"\n' "$4" "$4" > "$tmp/$1/src-tauri/Cargo.lock"
    printf 'source = "git+https://github.com/beebeeb-io/core?rev=%s#%s"\n' "$5" "$5" > "$tmp/$1/windows/Cargo.lock"
  }
  dep() { printf 'beebeeb-core = { git = "https://github.com/beebeeb-io/core", rev = "%s" }' "$1"; }
  mk ok "$(dep "$want")" "$(dep "$want")" "$want" "$want"
  mk split "$(dep "$want")" "$(dep "$other")" "$want" "$want"
  mk no-rev "$(dep "$want")" 'beebeeb-core = { git = "https://github.com/beebeeb-io/core", branch = "main" }' "$want" "$want"
  mk no-core "$(dep "$want")" 'tokio = "1"' "$want" "$want"
  mk stale-lock "$(dep "$want")" "$(dep "$want")" "$want" "$other"
  mk stray "$(dep "$want")" "$(dep "$want")" "$want" "$want"
  printf 'source = "git+https://github.com/beebeeb-io/core?rev=%s#%s"\n' "$other" "$other" >> "$tmp/stray/windows/Cargo.lock"
  behaved=0
  total=0
  for case in ok split no-rev no-core stale-lock stray; do
    total=$((total + 1))
    set +e
    out=$(sh "$0" --root "$tmp/$case" 2>&1)
    rc=$?
    set -e
    if [ "$case" = ok ]; then
      [ "$rc" -eq 0 ] && behaved=$((behaved + 1))
    elif [ "$rc" -eq 1 ] && [ -n "$out" ]; then
      behaved=$((behaved + 1))
    else
      echo "self-test: case $case exited $rc with '${out}'"
    fi
  done
  [ "$behaved" -eq "$total" ] || { echo "self-test: $behaved of $total cases behaved"; exit 1; }
  echo "self-test: $behaved of $total cases behaved (accepts the pin; refuses a split pin, a branch pin, a missing core dependency, a stale lock and a stray lock rev, each with a message)"
}

case "${1:-}" in
  --self-test) self_test ;;
  --root) check "$2" ;;
  "") check "$(cd "$(dirname "$0")/.." && pwd)" ;;
  *) echo "usage: $0 [--self-test | --root <dir>]"; exit 2 ;;
esac
