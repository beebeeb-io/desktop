#!/bin/sh
# D-P3-1: both crates (src-tauri, windows) and both locks pin the SAME core rev, and no other one.
# Core v0.2.8 = f76ccfc (desktop v2, batch V028); was v0.2.5 = f9257d7 (LD-0139), v0.2.3 = df93011,
# v0.2.2 = 2d89adc, v0.2.1 = a5413d2, v0.2.0 = 3db0cc2.
# Self-test: `scripts/check-core-pin.sh --self-test` proves it refuses a split pin and a stray lock rev.
set -eu
want=f76ccfc32429d80b6c66ba02c44e9959c9afa93d

check() {
  root=$1
  got=$(grep -ho 'rev = "[0-9a-f]\{40\}"' "$root/src-tauri/Cargo.toml" "$root/windows/Cargo.toml" | sed 's/.*"\(.*\)"/\1/' | sort -u)
  [ "$got" = "$want" ] || { echo "core pin mismatch: want $want, got $(echo "${got:-none}" | tr '\n' ' ')"; return 1; }
  for crate in src-tauri windows; do
    n=$(grep -c "rev = \"$want\"" "$root/$crate/Cargo.toml")
    [ "$n" -ge 1 ] || { echo "$crate/Cargo.toml does not pin core at $want"; return 1; }
    lock="$root/$crate/Cargo.lock"
    grep -q "rev=$want#" "$lock" || { echo "$crate/Cargo.lock is not pinned at $want"; return 1; }
    stray=$(grep -o 'beebeeb-io/core?rev=[0-9a-f]\{40\}' "$lock" | sed 's/.*rev=//' | sort -u | grep -v "^$want\$" || true)
    [ -z "$stray" ] || { echo "$crate/Cargo.lock also pins core at $stray"; return 1; }
  done
  echo "core pinned at $want"
}

if [ "${1:-}" = "--self-test" ]; then
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  mk() {
    mkdir -p "$tmp/$1/src-tauri" "$tmp/$1/windows"
    printf 'beebeeb-core = { git = "x", rev = "%s" }\n' "$2" > "$tmp/$1/src-tauri/Cargo.toml"
    printf 'beebeeb-core = { git = "x", rev = "%s" }\n' "$3" > "$tmp/$1/windows/Cargo.toml"
    printf 'source = "git+https://github.com/beebeeb-io/core?rev=%s#%s"\n' "$2" "$2" > "$tmp/$1/src-tauri/Cargo.lock"
    printf 'source = "git+https://github.com/beebeeb-io/core?rev=%s#%s"\n' "$3" "$3" > "$tmp/$1/windows/Cargo.lock"
  }
  other=f9257d7362376e77b9a04fc0a014d0d2b8a84987
  mk ok "$want" "$want"
  mk split "$want" "$other"
  mk stray "$want" "$want"
  printf 'source = "git+https://github.com/beebeeb-io/core?rev=%s#%s"\n' "$other" "$other" >> "$tmp/stray/windows/Cargo.lock"
  pass=0
  check "$tmp/ok" > /dev/null && pass=$((pass + 1))
  check "$tmp/split" > /dev/null || pass=$((pass + 1))
  check "$tmp/stray" > /dev/null || pass=$((pass + 1))
  [ "$pass" -eq 3 ] || { echo "self-test: $pass of 3 cases behaved"; exit 1; }
  echo "self-test: 3 of 3 cases behaved (accepts the pin, refuses a split pin, refuses a stray lock rev)"
  exit 0
fi

check "$(cd "$(dirname "$0")/.." && pwd)"
