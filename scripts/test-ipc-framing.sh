#!/usr/bin/env bash
# Compile and run the File Provider IPC framing tests (task 1670 issue 3).
#
# BeebeebFileProvider/IPCFraming.swift is pure Foundation on purpose, so it can
# be compiled with BeebeebFileProviderTests/main.swift by plain `swiftc` (the
# extension target has no XCTest target). macOS only: it needs swiftc and
# Darwin. CI runs it in the "File Provider Swift (macOS)" job.
#
# The truth line is a COUNT ("ipc-framing: N passed, 0 failed"), asserted
# against EXPECTED_TESTS -- a run that executed fewer tests than declared, or
# printed no count, or exited non-zero, is RED. `--self-test` proves the
# assertion can go red (it needs no Swift compiler).
#
# Usage:
#   scripts/test-ipc-framing.sh              # compile + run + assert the count
#   scripts/test-ipc-framing.sh --self-test  # red-proof of the count guard

set -euo pipefail

EXPECTED_TESTS=94

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# assert_counts <log file> <exit code of the test binary>
assert_counts() {
  local log="$1" code="$2" lines passed failed
  if [[ "$code" != "0" ]]; then
    echo "FAIL: test binary exited $code" >&2
    return 1
  fi
  lines="$(grep -E '^ipc-framing: [0-9]+ passed, [0-9]+ failed$' "$log" || true)"
  if [[ "$(printf '%s' "$lines" | grep -c .)" != "1" ]]; then
    echo "FAIL: expected exactly one 'ipc-framing: N passed, M failed' line, found:" >&2
    printf '%s\n' "$lines" >&2
    return 1
  fi
  passed="$(printf '%s' "$lines" | sed -E 's/^ipc-framing: ([0-9]+) passed, ([0-9]+) failed$/\1/')"
  failed="$(printf '%s' "$lines" | sed -E 's/^ipc-framing: ([0-9]+) passed, ([0-9]+) failed$/\2/')"
  if [[ "$failed" != "0" ]]; then
    echo "FAIL: $failed test(s) failed" >&2
    return 1
  fi
  if [[ "$passed" != "$EXPECTED_TESTS" ]]; then
    echo "FAIL: $passed test(s) passed, expected $EXPECTED_TESTS" >&2
    return 1
  fi
  echo "OK: ipc-framing: $passed passed, 0 failed (expected $EXPECTED_TESTS)"
}

self_test() {
  local dir rc
  dir="$(mktemp -d)"
  trap 'rm -rf "$dir"' RETURN

  printf 'ok   x\nipc-framing: %s passed, 0 failed\n' "$EXPECTED_TESTS" > "$dir/good.log"
  assert_counts "$dir/good.log" 0 >/dev/null 2>&1 || { echo "self-test: a correct log was rejected" >&2; return 1; }

  printf 'ipc-framing: %s passed, 0 failed\n' "$((EXPECTED_TESTS - 1))" > "$dir/fewer.log"
  ! assert_counts "$dir/fewer.log" 0 >/dev/null 2>&1 || { echo "self-test: a run with too few tests was accepted" >&2; return 1; }

  printf 'ipc-framing: %s passed, 1 failed\n' "$EXPECTED_TESTS" > "$dir/failed.log"
  ! assert_counts "$dir/failed.log" 1 >/dev/null 2>&1 || { echo "self-test: a run with a failure was accepted" >&2; return 1; }
  ! assert_counts "$dir/failed.log" 0 >/dev/null 2>&1 || { echo "self-test: a failed count with exit 0 was accepted" >&2; return 1; }

  : > "$dir/empty.log"
  ! assert_counts "$dir/empty.log" 0 >/dev/null 2>&1 || { echo "self-test: a log with no count line was accepted" >&2; return 1; }

  printf 'ipc-framing: %s passed, 0 failed\n' "$EXPECTED_TESTS" > "$dir/nonzero.log"
  ! assert_counts "$dir/nonzero.log" 139 >/dev/null 2>&1 || { echo "self-test: a crashed binary with a good count was accepted" >&2; return 1; }

  echo "self-test OK: the count guard rejected 5 bad logs and accepted the good one"
  rc=0
  return $rc
}

if [[ "${1:-}" == "--self-test" ]]; then
  self_test
  exit $?
fi

cd "$ROOT_DIR"
OUT_DIR="${IPC_FRAMING_TEST_OUT:-src-tauri/target/ipc-framing-tests}"
mkdir -p "$OUT_DIR"
BIN="$OUT_DIR/ipc-framing-tests"
LOG="$OUT_DIR/run.log"

# task 1694: FileProviderItem.swift joins the compile line too. It imports
# FileProvider + UniformTypeIdentifiers, both system frameworks on macOS, so
# plain swiftc still works and the add-subitems capability mapping
# (BeebeebProviderItem flags -> NSFileProviderItemCapabilities) is now under
# test, not just the framing.
#
# task 1697: WorkingSetStore.swift (anchor codec, materialized-set filter,
# App Group state) and FileProviderExtension.swift + XPCBridge.swift (the
# enumerator's paging/anchor/change-filter decisions) join too, all pure
# Foundation/Darwin at heart.

xcrun swiftc \
  -o "$BIN" \
  BeebeebFileProvider/IPCFraming.swift \
  BeebeebFileProvider/FileProviderItem.swift \
  BeebeebFileProvider/WorkingSetStore.swift \
  BeebeebFileProvider/XPCBridge.swift \
  BeebeebFileProvider/FileProviderExtension.swift \
  BeebeebFileProvider/UploadStaging.swift \
  BeebeebFileProviderTests/main.swift

code=0
"$BIN" > "$LOG" 2>&1 || code=$?
cat "$LOG"
echo "$code" > "$OUT_DIR/run.exit"
assert_counts "$LOG" "$code"
