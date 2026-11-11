#!/usr/bin/env bash
# Publishes a staged tree of release files to wherever releases.beebeeb.io is served from.
#
#   scripts/publish-release-files.sh <staging dir> <target>
#
# The staging dir holds paths exactly as they are served, for example
#   desktop/artifacts/0.9.0/Beebeeb_0.9.0_x64-setup.exe
#   desktop/v2/stable.json
#   desktop/v2/stable.json.sig
# Targets (the host is storage only: every file is already signed in CI, and no key is ever given to this script
# or to the host):
#   dir:<path>                  copy into a directory (today: the checkout of beebeeb-io/releases, GitHub Pages)
#   s3://<bucket>[/<prefix>]    an S3-compatible bucket; endpoint and credentials from the environment
#                               (AWS_ENDPOINT_URL, AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY), never from argv
#   rsync:<user@host:/path>     a directory on our own web server, over ssh
#
# Order: artefacts first, manifests last, so a manifest never points at a file that is not there yet. A manifest
# and its .sig are two files; a client that reads one new and one old refuses the pair and asks again later.
# Cache headers (s3 target; a web server sets the same by path): artefacts are immutable (their name carries the
# version), manifests and signatures are revalidated after 5 minutes.
#
# Refuses a staging tree with anything but desktop/artifacts/<version>/<file> and desktop/v2/<channel>.json[.sig],
# and refuses to overwrite a published artefact with different bytes (dir target).
# `--self-test` proves the refusals and the order in a temp dir and prints "publish-release-files: N passed, 0 failed".
set -euo pipefail

ARTIFACT_RE='^desktop/artifacts/(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)/[A-Za-z0-9._-]+$'
MANIFEST_RE='^desktop/v2/(stable|beta|alpha)\.json(\.sig)?$'

fail() { echo "publish-release-files: $*" >&2; exit 1; }

# Prints the staged files, artefacts first, then manifests; fails on any other path.
staged_files() {
  local staging="$1" file artifacts=() manifests=()
  while IFS= read -r file; do
    if [[ "$file" =~ $ARTIFACT_RE ]]; then artifacts+=("$file")
    elif [[ "$file" =~ $MANIFEST_RE ]]; then manifests+=("$file")
    else fail "unexpected file in the staging tree: $file"
    fi
  done < <(cd "$staging" && find . -type f | sed 's|^\./||' | LC_ALL=C sort)
  [[ ${#manifests[@]} -gt 0 ]] || fail "the staging tree holds no manifest"
  printf '%s\n' "${artifacts[@]}" "${manifests[@]}" | grep -v '^$'
}

publish() {
  local staging="$1" target="$2" file
  [[ -d "$staging" ]] || fail "no staging dir at $staging"
  local files
  # Explicit: inside a negated test (`refuses`) errexit is off, and a refused tree must still stop here.
  files="$(staged_files "$staging")" || return 1
  while IFS= read -r file; do
    case "$target" in
      dir:*)
        local dest="${target#dir:}/$file"
        if [[ "$file" =~ $ARTIFACT_RE && -e "$dest" ]] && ! cmp -s "$staging/$file" "$dest"; then
          fail "$file is already published with other bytes; an artefact is never replaced"
        fi
        mkdir -p "$(dirname "$dest")"
        cp "$staging/$file" "$dest"
        ;;
      s3://*)
        local cache='public, max-age=300, must-revalidate'
        [[ "$file" =~ $ARTIFACT_RE ]] && cache='public, max-age=31536000, immutable'
        aws s3 cp --only-show-errors --cache-control "$cache" "$staging/$file" "${target%/}/$file"
        ;;
      rsync:*)
        rsync --relative --times "$staging/./$file" "${target#rsync:}/"
        ;;
      *) fail "unknown target '$target' (dir:<path>, s3://<bucket>[/<prefix>] or rsync:<user@host:/path>)" ;;
    esac
    echo "published $file"
  done <<< "$files"
}

self_test() {
  local tmp passed=0 failed=0
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN
  check() { if "$@"; then passed=$((passed + 1)); else failed=$((failed + 1)); echo "FAIL: $*"; fi; }
  refuses() { ! ( "$@" ) >/dev/null 2>&1; }

  mkdir -p "$tmp/stage/desktop/v2" "$tmp/stage/desktop/artifacts/0.9.0" "$tmp/site"
  echo installer > "$tmp/stage/desktop/artifacts/0.9.0/Beebeeb_0.9.0_x64-setup.exe"
  echo manifest > "$tmp/stage/desktop/v2/stable.json"
  echo sig > "$tmp/stage/desktop/v2/stable.json.sig"

  local out
  out="$(publish "$tmp/stage" "dir:$tmp/site")"
  check test "$(sed -n 1p <<< "$out")" = 'published desktop/artifacts/0.9.0/Beebeeb_0.9.0_x64-setup.exe'
  check test "$(wc -l <<< "$out")" -eq 3
  check cmp -s "$tmp/stage/desktop/v2/stable.json.sig" "$tmp/site/desktop/v2/stable.json.sig"
  check test "$(cd "$tmp/site" && find . -type f | wc -l)" -eq 3
  # The same files again: nothing to refuse.
  check publish "$tmp/stage" "dir:$tmp/site" >/dev/null
  # A manifest may change; a published artefact may not.
  echo manifest2 > "$tmp/stage/desktop/v2/stable.json"
  check publish "$tmp/stage" "dir:$tmp/site" >/dev/null
  echo other > "$tmp/stage/desktop/artifacts/0.9.0/Beebeeb_0.9.0_x64-setup.exe"
  check refuses publish "$tmp/stage" "dir:$tmp/site"
  echo installer > "$tmp/stage/desktop/artifacts/0.9.0/Beebeeb_0.9.0_x64-setup.exe"
  # Nothing outside the two known places, and never the 0.8.x manifests.
  echo legacy > "$tmp/stage/desktop/latest.json"
  check refuses publish "$tmp/stage" "dir:$tmp/site"
  rm "$tmp/stage/desktop/latest.json"
  check test ! -e "$tmp/site/desktop/latest.json"
  check refuses publish "$tmp/stage" "ftp://example.invalid"
  check refuses publish "$tmp/missing" "dir:$tmp/site"
  rm "$tmp/stage/desktop/v2/stable.json" "$tmp/stage/desktop/v2/stable.json.sig"
  check refuses publish "$tmp/stage" "dir:$tmp/site"

  echo "publish-release-files: $passed passed, $failed failed"
  [[ $failed -eq 0 && $passed -eq 12 ]]
}

if [[ "${1:-}" == "--self-test" ]]; then
  self_test
  exit $?
fi
[[ $# -eq 2 ]] || fail "usage: publish-release-files.sh <staging dir> <target> | --self-test"
publish "$1" "$2"
