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
# Refuses a staging tree with anything but desktop/artifacts/<version>/<file> and desktop/v2/<channel>.json[.sig].
# A published artefact is never replaced: its name carries the version and it is served as immutable. The dir
# target refuses other bytes under the same name; the s3 and rsync targets keep the file that is already there
# (a manifest for other bytes then fails in every client, loudly, instead of in some caches only).
# `--self-test` proves the refusals and the order in a temp dir and prints "publish-release-files: N passed, 0 failed".
set -euo pipefail

ARTIFACT_RE='^desktop/artifacts/(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)/[A-Za-z0-9._-]+$'
MANIFEST_RE='^desktop/v2/(stable|beta|alpha)\.json(\.sig)?$'

fail() { echo "publish-release-files: $*" >&2; exit 1; }

# Prints the staged files, artefacts first, then manifests; fails on any other path.
# True when an `aws s3 ls <key>` listing ($1) has a line for exactly the file name $2. The listing is a prefix
# match: asking for `…/a.exe` also lists `…/a.exe.sig`, which must not count as "already published".
s3_has_exact_key() {
  local line
  while IFS= read -r line; do
    [[ "$line" =~ ^[0-9-]+[[:space:]]+[0-9:]+[[:space:]]+[0-9]+[[:space:]](.+)$ && "${BASH_REMATCH[1]}" == "$2" ]] && return 0
  done <<< "$1"
  return 1
}

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
        if [[ "$file" =~ $ARTIFACT_RE ]]; then
          cache='public, max-age=31536000, immutable'
          # Exact key only: `aws s3 ls <key>` also answers for every key that merely starts with it.
          if s3_has_exact_key "$(aws s3 ls "${target%/}/$file" 2> /dev/null || true)" "$(basename "$file")"; then
            echo "kept $file (already published)"
            continue
          fi
        fi
        aws s3 cp --only-show-errors --cache-control "$cache" "$staging/$file" "${target%/}/$file"
        ;;
      rsync:*)
        local keep=()
        [[ "$file" =~ $ARTIFACT_RE ]] && keep=(--ignore-existing)
        # `--`: a target that starts with a dash is a path, never an option.
        rsync --relative --times "${keep[@]}" -- "$staging/./$file" "${target#rsync:}/"
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

  # The s3 "already published" check is an exact match on the listing, not a prefix match.
  local listing=$'2026-11-20 10:00:00    9000001 Beebeeb_0.9.0_x64-setup.exe.sig\n2026-11-20 10:00:00         12 Beebeeb_0.9.0_x64-setup.exe.old'
  check refuses s3_has_exact_key "$listing" 'Beebeeb_0.9.0_x64-setup.exe'
  check refuses s3_has_exact_key '' 'Beebeeb_0.9.0_x64-setup.exe'
  check s3_has_exact_key "$listing"$'\n2026-11-20 10:00:00    9000001 Beebeeb_0.9.0_x64-setup.exe' 'Beebeeb_0.9.0_x64-setup.exe'
  # rsync gets `--` before its paths (a fake rsync records its arguments).
  mkdir -p "$tmp/bin"
  printf '#!/usr/bin/env bash\nprintf "%%s\\n" "$@" >> "%s"\n' "$tmp/rsync-args" > "$tmp/bin/rsync"
  chmod +x "$tmp/bin/rsync"
  echo manifest > "$tmp/stage/desktop/v2/stable.json"
  echo sig > "$tmp/stage/desktop/v2/stable.json.sig"
  ( PATH="$tmp/bin:$PATH" publish "$tmp/stage" "rsync:-e-evil@host:/srv" ) >/dev/null
  check test "$(grep -c -x -- '--' "$tmp/rsync-args")" -eq 3
  check test "$(grep -n -x -- '--' "$tmp/rsync-args" | head -1 | cut -d: -f1)" -lt "$(grep -n -x -- '-e-evil@host:/srv/' "$tmp/rsync-args" | head -1 | cut -d: -f1)"

  echo "publish-release-files: $passed passed, $failed failed"
  [[ $failed -eq 0 && $passed -eq 17 ]]
}

if [[ "${1:-}" == "--self-test" ]]; then
  self_test
  exit $?
fi
[[ $# -eq 2 ]] || fail "usage: publish-release-files.sh <staging dir> <target> | --self-test"
publish "$1" "$2"
