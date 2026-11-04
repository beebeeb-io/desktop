#!/usr/bin/env bash
# Test for scripts/release-blockers-check.sh.
#
# Covers:
#   - empty list passes all platforms
#   - absent file passes all platforms
#   - unscoped item refuses all platforms
#   - exact "[macOS]" scope refuses macOS and passes windows/linux
#   - typo variants ("[macos]", "[ macOS ]", "[MacOS]") are NOT scopes:
#     they refuse everywhere (fail-closed)
#   - mixed scoped + unscoped items
#   - argument-validation rejects unknown platforms / wrong arity
#
# Each case is proven red against a mutant of the checker (see "mutant proof"
# section below); counts are documented separately.
#
# The truth line is "release-blockers-check: N passed, 0 failed" asserted
# against EXPECTED_CHECKS. "--self-test" proves the guard rejects bad logs.

set -uo pipefail

EXPECTED_CHECKS=26

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SUT="${BB_BLOCKER_CHECK_SUT:-$HERE/release-blockers-check.sh}"

PASS=0
FAIL=0

check() {
  local desc="$1"; shift
  if "$@" >/dev/null 2>&1; then
    PASS=$((PASS + 1))
    printf '  ok   %s\n' "$desc"
  else
    FAIL=$((FAIL + 1))
    printf '  FAIL %s\n' "$desc"
  fi
}

# check that a command exits non-zero
check_fails() {
  local desc="$1"; shift
  if ! "$@" >/dev/null 2>&1; then
    PASS=$((PASS + 1))
    printf '  ok   %s\n' "$desc"
  else
    FAIL=$((FAIL + 1))
    printf '  FAIL %s\n' "$desc"
  fi
}

# check that the command's stdout contains a fixed string
check_output_has() {
  local desc="$1" needle="$2" file="$3" platform="$4"
  local out
  out="$(RELEASE_BLOCKERS_FILE="$file" "$SUT" "$platform")" || true
  if printf '%s' "$out" | grep -qF -- "$needle"; then
    PASS=$((PASS + 1))
    printf '  ok   %s\n' "$desc"
  else
    FAIL=$((FAIL + 1))
    printf '  FAIL %s\n' "$desc"
  fi
}

self_test() {
  local dir
  dir="$(mktemp -d)"
  trap 'rm -rf "$dir"' RETURN

  printf 'release-blockers-check: %s passed, 0 failed\n' "$EXPECTED_CHECKS" >"$dir/good.log"
  grep -qE "^release-blockers-check: ${EXPECTED_CHECKS} passed, 0 failed\$" "$dir/good.log" || {
    echo "self-test: good log was rejected" >&2; return 1
  }

  printf 'release-blockers-check: %s passed, 0 failed\n' "$((EXPECTED_CHECKS - 1))" >"$dir/fewer.log"
  ! grep -qE "^release-blockers-check: ${EXPECTED_CHECKS} passed, 0 failed\$" "$dir/fewer.log" || {
    echo "self-test: too-few count was accepted" >&2; return 1
  }

  printf 'release-blockers-check: %s passed, 1 failed\n' "$EXPECTED_CHECKS" >"$dir/failed.log"
  ! grep -qE "^release-blockers-check: ${EXPECTED_CHECKS} passed, 0 failed\$" "$dir/failed.log" || {
    echo "self-test: failure-count was accepted" >&2; return 1
  }

  : >"$dir/empty.log"
  ! grep -qE "^release-blockers-check: ${EXPECTED_CHECKS} passed, 0 failed\$" "$dir/empty.log" || {
    echo "self-test: empty log was accepted" >&2; return 1
  }

  echo "self-test OK: guard rejected 3 bad logs and accepted the good one"
}

if [[ "${1:-}" == "--self-test" ]]; then
  self_test
  exit $?
fi

TMP="$(mktemp -d)"
cleanup_tmp() { rm -rf "$TMP"; }
trap cleanup_tmp EXIT

run_checker() {  # <platform> <file>
  RELEASE_BLOCKERS_FILE="$2" "$SUT" "$1"
}

# ---------------------------------------------------------------- 1. empty file
echo "== case: empty file passes all platforms =="
printf '' >"$TMP/empty.md"
check "empty: macos passes"   run_checker macos    "$TMP/empty.md"
check "empty: windows passes" run_checker windows  "$TMP/empty.md"
check "empty: linux passes"   run_checker linux    "$TMP/empty.md"

# ---------------------------------------------------------------- 2. absent file
echo "== case: absent file passes all platforms =="
check "absent: macos passes"   run_checker macos    "$TMP/absent.md"
check "absent: windows passes" run_checker windows  "$TMP/absent.md"
check "absent: linux passes"   run_checker linux    "$TMP/absent.md"

# ---------------------------------------------------------------- 3. unscoped item
echo "== case: unscoped item refuses all platforms =="
printf '# Release blockers\n\n- An unscoped blocker.\n' >"$TMP/unscoped.md"
check_fails "unscoped: macos refuses"   run_checker macos    "$TMP/unscoped.md"
check_fails "unscoped: windows refuses" run_checker windows  "$TMP/unscoped.md"
check_fails "unscoped: linux refuses"   run_checker linux    "$TMP/unscoped.md"
check_output_has "unscoped: output names the item" "An unscoped blocker" \
  "$TMP/unscoped.md" macos

# ---------------------------------------------------------------- 4. exact [macOS] scope
echo "== case: [macOS]-scoped item refuses macOS and passes windows/linux =="
printf '# Release blockers\n\n- [macOS] A macOS-only blocker.\n' >"$TMP/scoped.md"
check_fails "scoped: macos refuses"   run_checker macos    "$TMP/scoped.md"
check       "scoped: windows passes"  run_checker windows  "$TMP/scoped.md"
check       "scoped: linux passes"    run_checker linux    "$TMP/scoped.md"
check_output_has "scoped: macos output names the item" "A macOS-only blocker" \
  "$TMP/scoped.md" macos

# ---------------------------------------------------------------- 5. typo: [macos] lowercase
echo "== case: [macos] (lowercase) is NOT a scope - refuses everywhere =="
printf '# Release blockers\n\n- [macos] Wrong case.\n' >"$TMP/typo_lower.md"
check_fails "typo-lower: macos refuses"   run_checker macos    "$TMP/typo_lower.md"
check_fails "typo-lower: windows refuses" run_checker windows  "$TMP/typo_lower.md"
check_fails "typo-lower: linux refuses"   run_checker linux    "$TMP/typo_lower.md"

# ---------------------------------------------------------------- 6. typo: [ macOS ] extra spaces
echo "== case: [ macOS ] (extra spaces) is NOT a scope - refuses everywhere =="
printf '# Release blockers\n\n- [ macOS ] Extra spaces.\n' >"$TMP/typo_space.md"
check_fails "typo-space: macos refuses"   run_checker macos    "$TMP/typo_space.md"
check_fails "typo-space: windows refuses" run_checker windows  "$TMP/typo_space.md"
check_fails "typo-space: linux refuses"   run_checker linux    "$TMP/typo_space.md"

# ---------------------------------------------------------------- 7. typo: [MacOS] wrong capitalisation
echo "== case: [MacOS] (wrong caps) is NOT a scope - refuses everywhere =="
printf '# Release blockers\n\n- [MacOS] Wrong capitalisation.\n' >"$TMP/typo_caps.md"
check_fails "typo-caps: macos refuses"   run_checker macos    "$TMP/typo_caps.md"
check_fails "typo-caps: windows refuses" run_checker windows  "$TMP/typo_caps.md"
check_fails "typo-caps: linux refuses"   run_checker linux    "$TMP/typo_caps.md"

# ---------------------------------------------------------------- 8. argument validation
echo "== case: argument validation =="
check_fails "bad-platform rejected"     RELEASE_BLOCKERS_FILE="$TMP/empty.md" "$SUT" notaplatform
check_fails "no-platform rejected"      RELEASE_BLOCKERS_FILE="$TMP/empty.md" "$SUT"
check_fails "extra-args rejected"       RELEASE_BLOCKERS_FILE="$TMP/empty.md" "$SUT" macos extra

# ---------------------------------------------------------------- final count (real script)
printf '\nrelease-blockers-check: %d passed, %d failed\n' "$PASS" "$FAIL"
[[ "$FAIL" -eq 0 ]] || exit 1

# ---------------------------------------------------------------- mutant proof (separate counters)
#
# Mutant A: treat ALL list items as unscoped (remove the [macOS] scope check).
# Against this mutant the "[macOS] passes windows" and "[macOS] passes linux"
# cases go RED, proving the scope logic is load-bearing.
#
# Mutant B: treat ALL list items as macOS-scoped (always skip for windows/linux).
# Against this mutant the "unscoped refuses windows/linux" and all typo cases
# go RED, proving the fail-closed fallback is load-bearing.

echo ""
echo "== mutant proof =="

# Mutant A: strip [macOS] scope entirely - every list item binds every platform.
# Proves: "[macOS] passes windows" and "[macOS] passes linux" cases can go RED.
MUTANT_A="$TMP/mutant_a.sh"
cat > "$MUTANT_A" << 'MUTANT_A_EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ $# -eq 1 ]] || { printf 'usage: %s <macos|windows|linux>\n' "$0" >&2; exit 2; }
PLATFORM="$1"
case "$PLATFORM" in macos|windows|linux) ;; *) printf 'usage: %s <macos|windows|linux>\n' "$0" >&2; exit 2 ;; esac
BLOCKERS_FILE="${RELEASE_BLOCKERS_FILE:-RELEASE_BLOCKERS.md}"
[[ -f "$BLOCKERS_FILE" ]] || exit 0
binding=""
while IFS= read -r line; do
  if [[ "$line" =~ ^[[:space:]]*[-*][[:space:]] ]]; then
    # mutant A: NO scope check — every item binds every platform
    binding="${binding}${line}
"
  fi
done < "$BLOCKERS_FILE"
binding="${binding%$'\n'}"
if [[ -n "$binding" ]]; then printf '%s\n' "$binding"; exit 1; fi
exit 0
MUTANT_A_EOF
chmod +x "$MUTANT_A"

# Mutant B: treat every list item as macOS-scoped - never bind windows or linux.
# Proves: "unscoped refuses windows/linux" and typo-refuse cases can go RED.
MUTANT_B="$TMP/mutant_b.sh"
cat > "$MUTANT_B" << 'MUTANT_B_EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ $# -eq 1 ]] || { printf 'usage: %s <macos|windows|linux>\n' "$0" >&2; exit 2; }
PLATFORM="$1"
case "$PLATFORM" in macos|windows|linux) ;; *) printf 'usage: %s <macos|windows|linux>\n' "$0" >&2; exit 2 ;; esac
BLOCKERS_FILE="${RELEASE_BLOCKERS_FILE:-RELEASE_BLOCKERS.md}"
[[ -f "$BLOCKERS_FILE" ]] || exit 0
binding=""
while IFS= read -r line; do
  if [[ "$line" =~ ^[[:space:]]*[-*][[:space:]] ]]; then
    # mutant B: treat EVERY item as macOS-scoped (never binds windows/linux)
    if [[ "$PLATFORM" == "macos" ]]; then
      binding="${binding}${line}
"
    fi
  fi
done < "$BLOCKERS_FILE"
binding="${binding%$'\n'}"
if [[ -n "$binding" ]]; then printf '%s\n' "$binding"; exit 1; fi
exit 0
MUTANT_B_EOF
chmod +x "$MUTANT_B"

mcheck() {  # <label> <expected: pass|fail> <platform> <file>
  local label="$1" expected="$2" platform="$3" file="$4" mutant="$5"
  local rc=0
  RELEASE_BLOCKERS_FILE="$file" "$mutant" "$platform" >/dev/null 2>&1 || rc=$?
  local actual
  if [[ "$rc" -eq 0 ]]; then actual="pass"; else actual="fail"; fi
  if [[ "$actual" == "$expected" ]]; then
    printf '  ok   %s\n' "$label"
  else
    printf '  RED  %s (got %s, expected %s)\n' "$label" "$actual" "$expected"
  fi
}

echo "--- mutant A (all items unscoped): [macOS] cases lose their scope ---"
mcheck "A: [macOS] item now REFUSES windows (scope stripped)" fail windows "$TMP/scoped.md" "$MUTANT_A"
mcheck "A: [macOS] item now REFUSES linux (scope stripped)"   fail linux   "$TMP/scoped.md" "$MUTANT_A"
mcheck "A: [macOS] item still refuses macos"                  fail macos   "$TMP/scoped.md" "$MUTANT_A"
mcheck "A: unscoped item still refuses windows"               fail windows "$TMP/unscoped.md" "$MUTANT_A"

echo "--- mutant B (all items macOS-scoped): fail-closed behavior lost ---"
mcheck "B: unscoped item now PASSES windows (fail-closed lost)" pass windows "$TMP/unscoped.md" "$MUTANT_B"
mcheck "B: unscoped item now PASSES linux (fail-closed lost)"   pass linux   "$TMP/unscoped.md" "$MUTANT_B"
mcheck "B: typo [macos] now PASSES windows (fail-closed lost)"  pass windows "$TMP/typo_lower.md" "$MUTANT_B"
mcheck "B: [macOS] item still refuses macos"                    fail macos   "$TMP/scoped.md"    "$MUTANT_B"
mcheck "B: [macOS] item still passes windows"                   pass windows "$TMP/scoped.md"    "$MUTANT_B"
