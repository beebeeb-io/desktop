#!/usr/bin/env bash
# spike(2176): compile SwapCore.swift with its harness and run it in a throwaway directory.
#   tests/run-core-tests.sh [work-dir]      (default: a new mktemp -d directory)
# Needs no signing identity, installs nothing, launches no app. Truth line:
#   swapcore-tests: N passed, 0 failed, K skipped
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK="${1:-$(mktemp -d)}"
mkdir -p "$WORK"
xcrun swiftc -target arm64-apple-macos14.0 -swift-version 5 -parse-as-library \
  -framework Foundation -framework Security \
  "$HERE/../BeebeebSwapProbe/SwapCore.swift" "$HERE/SwapCoreTests.swift" \
  -o "$WORK/swapcore-tests"
"$WORK/swapcore-tests" "$WORK"
