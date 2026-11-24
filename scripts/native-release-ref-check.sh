#!/usr/bin/env bash
# The two ref rules of the native release workflow (.github/workflows/release-native-windows.yml), as one script so
# that they can be tested without running the workflow.
#
#   native-release-ref-check.sh ref <github.ref>
#       Exit 0 only for refs/heads/main or refs/heads/release/<name>: a native release is built and promoted from
#       a release ref and from nowhere else.
#
#   native-release-ref-check.sh tag <repo dir> <tag name> <head sha>
#       Prints the tag's commit. Exit 0 only when ALL hold:
#         - the name is desktop-v<plain semver>;
#         - refs/tags/<name> exists. The tag is always read by its full ref name: a bare name resolves a BRANCH of
#           that name first in several git commands and in actions/checkout, and anyone with write access can push
#           a branch called desktop-v0.9.0 whose code would then run in the step that holds the signing key;
#         - no branch of that name exists (local or origin): its existence alone is refused;
#         - the tag's commit is <head sha> or an ancestor of it: what is promoted was released from this history.
#       The workflow never checks the tag out: scripts, the signer and the public key come from <head sha>, the
#       reviewed commit the run was dispatched on. The tag names the build (its assets), nothing that executes.
#
#   native-release-ref-check.sh --self-test
#       Proves each refusal in a temp repository; prints "native-release-ref-check: N passed, 0 failed".
set -euo pipefail

fail() { echo "native-release-ref-check: $*" >&2; exit 1; }

check_ref() {
  case "$1" in
    refs/heads/main) ;;
    refs/heads/release/?*) ;;
    *) fail "$1 is not a release ref (main or release/*)" ;;
  esac
  echo "release ref: $1"
}

check_tag() {
  local repo="$1" tag="$2" head="$3" tag_sha
  [[ "$tag" =~ ^desktop-v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || fail "'$tag' is not desktop-v<plain semver>"
  [[ "$head" =~ ^[0-9a-f]{40}$ ]] || fail "'$head' is not a full commit id"
  tag_sha="$(git -C "$repo" rev-parse --verify --quiet "refs/tags/$tag^{commit}")" || fail "there is no tag refs/tags/$tag"
  local branch
  for branch in "refs/heads/$tag" "refs/remotes/origin/$tag"; do
    if git -C "$repo" show-ref --verify --quiet "$branch"; then
      fail "a BRANCH named $tag exists ($branch): refusing, a branch must never carry a release tag's name"
    fi
  done
  git -C "$repo" cat-file -e "$head^{commit}" 2> /dev/null || fail "commit $head is not in this checkout"
  git -C "$repo" merge-base --is-ancestor "$tag_sha" "$head" || fail "refs/tags/$tag ($tag_sha) is not in the history of $head"
  echo "$tag_sha"
}

self_test() {
  local tmp passed=0 failed=0
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN
  check() { if "$@"; then passed=$((passed + 1)); else failed=$((failed + 1)); echo "FAIL: $*"; fi; }
  refuses() { ! ( "$@" ) > /dev/null 2>&1; }
  local g=(git -C "$tmp" -c user.name=test -c user.email=test@example.invalid -c commit.gpgsign=false -c tag.gpgsign=false)

  check check_ref refs/heads/main > /dev/null
  check check_ref refs/heads/release/0.9.0 > /dev/null
  check refuses check_ref refs/heads/feature
  check refuses check_ref refs/heads/release/
  check refuses check_ref refs/heads/main-evil
  check refuses check_ref refs/tags/desktop-v0.9.0
  check refuses check_ref refs/heads/desktop-v0.9.0

  git init -q -b main "$tmp"
  "${g[@]}" commit -q --allow-empty -m released
  local released; released="$(git -C "$tmp" rev-parse HEAD)"
  "${g[@]}" tag desktop-v0.9.0
  "${g[@]}" commit -q --allow-empty -m later
  local head; head="$(git -C "$tmp" rev-parse HEAD)"

  # The honest case: the tag is an ancestor of the dispatched commit (and the commit itself is fine too).
  check test "$(check_tag "$tmp" desktop-v0.9.0 "$head")" = "$released"
  check test "$(check_tag "$tmp" desktop-v0.9.0 "$released")" = "$released"
  # The attack: a branch with the tag's name, pointing at other code. A bare name would resolve to it.
  "${g[@]}" checkout -q -b evil
  "${g[@]}" commit -q --allow-empty -m "other code"
  local evil; evil="$(git -C "$tmp" rev-parse HEAD)"
  "${g[@]}" checkout -q main
  git -C "$tmp" update-ref refs/heads/desktop-v0.9.0 "$evil"
  check test "$(git -C "$tmp" rev-parse "refs/tags/desktop-v0.9.0^{commit}")" = "$released"
  check refuses check_tag "$tmp" desktop-v0.9.0 "$head"
  git -C "$tmp" update-ref -d refs/heads/desktop-v0.9.0
  git -C "$tmp" update-ref refs/remotes/origin/desktop-v0.9.0 "$evil"
  check refuses check_tag "$tmp" desktop-v0.9.0 "$head"
  git -C "$tmp" update-ref -d refs/remotes/origin/desktop-v0.9.0
  check check_tag "$tmp" desktop-v0.9.0 "$head" > /dev/null
  # A tag that is not in the dispatched history, no tag at all, and names that are not release tags.
  "${g[@]}" tag desktop-v0.9.1 "$evil"
  check refuses check_tag "$tmp" desktop-v0.9.1 "$head"
  check refuses check_tag "$tmp" desktop-v0.9.2 "$head"
  check refuses check_tag "$tmp" main "$head"
  check refuses check_tag "$tmp" 'desktop-v0.9.0^{commit}' "$head"
  check refuses check_tag "$tmp" desktop-v0.9.0 HEAD
  check refuses check_tag "$tmp" desktop-v0.9.0 0000000000000000000000000000000000000000

  # The workflow itself: no checkout step names the release tag as its ref (comment lines aside).
  local workflow; workflow="$(dirname "${BASH_SOURCE[0]}")/../.github/workflows/release-native-windows.yml"
  check test -s "$workflow"
  check refuses grep -Eq '^[^#]*ref:[[:space:]]*["'"'"']?(refs/tags/)?desktop-v' "$workflow"

  echo "native-release-ref-check: $passed passed, $failed failed"
  [[ $failed -eq 0 && $passed -eq 21 ]]
}

case "${1:-}" in
  --self-test) self_test ;;
  ref) [[ $# -eq 2 ]] || fail "usage: ref <github.ref>"; check_ref "$2" ;;
  tag) [[ $# -eq 4 ]] || fail "usage: tag <repo dir> <tag name> <head sha>"; check_tag "$2" "$3" "$4" ;;
  *) fail "usage: native-release-ref-check.sh ref <github.ref> | tag <repo dir> <tag> <head sha> | --self-test" ;;
esac
