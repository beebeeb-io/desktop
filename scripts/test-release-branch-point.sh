#!/usr/bin/env bash
# Linux-runnable, offline test for scripts/verify-release-branch-point.sh, the check release.yml
# runs before `build` (and so before any signing): a release branch may only be dispatched when
# its first-parent history since its starting point is all merges of pull requests into that
# branch, and the starting point is a published `desktop-v*` tag commit that is itself a pull
# request merge into main or a `release/*` branch, or a commit on main.
#
# It builds a real local git repository and puts a fake `gh`
# (scripts/fixtures/release-branch-point-fake-gh.sh) on PATH, so no network and no GitHub are
# involved. Scenarios:
#   1. cut from a tag whose commit is a PR merge into release/0.8.18, with a published release,
#      PR merges only -> passes, names the tag, the release's published_at and URL;
#   2. cut from a main commit, PR merges only    -> passes, names main;
#   3. starting point on neither tag nor main    -> fails, names that commit;
#   4. a direct, non-PR commit after the start   -> fails, names that commit;
#   5. a PR merge whose base was another branch  -> fails, names that merge commit;
#   6. a non-release ref                         -> fails (defense in depth; release.yml only
#      calls this for refs/heads/release/*);
#   7. a tag with no GitHub release              -> fails: a tag alone is not trusted;
#   8. a tag whose release is a draft            -> fails: a draft is not published;
#   9. the release lookup fails                  -> fails closed (never accepts);
#  10. a tag whose commit is a PR merge into main, with a published release -> passes;
#  11. a tag on a direct commit, with a published release -> fails: the tag commit is not
#      reviewed (this is 0.8.18's shape; the published release alone must not be enough);
#  12. a tag whose commit is a PR merge into feature/x, with a published release -> fails;
#  13. the pulls lookup for the tag commit fails -> fails closed.
#
# The truth line is a COUNT ("release-branch-point: N passed, 0 failed"). CI asserts it against
# EXPECTED_CHECKS with `--assert-log` on the run's own log, and `--self-test` proves that same
# assertion can go red.
#
# Usage:
#   scripts/test-release-branch-point.sh                    # run every scenario
#   scripts/test-release-branch-point.sh --self-test        # red-proof of the count guard
#   scripts/test-release-branch-point.sh --assert-log <log> # the count guard on a run's log

# shellcheck disable=SC2329  # predicates are invoked indirectly through `check`
set -uo pipefail

EXPECTED_CHECKS=49

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$HERE/.." && pwd)"
SCRIPT="${BB_BRANCH_POINT_SCRIPT_UNDER_TEST:-$ROOT_DIR/scripts/verify-release-branch-point.sh}"
FAKE_GH_SRC="$ROOT_DIR/scripts/fixtures/release-branch-point-fake-gh.sh"
FAKE_REPO="beebeeb-io/desktop"

assert_counts() {
  local log="$1" code="$2" lines passed failed
  if [[ "$code" != "0" ]]; then
    echo "FAIL: test run exited $code" >&2
    return 1
  fi
  lines="$(grep -E '^release-branch-point: [0-9]+ passed, [0-9]+ failed$' "$log" || true)"
  if [[ "$(printf '%s' "$lines" | grep -c .)" != "1" ]]; then
    echo "FAIL: expected exactly one 'release-branch-point: N passed, M failed' line, found:" >&2
    printf '%s\n' "$lines" >&2
    return 1
  fi
  passed="$(printf '%s' "$lines" | sed -E 's/^release-branch-point: ([0-9]+) passed, ([0-9]+) failed$/\1/')"
  failed="$(printf '%s' "$lines" | sed -E 's/^release-branch-point: ([0-9]+) passed, ([0-9]+) failed$/\2/')"
  if [[ "$failed" != "0" ]]; then
    echo "FAIL: $failed check(s) failed" >&2
    return 1
  fi
  if [[ "$passed" != "$EXPECTED_CHECKS" ]]; then
    echo "FAIL: $passed check(s) passed, expected $EXPECTED_CHECKS" >&2
    return 1
  fi
  echo "OK: release-branch-point: $passed passed, 0 failed (expected $EXPECTED_CHECKS)"
}

self_test() {
  local dir
  dir="$(mktemp -d)"
  trap 'rm -rf "$dir"' RETURN

  printf 'release-branch-point: %s passed, 0 failed\n' "$EXPECTED_CHECKS" >"$dir/good.log"
  assert_counts "$dir/good.log" 0 >/dev/null 2>&1 || { echo "self-test: a correct log was rejected" >&2; return 1; }

  printf 'release-branch-point: %s passed, 0 failed\n' "$((EXPECTED_CHECKS - 1))" >"$dir/fewer.log"
  ! assert_counts "$dir/fewer.log" 0 >/dev/null 2>&1 || { echo "self-test: a run with too few checks was accepted" >&2; return 1; }

  printf 'release-branch-point: %s passed, 1 failed\n' "$EXPECTED_CHECKS" >"$dir/failed.log"
  ! assert_counts "$dir/failed.log" 1 >/dev/null 2>&1 || { echo "self-test: a run with a failure was accepted" >&2; return 1; }
  ! assert_counts "$dir/failed.log" 0 >/dev/null 2>&1 || { echo "self-test: a failed count with exit 0 was accepted" >&2; return 1; }

  : >"$dir/empty.log"
  ! assert_counts "$dir/empty.log" 0 >/dev/null 2>&1 || { echo "self-test: a log with no count line was accepted" >&2; return 1; }

  printf 'release-branch-point: %s passed, 0 failed\n' "$EXPECTED_CHECKS" >"$dir/nonzero.log"
  ! assert_counts "$dir/nonzero.log" 139 >/dev/null 2>&1 || { echo "self-test: a crashed run with a good count was accepted" >&2; return 1; }

  echo "self-test OK: the count guard rejected 5 bad logs and accepted the good one"
}

case "${1:-}" in
  --self-test) self_test; exit $? ;;
  --assert-log) assert_counts "${2:?usage: --assert-log <log file>}" 0; exit $? ;;
esac

for tool in jq git; do
  command -v "$tool" >/dev/null 2>&1 || { echo "missing $tool" >&2; exit 2; }
done
[[ -f "$SCRIPT" ]] || { echo "missing $SCRIPT" >&2; exit 2; }

TMP="$(mktemp -d)"
cleanup_tmp() { if [[ "${BB_TEST_KEEP_TMP:-}" == 1 ]]; then echo "kept scenario files in $TMP" >&2; else rm -rf "$TMP"; fi; }
trap cleanup_tmp EXIT

PASSED=0
FAILED=0
FAILED_FX=""
FX=""

check() {
  local desc="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    PASSED=$((PASSED + 1))
    printf '  ok   %s\n' "$desc"
  else
    FAILED=$((FAILED + 1))
    printf '  FAIL %s\n' "$desc"
    [[ " $FAILED_FX " == *" $FX "* ]] || FAILED_FX="$FAILED_FX $FX"
  fi
}

# ---------------------------------------------------------------- fake gh on PATH
SHIM="$TMP/bin"
mkdir -p "$SHIM"
cp "$FAKE_GH_SRC" "$SHIM/gh"
chmod +x "$SHIM/gh"
GH_LOG="$TMP/gh.log"
FIX="$TMP/fix"
mkdir -p "$FIX/pulls" "$FIX/tag-objects"
: >"$GH_LOG"

# ---------------------------------------------------------------- a real local repository
REPO_DIR="$TMP/repo"
MAIN_REF="refs/remotes/origin/main"
git init -q -b main "$REPO_DIR"
git -C "$REPO_DIR" config user.name "branch point test"
git -C "$REPO_DIR" config user.email "branch-point@example.invalid"

commit() { # <message> -> sha
  git -C "$REPO_DIR" commit -q --allow-empty -m "$1"
  git -C "$REPO_DIR" rev-parse HEAD
}
merge() { # <branch> <message> -> merge commit sha
  git -C "$REPO_DIR" merge -q --no-ff --no-edit -m "$2" "$1" >/dev/null
  git -C "$REPO_DIR" rev-parse HEAD
}

M0="$(commit "main: base")"
# A release tag whose commit is NOT on main: the tag check has to pass on its own. Its commit is
# the merge of a pull request into release/0.8.18, which is the shape 0.8.19 introduced.
git -C "$REPO_DIR" switch -q -c release/0.8.18 "$M0"
commit "release/0.8.18: the branch" >/dev/null
git -C "$REPO_DIR" switch -q -c feat/rel18 release/0.8.18
commit "feat: the change release/0.8.18 shipped" >/dev/null
git -C "$REPO_DIR" switch -q release/0.8.18
TAGGED="$(merge feat/rel18 "Merge pull request #118 from feat/rel18")"
git -C "$REPO_DIR" tag -a desktop-v0.8.18 -m "desktop-v0.8.18" "$TAGGED"
TAG_OBJECT="$(git -C "$REPO_DIR" rev-parse desktop-v0.8.18)"
# Three more tagged commits, all off the TAGGED history, never on main: their tags must not
# count because GitHub has no published release for them.
git -C "$REPO_DIR" switch -q -c legacy "$TAGGED"
NOREL="$(commit "legacy: tagged desktop-v0.8.17 with no GitHub release")"
git -C "$REPO_DIR" tag desktop-v0.8.17 "$NOREL"
DRAFT="$(commit "legacy: tagged desktop-v0.8.16, release is a draft")"
git -C "$REPO_DIR" tag desktop-v0.8.16 "$DRAFT"
APIERR="$(commit "legacy: tagged desktop-v0.8.15, the release lookup fails")"
git -C "$REPO_DIR" tag desktop-v0.8.15 "$APIERR"
git -C "$REPO_DIR" switch -q main
M1="$(commit "main: after the legacy branch")"
git -C "$REPO_DIR" update-ref "$MAIN_REF" "$M1"

# Scenario 1: cut from the tag, one PR merge, nothing else on first-parent.
git -C "$REPO_DIR" switch -q -c release/0.8.20 "$TAGGED"
git -C "$REPO_DIR" switch -q -c feat/1 "$TAGGED"
commit "feat: one reviewed change" >/dev/null
git -C "$REPO_DIR" switch -q release/0.8.20
MERGE1="$(merge feat/1 "Merge pull request #101 from feat/1")"

# Scenario 2: cut from a main commit, one PR merge.
git -C "$REPO_DIR" switch -q -c release/0.8.21 "$M1"
git -C "$REPO_DIR" switch -q -c feat/2 "$M1"
commit "feat: two reviewed changes" >/dev/null
git -C "$REPO_DIR" switch -q release/0.8.21
MERGE2="$(merge feat/2 "Merge pull request #102 from feat/2")"

# Scenario 3: the branch starts at a commit on neither main nor a tag.
git -C "$REPO_DIR" switch -q -c scratch "$M0"
UNREVIEWED="$(commit "scratch: never reviewed")"
git -C "$REPO_DIR" switch -q -c release/0.8.22 "$UNREVIEWED"
git -C "$REPO_DIR" switch -q -c feat/3 "$UNREVIEWED"
commit "feat: three reviewed changes" >/dev/null
git -C "$REPO_DIR" switch -q release/0.8.22
MERGE3="$(merge feat/3 "Merge pull request #103 from feat/3")"

# Scenario 4: a direct, non-PR commit lands on the release branch after its start.
git -C "$REPO_DIR" switch -q -c release/0.8.23 "$M1"
DIRECT="$(commit "release: a direct push, never reviewed")"
git -C "$REPO_DIR" switch -q -c feat/4 "$DIRECT"
commit "feat: four reviewed changes" >/dev/null
git -C "$REPO_DIR" switch -q release/0.8.23
MERGE4="$(merge feat/4 "Merge pull request #104 from feat/4")"

# Scenario 7-9: release branches cut exactly at a tagged commit whose tag does not count.
git -C "$REPO_DIR" switch -q -c release/0.8.26 "$NOREL"
git -C "$REPO_DIR" switch -q -c release/0.8.27 "$DRAFT"
git -C "$REPO_DIR" switch -q -c release/0.8.28 "$APIERR"

# Scenario 5: a merge commit whose pull request was merged into ANOTHER branch.
git -C "$REPO_DIR" switch -q -c release/0.8.24 "$M1"
git -C "$REPO_DIR" switch -q -c feat/5 "$M1"
commit "feat: five reviewed changes" >/dev/null
git -C "$REPO_DIR" switch -q release/0.8.24
MERGE5="$(merge feat/5 "Merge pull request #105 from feat/5 (into release/0.8.25)")"

# Scenario 10: cut from a tag whose commit is the merge of a pull request into main.
git -C "$REPO_DIR" switch -q -c feat/110 "$M1"
commit "feat: a change merged into main" >/dev/null
git -C "$REPO_DIR" switch -q main
TAGMAIN="$(merge feat/110 "Merge pull request #110 from feat/110")"
git -C "$REPO_DIR" tag desktop-v0.8.20 "$TAGMAIN"
git -C "$REPO_DIR" switch -q -c release/0.8.30 "$TAGMAIN"
git -C "$REPO_DIR" switch -q -c feat/10 "$TAGMAIN"
commit "feat: ten reviewed changes" >/dev/null
git -C "$REPO_DIR" switch -q release/0.8.30
MERGE10="$(merge feat/10 "Merge pull request #1100 from feat/10")"

# Scenario 11: a tag on a direct commit. It has a published release, but its commit was never
# reviewed, so the tag must not count (0.8.18's shape).
git -C "$REPO_DIR" switch -q -c legacy2 "$M0"
TAGDIRECT="$(commit "legacy2: tagged desktop-v0.8.21, a direct push with a published release")"
git -C "$REPO_DIR" tag desktop-v0.8.21 "$TAGDIRECT"
git -C "$REPO_DIR" switch -q -c release/0.8.31 "$TAGDIRECT"

# Scenario 12: a tag on a PR merge, but into a branch that is neither main nor release/*.
git -C "$REPO_DIR" switch -q -c feature/x "$M0"
git -C "$REPO_DIR" switch -q -c feat/112 "$M0"
commit "feat: a change merged into feature/x" >/dev/null
git -C "$REPO_DIR" switch -q feature/x
TAGOTHER="$(merge feat/112 "Merge pull request #112 from feat/112")"
git -C "$REPO_DIR" tag desktop-v0.8.22 "$TAGOTHER"
git -C "$REPO_DIR" switch -q -c release/0.8.32 "$TAGOTHER"

# Scenario 13: the pulls lookup for the tag commit fails; that must fail closed.
git -C "$REPO_DIR" switch -q -c legacy3 "$M0"
TAGPULLERR="$(commit "legacy3: tagged desktop-v0.8.23; the pulls lookup for it fails")"
git -C "$REPO_DIR" tag desktop-v0.8.23 "$TAGPULLERR"
git -C "$REPO_DIR" switch -q -c release/0.8.33 "$TAGPULLERR"

# ---------------------------------------------------------------- fake GitHub data
printf '[{"ref":"refs/tags/desktop-v0.8.18","object":{"type":"tag","sha":"%s"}},{"ref":"refs/tags/desktop-v0.8.17","object":{"type":"commit","sha":"%s"}},{"ref":"refs/tags/desktop-v0.8.16","object":{"type":"commit","sha":"%s"}},{"ref":"refs/tags/desktop-v0.8.15","object":{"type":"commit","sha":"%s"}},{"ref":"refs/tags/desktop-v0.8.20","object":{"type":"commit","sha":"%s"}},{"ref":"refs/tags/desktop-v0.8.21","object":{"type":"commit","sha":"%s"}},{"ref":"refs/tags/desktop-v0.8.22","object":{"type":"commit","sha":"%s"}},{"ref":"refs/tags/desktop-v0.8.23","object":{"type":"commit","sha":"%s"}}]\n' \
  "$TAG_OBJECT" "$NOREL" "$DRAFT" "$APIERR" "$TAGMAIN" "$TAGDIRECT" "$TAGOTHER" "$TAGPULLERR" >"$FIX/tags.json"

mkdir -p "$FIX/releases" "$FIX/release-errors" "$FIX/pull-errors"
release_json() { # <tag> <published_at> <html_url> <draft> <prerelease>
  printf '{"tag_name":"%s","draft":%s,"prerelease":%s,"published_at":"%s","html_url":"%s"}\n' \
    "$1" "$4" "$5" "$2" "$3" >"$FIX/releases/$1.json"
}
release_json desktop-v0.8.18 2026-10-28T09:15:00Z https://github.com/beebeeb-io/desktop/releases/tag/desktop-v0.8.18 false false
release_json desktop-v0.8.16 2026-10-27T09:00:00Z https://github.com/beebeeb-io/desktop/releases/tag/desktop-v0.8.16 true false
release_json desktop-v0.8.20 2026-10-29T09:00:00Z https://github.com/beebeeb-io/desktop/releases/tag/desktop-v0.8.20 false false
release_json desktop-v0.8.21 2026-10-29T09:10:00Z https://github.com/beebeeb-io/desktop/releases/tag/desktop-v0.8.21 false false
release_json desktop-v0.8.22 2026-10-29T09:20:00Z https://github.com/beebeeb-io/desktop/releases/tag/desktop-v0.8.22 false false
release_json desktop-v0.8.23 2026-10-29T09:30:00Z https://github.com/beebeeb-io/desktop/releases/tag/desktop-v0.8.23 false false
printf 'HTTP 500\n' >"$FIX/release-errors/desktop-v0.8.15"
printf 'HTTP 500\n' >"$FIX/pull-errors/$TAGPULLERR"
printf '{"sha":"%s","object":{"type":"commit","sha":"%s"}}\n' "$TAG_OBJECT" "$TAGGED" >"$FIX/tag-objects/$TAG_OBJECT.json"
pr_json() { # <merge sha> <number> <base ref>
  printf '[{"number":%s,"merged_at":"2026-10-29T10:00:00Z","merge_commit_sha":"%s","base":{"ref":"%s"},"head":{"ref":"feat"}}]\n' \
    "$2" "$1" "$3" >"$FIX/pulls/$1.json"
}
pr_json "$MERGE1" 101 release/0.8.20
pr_json "$MERGE2" 102 release/0.8.21
pr_json "$MERGE3" 103 release/0.8.22
pr_json "$MERGE4" 104 release/0.8.23
pr_json "$MERGE5" 105 release/0.8.25
pr_json "$TAGGED" 118 release/0.8.18
pr_json "$TAGMAIN" 110 main
pr_json "$MERGE10" 1100 release/0.8.30
pr_json "$TAGOTHER" 112 feature/x

OUT="$TMP/out.log"
CODE=0
run_case() { # <ref> <head sha>
  : >"$GH_LOG"
  (
    cd "$REPO_DIR" || exit 2
    PATH="$SHIM:$PATH" \
      REPO="$FAKE_REPO" \
      REF="$1" \
      HEAD_SHA="$2" \
      MAIN_REF="$MAIN_REF" \
      BB_FAKE_GH_DIR="$FIX" \
      BB_FAKE_GH_LOG="$GH_LOG" \
      GH_TOKEN="fake" \
      bash "$SCRIPT"
  ) >"$OUT" 2>&1
  CODE=$?
}

echo "verify-release-branch-point.sh (from ${SCRIPT#"$ROOT_DIR"/})"

# --- 1. cut from a tag, PR merges only
FX="tag-start"
run_case refs/heads/release/0.8.20 "$MERGE1"
check "tag start: passes" test "$CODE" -eq 0
check "tag start: names the tag commit as the starting point" grep -q "$TAGGED" "$OUT"
check "tag start: says which tag it is" grep -q 'desktop-v0.8.18' "$OUT"
check "tag start: counts the PR merge it walked" grep -q '1 first-parent commit' "$OUT"

# The endpoint calls of that run prove the check asked GitHub instead of guessing.
check "tag start: asked GitHub for the pulls of the walked commit" grep -q "api repos/$FAKE_REPO/commits/$MERGE1/pulls" "$GH_LOG"
check "tag start: asked GitHub for the desktop-v tags" grep -q "api repos/$FAKE_REPO/git/matching-refs/tags/desktop-v" "$GH_LOG"
check "tag start: resolved the annotated tag object to its commit" grep -q "api repos/$FAKE_REPO/git/tags/$TAG_OBJECT" "$GH_LOG"
check "tag start: asked GitHub for the release of the tag" grep -q "api repos/$FAKE_REPO/releases/tags/desktop-v0.8.18" "$GH_LOG"
check "tag start: prints the release's published_at" grep -q '2026-10-28T09:15:00Z' "$OUT"
check "tag start: prints the release's URL" grep -q 'https://github.com/beebeeb-io/desktop/releases/tag/desktop-v0.8.18' "$OUT"
check "tag start: says the tag commit is a pull request merged into release/0.8.18" grep -q 'merged into release/0.8.18' "$OUT"
check "tag start: asked GitHub for the pulls of the tagged merge" grep -q "api repos/$FAKE_REPO/commits/$TAGGED/pulls" "$GH_LOG"

# --- 2. cut from a main commit, PR merges only
FX="main-start"
run_case refs/heads/release/0.8.21 "$MERGE2"
check "main start: passes" test "$CODE" -eq 0
check "main start: names the main commit as the starting point" grep -q "$M1" "$OUT"
check "main start: says it is a commit on main" grep -q 'on main' "$OUT"

# --- 3. starting point on neither tag nor main
FX="bad-start"
run_case refs/heads/release/0.8.22 "$MERGE3"
check "unreviewed start: fails" test "$CODE" -ne 0
check "unreviewed start: names the offending commit" grep -q "$UNREVIEWED" "$OUT"
check "unreviewed start: says why (not a tag, not on main)" grep -q 'not.*desktop-v\* tag.*not.*on main\|neither' "$OUT"
check "unreviewed start: reports the failure as an error annotation" grep -q '::error::' "$OUT"

# --- 4. a direct, non-PR commit after the start
FX="direct-commit"
run_case refs/heads/release/0.8.23 "$MERGE4"
check "direct commit: fails" test "$CODE" -ne 0
check "direct commit: names the direct commit" grep -q "$DIRECT" "$OUT"
check "direct commit: says it is not a merged pull request" grep -q 'pull request' "$OUT"

# --- 5. a PR merge whose base was another branch
FX="other-base"
run_case refs/heads/release/0.8.24 "$MERGE5"
check "other base: fails" test "$CODE" -ne 0
check "other base: names the merge commit" grep -q "$MERGE5" "$OUT"
check "other base: says the pull request was not merged into this branch" grep -q "release/0.8.24" "$OUT"

# --- 7. a tag with no GitHub release does not count
FX="tag-without-release"
run_case refs/heads/release/0.8.26 "$NOREL"
check "tag without release: fails" test "$CODE" -ne 0
check "tag without release: names the offending commit" grep -q "$NOREL" "$OUT"
check "tag without release: says the tag has no GitHub release" grep -q 'no GitHub release' "$OUT"

# --- 8. a draft release is not published
FX="draft-release"
run_case refs/heads/release/0.8.27 "$DRAFT"
check "draft release: fails" test "$CODE" -ne 0
check "draft release: names the offending commit" grep -q "$DRAFT" "$OUT"
check "draft release: says the release is a draft" grep -q 'draft' "$OUT"

# --- 9. a failing release lookup fails closed
FX="release-api-error"
run_case refs/heads/release/0.8.28 "$APIERR"
check "release lookup error: fails" test "$CODE" -ne 0
check "release lookup error: names the tag and the HTTP status" grep -q 'desktop-v0.8.15.*500\|500.*desktop-v0.8.15' "$OUT"
check "release lookup error: says it refuses to guess" grep -q 'refusing to guess' "$OUT"

# --- 10. a tag whose commit is the merge of a pull request into main
FX="tag-pr-merge-main"
run_case refs/heads/release/0.8.30 "$MERGE10"
check "tag on a main PR merge: passes" test "$CODE" -eq 0
check "tag on a main PR merge: names the tag commit as the starting point" grep -q "$TAGMAIN" "$OUT"
check "tag on a main PR merge: says the pull request was merged into main" grep -q 'merged into main' "$OUT"
check "tag on a main PR merge: asked GitHub for the pulls of the tag commit" grep -q "api repos/$FAKE_REPO/commits/$TAGMAIN/pulls" "$GH_LOG"

# --- 11. a tag on a direct commit with a published release: not reviewed, so it must not count
FX="tag-direct-commit"
run_case refs/heads/release/0.8.31 "$TAGDIRECT"
check "tag on a direct commit: fails" test "$CODE" -ne 0
check "tag on a direct commit: names the offending commit" grep -q "$TAGDIRECT" "$OUT"
check "tag on a direct commit: says the tag does not count" grep -q 'tag does not count' "$OUT"
check "tag on a direct commit: says it is not a reviewable pull request merge" grep -q 'not the merge commit of a pull request' "$OUT"

# --- 12. a tag on a PR merge into a branch that is neither main nor release/*
FX="tag-pr-merge-other-base"
run_case refs/heads/release/0.8.32 "$TAGOTHER"
check "tag on a feature/* PR merge: fails" test "$CODE" -ne 0
check "tag on a feature/* PR merge: names the offending commit" grep -q "$TAGOTHER" "$OUT"
check "tag on a feature/* PR merge: says the tag does not count" grep -q 'tag does not count' "$OUT"

# --- 13. the pulls lookup for the tag commit fails: fail closed
FX="tag-pulls-api-error"
run_case refs/heads/release/0.8.33 "$TAGPULLERR"
check "pulls lookup error for the tag commit: fails" test "$CODE" -ne 0
check "pulls lookup error for the tag commit: says it refuses to guess" grep -q 'refusing to guess' "$OUT"

# --- 6. a non-release ref is refused (release.yml only calls this for release/*)
FX="non-release-ref"
run_case refs/heads/main "$M1"
check "non-release ref: fails closed" test "$CODE" -ne 0
check "non-release ref: says it is not a release branch" grep -q 'release/' "$OUT"

printf 'release-branch-point: %s passed, %s failed\n' "$PASSED" "$FAILED"
if ((FAILED > 0)); then
  echo "failing scenarios:$FAILED_FX" >&2
  exit 1
fi
exit 0
