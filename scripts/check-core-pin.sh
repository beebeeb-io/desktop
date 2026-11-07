#!/bin/sh
# D-P3-1: both crates and both locks pin the SAME core rev (LD-0139: core v0.2.2 = 2d89adc; was v0.2.1 = a5413d2, v0.2.0 = 3db0cc2).
set -eu
want=2d89adc1e2e3b9c0b3e2d76c295cbd10df4812c3
got=$(grep -ho 'rev = "[0-9a-f]\{40\}"' src-tauri/Cargo.toml windows/Cargo.toml | sed 's/.*"\(.*\)"/\1/' | sort -u)
[ "$got" = "$want" ] || { echo "core pin mismatch: want $want, got ${got:-none}"; exit 1; }
grep -q "rev=$want#" src-tauri/Cargo.lock || { echo "src-tauri/Cargo.lock is not pinned at $want"; exit 1; }
grep -q "rev=$want#" windows/Cargo.lock || { echo "windows/Cargo.lock is not pinned at $want"; exit 1; }
echo "core pinned at $want"
