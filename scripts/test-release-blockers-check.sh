#!/usr/bin/env bash
# Test for scripts/release-blockers-check.sh.
#
# Covers:
#   - empty list passes all platforms
#   - absent file passes all platforms
#   - no trailing newline in file: last item still binds (|| [[ -n "$line" ]])
#   - unscoped item (hyphen bullet) refuses all platforms
#   - unscoped item (asterisk bullet) refuses all platforms
#   - unscoped item (indented with leading spaces) refuses all platforms
#   - unscoped item output carries "N: " line-number prefix
#   - exact "[macOS]" scope refuses macOS and passes windows/linux
#   - "[macOS]" output carries line-number prefix
#   - mixed scoped + unscoped: unscoped binds all; scoped binds macOS only
#   - typo variants ("[macos]", "[ macOS ]", "[MacOS]") are NOT scopes:
#     they refuse everywhere (fail-closed)
#   - argument-validation: bad platform, no platform, extra args each exit 2;
#     proven against a stand-in that always exits 0
#
# The truth line is "release-blockers-check: N passed, 0 failed" asserted
# against EXPECTED_CHECKS. "--self-test" proves the guard rejects bad logs.
# A surviving mutant is a test-coverage gap; the suite exits non-zero on one.

set -uo pipefail

EXPECTED_CHECKS=46

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

# ---------------------------------------------------------------- 3. no trailing newline
echo "== case: no trailing newline - last item still binds =="
# printf '%s' (no \n): file ends at the last character, no trailing newline.
printf '%s' '- Last item no newline' >"$TMP/no_newline.md"
check_fails "no-newline: macos refuses"   run_checker macos    "$TMP/no_newline.md"
check_fails "no-newline: windows refuses" run_checker windows  "$TMP/no_newline.md"
check_fails "no-newline: linux refuses"   run_checker linux    "$TMP/no_newline.md"

# ---------------------------------------------------------------- 4. unscoped item (hyphen bullet)
echo "== case: unscoped hyphen-bullet item refuses all platforms =="
printf '# Release blockers\n\n- An unscoped blocker.\n' >"$TMP/unscoped.md"
check_fails "unscoped-hyphen: macos refuses"   run_checker macos    "$TMP/unscoped.md"
check_fails "unscoped-hyphen: windows refuses" run_checker windows  "$TMP/unscoped.md"
check_fails "unscoped-hyphen: linux refuses"   run_checker linux    "$TMP/unscoped.md"
check_output_has "unscoped-hyphen: output names the item"  "An unscoped blocker" "$TMP/unscoped.md" macos
check_output_has "unscoped-hyphen: output has line number" "3: "                 "$TMP/unscoped.md" macos

# ---------------------------------------------------------------- 5. unscoped item (asterisk bullet)
echo "== case: unscoped asterisk-bullet item refuses all platforms =="
printf '# Release blockers\n\n* An asterisk blocker.\n' >"$TMP/asterisk.md"
check_fails "unscoped-asterisk: macos refuses"   run_checker macos    "$TMP/asterisk.md"
check_fails "unscoped-asterisk: windows refuses" run_checker windows  "$TMP/asterisk.md"
check_fails "unscoped-asterisk: linux refuses"   run_checker linux    "$TMP/asterisk.md"

# ---------------------------------------------------------------- 6. unscoped item (indented with spaces)
echo "== case: indented item refuses all platforms =="
printf '# Release blockers\n\n  - An indented blocker.\n' >"$TMP/indented.md"
check_fails "indented: macos refuses"   run_checker macos    "$TMP/indented.md"
check_fails "indented: windows refuses" run_checker windows  "$TMP/indented.md"
check_fails "indented: linux refuses"   run_checker linux    "$TMP/indented.md"

# ---------------------------------------------------------------- 7. exact [macOS] scope
echo "== case: [macOS]-scoped item refuses macOS and passes windows/linux =="
printf '# Release blockers\n\n- [macOS] A macOS-only blocker.\n' >"$TMP/scoped.md"
check_fails "scoped: macos refuses"   run_checker macos    "$TMP/scoped.md"
check       "scoped: windows passes"  run_checker windows  "$TMP/scoped.md"
check       "scoped: linux passes"    run_checker linux    "$TMP/scoped.md"
check_output_has "scoped: macos output names the item"  "A macOS-only blocker" "$TMP/scoped.md" macos
check_output_has "scoped: macos output has line number" "3: "                  "$TMP/scoped.md" macos

# ---------------------------------------------------------------- 8. mixed scoped + unscoped
echo "== case: mixed scoped and unscoped =="
printf '# Release blockers\n\n- [macOS] Mac-only item.\n- Shared item.\n' >"$TMP/mixed.md"
check_fails "mixed: macos refuses (both items)"    run_checker macos    "$TMP/mixed.md"
check_fails "mixed: windows refuses (shared item)" run_checker windows  "$TMP/mixed.md"
check_fails "mixed: linux refuses (shared item)"   run_checker linux    "$TMP/mixed.md"
check_output_has "mixed: macos output has mac-only item" "Mac-only item" "$TMP/mixed.md" macos
check_output_has "mixed: macos output has shared item"   "Shared item"   "$TMP/mixed.md" macos
check_output_has "mixed: windows output has shared item" "Shared item"   "$TMP/mixed.md" windows

# ---------------------------------------------------------------- 9. typo: [macos] lowercase
echo "== case: [macos] (lowercase) is NOT a scope - refuses everywhere =="
printf '# Release blockers\n\n- [macos] Wrong case.\n' >"$TMP/typo_lower.md"
check_fails "typo-lower: macos refuses"   run_checker macos    "$TMP/typo_lower.md"
check_fails "typo-lower: windows refuses" run_checker windows  "$TMP/typo_lower.md"
check_fails "typo-lower: linux refuses"   run_checker linux    "$TMP/typo_lower.md"

# ---------------------------------------------------------------- 10. typo: [ macOS ] extra spaces
echo "== case: [ macOS ] (extra spaces) is NOT a scope - refuses everywhere =="
printf '# Release blockers\n\n- [ macOS ] Extra spaces.\n' >"$TMP/typo_space.md"
check_fails "typo-space: macos refuses"   run_checker macos    "$TMP/typo_space.md"
check_fails "typo-space: windows refuses" run_checker windows  "$TMP/typo_space.md"
check_fails "typo-space: linux refuses"   run_checker linux    "$TMP/typo_space.md"

# ---------------------------------------------------------------- 11. typo: [MacOS] wrong capitalisation
echo "== case: [MacOS] (wrong caps) is NOT a scope - refuses everywhere =="
printf '# Release blockers\n\n- [MacOS] Wrong capitalisation.\n' >"$TMP/typo_caps.md"
check_fails "typo-caps: macos refuses"   run_checker macos    "$TMP/typo_caps.md"
check_fails "typo-caps: windows refuses" run_checker windows  "$TMP/typo_caps.md"
check_fails "typo-caps: linux refuses"   run_checker linux    "$TMP/typo_caps.md"

# ---------------------------------------------------------------- 12. argument validation
# Call the checker directly (no env-var prefix) so the shell sees the
# argument count correctly.  An earlier version passed RELEASE_BLOCKERS_FILE=X
# as the first word of "$@", which made bash execute it as a command name.
echo "== case: argument validation =="
check_fails "bad-platform rejected" bash "$SUT" notaplatform
check_fails "no-platform rejected"  bash "$SUT"
check_fails "extra-args rejected"   bash "$SUT" macos extra

# Prove each arg-validation case fails against a stand-in that always exits 0.
# The passall checker exits 0 for every invocation, including bad arguments.
# That means check_fails with passall would mark each test FAIL — confirming
# the tests measure the real checker's exit code rather than a coincidental failure.
PASSALL="$TMP/passall.sh"
printf '#!/usr/bin/env bash\nexit 0\n' >"$PASSALL"
chmod +x "$PASSALL"
OLD_SUT="$SUT"; SUT="$PASSALL"
check "arg-val proof: passall exits 0 for bad-platform (check_fails would FAIL)" \
  bash "$SUT" notaplatform
check "arg-val proof: passall exits 0 for no-platform  (check_fails would FAIL)" \
  bash "$SUT"
check "arg-val proof: passall exits 0 for extra-args   (check_fails would FAIL)" \
  bash "$SUT" macos extra
SUT="$OLD_SUT"

# ---------------------------------------------------------------- final count (real script)
printf '\nrelease-blockers-check: %d passed, %d failed\n' "$PASS" "$FAIL"
if [[ "$PASS" -ne "$EXPECTED_CHECKS" ]]; then
  printf 'FAIL: expected %d checks, ran %d (a check was added or deleted without updating EXPECTED_CHECKS)\n' \
    "$EXPECTED_CHECKS" "$PASS" >&2
  exit 1
fi
[[ "$FAIL" -eq 0 ]] || exit 1

# ---------------------------------------------------------------- mutant proof (separate counters)
#
# Mutants are generated by patching a copy of the REAL checker.
# sed-based mutations use line-independent text substitution; Python is used for
# the two mutations that require multi-line edits (drop else-branch, expand regex).
# Each mutant is counted.  A surviving mutant increments FAIL and fails the suite.

echo ""
echo "== mutant proof =="

REAL_SUT="$SUT"
MUTANTS_TOTAL=0
MUTANTS_KILLED=0

# Helper: check that a mutant produces the EXPECTED behavior for a given input.
# "expected" is "pass" (checker exits 0) or "fail" (checker exits non-zero).
# If the actual behavior matches "expected", the mutant IS KILLING this test case.
mcheck() {  # <label> <expected:pass|fail> <platform> <file> <mutant>
  local label="$1" expected="$2" platform="$3" file="$4" mutant="$5"
  local rc=0
  RELEASE_BLOCKERS_FILE="$file" "$mutant" "$platform" >/dev/null 2>&1 || rc=$?
  local actual
  if [[ "$rc" -eq 0 ]]; then actual="pass"; else actual="fail"; fi
  if [[ "$actual" == "$expected" ]]; then
    MUTANTS_KILLED=$((MUTANTS_KILLED + 1))
    printf '  RED  %s\n' "$label"
  else
    printf '  survived (BUG): %s — expected mutated result %s, got %s\n' \
      "$label" "$expected" "$actual" >&2
    FAIL=$((FAIL + 1))
  fi
}

# --- Mutant D (sed): remove "|| [[ -n "$line" ]]" — restores the last-line bug.
# Proves: "no-newline linux refuses" can go RED.
MUTANT_D="$TMP/mutant_D.sh"
sed 's/ || \[\[ -n "\$line" \]\]//' "$REAL_SUT" > "$MUTANT_D"
chmod +x "$MUTANT_D"
MUTANTS_TOTAL=$((MUTANTS_TOTAL + 2))
echo "--- mutant D (last-line bug restored) ---"
mcheck "D: no-newline item now PASSES linux  (last-line bug)" pass linux  "$TMP/no_newline.md" "$MUTANT_D"
mcheck "D: no-newline item now PASSES macos  (last-line bug)" pass macos  "$TMP/no_newline.md" "$MUTANT_D"

# --- Mutant C (sed): treat linux as macos — a [macOS] item now also binds linux.
# Proves: "scoped: linux passes" can go RED.
MUTANT_C="$TMP/mutant_C.sh"
sed 's/\[\[ "\$PLATFORM" == "macos" \]\]/[[ "$PLATFORM" == "macos" || "$PLATFORM" == "linux" ]]/' "$REAL_SUT" > "$MUTANT_C"
chmod +x "$MUTANT_C"
MUTANTS_TOTAL=$((MUTANTS_TOTAL + 1))
echo "--- mutant C (linux treated as macos) ---"
mcheck "C: [macOS] item now REFUSES linux (platform confusion)" fail linux "$TMP/scoped.md" "$MUTANT_C"

# --- Mutant A (Python): expand macOS scope regex to also accept [macos].
# Proves: "typo-lower windows refuses" can go RED.
MUTANT_A="$TMP/mutant_A.sh"
python3 - "$REAL_SUT" "$MUTANT_A" << 'PYEOF'
import sys
src = open(sys.argv[1]).read()
# Change the exact-match guard so it also accepts [macos] (case-insensitive expand)
mutant = src.replace(
    "[[:space:]]\\[macOS\\][[:space:]]",
    "[[:space:]]\\[[Mm][Aa][Cc][Oo][Ss]\\][[:space:]]"
)
open(sys.argv[2], "w").write(mutant)
PYEOF
chmod +x "$MUTANT_A"
MUTANTS_TOTAL=$((MUTANTS_TOTAL + 2))
echo "--- mutant A (scope regex accepts [macos]) ---"
mcheck "A: [macos] item now PASSES windows (typo-lower broken)" pass windows "$TMP/typo_lower.md" "$MUTANT_A"
mcheck "A: [macos] item now PASSES linux  (typo-lower broken)"  pass linux   "$TMP/typo_lower.md" "$MUTANT_A"

# --- Mutant B (Python): drop the else-branch binding — unscoped items no longer bind.
# Proves: "unscoped windows refuses" can go RED.
MUTANT_B="$TMP/mutant_B.sh"
python3 - "$REAL_SUT" "$MUTANT_B" << 'PYEOF'
import sys
lines = open(sys.argv[1]).readlines()
result = []
i = 0
while i < len(lines):
    if '# Unscoped (or any non-exact bracket pattern)' in lines[i]:
        result.append(lines[i])         # keep the comment
        i += 1
        # Skip the binding assignment ("binding=...") and its closing bare "
        if i < len(lines) and 'binding="${binding}' in lines[i]:
            result.append('      : # MUTANT B: unscoped branch suppressed\n')
            i += 1
            if i < len(lines) and lines[i].strip() == '"':
                i += 1
            continue
    result.append(lines[i])
    i += 1
open(sys.argv[2], "w").write("".join(result))
PYEOF
chmod +x "$MUTANT_B"
MUTANTS_TOTAL=$((MUTANTS_TOTAL + 2))
echo "--- mutant B (unscoped branch dropped) ---"
mcheck "B: unscoped item now PASSES windows (fail-closed lost)" pass windows "$TMP/unscoped.md" "$MUTANT_B"
mcheck "B: unscoped item now PASSES linux  (fail-closed lost)"  pass linux   "$TMP/unscoped.md" "$MUTANT_B"

printf '\nmutant proof: %d/%d killed\n' "$MUTANTS_KILLED" "$MUTANTS_TOTAL"
[[ "$FAIL" -eq 0 ]] || exit 1
