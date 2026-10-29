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
#       commit of a `desktop-v*` tag with a PUBLISHED GitHub release, or a commit on main.
#       A tag alone is not trusted: pushing a tag is not reviewed, so only a release created
#       through the release workflow counts. A 404 / draft / failed lookup is not published.
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

# Reads the GitHub release for tag <tag> into RELEASE_STATUS and RELEASE_BODY.
# `--include` makes `gh api` print the status line, which is how a 404 (no release) is told
# apart from a real API failure. Never exits the script.
release_lookup() {
  local tag="$1" response
  response="$(gh api "repos/$REPO/releases/tags/$tag" --include 2>&1)" || true
  RELEASE_STATUS="$(printf '%s\n' "$response" | sed -n 's|^HTTP/[0-9.]* \([0-9][0-9][0-9]\).*|\1|p' | tail -1)"
  RELEASE_BODY="$(printf '%s\n' "$response" | sed '1,/^[[:space:]]*$/d')"
}

# Prints "<ref>\t<published_at>\t<html_url>" for the desktop-v* tag that points at <commit> and
# has a published GitHub release (draft == false), or nothing when no such tag does.
# A tag with no release (404) or a draft does not count; any other lookup failure fails closed.
tag_at_commit() {
  local commit="$1" refs listing ref type sha tag draft published_at url
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
    [[ "$sha" == "$commit" ]] || continue
    tag="${ref#refs/tags/}"
    release_lookup "$tag"
    case "$RELEASE_STATUS" in
      200)
        if ! draft="$(printf '%s\n' "$RELEASE_BODY" | jq -r 'if .draft == true then "true" else "false" end')"; then
          fail "could not read the GitHub release for tag $tag"
        fi
        if [[ "$draft" != "true" ]]; then
          # `published_at`/`html_url` are printed in the success note; require them.
          if ! published_at="$(printf '%s\n' "$RELEASE_BODY" | jq -r '.published_at // empty')" ||
            ! url="$(printf '%s\n' "$RELEASE_BODY" | jq -r '.html_url // empty')"; then
            fail "could not read the GitHub release for tag $tag"
          fi
          printf '%s\t%s\t%s\n' "$ref" "$published_at" "$url"
          return 0
        fi
        printf 'verify-release-branch-point: tag %s at %s has a draft GitHub release, which is not published; the tag does not count\n' "$tag" "$commit" >&2
        ;;
      404)
        printf 'verify-release-branch-point: tag %s at %s has no GitHub release; the tag does not count\n' "$tag" "$commit" >&2
        ;;
      *)
        fail "gh api repos/$REPO/releases/tags/$tag failed (HTTP ${RELEASE_STATUS:-no response}); refusing to guess whether $tag is a published release"
        ;;
    esac
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
tag_line="$(tag_at_commit "$start" || true)"
tag_ref=""
tag_published_at=""
tag_url=""
if [[ -n "$tag_line" ]]; then
  tag_ref="${tag_line%%$'\t'*}"
  tag_rest="${tag_line#*$'\t'}"
  tag_published_at="${tag_rest%%$'\t'*}"
  tag_url="${tag_rest#*$'\t'}"
fi

if [[ -n "$tag_ref" ]]; then
  note "OK: $BRANCH starts at $start, the commit of tag ${tag_ref#refs/tags/} (GitHub release published $tag_published_at: $tag_url); $merge_count first-parent commit(s) since the start are merged pull requests into $BRANCH"
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
