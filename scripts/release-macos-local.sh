#!/usr/bin/env bash
# release-macos-local.sh: the ONE command that turns an existing CI-created
# desktop-v<VERSION> release into a macOS (Apple Silicon) release.
#
# macOS is deliberately not built in CI (release.yml matrix comment, task 1608):
# the Developer ID key never leaves the keychain of the Mac this runs on. The
# sequence below is the one docs/RELEASING.md describes by hand, made fail
# closed: every gate that can say "no" runs BEFORE the first upload, and
# nothing is published unless the notarized, stapled artifacts were verified.
#
# Usage (from the repo root of a release worktree, on an Apple Silicon Mac):
#   scripts/release-macos-local.sh <VERSION> <CHANNEL> [--dry-run] [--from-step N] [--skip-local-sig-check]
#     VERSION  plain semver, e.g. 0.8.7 (no -alpha/-beta suffix)
#     CHANNEL  alpha | beta | stable   (the manifest the release is published to)
#     --dry-run      print every command of every step, run none
#     --from-step N  resume at step N (1-13). Step 1 (the release-state gate:
#                    clean tree, HEAD == tag commit, release exists) ALWAYS runs.
#                    A resume past step 2 also requires the artifacts on disk to have been
#                    built from HEAD (step 2 records the commit).
#     --skip-local-sig-check  do not require minisign and do not verify the updater signature
#                    against the tarball in step 11. Explicit opt-out: the default is fail
#                    closed (no minisign = no release). Step 13 is only a CONSISTENCY check
#                    (manifest signature string == uploaded .sig), not proof that the
#                    signature verifies the tarball: only step 11 proves that.
#
# Preconditions: release.yml already ran for this VERSION (it creates the
# desktop-v<VERSION> tag + release with the Windows/Linux assets), and this
# worktree is checked out at that tag's commit. Nothing is committed: the
# tauri.conf.json version patch is applied for the build only and restored.
#
# Environment (defaults are the documented ones; no secret is stored here):
#   APPLE_SIGNING_IDENTITY  Developer ID identity (name or SHA-1 hash)
#                           [Developer ID Application: Devidee B.V. (R8352WDJJR)]
#   MACOS_APP_PROVISION_PROFILE, MACOS_FILE_PROVIDER_PROVISION_PROFILE
#                           [~/.private_keys/macos-developer-id-profiles/{BeebeebApp,BeebeebFileProvider}.provisionprofile]
#   BB_NOTARY_KEY_ID        App Store Connect API key id        [5KRWK96245]
#   BB_NOTARY_ISSUER        App Store Connect issuer id         [8cacf7df-c877-47db-aab2-7c9f1f9f5fda]
#   BB_NOTARY_KEY_PATH      the .p8 key file   [~/.private_keys/AuthKey_<key id>.p8]
#   BB_NOTARY_TIMEOUT       notarytool --timeout per submission [45m]
#   BB_MINISIGN             the minisign binary (REQUIRED unless --skip-local-sig-check: step 11
#                           verifies the updater signature before anything is published) [minisign]
#   BB_UPDATER_VERIFY_PUBKEY  the bare minisign public key (RW...) step 11 verifies the
#                           updater signature against. Default: the key baked into
#                           tauri.conf.json. Set it ONLY for a key-rotation transition
#                           release (docs/RELEASING.md), where CI still signs with the
#                           OLD key while the new one is already baked in.
#   BB_RELEASE_REPO         [beebeeb-io/desktop]   BB_RELEASES_REPO [beebeeb-io/releases]
#   BB_WORKFLOW_REF         ref the two workflows are dispatched on [main]
#   BB_POLL_INTERVAL        seconds between workflow polls [15]
#   BB_WORKFLOW_TIMEOUT     seconds to wait for each workflow [1800]

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

readonly TOTAL_STEPS=13
readonly STEP_NAMES=(
  ""
  "Verify release state (clean tree, HEAD == tag commit, release exists)"
  "Build the app (hardened runtime, version patched for the build only)"
  "Preflight the unnotarized .app"
  "Notarize the .app"
  "Staple the .app, then verify it (spctl + preflight)"
  "Rebuild the .dmg from the STAPLED app, sign it"
  "Notarize the .dmg"
  "Staple the .dmg, then verify it (spctl + preflight + contents)"
  "Recreate the updater Beebeeb.app.tar.gz from the STAPLED app"
  "Upload .dmg + updater bundle to the GitHub release"
  "Sign the updater bundle (CI workflow sign-macos-updater-artifact)"
  "Backfill the channel manifest (CI workflow release.yml, publish_existing)"
  "Assert the published manifest points darwin-aarch64 at this release"
)

# ---------------------------------------------------------------- arguments
DRY=0
FROM_STEP=1
SKIP_SIG_CHECK=0
VERSION=""
CHANNEL=""

usage() {
  sed -n '2,/^set -euo pipefail$/p' "$0" | sed -e '$d' -e 's/^# \{0,1\}//'
}

while (($#)); do
  case "$1" in
    --dry-run) DRY=1 ;;
    --skip-local-sig-check) SKIP_SIG_CHECK=1 ;;
    --from-step)
      shift
      [[ $# -gt 0 ]] || { echo "--from-step needs a number (1-$TOTAL_STEPS)" >&2; exit 2; }
      FROM_STEP="$1"
      ;;
    -h | --help) usage; exit 0 ;;
    -*) echo "unknown option: $1" >&2; exit 2 ;;
    *)
      if [[ -z "$VERSION" ]]; then VERSION="$1"
      elif [[ -z "$CHANNEL" ]]; then CHANNEL="$1"
      else echo "unexpected argument: $1" >&2; exit 2
      fi
      ;;
  esac
  shift
done

[[ -n "$VERSION" && -n "$CHANNEL" ]] || { usage >&2; exit 2; }
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] ||
  { echo "VERSION must be plain semver like 0.8.7 (got '$VERSION'); channels are not baked into versions" >&2; exit 2; }
case "$CHANNEL" in
  alpha) MANIFEST_FILE="alpha.json" ;;
  beta) MANIFEST_FILE="beta.json" ;;
  stable) MANIFEST_FILE="latest.json" ;;
  *) echo "CHANNEL must be alpha, beta or stable (got '$CHANNEL')" >&2; exit 2 ;;
esac
if [[ ! "$FROM_STEP" =~ ^[0-9]+$ ]] || ((FROM_STEP < 1 || FROM_STEP > TOTAL_STEPS)); then
  echo "--from-step must be a number from 1 to $TOTAL_STEPS (got '$FROM_STEP')" >&2
  exit 2
fi

# ---------------------------------------------------------------- settings
TAG="desktop-v$VERSION"
REPO="${BB_RELEASE_REPO:-beebeeb-io/desktop}"
RELEASES_REPO="${BB_RELEASES_REPO:-beebeeb-io/releases}"
WORKFLOW_REF="${BB_WORKFLOW_REF:-main}"
POLL_INTERVAL="${BB_POLL_INTERVAL:-15}"
WORKFLOW_TIMEOUT="${BB_WORKFLOW_TIMEOUT:-1800}"

SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:-Developer ID Application: Devidee B.V. (R8352WDJJR)}"
PROFILE_DIR_DEFAULT="$HOME/.private_keys/macos-developer-id-profiles"
APP_PROFILE="${MACOS_APP_PROVISION_PROFILE:-$PROFILE_DIR_DEFAULT/BeebeebApp.provisionprofile}"
EXT_PROFILE="${MACOS_FILE_PROVIDER_PROVISION_PROFILE:-$PROFILE_DIR_DEFAULT/BeebeebFileProvider.provisionprofile}"
NOTARY_KEY_ID="${BB_NOTARY_KEY_ID:-5KRWK96245}"
NOTARY_ISSUER="${BB_NOTARY_ISSUER:-8cacf7df-c877-47db-aab2-7c9f1f9f5fda}"
NOTARY_KEY_PATH="${BB_NOTARY_KEY_PATH:-$HOME/.private_keys/AuthKey_${NOTARY_KEY_ID}.p8}"
NOTARY_TIMEOUT="${BB_NOTARY_TIMEOUT:-45m}"

MINISIGN="${BB_MINISIGN:-minisign}"
VERIFY_PUBKEY_OVERRIDE="${BB_UPDATER_VERIFY_PUBKEY:-}"

CONF="src-tauri/tauri.conf.json"
TARGET_TRIPLE="aarch64-apple-darwin"
BUNDLE_DIR="src-tauri/target/$TARGET_TRIPLE/release/bundle"
APP="$BUNDLE_DIR/macos/Beebeeb.app"
WORK="$ROOT_DIR/src-tauri/target/release-macos-local/$VERSION"
DMG_NAME="Beebeeb_${VERSION}_${TARGET_TRIPLE%%-*}.dmg"
DMG="$WORK/$DMG_NAME"
# The commit step 2 built from. A resume (--from-step N > 2) reuses those artifacts and
# step 1 refuses it unless this equals HEAD (the tag may have been re-cut since).
BUILD_STAMP="$WORK/build-commit"
TARBALL="$WORK/Beebeeb.app.tar.gz"
SIG_NAME="Beebeeb.app.tar.gz.sig"
PREFLIGHT="scripts/macos-release-preflight.sh"

# ---------------------------------------------------------------- plumbing
CURRENT_STEP=0
HINT=""
UPLOADED=0
PUBLISHED=0
CONF_BACKUP=""
DIST_BACKUP=""
CAPTURE_RC=0
LAST_RUN_ID=""
RELEASE_COMMIT=""
NOTARY_ID_APP=""
NOTARY_ID_DMG=""
SIGN_RUN_ID=""
PUBLISH_RUN_ID=""

fail() {
  {
    printf '\nFAILED at step %s/%s (%s)\n' "$CURRENT_STEP" "$TOTAL_STEPS" "${STEP_NAMES[$CURRENT_STEP]:-setup}"
    printf '  %s\n' "$1"
    [[ -n "$HINT" ]] && printf '  Remediation: %s\n' "$HINT"
    if ((PUBLISHED)); then
      printf '  State: the release workflow was dispatched (it may or may not have finished); check the channel manifest before anything else.\n'
    elif ((UPLOADED)); then
      printf '  State: assets were uploaded to %s but the manifest was NOT published.\n' "$TAG"
    else
      printf '  State: nothing was uploaded to GitHub and nothing was published.\n'
    fi
    printf '  Resume: %s %s %s --from-step %s\n' "scripts/release-macos-local.sh" "$VERSION" "$CHANNEL" "$CURRENT_STEP"
  } >&2
  exit 1
}

banner() {
  CURRENT_STEP="$1"
  printf '\n==> [%s/%s] %s\n' "$1" "$TOTAL_STEPS" "${STEP_NAMES[$1]}"
}

say() { printf '    %s\n' "$*"; }

is_dry() { ((DRY)); }

# Shell-quote only what needs it, so a printed command is readable AND copy-pasteable.
quote() {
  case "$1" in
    '' | *[!A-Za-z0-9_@%+=:,./-]*) printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")" ;;
    *) printf '%s' "$1" ;;
  esac
}

show() {
  local out="" a
  for a in "$@"; do out="$out $(quote "$a")"; done
  printf '    +%s\n' "$out"
}

# run <cmd...>: print it, run it unless --dry-run, fail (with the step's HINT) if it fails.
run() {
  show "$@"
  is_dry && return 0
  local rc=0
  "$@" || rc=$?
  ((rc == 0)) || fail "command failed (exit $rc): $*"
}

# capture <var> <cmd...>: stdout into var; exit code in CAPTURE_RC (caller decides).
capture() {
  local __var="$1" __out=""
  shift
  show "$@"
  CAPTURE_RC=0
  if is_dry; then printf -v "$__var" '%s' ""; return 0; fi
  __out="$("$@")" || CAPTURE_RC=$?
  printf -v "$__var" '%s' "$__out"
}

# capture_all <var> <cmd...>: stdout+stderr into var (spctl/codesign write to stderr).
capture_all() {
  local __var="$1" __out=""
  shift
  show "$@"
  CAPTURE_RC=0
  if is_dry; then printf -v "$__var" '%s' ""; return 0; fi
  __out="$("$@" 2>&1)" || CAPTURE_RC=$?
  printf -v "$__var" '%s' "$__out"
}

# b64decode: base64 on stdin -> decoded bytes on stdout. The macOS base64(1) decodes with -D
# (older releases know nothing else), GNU coreutils with -d/--decode, and on some macOS versions
# -d means something else, so try -D first and fall back to -d. Input is read once so the
# retry sees it again.
b64decode() {
  local in=""
  in="$(cat)"
  printf '%s' "$in" | base64 -D 2>/dev/null || printf '%s' "$in" | base64 -d
}

need_cmd() {
  if is_dry; then say "would require command: $1"; return 0; fi
  command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

need_file() { # path, what
  if is_dry; then say "would require $2: $1"; return 0; fi
  [[ -e "$1" ]] || fail "$2 not found: $1"
}

sha256_of() {
  if command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | awk '{print $1}'
  else sha256sum "$1" | awk '{print $1}'
  fi
}

# The build patches tauri.conf.json and rewrites the tracked dist/index.html (a committed
# vite output). Both are put back the moment the build ends, and again on ANY exit.
restore_tracked() {
  if [[ -n "$CONF_BACKUP" && -f "$CONF_BACKUP" ]]; then
    cp "$CONF_BACKUP" "$ROOT_DIR/$CONF"
    CONF_BACKUP=""
  fi
  if [[ -n "$DIST_BACKUP" && -f "$DIST_BACKUP" ]]; then
    cp "$DIST_BACKUP" "$ROOT_DIR/dist/index.html"
    DIST_BACKUP=""
  fi
}
trap restore_tracked EXIT
trap 'exit 130' INT TERM

# Safety net: any bare command that fails in the main shell stops with the step banner,
# the command and the resume line, never a silent exit. (Subshells keep plain set -e.)
# (BASH_SUBSHELL, not BASHPID: macOS ships bash 3.2, which has no BASHPID.)
on_err() {
  local rc=$?
  [[ "$BASH_SUBSHELL" == 0 ]] || return 0
  fail "unexpected error (exit $rc) running: $BASH_COMMAND (line ${BASH_LINENO[0]})"
}
set -E
trap on_err ERR


# ---------------------------------------------------------------- helpers
# notarize <file> <label>: submit, wait, require status Accepted. Sets NOTARY_ID_APP / NOTARY_ID_DMG.
notarize() {
  local file="$1" label="$2"
  local out="$WORK/notary-$label.json" err="$WORK/notary-$label.err" log="$WORK/notary-$label-log.json"
  local rc=0 id="" status="" issues=""
  local creds=(--key "$NOTARY_KEY_PATH" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER")
  local cmd=(xcrun notarytool submit "$file" "${creds[@]}" --wait --timeout "$NOTARY_TIMEOUT" --output-format json)

  HINT="Apple's notary service did not accept the $label. Read $log (the 'issues' list; the usual cause is a nested binary without hardened runtime or a secure timestamp) and $err, fix that, and resume from the step that builds the $label."
  show "${cmd[@]}"
  if is_dry; then say "(dry-run) would require status Accepted in the JSON result, else fetch 'notarytool log' and stop"; return 0; fi

  "${cmd[@]}" >"$out" 2>"$err" || rc=$?
  id="$(jq -r '.id // empty' "$out" 2>/dev/null || true)"
  status="$(jq -r '.status // empty' "$out" 2>/dev/null || true)"
  if [[ "$status" != "Accepted" ]]; then
    if [[ -n "$id" ]]; then
      show xcrun notarytool log "$id" "${creds[@]}" "$log"
      xcrun notarytool log "$id" "${creds[@]}" "$log" >/dev/null 2>&1 || true
      issues="$(jq -r '(.issues // [])[]? | "    - " + (.path // "?") + ": " + (.message // "?")' "$log" 2>/dev/null | head -20 || true)"
    fi
    fail "notarization of the $label was not Accepted: status '${status:-none}', notarytool exit $rc, submission id '${id:-none}'.${issues:+
$issues}"
  fi
  ((rc == 0)) || fail "notarytool exited $rc even though the JSON said Accepted; not trusting it (see $err)"
  say "notarized $label: Accepted (submission $id)"
  case "$label" in
    app) NOTARY_ID_APP="$id" ;;
    dmg) NOTARY_ID_DMG="$id" ;;
  esac
}

# staple_and_validate <path>: staple the ticket, then require `stapler validate` to pass.
staple_and_validate() {
  HINT="The ticket could not be stapled or validated. A fresh notarization is usually needed: resume from the notarize step for this artifact."
  run xcrun stapler staple "$1"
  run xcrun stapler validate "$1"
}

# require_notarized <kind: execute|install> <path>: spctl must accept with source=Notarized Developer ID.
require_notarized() {
  local kind="$1" path="$2" out=""
  HINT="Gatekeeper does not see $path as a notarized Developer ID artifact, so users would get a warning. Do not upload. Re-check the notarization and stapling steps for it."
  if [[ "$kind" == "install" ]]; then
    capture_all out spctl -a -vv -t install "$path"
  else
    capture_all out spctl -a -vv "$path"
  fi
  is_dry && { say "(dry-run) would require 'accepted' and 'source=Notarized Developer ID'"; return 0; }
  printf '%s\n' "$out" | sed 's/^/      /'
  ((CAPTURE_RC == 0)) || fail "spctl rejected $path (exit $CAPTURE_RC)"
  grep -q 'source=Notarized Developer ID' <<<"$out" ||
    fail "spctl accepted $path but not as 'source=Notarized Developer ID'"
}

# Plain reader for the checks that run inside other checks (no echo of the command).
app_version() { plutil -extract CFBundleShortVersionString raw -o - "$1/Contents/Info.plist"; }

require_app_version() { # <path to .app> <label>
  local got=""
  HINT="The bundle reports a different version than the release. The tauri.conf.json patch must reach the build: resume from step 2 (build) and check the 'jq -r .version' line printed there."
  capture got plutil -extract CFBundleShortVersionString raw -o - "$1/Contents/Info.plist"
  is_dry && { say "(dry-run) would require CFBundleShortVersionString == $VERSION"; return 0; }
  ((CAPTURE_RC == 0)) || fail "could not read CFBundleShortVersionString from $2"
  [[ "$got" == "$VERSION" ]] || fail "$2 reports CFBundleShortVersionString '$got', expected '$VERSION'"
  say "$2 CFBundleShortVersionString = $got"
}

# dispatch_and_wait <workflow file> <gh workflow run args...>: dispatch, identify the run THIS
# dispatch created, wait for it to succeed. Recent gh prints the created run's URL; its id is
# the identity. Without a URL the run is found by elimination (newer than a snapshot taken before
# the dispatch) and only if exactly one candidate exists: another dispatch of the same workflow
# running at the same moment makes that ambiguous, and the script then stops rather than wait on
# (and report the result of) somebody else's run.
dispatch_and_wait() {
  local wf="$1" list="" before=0 id="" view="" status="" conclusion="" url="" waited=0 out="" cands="" n=0
  shift
  capture list gh run list --repo "$REPO" --workflow "$wf" --event workflow_dispatch --limit 20 --json databaseId
  ((CAPTURE_RC == 0)) || is_dry || fail "could not list runs of $wf"
  is_dry || before="$(jq '[.[].databaseId] | max // 0' <<<"$list")"
  capture_all out gh workflow run "$wf" --repo "$REPO" --ref "$WORKFLOW_REF" "$@"
  if is_dry; then say "(dry-run) would take the run id from the URL 'gh workflow run' prints (else the one new run of $wf), then 'gh run view' until it completes with conclusion success"; return 0; fi
  ((CAPTURE_RC == 0)) || fail "command failed (exit $CAPTURE_RC): gh workflow run $wf: $out"

  [[ -z "$out" ]] || printf '%s\n' "$out" | sed 's/^/    /'
  id="$(grep -oE '/actions/runs/[0-9]+' <<<"$out" | head -1 | grep -oE '[0-9]+$' || true)"
  if [[ -z "$id" ]]; then
    say "no run URL from gh; looking for the one new run of $wf"
    while :; do
      list="$(gh run list --repo "$REPO" --workflow "$wf" --event workflow_dispatch --limit 20 --json databaseId 2>/dev/null || true)"
      cands="$(jq -r --argjson b "$before" '[.[] | select(.databaseId > $b) | .databaseId] | sort | .[]' <<<"${list:-[]}" 2>/dev/null || true)"
      n="$(grep -c . <<<"$cands" || true)"
      ((n >= 1)) && break
      ((waited < 300)) || fail "the dispatched run of $wf never appeared within 300 s"
      sleep "$POLL_INTERVAL"
      waited=$((waited + POLL_INTERVAL + 1))
    done
    ((n == 1)) || fail "$n runs of $wf appeared after the dispatch ($(tr '\n' ' ' <<<"$cands")) and gh gave no run URL, so I cannot tell which one is ours. Another dispatch of the same workflow is running; wait for it to finish, then resume with --from-step $CURRENT_STEP. Nothing was guessed."
    id="$cands"
  fi
  say "run $id started: https://github.com/$REPO/actions/runs/$id"
  waited=0
  while :; do
    view="$(gh run view "$id" --repo "$REPO" --json status,conclusion,url 2>/dev/null || true)"
    [[ -n "$view" ]] || view='{}'
    status="$(jq -r '.status // empty' <<<"$view" 2>/dev/null || true)"
    conclusion="$(jq -r '.conclusion // empty' <<<"$view" 2>/dev/null || true)"
    url="$(jq -r '.url // empty' <<<"$view" 2>/dev/null || true)"
    [[ "$status" == "completed" ]] && break
    ((waited < WORKFLOW_TIMEOUT)) || fail "$wf run $id did not finish within ${WORKFLOW_TIMEOUT}s (status '${status:-unknown}')"
    sleep "$POLL_INTERVAL"
    waited=$((waited + POLL_INTERVAL + 1))
  done
  LAST_RUN_ID="$id"
  [[ "$conclusion" == "success" ]] ||
    fail "$wf run $id finished with conclusion '$conclusion' (${url:-https://github.com/$REPO/actions/runs/$id}); read it with: gh run view $id --repo $REPO --log-failed"
  say "run $id: success"
}
# ---------------------------------------------------------------- steps
step_1_verify_release_state() {
  local head="" refs="" peeled="" direct="" tag_sha="" notes_bad="" rel="" missing="" t built_commit="" flat=""

  HINT="Install/sign in the missing tool (minisign: brew install minisign) or provide the missing file, then re-run."
  for t in git gh jq bun bunx xcrun codesign spctl hdiutil ditto plutil security tar; do need_cmd "$t"; done
  if ((SKIP_SIG_CHECK)); then
    say "WARNING: --skip-local-sig-check: the updater signature will NOT be verified against the tarball before publishing (step 13 only compares strings)"
  else
    need_cmd "$MINISIGN"
  fi
  if [[ -n "$VERIFY_PUBKEY_OVERRIDE" ]] && ! [[ "$VERIFY_PUBKEY_OVERRIDE" =~ ^RW[A-Za-z0-9+/]+={0,2}$ ]]; then
    fail "BB_UPDATER_VERIFY_PUBKEY is not a minisign public key (expected the bare base64 key starting RW, the second line of the .pub file)"
  fi
  if is_dry; then
    say "would require: Darwin arm64 (Apple Silicon only; Intel is not built)"
  else
    [[ "$(uname -s)" == "Darwin" && "$(uname -m)" == "arm64" ]] ||
      fail "this must run on an Apple Silicon Mac (uname says $(uname -s) $(uname -m))"
  fi
  run gh auth status

  capture_all t security find-identity -v -p codesigning
  if ! is_dry; then
    grep -qF -- "$SIGNING_IDENTITY" <<<"$t" ||
      fail "signing identity not found in the keychain: $SIGNING_IDENTITY (set APPLE_SIGNING_IDENTITY to the Developer ID hash or full name)"
  fi
  need_file "$APP_PROFILE" "app provisioning profile (MACOS_APP_PROVISION_PROFILE)"
  need_file "$EXT_PROFILE" "File Provider provisioning profile (MACOS_FILE_PROVIDER_PROVISION_PROFILE)"
  need_file "$NOTARY_KEY_PATH" "App Store Connect API key (BB_NOTARY_KEY_PATH)"

  HINT="Build from a clean checkout of the release commit: 'git worktree add <path> --detach $TAG' (fetch the tag first), cd into it, re-run. If tauri.conf.json shows as modified, an earlier run was killed mid-build: restore it with 'git checkout -- $CONF'."
  capture t git status --porcelain
  if ! is_dry; then
    [[ -z "$t" ]] || fail "working tree is not clean:
$(printf '%s\n' "$t" | head -20 | sed 's/^/      /')"
  fi

  HINT="The Windows/Linux build for $VERSION must exist first: run release.yml (publish_existing=false) and wait for it, then 'git fetch origin tag $TAG' and check out that commit. The macOS build must come from exactly the commit CI built."
  capture head git rev-parse HEAD
  capture refs git ls-remote --tags origin "refs/tags/$TAG" "refs/tags/$TAG^{}"
  if ! is_dry; then
    peeled="$(awk -v r="refs/tags/$TAG^{}" '$2 == r {print $1}' <<<"$refs")"
    direct="$(awk -v r="refs/tags/$TAG" '$2 == r {print $1}' <<<"$refs")"
    tag_sha="${peeled:-$direct}"
    [[ -n "$tag_sha" ]] || fail "tag $TAG does not exist on origin"
    [[ "$head" == "$tag_sha" ]] ||
      fail "HEAD ($head) is not the commit tag $TAG points to ($tag_sha); the Mac build would not match the Windows/Linux build"
    say "HEAD == $TAG == $head"
    RELEASE_COMMIT="$head"
  fi

  # A resume past step 2 reuses the app/dmg/tarball on disk, which are keyed by VERSION only.
  # If the tag was re-cut since they were built, they come from another commit than the one
  # CI built; nothing else would notice (the version string is the same).
  HINT="The artifacts under $WORK do not come from HEAD. Rebuild from the current commit: scripts/release-macos-local.sh $VERSION $CHANNEL (a full run, or --from-step 2). Do not resume past step 2 after the tag moved."
  if ((FROM_STEP > 2)); then
    if is_dry; then
      say "would require $BUILD_STAMP to exist and equal HEAD (--from-step $FROM_STEP reuses the artifacts step 2 built)"
    else
      [[ -f "$BUILD_STAMP" ]] ||
        fail "--from-step $FROM_STEP reuses the artifacts of step 2, but $BUILD_STAMP (the commit they were built from) does not exist"
      built_commit="$(<"$BUILD_STAMP")"
      [[ "$built_commit" == "$head" ]] ||
        fail "the artifacts under $WORK were built from commit $built_commit, but HEAD is $head (the tag was re-cut or this is another checkout); resuming would upload a build of the wrong commit"
      say "artifacts on disk were built from $built_commit == HEAD"
    fi
  fi

  HINT="RELEASE_NOTES.md at the release commit must name $VERSION, carry no '<!-- lead:' markers, and its Verification section must carry real test counts ('<N> pass / 0 fail' for bun test, 'test result: ok. <N> passed' for cargo test). The lead resolves all of that before the release commit is cut. Fix on main, re-cut the release, then build from the new tag."
  if is_dry; then
    say "would require RELEASE_NOTES.md to mention $VERSION, contain no '<!-- lead:' marker, and carry real test counts ('<N> pass / 0 fail', 'test result: ok. <N> passed')"
  else
    [[ -f RELEASE_NOTES.md ]] || fail "RELEASE_NOTES.md is missing"
    grep -qF -- "$VERSION" RELEASE_NOTES.md || fail "RELEASE_NOTES.md does not mention $VERSION"
    notes_bad="$(grep -n '<!-- lead:' RELEASE_NOTES.md || true)"
    [[ -z "$notes_bad" ]] || fail "RELEASE_NOTES.md still has unresolved lead markers:
$(printf '%s\n' "$notes_bad" | sed 's/^/      /')"
    # The marker is only a reminder; the counts are what it stands for. Reading the normalised
    # text (line breaks and runs of blanks collapsed) keeps a hard-wrapped bullet valid, and a
    # zero count is rejected: a run that executed nothing is not a green.
    flat="$(tr '\n' ' ' <RELEASE_NOTES.md | tr -s ' ')"
    grep -qE '(^|[^0-9])[1-9][0-9]* pass / 0 fail' <<<"$flat" ||
      fail "RELEASE_NOTES.md Verification section carries no test counts: no '<N> pass / 0 fail' for bun test (a placeholder such as 'pass / 0 fail' without the number is not a count)"
    grep -qE 'test result: ok\. [1-9][0-9]* passed' <<<"$flat" ||
      fail "RELEASE_NOTES.md Verification section carries no test counts: no 'test result: ok. <N> passed' for cargo test (a literal 'N passed' is not a count)"
    say "RELEASE_NOTES.md names $VERSION, has no lead markers, and carries test counts"
  fi

  HINT="The CI run must have created the release with the Windows/Linux assets before the Mac part starts. Run release.yml first."
  capture rel gh release view "$TAG" --repo "$REPO" --json isDraft,assets
  if ! is_dry; then
    ((CAPTURE_RC == 0)) || fail "release $TAG not found in $REPO"
    [[ "$(jq -r '.isDraft' <<<"$rel")" == "false" ]] || fail "release $TAG is still a draft"
    for t in "Beebeeb_${VERSION}_amd64.AppImage.sig" "Beebeeb_${VERSION}_x64-setup.exe.sig" "Beebeeb_${VERSION}_x64_en-US.msi.sig"; do
      jq -e --arg n "$t" 'any(.assets[]; .name == $n)' <<<"$rel" >/dev/null || missing="$missing $t"
    done
    [[ -z "$missing" ]] || fail "release $TAG is missing the Windows/Linux signatures:$missing (the manifest backfill fails closed without them)"
    say "release $TAG exists with its Windows/Linux signatures"
  fi
  run mkdir -p "$WORK"
}
step_2_build() {
  local tmp="$CONF.tmp"
  HINT="Fix the build error above, then resume with --from-step 2. tauri.conf.json is restored automatically."
  run bun install --frozen-lockfile

  # Nothing of an earlier attempt may survive into this build: a stale .app (or a stamp that
  # vouches for one) is how a resume would upload a build of the wrong commit.
  run rm -f "$BUILD_STAMP"
  run rm -rf "$BUNDLE_DIR"

  say "patching $CONF version -> $VERSION (build only, restored on exit, never committed)"
  if ! is_dry; then
    need_file "$WORK" "work dir"
    CONF_BACKUP="$WORK/tauri.conf.json.orig"
    cp "$CONF" "$CONF_BACKUP"
    DIST_BACKUP="$WORK/dist-index.html.orig"
    cp dist/index.html "$DIST_BACKUP"
  fi
  # shellcheck disable=SC2016  # $version is a jq variable, not a shell one
  show jq --arg version "$VERSION" '.version = $version' "$CONF"
  if ! is_dry; then
    jq --arg version "$VERSION" '.version = $version' "$CONF" >"$tmp" || fail "could not patch $CONF"
    mv "$tmp" "$CONF"
    [[ "$(jq -r .version "$CONF")" == "$VERSION" ]] || fail "$CONF did not take version $VERSION"
    say "$CONF .version = $(jq -r .version "$CONF")"
  fi

  # Only the .app is built here, and updater artifacts are switched off for this build by a CLI
  # override (the file keeps createUpdaterArtifacts true: macos-release-preflight.sh asserts it,
  # and the Windows/Linux CI build needs it).
  #   - Why off: with createUpdaterArtifacts on and a pubkey configured, Tauri 2 builds the .app
  #     and the .tar.gz and then exits non-zero ("A public key has been found, but no private
  #     key") because TAURI_SIGNING_PRIVATE_KEY is a CI-only secret since the 2026-09-25 rotation
  #     (workspace evidence: task 1524 gate 63, tasks 0341-0354). Step 9 recreates the tarball
  #     from the stapled app and the sign workflow signs it in CI, so nothing is lost.
  #   - Why --bundles app: the default ("all") also runs Tauri's bundle_dmg.sh, which drives
  #     Finder over AppleScript and failed with -10006 (task 1608). Step 6 builds the dmg with
  #     hdiutil from the stapled app; Tauri's dmg would be discarded anyway.
  # The updater signing variables are stripped so the private key can never reach this build.
  run env \
    -u APPLE_ID -u APPLE_PASSWORD -u APPLE_TEAM_ID -u APPLE_API_KEY -u APPLE_API_ISSUER -u APPLE_API_KEY_PATH \
    -u APPLE_CERTIFICATE -u APPLE_CERTIFICATE_PASSWORD \
    -u TAURI_SIGNING_PRIVATE_KEY -u TAURI_SIGNING_PRIVATE_KEY_PASSWORD -u TAURI_SIGNING_PRIVATE_KEY_PATH \
    "APPLE_SIGNING_IDENTITY=$SIGNING_IDENTITY" \
    "MACOS_APP_PROVISION_PROFILE=$APP_PROFILE" \
    "MACOS_FILE_PROVIDER_PROVISION_PROFILE=$EXT_PROFILE" \
    "BEEBEEB_RELEASE_VERSION=$VERSION" \
    "BEEBEEB_MACOS_HARDENED_RUNTIME=1" \
    bunx tauri build --target "$TARGET_TRIPLE" --bundles app \
    --config '{"bundle":{"createUpdaterArtifacts":false}}' -- --locked

  if ! is_dry; then
    restore_tracked
    git diff --quiet || fail "the working tree is not back to the committed content after the build: $(git status --porcelain | head -5 | tr '\n' ' ')"
    say "$CONF and dist/index.html restored; working tree clean"
    [[ -d "$APP" ]] || fail "the build did not produce $APP"
    git rev-parse HEAD >"$BUILD_STAMP"
    say "artifacts built from $(<"$BUILD_STAMP") (recorded in $BUILD_STAMP; a resume past this step requires it to equal HEAD)"
  fi
}

step_3_preflight_unnotarized_app() {
  HINT="The unnotarized app failed a local trust check (hardened runtime on the app, the extension and the CTL helper; provisioning profiles; File Provider entry point). Fix and rebuild: resume with --from-step 2."
  need_file "$APP" "built app (run step 2 first)"
  require_app_version "$APP" "the built app"
  # spctl cannot accept an app Apple has not notarized yet, so the execute
  # assessment is skipped here and enforced in step 5, after stapling.
  run env BB_PREFLIGHT_SKIP_SPCTL_EXECUTE=1 "$PREFLIGHT" "$APP"
}

step_4_notarize_app() {
  need_file "$APP" "built app (run step 2 first)"
  run rm -f "$WORK/Beebeeb-app.zip"
  run ditto -c -k --keepParent "$APP" "$WORK/Beebeeb-app.zip"
  notarize "$WORK/Beebeeb-app.zip" app
}

step_5_staple_verify_app() {
  need_file "$APP" "built app (run step 2 first)"
  staple_and_validate "$APP"
  HINT="The stapled app fails a signature check. Rebuild from step 2."
  run codesign --verify --deep --strict --verbose=2 "$APP"
  require_notarized execute "$APP"
  HINT="The stapled app fails the full preflight. Do not upload; rebuild from step 2."
  run "$PREFLIGHT" "$APP"
}

step_6_build_dmg() {
  local stage="$WORK/dmg-stage" details=""
  need_file "$APP" "built app (run step 2 first)"
  HINT="The dmg could not be rebuilt or signed. Fix the error and resume with --from-step 6 (the stapled app from step 5 is reused)."
  run rm -rf "$stage" "$DMG"
  run mkdir -p "$stage"
  # The dmg is rebuilt from the STAPLED app (ditto keeps the ticket, xattrs and the signature),
  # so the ticket travels inside the dmg as well. hdiutil, not Tauri's bundle_dmg.sh: that one
  # drives Finder via AppleScript and fails with -10006 without Automation permission (task 1608).
  run ditto "$APP" "$stage/Beebeeb.app"
  run ln -s /Applications "$stage/Applications"
  run xcrun stapler validate "$stage/Beebeeb.app"
  run hdiutil create -volname Beebeeb -srcfolder "$stage" -fs HFS+ -format UDZO -ov "$DMG"
  run codesign --force --sign "$SIGNING_IDENTITY" --timestamp "$DMG"
  run codesign --verify --strict --verbose=2 "$DMG"
  capture_all details codesign -dvv "$DMG"
  if ! is_dry; then
    grep -q 'Authority=Developer ID Application:' <<<"$details" ||
      fail "the dmg signature does not show a Developer ID Application authority"
  fi
  run rm -rf "$stage"
}

step_7_notarize_dmg() {
  need_file "$DMG" "rebuilt dmg (run step 6 first)"
  notarize "$DMG" dmg
}

# Mount the dmg read-only, prove the app inside is stapled and has the right version, detach.
verify_dmg_contents() {
  local mnt="$WORK/dmg-mount" rc=0 got=""
  HINT="The app inside the dmg is not the stapled, correctly versioned app. Do not upload; resume with --from-step 6."
  run rm -rf "$mnt"
  run mkdir -p "$mnt"
  run hdiutil attach -readonly -nobrowse -noverify -mountpoint "$mnt" "$DMG"
  if is_dry; then
    say "(dry-run) would require the mounted app to pass 'stapler validate' and report version $VERSION, then detach"
    run hdiutil detach "$mnt"
    return 0
  fi
  if ! xcrun stapler validate "$mnt/Beebeeb.app" >/dev/null 2>&1; then
    say "the app inside the dmg is not stapled"
    rc=1
  elif ! got="$(app_version "$mnt/Beebeeb.app")"; then
    say "cannot read the version of the app inside the dmg"
    rc=1
  elif [[ "$got" != "$VERSION" ]]; then
    say "the app inside the dmg reports version '$got', expected '$VERSION'"
    rc=1
  fi
  hdiutil detach "$mnt" >/dev/null 2>&1 || hdiutil detach -force "$mnt" >/dev/null 2>&1 || say "warning: could not detach $mnt"
  ((rc == 0)) || fail "contents check of the dmg failed (see above)"
  say "dmg contains the stapled app, version $got"
}

step_8_staple_verify_dmg() {
  need_file "$DMG" "rebuilt dmg (run step 6 first)"
  staple_and_validate "$DMG"
  require_notarized install "$DMG"
  HINT="The dmg fails the full preflight (hdiutil verify + spctl). Do not upload; resume with --from-step 6."
  run "$PREFLIGHT" "$DMG"
  verify_dmg_contents
}

step_9_updater_bundle() {
  local new_tops="" extract="$WORK/updater-verify" listing=""
  need_file "$APP" "built app (run step 2 first)"
  HINT="The updater bundle could not be recreated or does not have the layout Tauri's updater expects (a single top-level Beebeeb.app). Inspect it with 'tar -tzf $TARBALL | head'."
  run rm -f "$TARBALL"
  # Step 2 builds with updater artifacts off (no private key on this Mac), so Tauri wrote no
  # tarball; this is the only one. One top-level Beebeeb.app directory, made from the STAPLED
  # app, is the layout Tauri's own updater bundle has and the updater extracts.
  run env COPYFILE_DISABLE=1 tar -czf "$TARBALL" -C "$(dirname "$APP")" Beebeeb.app
  capture listing tar -tzf "$TARBALL"
  if is_dry; then
    say "(dry-run) would require every entry to sit under a single top-level Beebeeb.app/, and the extracted app to be stapled with version $VERSION"
    return 0
  fi
  new_tops="$(cut -d/ -f1 <<<"$listing" | sort -u)"
  [[ "$new_tops" == "Beebeeb.app" ]] || fail "updater bundle has unexpected top-level entries: $new_tops"
  rm -rf "$extract"
  mkdir -p "$extract"
  tar -xzf "$TARBALL" -C "$extract"
  xcrun stapler validate "$extract/Beebeeb.app" >/dev/null 2>&1 || fail "the app inside the updater bundle is not stapled"
  [[ "$(app_version "$extract/Beebeeb.app")" == "$VERSION" ]] || fail "the app inside the updater bundle is not version $VERSION"
  rm -rf "$extract"
  say "updater bundle: top level Beebeeb.app, stapled, version $VERSION"
}

step_10_upload() {
  local asset f got="" rel=""
  need_file "$DMG" "rebuilt dmg (run step 6 first)"
  need_file "$TARBALL" "updater bundle (run step 9 first)"
  HINT="The upload or its read-back failed. It is safe to resume with --from-step 10 (assets are replaced with --clobber)."
  # A .sig from an earlier attempt belongs to an earlier tarball: drop it so the
  # sign step below is the only possible source of the signature that gets published.
  capture rel gh release view "$TAG" --repo "$REPO" --json assets
  if is_dry || jq -e --arg n "$SIG_NAME" 'any(.assets[]; .name == $n)' <<<"$rel" >/dev/null 2>&1; then
    run gh release delete-asset "$TAG" "$SIG_NAME" --repo "$REPO" --yes
  fi
  UPLOADED=1
  run gh release upload "$TAG" "$DMG" "$TARBALL" --repo "$REPO" --clobber
  run rm -rf "$WORK/readback"
  run mkdir -p "$WORK/readback"
  for asset in "$DMG_NAME" "Beebeeb.app.tar.gz"; do
    run gh release download "$TAG" --repo "$REPO" --pattern "$asset" --dir "$WORK/readback" --clobber
    if ! is_dry; then
      f="$WORK/$asset"
      got="$(sha256_of "$WORK/readback/$asset")"
      [[ "$got" == "$(sha256_of "$f")" ]] || fail "$asset downloaded from the release differs from the local file"
      say "$asset read back from the release: sha256 $got"
    fi
  done
}

step_11_sign_updater_bundle() {
  local sig_file="$WORK/readback/$SIG_NAME" rel="" pub="" key="" pub_from=""
  HINT="The signing workflow failed or did not attach $SIG_NAME, or the signature does not verify. Check the run above (the secrets TAURI_SIGNING_PRIVATE_KEY/_PASSWORD live only in the repo's CI), then resume with --from-step 11."
  dispatch_and_wait sign-macos-updater-artifact.yml -f "release_tag=$TAG"
  SIGN_RUN_ID="$LAST_RUN_ID"
  run rm -rf "$WORK/readback"
  run mkdir -p "$WORK/readback"
  capture rel gh release view "$TAG" --repo "$REPO" --json assets
  if is_dry; then
    if ((SKIP_SIG_CHECK)); then
      say "(dry-run) would require $SIG_NAME among the release assets and download it; local signature check SKIPPED (--skip-local-sig-check)"
    else
      say "(dry-run) would require $SIG_NAME among the release assets, download it, and verify it against the tarball with minisign ($MINISIGN, required) before anything is published"
    fi
    run gh release download "$TAG" --repo "$REPO" --pattern "$SIG_NAME" --dir "$WORK/readback" --clobber
    return 0
  fi
  jq -e --arg n "$SIG_NAME" 'any(.assets[]; .name == $n)' <<<"$rel" >/dev/null ||
    fail "the signing run succeeded but the release has no $SIG_NAME asset; not publishing a manifest without a signature"
  run gh release download "$TAG" --repo "$REPO" --pattern "$SIG_NAME" --dir "$WORK/readback" --clobber
  [[ -s "$sig_file" ]] || fail "$SIG_NAME downloaded from the release is empty"
  if ((SKIP_SIG_CHECK)); then
    say "--skip-local-sig-check: signature NOT verified against the tarball; step 13 will only show the manifest carries the uploaded $SIG_NAME (consistency, not validity)"
    return 0
  fi
  # Step 10 proved the uploaded tarball is byte-identical to $TARBALL, so verifying the local file
  # verifies what users will download. The check is a hard requirement (step 1 needs minisign
  # unless --skip-local-sig-check): comparing the manifest string with the .sig in step 13 only
  # shows consistency, not validity.
  if [[ -n "$VERIFY_PUBKEY_OVERRIDE" ]]; then
    pub="$VERIFY_PUBKEY_OVERRIDE"
    pub_from="BB_UPDATER_VERIFY_PUBKEY (NOT the key baked into $CONF; correct only for a key-rotation transition release, where CI still signs with the old key)"
  else
    pub="$(jq -r .plugins.updater.pubkey "$CONF" | b64decode | sed -n 2p)"
    pub_from="the pubkey baked into $CONF"
  fi
  [[ -n "$pub" ]] || fail "could not read the updater public key from $CONF (and BB_UPDATER_VERIFY_PUBKEY is not set)"
  key="$WORK/readback/tarball.minisig"
  b64decode <"$sig_file" >"$key" || fail "$SIG_NAME is not valid base64 (a Tauri updater signature is the base64 of a minisign file)"
  say "verifying against $pub_from"
  run "$MINISIGN" -Vm "$TARBALL" -x "$key" -P "$pub"
  say "signature verified against $pub_from"
}

step_12_publish_manifest() {
  HINT="The manifest backfill failed. Read the run, and read the channel manifest before anything else (see step 13's command) to know what users would be offered; then resume with --from-step 12."
  PUBLISHED=1
  dispatch_and_wait release.yml -f "version=$VERSION" -f "channel=$CHANNEL" -f publish_existing=true
  PUBLISH_RUN_ID="$LAST_RUN_ID"
}

step_13_assert_manifest() {
  local manifest="" want_url="https://github.com/$REPO/releases/download/$TAG/Beebeeb.app.tar.gz" sig="" msig=""
  local sigdir="$WORK/manifest-check"
  HINT="The channel manifest does not point darwin-aarch64 at $TAG. Read it with: gh api -H 'Accept: application/vnd.github.raw+json' repos/$RELEASES_REPO/contents/desktop/$MANIFEST_FILE ; re-run release.yml (publish_existing=true) if it is stale. Note releases.beebeeb.io can lag the repo by about 90 s."
  run rm -rf "$sigdir"
  run mkdir -p "$sigdir"
  run gh release download "$TAG" --repo "$REPO" --pattern "$SIG_NAME" --dir "$sigdir" --clobber
  capture manifest gh api -H "Accept: application/vnd.github.raw+json" "repos/$RELEASES_REPO/contents/desktop/$MANIFEST_FILE"
  if is_dry; then
    say "(dry-run) would require version == $VERSION, platforms.darwin-aarch64.url == $want_url, its signature string == the uploaded $SIG_NAME (a consistency check; step 11 is the one that verifies the signature), and the linux/windows entries on $TAG"
    return 0
  fi
  ((CAPTURE_RC == 0)) || fail "could not read desktop/$MANIFEST_FILE from $RELEASES_REPO"
  [[ "$(jq -r '.version // empty' <<<"$manifest")" == "$VERSION" ]] ||
    fail "manifest version is '$(jq -r '.version // "none"' <<<"$manifest")', expected '$VERSION'"
  [[ "$(jq -r '.platforms["darwin-aarch64"].url // empty' <<<"$manifest")" == "$want_url" ]] ||
    fail "darwin-aarch64 url is '$(jq -r '.platforms["darwin-aarch64"].url // "absent"' <<<"$manifest")', expected '$want_url'"
  msig="$(jq -r '.platforms["darwin-aarch64"].signature // empty' <<<"$manifest")"
  sig="$(cat "$sigdir/$SIG_NAME")"
  [[ -n "$msig" && "$msig" == "$sig" ]] || fail "darwin-aarch64 signature in the manifest is not the signature that was uploaded to the release"
  jq -e --arg tag "$TAG" '[.platforms["linux-x86_64"].url, .platforms["windows-x86_64-nsis"].url] | all(contains($tag))' <<<"$manifest" >/dev/null ||
    fail "the linux/windows manifest entries do not point at $TAG"
  say "desktop/$MANIFEST_FILE: version $VERSION, darwin-aarch64 -> $want_url, signature string equals the uploaded $SIG_NAME (consistency check only; validity is step 11's minisign check)"
}

write_summary() {
  local f="$WORK/release-summary.txt"
  {
    printf 'Beebeeb desktop %s (%s) macOS release, finished %s\n' "$VERSION" "$CHANNEL" "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf 'commit           %s\n' "${RELEASE_COMMIT:-unknown}"
    printf 'notary app       %s\n' "${NOTARY_ID_APP:-not run in this invocation}"
    printf 'notary dmg       %s\n' "${NOTARY_ID_DMG:-not run in this invocation}"
    printf 'dmg sha256       %s  %s\n' "$(sha256_of "$DMG")" "$DMG_NAME"
    printf 'updater sha256   %s  Beebeeb.app.tar.gz\n' "$(sha256_of "$TARBALL")"
    printf 'sign run         %s\n' "${SIGN_RUN_ID:-not run in this invocation}"
    printf 'manifest run     %s\n' "${PUBLISH_RUN_ID:-not run in this invocation}"
  } >"$f"
  say "summary written to $f"
}

# ---------------------------------------------------------------- main
printf 'Beebeeb desktop macOS release: version %s, channel %s, tag %s%s\n' "$VERSION" "$CHANNEL" "$TAG" "$( ((DRY)) && echo ' (DRY RUN: nothing is executed)')"
if ((FROM_STEP > 1)); then
  printf 'Resuming at step %s; steps 2-%s are skipped. Step 1 (the release-state gate) always runs.\n' "$FROM_STEP" "$((FROM_STEP - 1))"
fi

banner 1
step_1_verify_release_state

for ((n = 2; n <= TOTAL_STEPS; n++)); do
  if ((n < FROM_STEP)); then
    printf '\n==> [%s/%s] skipped (resume): %s\n' "$n" "$TOTAL_STEPS" "${STEP_NAMES[$n]}"
    continue
  fi
  banner "$n"
  HINT=""
  case "$n" in
    2) step_2_build ;;
    3) step_3_preflight_unnotarized_app ;;
    4) step_4_notarize_app ;;
    5) step_5_staple_verify_app ;;
    6) step_6_build_dmg ;;
    7) step_7_notarize_dmg ;;
    8) step_8_staple_verify_dmg ;;
    9) step_9_updater_bundle ;;
    10) step_10_upload ;;
    11) step_11_sign_updater_bundle ;;
    12) step_12_publish_manifest ;;
    13) step_13_assert_manifest ;;
  esac
done

CURRENT_STEP=$TOTAL_STEPS
printf '\n'
if is_dry; then
  printf 'DRY RUN complete: %s steps listed, no command was executed.\n' "$TOTAL_STEPS"
else
  if ((FROM_STEP <= 10)); then write_summary; fi
  printf 'DONE: macOS %s is published on the %s channel (manifest %s).\n' "$VERSION" "$CHANNEL" "$MANIFEST_FILE"
  printf 'Not done by this script (by design): install the dmg on a Mac and exercise it by hand.\n'
  printf 'Public URL: https://github.com/%s/releases/download/%s/%s\n' "$REPO" "$TAG" "$DMG_NAME"
fi
