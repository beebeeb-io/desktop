#!/bin/sh
# D-P3-1: both crates and both locks pin the SAME core rev (LD-0113: feat/LD-0113-backup-rules 7e40bf2 until the v0.2.1 tag; was v0.2.0 = 3db0cc2).
set -eu
want=7e40bf245f604323b002261d682c6af623446841
got=$(grep -ho 'rev = "[0-9a-f]\{40\}"' src-tauri/Cargo.toml windows/Cargo.toml | sed 's/.*"\(.*\)"/\1/' | sort -u)
[ "$got" = "$want" ] || { echo "core pin mismatch: want $want, got ${got:-none}"; exit 1; }
grep -q "rev=$want#" src-tauri/Cargo.lock || { echo "src-tauri/Cargo.lock is not pinned at $want"; exit 1; }
grep -q "rev=$want#" windows/Cargo.lock || { echo "windows/Cargo.lock is not pinned at $want"; exit 1; }
echo "core pinned at $want"
