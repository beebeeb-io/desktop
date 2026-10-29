#!/usr/bin/env bash
# Refuse a release branch whose starting commit was never reviewed (task 2281).
#
# release.yml runs this before `build`, so before the `desktop-release` environment and before
# any Azure signing token exists. A `release/*` branch is only dispatched when:
#   (a) every commit on its first-parent history from the dispatched head back to its starting
#       point is a merge commit of a pull request that was merged into this same branch
#       (`gh api repos/{repo}/commits/{sha}/pulls`: merged_at set, base.ref this branch,
#       merge_commit_sha this commit); and
#   (b) that starting point -- the first first-parent commit that is not such a merge -- is the
#       commit of a published `desktop-v*` tag, or a commit on main.
#
# main is protected by pull requests, so a dispatch from main skips this check: release.yml's
# `verify-branch-point` job only runs for refs/heads/release/*.
#
# Inputs (environment):
#   REPO      owner/name of this repository; default $GITHUB_REPOSITORY
#   REF       the dispatched ref, for example refs/heads/release/0.8.20; default $GITHUB_REF
#   HEAD_SHA  the dispatched commit; default `git rev-parse HEAD`
#   MAIN_REF  the local ref that names main; default origin/main. The caller fetches it.
#   GH_TOKEN  token with contents:read and pull-requests:read, for `gh api`
#
# Exits 1 and names the offending commit when the branch point is not approved.
set -euo pipefail

REPO="${REPO:-${GITHUB_REPOSITORY:-}}"
REF="${REF:-${GITHUB_REF:-}}"
MAIN_REF="${MAIN_REF:-origin/main}"
HEAD_SHA="${HEAD_SHA:-}"
BRANCH="${REF#refs/heads/}"

fail() {
  printf '::error::%s\n' "$*" >&2
  printf 'verify-release-branch-point: FAIL: %s\n' "$*" >&2
  exit 1
}

note() { printf 'verify-release-branch-point: %s\n' "$*"; }

[[ -n "$REPO" ]] || fail "REPO (owner/name) is not set"
[[ -n "$REF" ]] || fail "REF is not set"
[[ "$REF" == refs/heads/* ]] || fail "REF '$REF' is not a branch ref; this check only runs for refs/heads/release/*"
[[ "$BRANCH" == release/* ]] || fail "ref '$REF' is not a release/* branch; this check only runs for refs/heads/release/*"
command -v gh >/dev/null 2>&1 || fail "gh is not installed"
command -v jq >/dev/null 2>&1 || fail "jq is not installed"

if [[ -z "$HEAD_SHA" ]]; then
  HEAD_SHA="$(git rev-parse HEAD)"
fi
git rev-parse --verify --quiet "$HEAD_SHA^{commit}" >/dev/null || fail "commit '$HEAD_SHA' is not in this checkout"
git rev-parse --verify --quiet "$MAIN_REF^{commit}" >/dev/null ||
  fail "main ref '$MAIN_REF' is not in this checkout; fetch main before running this check"

# True when <sha> is a merge commit of a pull request merged into $BRANCH (its merge commit).
is_pr_merge() {
  local sha="$1" pulls count
  if ! pulls="$(gh api "repos/$REPO/commits/$sha/pulls" --paginate)"; then
    fail "gh api repos/$REPO/commits/$sha/pulls failed; refusing to guess whether $sha is a merged pull request"
  fi
  if ! count="$(printf '%s\n' "$pulls" | jq -s --arg branch "$BRANCH" --arg sha "$sha" \
    '[.[][]? | select(.merged_at != null and .base.ref == $branch and .merge_commit_sha == $sha)] | length')"; then
    fail "could not read the pull requests GitHub returned for $sha"
  fi
  [[ "$count" -gt 0 ]]
}

# Prints the desktop-v* tag ref that points at <commit>, or nothing when none does.
tag_at_commit() {
  local commit="$1" refs listing ref type sha
  if ! refs="$(gh api "repos/$REPO/git/matching-refs/tags/desktop-v" --paginate)"; then
    fail "gh api repos/$REPO/git/matching-refs/tags/desktop-v failed"
  fi
  if ! listing="$(printf '%s\n' "$refs" | jq -s -r \
    '.[][] | select(.ref | startswith("refs/tags/desktop-v")) | "\(.ref)\t\(.object.type)\t\(.object.sha)"')"; then
    fail "could not read the desktop-v tags GitHub returned"
  fi
  while IFS=$'\t' read -r ref type sha; do
    [[ -n "${ref:-}" ]] || continue
    if [[ "$type" == "tag" ]]; then
      # An annotated tag points at a tag object; resolve it to the commit it names.
      if ! sha="$(gh api "repos/$REPO/git/tags/$sha" | jq -r '.object.sha // empty')"; then
        fail "could not resolve the tag object for $ref"
      fi
    fi
    if [[ "$sha" == "$commit" ]]; then
      printf '%s\n' "$ref"
      return 0
    fi
  done <<<"$listing"
  return 1
}

# Walk first-parent from the dispatched head back to the starting point: the first commit that
# is not a merge of a pull request into this branch.
start=""
start_is_merge=0
merge_count=0
while read -r sha _ p2 _rest; do
  [[ -n "${sha:-}" ]] || continue
  if [[ -n "${p2:-}" ]] && is_pr_merge "$sha"; then
    merge_count=$((merge_count + 1))
    continue
  fi
  start="$sha"
  if [[ -n "${p2:-}" ]]; then start_is_merge=1; fi
  break
done < <(git rev-list --first-parent --parents "$HEAD_SHA")

[[ -n "$start" ]] || fail "could not walk the first-parent history of $HEAD_SHA"

subject="$(git log -1 --format=%s "$start")"
tag_ref="$(tag_at_commit "$start" || true)"

if [[ -n "$tag_ref" ]]; then
  note "OK: $BRANCH starts at $start, the commit of tag ${tag_ref#refs/tags/}; $merge_count first-parent commit(s) since the start are merged pull requests into $BRANCH"
  exit 0
fi

if git merge-base --is-ancestor "$start" "$MAIN_REF" 2>/dev/null; then
  note "OK: $BRANCH starts at $start, a commit on main ($MAIN_REF); $merge_count first-parent commit(s) since the start are merged pull requests into $BRANCH"
  exit 0
fi

if [[ "$start_is_merge" == 1 ]]; then
  pr_reason="it is a merge commit, but not of a pull request merged into '$BRANCH' (no pull request with merged_at set, base.ref '$BRANCH' and this merge commit)"
else
  pr_reason="it is not a merge commit of a pull request merged into '$BRANCH'"
fi

fail "release branch '$BRANCH' is refused: commit $start ('$subject') is neither the commit of a published desktop-v* tag nor a commit on main ($MAIN_REF), and $pr_reason. Every commit from the dispatched head back to the starting point must have arrived through a pull request merged into '$BRANCH', and the starting point must be a reviewed commit."
