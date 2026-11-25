#!/usr/bin/env bash
# Runs the wall-clock budget tests listed in scripts/ci-timing-tests.txt for one OS (linux|windows), one by one,
# as ignored tests. Used by the non-blocking "Timing tests" job; exits 1 if any fails (the job is continue-on-error).
set -uo pipefail
os="${1:?usage: ci-run-timing-tests.sh linux|windows}"
root="$(cd "$(dirname "$0")/.." && pwd)"
failed=0; ran=0
while read -r want dir name; do
  case "$want" in ''|'#'*) continue ;; esac
  [ "$want" = "$os" ] || continue
  ran=$((ran + 1))
  echo "::group::$dir $name"
  (cd "$root/$dir" && cargo test --locked "$name" -- --ignored --test-threads=1) || { failed=$((failed + 1)); echo "::warning::timing test failed: $name"; }
  echo "::endgroup::"
done < "$root/scripts/ci-timing-tests.txt"
echo "timing tests: $ran run, $failed failed"
[ "$ran" -gt 0 ] && [ "$failed" -eq 0 ]
