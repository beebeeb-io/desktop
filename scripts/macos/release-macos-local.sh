#!/usr/bin/env bash
# shellcheck disable=SC2015,SC2001  # style notes only: the sed prefixing and the guarded exit are deliberate
# The local macOS release of the native app (packaging-signing-update.md §4-§7). Fail closed: every gate that can say
# no runs before the first upload; nothing is published unless the notarised, stapled artefacts were verified.
#   scripts/macos/release-macos-local.sh <VERSION> <CHANNEL> [--dry-run] [--from-step N]
#     VERSION plain semver; CHANNEL alpha|beta|stable; --dry-run prints each step's commands and runs none (steps
#     that only read state still print what they would read); --from-step N resumes (step 1 always runs; past
#     step 2 the artefacts must come from HEAD, recorded in a build stamp).
# Steps: 1 state  2 build+bundle+sign  3 preflight  4 notarise .app  5 staple .app, spctl  6 .dmg, sign
#        7 notarise .dmg  8 staple .dmg, spctl  9 updater tarball  10 upload  11 publish workflow
# Configuration by env name only (nothing secret lives here):
#   APPLE_SIGNING_IDENTITY (required: the Developer ID Application identity, its 40-hex SHA-1)
#   MACOS_APP_PROVISION_PROFILE, MACOS_FILE_PROVIDER_PROVISION_PROFILE (paths, required)
#   BB_NOTARY_KEY_ID, BB_NOTARY_ISSUER, BB_NOTARY_KEY_PATH, BB_NOTARY_TIMEOUT
#   BB_RELEASE_REPO [beebeeb-io/desktop], BB_WORKFLOW_REF [the current branch], BB_MACOS_PUBLISH_WORKFLOW [release-native-macos.yml]
#   BB_POLL_INTERVAL [15], BB_WORKFLOW_TIMEOUT [1800]
# The updater signing key is a CI-only secret: it is unset for the build and never read here.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

TOTAL=11
NAMES=("" "Verify release state" "Build, bundle, sign (hardened runtime)" "Preflight the unnotarised .app" "Notarise the .app"
  "Staple the .app, verify" "Build the .dmg from the stapled app, sign" "Notarise the .dmg" "Staple the .dmg, verify"
  "Updater tarball from the stapled app" "Upload to the GitHub release" "Publish the channel manifest (workflow)")

DRY=0 FROM=1 VERSION="" CHANNEL=""
while (($#)); do
  case "$1" in
    --dry-run) DRY=1 ;;
    --from-step) shift; [[ $# -gt 0 ]] || { echo "--from-step needs a number" >&2; exit 2; }; FROM="$1" ;;
    -*) echo "unknown option: $1" >&2; exit 2 ;;
    *) if [[ -z "$VERSION" ]]; then VERSION="$1"; elif [[ -z "$CHANNEL" ]]; then CHANNEL="$1"; else echo "unexpected argument: $1" >&2; exit 2; fi ;;
  esac
  shift
done
[[ -n "$VERSION" && -n "$CHANNEL" ]] || { echo "usage: release-macos-local.sh <VERSION> <CHANNEL> [--dry-run] [--from-step N]" >&2; exit 2; }
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "VERSION must be plain semver like 0.9.0 (got '$VERSION')" >&2; exit 2; }
case "$CHANNEL" in alpha | beta | stable) ;; *) echo "CHANNEL must be alpha, beta or stable (got '$CHANNEL')" >&2; exit 2 ;; esac
[[ "$FROM" =~ ^[0-9]+$ ]] && ((FROM >= 1 && FROM <= TOTAL)) || { echo "--from-step must be 1-$TOTAL (got '$FROM')" >&2; exit 2; }

TAG="desktop-v$VERSION"
REPO="${BB_RELEASE_REPO:-beebeeb-io/desktop}"
WF_REF="${BB_WORKFLOW_REF:-$(git -C "$ROOT_DIR" branch --show-current 2>/dev/null || true)}"
[[ -n "$WF_REF" ]] || { echo "detached HEAD: set BB_WORKFLOW_REF to the branch the workflow runs from" >&2; exit 2; }
WF_PUBLISH="${BB_MACOS_PUBLISH_WORKFLOW:-release-native-macos.yml}"
POLL="${BB_POLL_INTERVAL:-15}" WF_TIMEOUT="${BB_WORKFLOW_TIMEOUT:-1800}"
IDENTITY="${APPLE_SIGNING_IDENTITY:-}"
WORK="${BB_RELEASE_WORK:-$ROOT_DIR/target/release-macos-local/$VERSION}"
STAMP="$WORK/build-commit"
OUT="$WORK/out"
APP="$OUT/Beebeeb.app"
DMG="$WORK/Beebeeb_${VERSION}_aarch64.dmg"
TARBALL="$WORK/Beebeeb.app.tar.gz"
APPZIP="$WORK/Beebeeb.app.zip"
BIN="${CARGO_TARGET_DIR:-$ROOT_DIR/native/beebeeb-drive-rune/target}/release/beebeeb-drive"

STEP=0 UPLOADED=0
fail() {
  {
    printf '\nFAILED at step %s/%s (%s)\n  %s\n' "$STEP" "$TOTAL" "${NAMES[$STEP]:-setup}" "$1"
    if ((UPLOADED)); then printf '  State: assets were uploaded to %s; the manifest may not be published.\n' "$TAG"
    else printf '  State: nothing was uploaded and nothing was published.\n'; fi
    printf '  Resume: scripts/macos/release-macos-local.sh %s %s --from-step %s\n' "$VERSION" "$CHANNEL" "$STEP"
  } >&2
  exit 1
}
banner() { STEP="$1"; printf '\n==> [%s/%s] %s\n' "$1" "$TOTAL" "${NAMES[$1]}"; }
say() { printf '    %s\n' "$*"; }
show() { printf '    +'; printf ' %q' "$@"; printf '\n'; }
run() { show "$@"; ((DRY)) && return 0; local rc=0; "$@" || rc=$?; ((rc == 0)) || fail "command failed (exit $rc): $*"; }
# capture <var> <cmd...>: stdout+stderr into var, exit code in RC. Dry: runs nothing.
capture() { local v="$1" o=""; shift; show "$@"; RC=0; if ((DRY)); then printf -v "$v" ''; return 0; fi; o="$("$@" 2>&1)" || RC=$?; printf -v "$v" '%s' "$o"; }
need_cmd() { if ((DRY)); then say "would require command: $1"; else command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"; fi; }
need_file() { if ((DRY)); then say "would require $2: $1"; else [[ -e "$1" ]] || fail "$2 not found: $1"; fi; }

step_1() {
  local t head refs tag_sha flat blockers rc=0 notes_bad
  for t in git gh jq xcrun codesign spctl hdiutil ditto plutil security tar cargo; do need_cmd "$t"; done
  if ((DRY)); then say "would require Darwin arm64 (Apple Silicon only)"
  else [[ "$(uname -s)" == Darwin && "$(uname -m)" == arm64 ]] || fail "this must run on an Apple Silicon Mac ($(uname -s) $(uname -m))"; fi
  [[ -n "$IDENTITY" ]] || ((DRY)) || fail "APPLE_SIGNING_IDENTITY is not set (the Developer ID Application identity, 40-hex SHA-1)"
  [[ -z "$IDENTITY" || "$IDENTITY" =~ ^[0-9A-Fa-f]{40}$ ]] || fail "APPLE_SIGNING_IDENTITY must be the 40-hex SHA-1 of the identity (a name can match several certificates)"
  capture t security find-identity -v -p codesigning
  if ! ((DRY)); then grep -qiF -- "$IDENTITY" <<<"$t" || fail "signing identity not in the keychain: $IDENTITY"; fi
  need_file "${MACOS_APP_PROVISION_PROFILE:-}" "app provisioning profile (MACOS_APP_PROVISION_PROFILE)"
  need_file "${MACOS_FILE_PROVIDER_PROVISION_PROFILE:-}" "File Provider provisioning profile (MACOS_FILE_PROVIDER_PROVISION_PROFILE)"
  need_file "${BB_NOTARY_KEY_PATH:-}" "notary API key (BB_NOTARY_KEY_PATH)"
  if ! ((DRY)); then
    : "${BB_NOTARY_KEY_ID:?}" "${BB_NOTARY_ISSUER:?}"
  fi
  run gh auth status

  capture t git status --porcelain
  ((DRY)) || [[ -z "$t" ]] || fail "working tree is not clean:
$(head -20 <<<"$t" | sed 's/^/      /')"

  capture head git rev-parse HEAD
  capture refs git ls-remote --tags origin "refs/tags/$TAG" "refs/tags/$TAG^{}"
  if ! ((DRY)); then
    tag_sha="$(awk -v r="refs/tags/$TAG^{}" '$2 == r {print $1}' <<<"$refs")"
    [[ -n "$tag_sha" ]] || tag_sha="$(awk -v r="refs/tags/$TAG" '$2 == r {print $1}' <<<"$refs")"
    [[ -n "$tag_sha" ]] || fail "tag $TAG does not exist on origin"
    [[ "$head" == "$tag_sha" ]] || fail "HEAD ($head) is not the commit tag $TAG points to ($tag_sha)"
    say "HEAD == $TAG == $head"
    if ((FROM > 2)); then
      [[ -f "$STAMP" ]] || fail "--from-step $FROM reuses the artefacts of step 2, but $STAMP does not exist"
      [[ "$(<"$STAMP")" == "$head" ]] || fail "the artefacts under $WORK were built from $(<"$STAMP"), HEAD is $head; rebuild from step 2"
    fi
  fi

  if ((DRY)); then say "would require RELEASE_NOTES.md to name $VERSION, no '<!-- lead:' marker, real test counts; RELEASE_BLOCKERS.md empty"
  else
    [[ -f RELEASE_NOTES.md ]] || fail "RELEASE_NOTES.md is missing"
    grep -qF -- "$VERSION" RELEASE_NOTES.md || fail "RELEASE_NOTES.md does not mention $VERSION"
    notes_bad="$(grep -n '<!-- lead:' RELEASE_NOTES.md || true)"
    [[ -z "$notes_bad" ]] || fail "RELEASE_NOTES.md has unresolved lead markers: $notes_bad"
    flat="$(tr '\n' ' ' <RELEASE_NOTES.md | tr -s ' ')"
    grep -qE '(^|[^0-9])[1-9][0-9]* pass / 0 fail' <<<"$flat" || fail "RELEASE_NOTES.md carries no test counts ('<N> pass / 0 fail')"
    grep -qE 'test result: ok\. [1-9][0-9]* passed' <<<"$flat" || fail "RELEASE_NOTES.md carries no test counts ('test result: ok. <N> passed')"
    blockers="$(bash scripts/release-blockers-check.sh macos)" || rc=$?
    ((rc <= 1)) || fail "release-blockers-check.sh exited $rc (expected 0 or 1)"
    ((rc == 0)) || fail "RELEASE_BLOCKERS.md lists release blockers:
$(sed 's/^/      /' <<<"$blockers")"
    say "notes and blockers pass"
  fi
  capture t gh release view "$TAG" --repo "$REPO" --json isDraft
  if ! ((DRY)); then
    ((RC == 0)) || fail "release $TAG not found in $REPO"
    [[ "$(jq -r .isDraft <<<"$t")" == false ]] || fail "release $TAG is still a draft"
  fi
  run mkdir -p "$WORK"
}

step_2() {
  run rm -f "$STAMP"
  run rm -rf "$OUT"
  run mkdir -p "$OUT"
  # The updater key and Apple credentials never reach the build; the identity, version, channel and the updater's
  # not-before mark (the commit time) are set for it only. Without the mark the app has no updater (NotARelease).
  local mark; mark="$(git log -1 --format=%ct HEAD)"
  [[ "$mark" =~ ^[0-9]+$ ]] || fail "cannot read the commit time of HEAD"
  show env -u TAURI_SIGNING_PRIVATE_KEY -u TAURI_SIGNING_PRIVATE_KEY_PASSWORD -u UPDATER_V2_PRIVATE_KEY -u UPDATER_V2_PRIVATE_KEY_PASSWORD \
    "BEEBEEB_APP_IDENTIFIER=io.beebeeb.app" "BEEBEEB_RELEASE_VERSION=$VERSION" "BEEBEEB_RELEASE_CHANNEL=$CHANNEL" "BEEBEEB_BUILD_NOT_BEFORE=$mark" \
    cargo build --release --locked --manifest-path native/beebeeb-drive-rune/Cargo.toml
  if ! ((DRY)); then
    env -u TAURI_SIGNING_PRIVATE_KEY -u TAURI_SIGNING_PRIVATE_KEY_PASSWORD -u UPDATER_V2_PRIVATE_KEY -u UPDATER_V2_PRIVATE_KEY_PASSWORD \
      -u BB_NOTARY_KEY_PATH BEEBEEB_APP_IDENTIFIER=io.beebeeb.app BEEBEEB_RELEASE_VERSION="$VERSION" \
      BEEBEEB_RELEASE_CHANNEL="$CHANNEL" BEEBEEB_BUILD_NOT_BEFORE="$mark" \
      cargo build --release --locked --manifest-path native/beebeeb-drive-rune/Cargo.toml || fail "release build failed"
    local marks; marks="$(grep -a -o 'beebeeb-updater-not-before=[0-9a-z]*;' "$BIN" | sort -u || true)"
    [[ "$marks" == "beebeeb-updater-not-before=${mark};" ]] || fail "the binary must carry exactly 'beebeeb-updater-not-before=${mark};', found: ${marks:-nothing}"
  else say "would check that $BIN carries beebeeb-updater-not-before=$mark;"; fi
  show env BEEBEEB_RELEASE_VERSION="$VERSION" BEEBEEB_MACOS_HARDENED_RUNTIME=1 scripts/macos/bundle-app.sh --identity real --binary "$BIN" --out "$OUT" --sign
  if ! ((DRY)); then
    env -u TAURI_SIGNING_PRIVATE_KEY -u TAURI_SIGNING_PRIVATE_KEY_PASSWORD -u UPDATER_V2_PRIVATE_KEY -u UPDATER_V2_PRIVATE_KEY_PASSWORD \
      BEEBEEB_RELEASE_VERSION="$VERSION" BEEBEEB_MACOS_HARDENED_RUNTIME=1 APPLE_SIGNING_IDENTITY="$IDENTITY" \
      scripts/macos/bundle-app.sh --identity real --binary "$BIN" --out "$OUT" --sign || fail "bundle/sign failed"
    local got; got="$(plutil -extract CFBundleShortVersionString raw -o - "$APP/Contents/Info.plist")"
    [[ "$got" == "$VERSION" ]] || fail "the bundle reports version '$got', expected '$VERSION'"
    git rev-parse HEAD >"$STAMP"
  else say "would record the build commit in $STAMP"; fi
}

# Unnotarised app: spctl cannot say Notarized yet, so step 3 passes only if preflight's only complaint is that line.
step_3() {
  local out rc=0
  show scripts/macos/preflight.sh "$APP"
  ((DRY)) && { say "(dry-run) would require only the spctl line to fail (the app is not notarised yet)"; return 0; }
  out="$(scripts/macos/preflight.sh "$APP" 2>&1)" || rc=$?
  printf '%s\n' "$out" | sed 's/^/      /'
  if ((rc != 0)); then
    local others; others="$(grep '^FAIL' <<<"$out" | grep -v 'spctl does not say Notarized Developer ID' || true)"
    [[ -z "$others" ]] || fail "preflight failed: $others"
  fi
}

notarise() { # <file> <label>
  show scripts/macos/notarise.sh "$1" "$2" "$WORK"
  ((DRY)) && { say "(dry-run) would require Accepted, else fetch the notarytool log and stop"; return 0; }
  scripts/macos/notarise.sh "$1" "$2" "$WORK" || fail "notarisation of the $2 was not accepted (log: $WORK/notary-$2-log.json)"
}

staple() { run xcrun stapler staple "$1"; run xcrun stapler validate "$1"; }

require_notarized() { # <path> [-t install]
  local path="$1" out
  shift
  capture out spctl -a -vv "$@" "$path"
  ((DRY)) && { say "(dry-run) would require 'accepted' and 'source=Notarized Developer ID'"; return 0; }
  sed 's/^/      /' <<<"$out"
  ((RC == 0)) || fail "spctl rejected $path (exit $RC)"
  grep -q 'source=Notarized Developer ID' <<<"$out" || fail "spctl accepted $path but not as Notarized Developer ID"
}

step_4() {
  run rm -f "$APPZIP"
  run ditto -c -k --keepParent "$APP" "$APPZIP"
  notarise "$APPZIP" app
}
step_5() {
  staple "$APP"
  run codesign --verify --deep --strict --verbose=2 "$APP"
  require_notarized "$APP"
  run scripts/macos/preflight.sh "$APP"
}
step_6() {
  local stage="$WORK/dmg-stage"
  run rm -rf "$stage" "$DMG"
  run mkdir -p "$stage"
  run ditto "$APP" "$stage/Beebeeb.app"
  run ln -s /Applications "$stage/Applications"
  run hdiutil create -volname Beebeeb -srcfolder "$stage" -fs HFS+ -format UDZO -ov "$DMG"
  run codesign --force --sign "$IDENTITY" --timestamp "$DMG"
  run codesign --verify --verbose=2 "$DMG"
  run rm -rf "$stage"
}
step_7() { notarise "$DMG" dmg; }
step_8() {
  staple "$DMG"
  run hdiutil verify "$DMG"
  require_notarized "$DMG" -t install
}
tarball_sha256() { shasum -a 256 "$TARBALL" | cut -d' ' -f1; }
step_9() {

  # The app inside the dmg must itself be stapled; the tarball is made from the stapled app.
  run xcrun stapler validate "$APP"
  run rm -f "$TARBALL"
  show env COPYFILE_DISABLE=1 tar -czf "$TARBALL" -C "$OUT" Beebeeb.app
  if ! ((DRY)); then COPYFILE_DISABLE=1 tar -czf "$TARBALL" -C "$OUT" Beebeeb.app || fail "tar failed"; fi
  # The publish workflow signs only a tarball with this hash.
  show shasum -a 256 "$TARBALL"
  if ! ((DRY)); then tarball_sha256 > "$WORK/tarball.sha256" || fail "sha256 failed"; fi
}
step_10() {
  local up=("$DMG" "$TARBALL")
  show gh release upload "$TAG" "${up[@]}" --clobber --repo "$REPO"
  if ((DRY)); then say "(dry-run) publishes nothing"; return 0; fi
  UPLOADED=1
  gh release upload "$TAG" "${up[@]}" --clobber --repo "$REPO" || fail "upload failed"
}
step_11() {
  local out id view status conclusion waited=0 sha
  if ((DRY)); then sha="<sha256 of $TARBALL>"; else sha="$(tarball_sha256)" || fail "cannot hash $TARBALL"; fi
  [[ "$sha" =~ ^[0-9a-f]{64}$ ]] || ((DRY)) || fail "bad tarball sha256 '$sha'"
  show gh workflow run "$WF_PUBLISH" --repo "$REPO" --ref "$WF_REF" -f "tag=$TAG" -f "channel=$CHANNEL" -f "tarball_sha256=$sha"
  if ((DRY)); then say "(dry-run) would wait for the run it created"; return 0; fi
  out="$(gh workflow run "$WF_PUBLISH" --repo "$REPO" --ref "$WF_REF" -f "tag=$TAG" -f "channel=$CHANNEL" -f "tarball_sha256=$sha" 2>&1)" || fail "dispatch failed: $out"
  id="$(grep -oE '/actions/runs/[0-9]+' <<<"$out" | head -1 | grep -oE '[0-9]+$' || true)"
  [[ -n "$id" ]] || fail "gh printed no run URL, so the run cannot be identified; check $WF_PUBLISH by hand. Nothing was guessed."
  while :; do
    view="$(gh run view "$id" --repo "$REPO" --json status,conclusion 2>/dev/null || echo '{}')"
    status="$(jq -r '.status // empty' <<<"$view")"; conclusion="$(jq -r '.conclusion // empty' <<<"$view")"
    [[ "$status" == completed ]] && break
    ((waited < WF_TIMEOUT)) || fail "run $id did not finish within ${WF_TIMEOUT}s"
    sleep "$POLL"; waited=$((waited + POLL + 1))
  done
  [[ "$conclusion" == success ]] || fail "$WF_PUBLISH run $id finished with '$conclusion'"
  say "run $id: success"
}

((DRY)) && printf 'DRY RUN: no command below is executed.\n'
banner 1; step_1
for n in 2 3 4 5 6 7 8 9 10 11; do
  ((n >= FROM)) || continue
  banner "$n"; "step_$n"
done
printf '\nDone: %s %s%s\n' "$VERSION" "$CHANNEL" "$( ((DRY)) && printf ' (dry run, nothing published)' || true)"
