#!/usr/bin/env bash
# File Provider QA harness (task 1699, audit doc G15).
#
# One-command local health report for the Beebeeb File Provider stack. It
# wraps Apple's system tooling — NOT testingModes (that entitlement is
# irreversible and out of scope until Guus signs off):
#
#   1. pluginkit -m -p com.apple.fileprovider-nonui   -> is our extension registered?
#   2. fileproviderctl dump                           -> does the system see our domain(s)?
#   3. fileproviderctl evaluate <Beebeeb mount>       -> does the system accept our items?
#   4. fileproviderctl diagnose                       -> per-domain dump logs (informational)
#   5. BeebeebFileProviderCtl status                  -> our own domain check (if built)
#
# Exit code: 0 when nothing FAILed, 1 otherwise. SKIPs (no mount, ctl not
# built) do not fail the run — this script must run green on a healthy Mac
# even before the app is installed. `fileproviderctl diagnose` returns
# nonzero whenever ANY provider on the machine has issues (Proton, OneDrive,
# iCloud...), so its exit code is reported but never gates; the gate is
# whether it produced dump logs for a beebeeb domain.
#
# Environment overrides: FPCTL, PLUGINKIT, FP_BUNDLE_ID, FP_DOMAIN_MOUNT, FP_CTL.

set -uo pipefail

FPCTL="${FPCTL:-/usr/bin/fileproviderctl}"
PLUGINKIT="${PLUGINKIT:-/usr/bin/pluginkit}"
BUNDLE_ID="${FP_BUNDLE_ID:-io.beebeeb.app.FileProvider}"
CTL="${FP_CTL:-src-tauri/target/fileprovider/BeebeebFileProviderCtl}"

passed=0
failed=0
skipped=0

pass() { passed=$((passed + 1)); printf '[PASS] %s\n' "$1"; }
fail() { failed=$((failed + 1)); printf '[FAIL] %s\n' "$1"; }
skip() { skipped=$((skipped + 1)); printf '[SKIP] %s\n' "$1"; }
info() { printf '       %s\n' "$1"; }

work="$(mktemp -d "${TMPDIR:-/tmp}/qa-fileprovider.XXXXXX")"
cleanup() { rm -rf "$work"; }
trap cleanup EXIT

headline() { printf '\n== %s\n' "$1"; }

# --- 1. pluginkit: extension registered with the fileprovider-nonui point ---
headline "pluginkit registration"
if plug_out="$("$PLUGINKIT" -m -p com.apple.fileprovider-nonui 2>"$work/plug.err")"; then
  if grep -q "$BUNDLE_ID" <<<"$plug_out"; then
    pass "pluginkit registers $BUNDLE_ID"
    grep "$BUNDLE_ID" <<<"$plug_out" | sed 's/^/       /'
  else
    fail "pluginkit does not list $BUNDLE_ID (extension not registered?)"
  fi
else
  fail "pluginkit itself failed (rc=$?)"
fi

# --- 2. fileproviderctl dump: system view of the providers ---
headline "fileproviderctl dump"
if "$FPCTL" dump >"$work/dump.txt" 2>"$work/dump.err"; then
  lines="$(wc -l <"$work/dump.txt" | tr -d ' ')"
  if grep -q '^FP Version' "$work/dump.txt"; then
    pass "dump succeeded ($lines lines)"
    sections="$(grep -c '^io\.beebeeb\.' "$work/dump.txt" || true)"
    if [[ "$sections" -ge 1 ]]; then
      pass "system dump names $sections beebeeb provider section(s)"
      grep '^io\.beebeeb\.' "$work/dump.txt" | sed 's/^/       section: /'
    else
      fail "dump has no io.beebeeb.* provider section (domain not registered?)"
    fi
    if grep -q '^io\.beebeeb\.desktop\.FileProvider' "$work/dump.txt"; then
      info "note: legacy io.beebeeb.desktop.FileProvider still registered (pre-rename install; candidates for cleanup)"
    fi
  else
    fail "dump output missing 'FP Version' header"
  fi
else
  fail "fileproviderctl dump failed (rc=$?) — see $work is gone, rerun manually"
fi

# --- 3. fileproviderctl evaluate: system accepts an item in our mount ---
headline "fileproviderctl evaluate"
mount="${FP_DOMAIN_MOUNT:-}"
if [[ -z "$mount" ]]; then
  for d in "$HOME/Library/CloudStorage"/Beebeeb-*; do
    [[ -d "$d" ]] && mount="$d" && break
  done
fi
if [[ -z "$mount" ]]; then
  skip "no Beebeeb mount under ~/Library/CloudStorage (domain not installed on this Mac)"
elif [[ ! -d "$mount" ]]; then
  fail "FP_DOMAIN_MOUNT is not a directory: $mount"
else
  # NOTE: evaluate with an empty/degenerate path crashes fileproviderctl
  # itself (NSRangeException, observed on this Mac), so the path must be a
  # real directory — guarded above.
  if "$FPCTL" evaluate "$mount" >"$work/eval.txt" 2>&1; then
    if grep -q 'Evaluating actions' "$work/eval.txt"; then
      pass "evaluate accepted an item in $mount"
    else
      fail "evaluate exited 0 but output is not an evaluation"
    fi
  else
    fail "fileproviderctl evaluate failed (rc=$?) for $mount"
    head -5 "$work/eval.txt" | sed 's/^/       /'
  fi
fi

# --- 4. fileproviderctl diagnose: per-domain dump logs (informational) ---
headline "fileproviderctl diagnose"
diag_rc=0
"$FPCTL" diagnose >"$work/diag.txt" 2>&1 || diag_rc=$?
# The line reads "Creating diagnose directory at file://<path>." — Apple
# appends sentence punctuation, so strip a trailing "." and prefer the
# candidate that actually exists.
diag_dir=""
while IFS= read -r candidate; do
  candidate="${candidate#file://}"
  if [[ -d "$candidate" ]]; then
    diag_dir="$candidate"
    break
  fi
  if [[ "$candidate" == *. && -d "${candidate%.}" ]]; then
    diag_dir="${candidate%.}"
    break
  fi
done < <(sed -n 's|^Creating diagnose directory at \(.*\)$|\1|p' "$work/diag.txt" | head -1)
if [[ -n "$diag_dir" && -d "$diag_dir" ]]; then
  # Log files are named generically (i{N}p.domain_dump.log); the beebeeb
  # signal is the parent directory (bundle id), so match the full path.
  bee_logs="$(find "$diag_dir" -type f | grep -ci beebeeb || true)"
  all_logs="$(find "$diag_dir" -type f | wc -l | tr -d ' ')"
  info "diagnose rc=$diag_rc (informational: nonzero reflects ALL providers on this Mac)"
  info "$all_logs log file(s) total, $bee_logs beebeeb log(s) under $diag_dir"
  if [[ "$bee_logs" -ge 1 ]]; then
    pass "diagnose produced beebeeb dump logs"
  else
    fail "diagnose produced no beebeeb dump logs"
  fi
else
  fail "diagnose did not report a diagnose directory (rc=$diag_rc)"
  head -5 "$work/diag.txt" | sed 's/^/       /'
fi

# --- 5. our own ctl, if built ---
headline "BeebeebFileProviderCtl status"
if [[ -x "$CTL" ]]; then
  if "$CTL" status >"$work/ctl.txt" 2>&1; then
    pass "ctl status: $(head -1 "$work/ctl.txt")"
  else
    fail "ctl status failed (rc=$?)"
    head -3 "$work/ctl.txt" | sed 's/^/       /'
  fi
else
  skip "ctl binary not built at $CTL (run scripts/build-fileprovider-extension.sh)"
fi

printf '\nqa-fileprovider: %d passed, %d failed, %d skipped\n' "$passed" "$failed" "$skipped"
[[ "$failed" -eq 0 ]]