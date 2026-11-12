#!/usr/bin/env bash
# ci-retry-flaky-tests.sh — retry listed flaky tests in CI (task 2448).
#
# Usage:
#   ci-retry-flaky-tests.sh <cargo-test-log> <allow-list> [--crate-dir <dir>] [--summary <file>] [--self-test]
#
# After the first `cargo test` run, call this script with its log and the allow-list.
# If the log shows only failures that are on the allow-list, the failing tests are rerun
# up to 2 more times (--exact, one by one). If ANY failure is NOT on the list, the script
# exits 1 immediately. If every retried test passes, the script exits 0 and prints a
# summary line. If a listed test fails 3 times total (the original + 2 retries), exit 1.
#
# The allow-list (scripts/ci-flaky-windows-cf-tests.txt) is one test path per line; lines
# starting with # and blank lines are ignored. The test path is everything before the first
# whitespace on a non-comment line.
#
# --self-test: run the built-in fixture tests and exit.
set -euo pipefail

# ── Parse arguments ──────────────────────────────────────────────────────────────────
log=""
allowlist=""
crate_dir="."
summary=""
self_test=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --crate-dir) crate_dir="$2"; shift 2 ;;
    --summary)   summary="$2";   shift 2 ;;
    --self-test) self_test=true;  shift ;;
    *)
      if [[ -z "$log" ]]; then
        log="$1"
      elif [[ -z "$allowlist" ]]; then
        allowlist="$1"
      else
        echo "ci-retry-flaky-tests: unknown argument: $1" >&2; exit 2
      fi
      shift ;;
  esac
done

# ── Helpers ──────────────────────────────────────────────────────────────────────────

# Parse the allow-list into an associative array.
declare -A allowed
load_allowlist() {
  local file="$1"
  allowed=()
  while IFS= read -r raw || [[ -n "$raw" ]]; do
    local line="${raw%%#*}"
    line="${line## }"
    line="${line%% *}"      # first word before whitespace
    line="${line## }"
    [[ -z "$line" ]] && continue
    allowed["$line"]=1
  done < "$file"
}

# Extract failed test names from a cargo test log.
extract_failures() {
  local logfile="$1"
  local in_summary=false
  local saw_first_failures=false
  while IFS= read -r raw; do
    local line
    line="$(echo "$raw" | sed 's/\x1b\[[0-9;]*m//g')"   # strip ANSI
    line="${line#"${line%%[![:space:]]*}"}"                # ltrim
    line="${line%"${line##*[![:space:]]}"}"                # rtrim
    if [[ "$line" == "failures:" ]]; then
      if $saw_first_failures; then
        in_summary=true
      else
        saw_first_failures=true
      fi
      continue
    fi
    if $in_summary; then
      if [[ "$line" =~ ^test\ result: ]]; then
        # Reset for the next binary in the same log; do not stop here.
        in_summary=false
        saw_first_failures=false
        continue
      fi
      [[ -z "$line" ]] && continue
      echo "$line"
    fi
  done < "$logfile"
}

# ── The retry runner (replaceable by a stub in self-test) ────────────────────────────

# Run one test retry. Override RETRY_RUNNER in tests to stub cargo.
# Arguments: <crate_dir> <test_name>
# Returns: 0 if the test passed, 1 otherwise.
retry_one_test() {
  local dir="$1" name="$2"
  (cd "$dir" && cargo test --locked -- --exact "$name") 2>&1
}

# ── Classify failures ────────────────────────────────────────────────────────────────

# Check whether every failure is on the allow-list.
# Sets: unlisted (array), to_retry (assoc array).
declare -A to_retry
unlisted=()
classify_failures() {
  local failures_text="$1"
  unlisted=()
  to_retry=()
  while IFS= read -r name; do
    [[ -z "$name" ]] && continue
    if [[ -z "${allowed[$name]+x}" ]]; then
      unlisted+=("$name")
    else
      to_retry["$name"]=1
    fi
  done <<< "$failures_text"
}

# ── Retry loop ───────────────────────────────────────────────────────────────────────

# Returns: 0 if all retried tests passed, 1 if any still fail.
# Sets: passed_on_retry (array), still_failing (array), summary_line (string).
passed_on_retry=()
still_failing=()
summary_line=""

run_retry_loop() {
  local dir="$1"
  local retried_count=${#to_retry[@]}
  passed_on_retry=()
  still_failing=()

  for test_name in "${!to_retry[@]}"; do
    local ok=false
    for attempt in 1 2; do
      echo "ci-retry-flaky-tests: retry $attempt/2 of $test_name"
      if retry_one_test "$dir" "$test_name"; then
        ok=true
        passed_on_retry+=("$test_name")
        break
      fi
    done
    if ! $ok; then
      still_failing+=("$test_name")
    fi
  done

  summary_line="flaky-retry: $retried_count test(s) retried, ${#passed_on_retry[@]} passed on retry: ${passed_on_retry[*]:-none}"
  echo "$summary_line"

  [[ ${#still_failing[@]} -eq 0 ]]
}

# ── Self-test ────────────────────────────────────────────────────────────────────────

run_self_tests() {
  local tmp
  tmp="$(mktemp -d)"
  local pass=0
  local fail=0

  # Create a fixture allow-list
  cat > "$tmp/allow.txt" <<'ALLOWEOF'
# A comment
windows_cf::ld0105_native_tests::f2b::ld0105_f2b_native_a_buffered_robocopy_copies_a_file_served_in_session_pieces   # 2448
windows_cf::ld0105_native_tests::ld0105_p1_native_a_write_into_a_partly_downloaded_file_is_kept                       # 2448
ALLOWEOF

  # Fixture: a log with one listed failure
  cat > "$tmp/log-listed.txt" <<'LOGEOF'
Running unittests src/lib.rs (target/debug/deps/beebeeb_engine-abc.exe)
running 1810 tests
test something::passes ... ok
test windows_cf::ld0105_native_tests::f2b::ld0105_f2b_native_a_buffered_robocopy_copies_a_file_served_in_session_pieces ... FAILED

failures:

---- windows_cf::ld0105_native_tests::f2b::ld0105_f2b_native_a_buffered_robocopy_copies_a_file_served_in_session_pieces stdout ----
thread 'windows_cf::...' panicked at src/windows_cf/ld0105_f2b_native_tests.rs:385:9:
assertion failed

failures:
    windows_cf::ld0105_native_tests::f2b::ld0105_f2b_native_a_buffered_robocopy_copies_a_file_served_in_session_pieces

test result: FAILED. 1809 passed; 1 failed; 5 ignored; 0 measured; 0 filtered out; finished in 240.11s
LOGEOF

  # ── Case (a): a listed failure is extracted and classified as retryable ─────
  load_allowlist "$tmp/allow.txt"
  local failures
  failures="$(extract_failures "$tmp/log-listed.txt")"
  classify_failures "$failures"

  if [[ ${#to_retry[@]} -eq 1 && ${#unlisted[@]} -eq 0 ]]; then
    echo "PASS case (a): a listed failure is extracted and classified for retry"
    pass=$((pass + 1))
  else
    echo "FAIL case (a): expected 1 retryable / 0 unlisted, got ${#to_retry[@]} / ${#unlisted[@]}"
    fail=$((fail + 1))
  fi

  # ── Case (b): an unlisted failure fails immediately ────────────────────────
  cat > "$tmp/log-unlisted.txt" <<'LOGEOF'
Running unittests src/lib.rs (target/debug/deps/beebeeb_engine-abc.exe)
running 1810 tests
test some_other::test ... FAILED

failures:

---- some_other::test stdout ----
thread 'some_other::test' panicked at ...

failures:
    some_other::test

test result: FAILED. 1809 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 100.00s
LOGEOF

  load_allowlist "$tmp/allow.txt"
  failures="$(extract_failures "$tmp/log-unlisted.txt")"
  classify_failures "$failures"

  if [[ ${#unlisted[@]} -eq 1 && ${#to_retry[@]} -eq 0 ]]; then
    echo "PASS case (b): an unlisted failure fails immediately (no retry)"
    pass=$((pass + 1))
  else
    echo "FAIL case (b): expected 0 retryable / 1 unlisted, got ${#to_retry[@]} / ${#unlisted[@]}"
    fail=$((fail + 1))
  fi

  # ── Case (c): a mix of listed + unlisted fails immediately ─────────────────
  cat > "$tmp/log-mix.txt" <<'LOGEOF'
Running unittests src/lib.rs (target/debug/deps/beebeeb_engine-abc.exe)
running 1810 tests
test windows_cf::ld0105_native_tests::f2b::ld0105_f2b_native_a_buffered_robocopy_copies_a_file_served_in_session_pieces ... FAILED
test some_other::real_bug ... FAILED

failures:

---- windows_cf::ld0105_native_tests::f2b::ld0105_f2b_native_a_buffered_robocopy_copies_a_file_served_in_session_pieces stdout ----
---- some_other::real_bug stdout ----

failures:
    windows_cf::ld0105_native_tests::f2b::ld0105_f2b_native_a_buffered_robocopy_copies_a_file_served_in_session_pieces
    some_other::real_bug

test result: FAILED. 1808 passed; 2 failed; 5 ignored; 0 measured; 0 filtered out; finished in 200.00s
LOGEOF

  load_allowlist "$tmp/allow.txt"
  failures="$(extract_failures "$tmp/log-mix.txt")"
  classify_failures "$failures"

  if [[ ${#unlisted[@]} -eq 1 && ${#to_retry[@]} -eq 1 ]]; then
    echo "PASS case (c): a mix of listed + unlisted fails immediately"
    pass=$((pass + 1))
  else
    echo "FAIL case (c): expected 1 retryable / 1 unlisted, got ${#to_retry[@]} / ${#unlisted[@]}"
    fail=$((fail + 1))
  fi

  # ── Case (d): fails once then passes on retry → green + summary line ───────
  # Stub: first call fails, second call passes.
  local stub_call_count=0
  retry_one_test() {
    stub_call_count=$((stub_call_count + 1))
    if [[ $stub_call_count -le 1 ]]; then return 1; fi  # first retry fails
    return 0  # second retry passes
  }

  load_allowlist "$tmp/allow.txt"
  failures="$(extract_failures "$tmp/log-listed.txt")"
  classify_failures "$failures"
  stub_call_count=0
  if run_retry_loop "$tmp" && [[ "$summary_line" == *"1 passed on retry"* ]]; then
    echo "PASS case (d): fails once then passes → green with summary line"
    pass=$((pass + 1))
  else
    echo "FAIL case (d): expected green + summary, got: $summary_line"
    fail=$((fail + 1))
  fi

  # ── Case (e): fails 3 times → red ─────────────────────────────────────────
  retry_one_test() { return 1; }  # always fails

  load_allowlist "$tmp/allow.txt"
  failures="$(extract_failures "$tmp/log-listed.txt")"
  classify_failures "$failures"
  local loop_exit=0
  run_retry_loop "$tmp" || loop_exit=$?
  if [[ $loop_exit -ne 0 && ${#still_failing[@]} -eq 1 ]]; then
    echo "PASS case (e): fails 3 times → red (still_failing=${#still_failing[@]})"
    pass=$((pass + 1))
  else
    echo "FAIL case (e): expected red, got exit $loop_exit still_failing=${#still_failing[@]}"
    fail=$((fail + 1))
  fi

  # ── Case (f): two listed, one passes on retry, other still fails → red ─────
  cat > "$tmp/log-two.txt" <<'LOGEOF'
Running unittests src/lib.rs (target/debug/deps/beebeeb_engine-abc.exe)
running 1810 tests
test windows_cf::ld0105_native_tests::f2b::ld0105_f2b_native_a_buffered_robocopy_copies_a_file_served_in_session_pieces ... FAILED
test windows_cf::ld0105_native_tests::ld0105_p1_native_a_write_into_a_partly_downloaded_file_is_kept ... FAILED

failures:

---- windows_cf::ld0105_native_tests::f2b::ld0105_f2b_native_a_buffered_robocopy_copies_a_file_served_in_session_pieces stdout ----
---- windows_cf::ld0105_native_tests::ld0105_p1_native_a_write_into_a_partly_downloaded_file_is_kept stdout ----

failures:
    windows_cf::ld0105_native_tests::f2b::ld0105_f2b_native_a_buffered_robocopy_copies_a_file_served_in_session_pieces
    windows_cf::ld0105_native_tests::ld0105_p1_native_a_write_into_a_partly_downloaded_file_is_kept

test result: FAILED. 1808 passed; 2 failed; 5 ignored; 0 measured; 0 filtered out; finished in 240.11s
LOGEOF

  # Stub: the f2b test passes on first retry, p1 always fails.
  retry_one_test() {
    local name="$2"
    if [[ "$name" == *f2b* ]]; then return 0; fi
    return 1  # p1 always fails
  }

  load_allowlist "$tmp/allow.txt"
  failures="$(extract_failures "$tmp/log-two.txt")"
  classify_failures "$failures"
  loop_exit=0
  run_retry_loop "$tmp" || loop_exit=$?
  if [[ $loop_exit -ne 0 && ${#passed_on_retry[@]} -eq 1 && ${#still_failing[@]} -eq 1 ]]; then
    echo "PASS case (f): one passes, another still fails → red"
    pass=$((pass + 1))
  else
    echo "FAIL case (f): expected 1 passed + 1 still failing + red, got p=${#passed_on_retry[@]} f=${#still_failing[@]} exit=$loop_exit"
    fail=$((fail + 1))
  fi

  # ── Case (g): a passing log needs no retry ─────────────────────────────────
  cat > "$tmp/log-pass.txt" <<'LOGEOF'
Running unittests src/lib.rs (target/debug/deps/beebeeb_engine-abc.exe)
running 1810 tests
test something::passes ... ok

test result: ok. 1810 passed; 0 failed; 5 ignored; 0 measured; 0 filtered out; finished in 200.00s
LOGEOF

  failures="$(extract_failures "$tmp/log-pass.txt")"
  if [[ -z "$failures" ]]; then
    echo "PASS case (g): a passing log has no failures to retry"
    pass=$((pass + 1))
  else
    echo "FAIL case (g): expected no failures, got: [$failures]"
    fail=$((fail + 1))
  fi

  # ── Case (h): two binaries — binary 1 on listed, binary 2 on unlisted → both visible ──
  cat > "$tmp/log-two-bin-real.txt" <<'LOGEOF'
Running unittests src/lib.rs (target/debug/deps/beebeeb_engine-abc.exe)
running 1810 tests
test windows_cf::ld0105_native_tests::f2b::ld0105_f2b_native_a_buffered_robocopy_copies_a_file_served_in_session_pieces ... FAILED

failures:

---- windows_cf::ld0105_native_tests::f2b::ld0105_f2b_native_a_buffered_robocopy_copies_a_file_served_in_session_pieces stdout ----
thread 'windows_cf::...' panicked at src/windows_cf/ld0105_f2b_native_tests.rs:385:9

failures:
    windows_cf::ld0105_native_tests::f2b::ld0105_f2b_native_a_buffered_robocopy_copies_a_file_served_in_session_pieces

test result: FAILED. 1809 passed; 1 failed; 5 ignored; 0 measured; 0 filtered out; finished in 240.11s

Running tests/native_drive.rs (target/debug/deps/beebeeb_drive-def.exe)
running 500 tests
test drive::some_real_bug ... FAILED

failures:

---- drive::some_real_bug stdout ----
thread 'drive::some_real_bug' panicked at tests/native_drive.rs:42:5

failures:
    drive::some_real_bug

test result: FAILED. 499 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 100.00s
LOGEOF

  load_allowlist "$tmp/allow.txt"
  failures="$(extract_failures "$tmp/log-two-bin-real.txt")"
  classify_failures "$failures"
  if [[ ${#unlisted[@]} -eq 1 && ${#to_retry[@]} -eq 1 ]]; then
    echo "PASS case (h): two binaries — binary 2's unlisted failure is not hidden"
    pass=$((pass + 1))
  else
    echo "FAIL case (h): expected 1 retryable / 1 unlisted, got ${#to_retry[@]} / ${#unlisted[@]}"
    fail=$((fail + 1))
  fi

  # ── Case (i): two binaries both on listed tests → retried, both pass ──────────────
  cat > "$tmp/log-two-bin-listed.txt" <<'LOGEOF'
Running unittests src/lib.rs (target/debug/deps/beebeeb_engine-abc.exe)
running 1810 tests
test windows_cf::ld0105_native_tests::f2b::ld0105_f2b_native_a_buffered_robocopy_copies_a_file_served_in_session_pieces ... FAILED

failures:

---- windows_cf::ld0105_native_tests::f2b::ld0105_f2b_native_a_buffered_robocopy_copies_a_file_served_in_session_pieces stdout ----

failures:
    windows_cf::ld0105_native_tests::f2b::ld0105_f2b_native_a_buffered_robocopy_copies_a_file_served_in_session_pieces

test result: FAILED. 1809 passed; 1 failed; 5 ignored; 0 measured; 0 filtered out; finished in 240.11s

Running tests/native_drive.rs (target/debug/deps/beebeeb_drive-def.exe)
running 500 tests
test windows_cf::ld0105_native_tests::ld0105_p1_native_a_write_into_a_partly_downloaded_file_is_kept ... FAILED

failures:

---- windows_cf::ld0105_native_tests::ld0105_p1_native_a_write_into_a_partly_downloaded_file_is_kept stdout ----

failures:
    windows_cf::ld0105_native_tests::ld0105_p1_native_a_write_into_a_partly_downloaded_file_is_kept

test result: FAILED. 499 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 100.00s
LOGEOF

  retry_one_test() { return 0; }  # always passes on first retry

  load_allowlist "$tmp/allow.txt"
  failures="$(extract_failures "$tmp/log-two-bin-listed.txt")"
  classify_failures "$failures"
  local iloop_exit=0
  run_retry_loop "$tmp" || iloop_exit=$?
  if [[ $iloop_exit -eq 0 && "$summary_line" == *"2 test(s) retried"* ]]; then
    echo "PASS case (i): two binaries both on listed tests → retried, both pass"
    pass=$((pass + 1))
  else
    echo "FAIL case (i): expected 2 retried / 2 passed, got: $summary_line unlisted=${#unlisted[@]}"
    fail=$((fail + 1))
  fi

  # Restore real retry_one_test for production use.
  retry_one_test() {
    local dir="$1" name="$2"
    (cd "$dir" && cargo test --locked -- --exact "$name") 2>&1
  }

  echo "ci-retry-flaky-tests self-test: $pass passed; $fail failed"
  rm -rf "$tmp"
  [[ $fail -eq 0 ]] || exit 1
}

if $self_test; then
  run_self_tests
  exit 0
fi

# ── Main: retry logic ───────────────────────────────────────────────────────────────

if [[ -z "$log" || -z "$allowlist" ]]; then
  echo "Usage: ci-retry-flaky-tests.sh <cargo-test-log> <allow-list> [--crate-dir <dir>] [--summary <file>]" >&2
  exit 2
fi

if [[ ! -f "$log" ]]; then
  echo "ci-retry-flaky-tests: log file not found: $log" >&2
  exit 2
fi
if [[ ! -f "$allowlist" ]]; then
  echo "ci-retry-flaky-tests: allow-list not found: $allowlist" >&2
  exit 2
fi

load_allowlist "$allowlist"

failures="$(extract_failures "$log")"

if [[ -z "$failures" ]]; then
  # No failures — nothing to retry.
  exit 0
fi

classify_failures "$failures"

if [[ ${#unlisted[@]} -gt 0 ]]; then
  echo "ci-retry-flaky-tests: ${#unlisted[@]} failure(s) NOT on the allow-list — no retry, failing now:" >&2
  for name in "${unlisted[@]}"; do
    echo "  $name" >&2
  done
  exit 1
fi

echo "ci-retry-flaky-tests: ${#to_retry[@]} listed test(s) failed; retrying each up to 2 more times."

if run_retry_loop "$crate_dir"; then
  if [[ -n "$summary" ]]; then
    echo "$summary_line" >> "$summary"
  fi
  exit 0
else
  echo "ci-retry-flaky-tests: ${#still_failing[@]} test(s) still failing after retries:" >&2
  for name in "${still_failing[@]}"; do
    echo "  $name" >&2
  done
  if [[ -n "$summary" ]]; then
    echo "$summary_line" >> "$summary"
  fi
  exit 1
fi
