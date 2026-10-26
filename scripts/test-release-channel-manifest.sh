#!/usr/bin/env bash
# Linux-runnable, offline test for release.yml's "Generate desktop channel manifest" step: the
# step that writes desktop/{alpha,beta,latest}.json, which every installed app reads to decide
# whether to update and which signature to trust.
#
# It extracts that step's `run:` block from the workflow, runs it with a fake `gh`, `sleep` and
# `curl` on PATH (scripts/fixtures/release-manifest-fake-gh.sh) against fixture releases, and
# asserts:
#   1. the manifest a complete release produces: platforms, URLs, signatures, notes, channel;
#   2. a release without macOS assets omits darwin, and only then;
#   3. fail closed: a half-present macOS release, a missing bundle or signature, a signature
#      that cannot be downloaded or is empty, and an unreadable release all fail the step
#      with no manifest written;
#   4. retries: one failed request is retried and succeeds; three fail the step, with no
#      sleep after the last attempt;
#   5. signatures come through the assets API, never the public download URL (no curl).
# Background: on 2026-10-26 the public .sig URL served a cached 504 for about 15 minutes, and
# the step before that treated any failed macOS download as "no macOS release".
#
# The truth line is a COUNT ("release-channel-manifest: N passed, 0 failed") asserted
# against EXPECTED_CHECKS. `--self-test` proves that assertion can go red. `--print-step`
# prints the extracted step (CI shellchecks it).
#
# Usage:
#   scripts/test-release-channel-manifest.sh               # run every scenario
#   scripts/test-release-channel-manifest.sh --self-test   # red-proof of the count guard
#   scripts/test-release-channel-manifest.sh --print-step  # the step's script, as tested

# shellcheck disable=SC2329  # predicates are invoked indirectly through `check`
set -uo pipefail

EXPECTED_CHECKS=82

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$HERE/.." && pwd)"
WORKFLOW="${BB_RELEASE_WORKFLOW_UNDER_TEST:-$ROOT_DIR/.github/workflows/release.yml}"
FAKE_GH="$ROOT_DIR/scripts/fixtures/release-manifest-fake-gh.sh"
STEP_NAME="Generate desktop channel manifest"

# assert_counts <log file> <exit code of the test run>
assert_counts() {
  local log="$1" code="$2" lines passed failed
  if [[ "$code" != "0" ]]; then
    echo "FAIL: test run exited $code" >&2
    return 1
  fi
  lines="$(grep -E '^release-channel-manifest: [0-9]+ passed, [0-9]+ failed$' "$log" || true)"
  if [[ "$(printf '%s' "$lines" | grep -c .)" != "1" ]]; then
    echo "FAIL: expected exactly one 'release-channel-manifest: N passed, M failed' line, found:" >&2
    printf '%s\n' "$lines" >&2
    return 1
  fi
  passed="$(printf '%s' "$lines" | sed -E 's/^release-channel-manifest: ([0-9]+) passed, ([0-9]+) failed$/\1/')"
  failed="$(printf '%s' "$lines" | sed -E 's/^release-channel-manifest: ([0-9]+) passed, ([0-9]+) failed$/\2/')"
  if [[ "$failed" != "0" ]]; then
    echo "FAIL: $failed check(s) failed" >&2
    return 1
  fi
  if [[ "$passed" != "$EXPECTED_CHECKS" ]]; then
    echo "FAIL: $passed check(s) passed, expected $EXPECTED_CHECKS" >&2
    return 1
  fi
  echo "OK: release-channel-manifest: $passed passed, 0 failed (expected $EXPECTED_CHECKS)"
}

self_test() {
  local dir
  dir="$(mktemp -d)"
  trap 'rm -rf "$dir"' RETURN

  printf 'release-channel-manifest: %s passed, 0 failed\n' "$EXPECTED_CHECKS" >"$dir/good.log"
  assert_counts "$dir/good.log" 0 >/dev/null 2>&1 || { echo "self-test: a correct log was rejected" >&2; return 1; }

  printf 'release-channel-manifest: %s passed, 0 failed\n' "$((EXPECTED_CHECKS - 1))" >"$dir/fewer.log"
  ! assert_counts "$dir/fewer.log" 0 >/dev/null 2>&1 || { echo "self-test: a run with too few checks was accepted" >&2; return 1; }

  printf 'release-channel-manifest: %s passed, 1 failed\n' "$EXPECTED_CHECKS" >"$dir/failed.log"
  ! assert_counts "$dir/failed.log" 1 >/dev/null 2>&1 || { echo "self-test: a run with a failure was accepted" >&2; return 1; }
  ! assert_counts "$dir/failed.log" 0 >/dev/null 2>&1 || { echo "self-test: a failed count with exit 0 was accepted" >&2; return 1; }

  : >"$dir/empty.log"
  ! assert_counts "$dir/empty.log" 0 >/dev/null 2>&1 || { echo "self-test: a log with no count line was accepted" >&2; return 1; }

  printf 'release-channel-manifest: %s passed, 0 failed\n' "$EXPECTED_CHECKS" >"$dir/nonzero.log"
  ! assert_counts "$dir/nonzero.log" 139 >/dev/null 2>&1 || { echo "self-test: a crashed run with a good count was accepted" >&2; return 1; }

  echo "self-test OK: the count guard rejected 5 bad logs and accepted the good one"
}

# The step's `run: |` block, de-indented. Empty if the step was renamed or reshaped.
extract_step() {
  awk -v name="- name: $STEP_NAME" '
    index($0, name) { found = 1; next }
    found && /^        run: \|$/ { body = 1; next }
    body && /^      - name:/ { exit }
    body { sub(/^          /, ""); print }
  ' "$WORKFLOW"
}

case "${1:-}" in
  --self-test) self_test; exit $? ;;
  --print-step) extract_step; exit $? ;;
esac

for tool in jq awk; do
  command -v "$tool" >/dev/null 2>&1 || { echo "missing $tool" >&2; exit 2; }
done

TMP="$(mktemp -d)"
# BB_TEST_KEEP_TMP=1 keeps every scenario's release fixture, call log (calls.log), step output
# (out.log) and manifest so a failing check can be re-read instead of guessed at.
cleanup_tmp() { if [[ "${BB_TEST_KEEP_TMP:-}" == 1 ]]; then echo "kept scenario files in $TMP" >&2; else rm -rf "$TMP"; fi; }
trap cleanup_tmp EXIT
PASSED=0
FAILED=0

# check <description> <command...>: the command must succeed.
check() {
  local desc="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    PASSED=$((PASSED + 1))
    printf '  ok   %s\n' "$desc"
  else
    FAILED=$((FAILED + 1))
    printf '  FAIL %s\n' "$desc"
  fi
}

STEP="$TMP/step.sh"
extract_step >"$STEP"
echo "step: $STEP_NAME (from ${WORKFLOW#"$ROOT_DIR"/})"
check "the step's run block was found in the workflow" test -s "$STEP"
check "the extracted block is the manifest script" grep -q 'releases-repo/\$MANIFEST_PATH' "$STEP"

# ---------------------------------------------------------------- fixtures
VER=0.9.1
TAG="desktop-v$VER"
REPO=beebeeb-io/desktop
BASE="https://github.com/$REPO/releases/download/$TAG"
MAC=Beebeeb.app.tar.gz
LINUX="Beebeeb_${VER}_amd64.AppImage"
NSIS="Beebeeb_${VER}_x64-setup.exe"
MSI="Beebeeb_${VER}_x64_en-US.msi"
NOTES_BODY=$'# Beebeeb Desktop 0.9.1\n\nNotes with `code`, "quotes" and a $dollar.'

FX="" REL="" ASSETS_JSON="[]" NEXT_ID=100 CHANNEL=alpha FAIL_RULES="" RC=0

new_scenario() { # name
  FX="$TMP/$1"
  REL="$FX/release"
  mkdir -p "$REL/assets" "$FX/bin" "$FX/state" "$FX/run" "$FX/tmp"
  ASSETS_JSON="[]" NEXT_ID=100 CHANNEL=alpha FAIL_RULES=""
  for name in gh sleep curl; do ln -s "$FAKE_GH" "$FX/bin/$name"; done
  : >"$FX/calls.log"
}

# asset <name> <content> [state]: add an asset to the scenario's release. Content "-" is a
# bundle placeholder.
asset() {
  local name="$1" content="$2" state="${3:-uploaded}"
  NEXT_ID=$((NEXT_ID + 1))
  printf '%s' "$content" >"$REL/assets/$NEXT_ID"
  ASSETS_JSON=$(jq -c --arg n "$name" --arg s "$state" --argjson id "$NEXT_ID" \
    '. + [{id: $id, name: $n, state: $s, size: 1}]' <<<"$ASSETS_JSON")
}

asset_id_of() { jq -r --arg n "$1" '.[] | select(.name == $n) | .id' <<<"$ASSETS_JSON"; }

full_release() {
  asset "$MAC" -
  asset "$MAC.sig" "SIG-MAC"
  asset "$LINUX" -
  asset "$LINUX.sig" "SIG-LINUX"
  asset "$NSIS" -
  asset "$NSIS.sig" "SIG-NSIS"
  asset "$MSI" -
  asset "$MSI.sig" "SIG-MSI"
  asset "Beebeeb_${VER}_amd64.deb" -
  asset "Beebeeb_${VER}_amd64.deb.sig" "SIG-DEB"
}

run_step() {
  jq -n --arg body "$NOTES_BODY" --arg tag "$TAG" --argjson assets "$ASSETS_JSON" \
    '{tag_name: $tag, body: $body, assets: $assets}' >"$REL/release.json"
  (
    cd "$FX/run" &&
      env PATH="$FX/bin:$PATH" TMPDIR="$FX/tmp" GH_TOKEN=fake GITHUB_ENV="$FX/github_env" \
        BB_FAKE_RELEASE_DIR="$REL" BB_FAKE_STATE_DIR="$FX/state" BB_FAKE_GH_LOG="$FX/calls.log" \
        BB_FAKE_GH_FAIL="$FAIL_RULES" REPO="$REPO" TAG="$TAG" VER="$VER" CHANNEL="$CHANNEL" \
        bash -e "$STEP"
  ) >"$FX/out.log" 2>&1
  RC=$?
}

manifest_path() {
  case "$CHANNEL" in
    stable) echo "$FX/run/releases-repo/desktop/latest.json" ;;
    *) echo "$FX/run/releases-repo/desktop/$CHANNEL.json" ;;
  esac
}

# ---------------------------------------------------------------- predicates
exited_ok() { [[ "$RC" == 0 ]]; }
exited_fail() { [[ "$RC" != 0 ]]; }
no_manifest() { [[ ! -e "$(manifest_path)" ]]; }
mf() { jq -r "$1" "$(manifest_path)"; }
mf_eq() { [[ "$(mf "$1")" == "$2" ]]; }
platforms_are() { mf_eq '.platforms | keys | join(",")' "$1"; }
output_has() { grep -qF -- "$1" "$FX/out.log"; }
calls_matching() { grep -cE -- "$1" "$FX/calls.log"; }
calls_eq() { [[ "$(calls_matching "$1")" == "$2" ]]; }
downloads_of() { calls_matching "^api repos/$REPO/releases/assets/$(asset_id_of "$1") octet=1"; }
sleeps_are() { [[ "$(grep '^sleep ' "$FX/calls.log" | tr '\n' ',')" == "$1" ]]; }
no_curl_or_unsupported() { ! grep -qE '^(curl|unsupported)' "$FX/calls.log"; }
env_has() { grep -qxF -- "$1" "$FX/github_env"; }

ALL5="darwin-aarch64,linux-x86_64,windows-x86_64,windows-x86_64-msi,windows-x86_64-nsis"
NO_MAC="linux-x86_64,windows-x86_64,windows-x86_64-msi,windows-x86_64-nsis"

# ---------------------------------------------------------------- scenarios
echo "complete release, alpha"
new_scenario complete
full_release
run_step
check "exits 0" exited_ok
check "version is $VER" mf_eq .version "$VER"
check "all five platforms" platforms_are "$ALL5"
check "darwin signature is the .sig's content" mf_eq '.platforms["darwin-aarch64"].signature' SIG-MAC
check "linux signature" mf_eq '.platforms["linux-x86_64"].signature' SIG-LINUX
check "windows (generic) signature is the NSIS one" mf_eq '.platforms["windows-x86_64"].signature' SIG-NSIS
check "windows nsis signature" mf_eq '.platforms["windows-x86_64-nsis"].signature' SIG-NSIS
check "windows msi signature" mf_eq '.platforms["windows-x86_64-msi"].signature' SIG-MSI
check "darwin url is the updater tarball" mf_eq '.platforms["darwin-aarch64"].url' "$BASE/$MAC"
check "linux url is the AppImage" mf_eq '.platforms["linux-x86_64"].url' "$BASE/$LINUX"
check "windows (generic) url is the NSIS installer" mf_eq '.platforms["windows-x86_64"].url' "$BASE/$NSIS"
check "windows msi url" mf_eq '.platforms["windows-x86_64-msi"].url' "$BASE/$MSI"
check "notes are the release body, byte for byte" mf_eq .notes "$NOTES_BODY"
check "pub_date is set" mf_eq '.pub_date | test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T")' true
check "GITHUB_ENV names the channel" env_has DESKTOP_RELEASE_CHANNEL=alpha
check "GITHUB_ENV names the manifest path" env_has DESKTOP_MANIFEST_PATH=desktop/alpha.json
check "the release is read once" calls_eq "^api repos/$REPO/releases/tags/$TAG$" 1
check "four signature downloads, all as octet-stream" calls_eq "^api repos/$REPO/releases/assets/[0-9]+ octet=1$" 4
check "no other asset requests" calls_eq "^api repos/$REPO/releases/assets/" 4
check "the darwin signature is downloaded by its asset id" eval '[[ "$(downloads_of "$MAC.sig")" == 1 ]]'
check "the deb signature is not downloaded" eval '[[ "$(downloads_of "Beebeeb_${VER}_amd64.deb.sig")" == 0 ]]'
check "no sleeps" sleeps_are ""
check "no curl and no other gh command" no_curl_or_unsupported

echo "complete release, stable"
new_scenario stable
full_release
CHANNEL=stable
run_step
check "exits 0" exited_ok
check "writes desktop/latest.json" test -s "$(manifest_path)"
check "GITHUB_ENV names desktop/latest.json" env_has DESKTOP_MANIFEST_PATH=desktop/latest.json
check "all five platforms" platforms_are "$ALL5"

echo "no macOS assets (the CI-created release)"
new_scenario no-macos
asset "$LINUX" -; asset "$LINUX.sig" SIG-LINUX; asset "$NSIS" -; asset "$NSIS.sig" SIG-NSIS
asset "$MSI" -; asset "$MSI.sig" SIG-MSI
run_step
check "exits 0" exited_ok
check "four platforms, no darwin" platforms_are "$NO_MAC"
check "says why darwin is omitted" output_has "::notice::Neither $MAC nor its signature is on $TAG"
check "three signature downloads" calls_eq "^api repos/$REPO/releases/assets/" 3

echo "macOS assets still uploading (state starter)"
new_scenario macos-starter
full_release
ASSETS_JSON=$(jq -c --arg a "$MAC" --arg b "$MAC.sig" 'map(if .name == $a or .name == $b then .state = "starter" else . end)' <<<"$ASSETS_JSON")
run_step
check "exits 0" exited_ok
check "four platforms, no darwin" platforms_are "$NO_MAC"
check "the starter signature is not downloaded" eval '[[ "$(downloads_of "$MAC.sig")" == 0 ]]'

echo "macOS signature cannot be downloaded (3 failures)"
new_scenario macos-sig-504
full_release
FAIL_RULES="assets/$(asset_id_of "$MAC.sig"):3"
run_step
check "fails" exited_fail
check "writes no manifest" no_manifest
check "three attempts at the darwin signature" calls_eq "^api repos/$REPO/releases/assets/$(asset_id_of "$MAC.sig") octet=1 -> 504$" 3
check "backs off 10 s then 20 s, no sleep after the last attempt" sleeps_are "sleep 10,sleep 20,"
check "reports the failed request" output_has "::error::GitHub API request repos/$REPO/releases/assets/$(asset_id_of "$MAC.sig") failed 3 times"

echo "macOS signature is empty"
new_scenario macos-sig-empty
full_release
: >"$REL/assets/$(asset_id_of "$MAC.sig")"
run_step
check "fails" exited_fail
check "writes no manifest" no_manifest
check "says the signature is empty" output_has "::error::$MAC.sig on $TAG is empty"

echo "macOS signature is whitespace only"
new_scenario macos-sig-blank
full_release
printf ' \n\t\n' >"$REL/assets/$(asset_id_of "$MAC.sig")"
run_step
check "fails" exited_fail
check "writes no manifest" no_manifest
check "says the signature is empty" output_has "::error::$MAC.sig on $TAG is empty"

echo "macOS bundle without its signature"
new_scenario macos-bundle-only
asset "$MAC" -; asset "$LINUX" -; asset "$LINUX.sig" SIG-LINUX; asset "$NSIS" -; asset "$NSIS.sig" SIG-NSIS
asset "$MSI" -; asset "$MSI.sig" SIG-MSI
run_step
check "fails" exited_fail
check "writes no manifest" no_manifest
check "names the missing signature" output_has "::error::$MAC.sig is not an uploaded asset of $TAG"
check "downloads nothing" calls_eq "^api repos/$REPO/releases/assets/" 0

echo "macOS signature without its bundle"
new_scenario macos-sig-only
asset "$MAC.sig" SIG-MAC; asset "$LINUX" -; asset "$LINUX.sig" SIG-LINUX; asset "$NSIS" -; asset "$NSIS.sig" SIG-NSIS
asset "$MSI" -; asset "$MSI.sig" SIG-MSI
run_step
check "fails" exited_fail
check "writes no manifest" no_manifest
check "names the missing bundle" output_has "::error::$MAC is not an uploaded asset of $TAG"

echo "macOS bundle uploaded, its signature still uploading"
new_scenario macos-sig-starter
full_release
ASSETS_JSON=$(jq -c --arg b "$MAC.sig" 'map(if .name == $b then .state = "starter" else . end)' <<<"$ASSETS_JSON")
run_step
check "fails" exited_fail
check "writes no manifest" no_manifest
check "names the signature" output_has "::error::$MAC.sig is not an uploaded asset of $TAG"

echo "AppImage missing, its signature present"
new_scenario linux-bundle-missing
full_release
ASSETS_JSON=$(jq -c --arg a "$LINUX" 'map(select(.name != $a))' <<<"$ASSETS_JSON")
run_step
check "fails" exited_fail
check "writes no manifest" no_manifest
check "names the missing bundle" output_has "::error::$LINUX is not an uploaded asset of $TAG"
check "fails at once, no retries" sleeps_are ""

echo "NSIS signature missing"
new_scenario nsis-sig-missing
full_release
ASSETS_JSON=$(jq -c --arg a "$NSIS.sig" 'map(select(.name != $a))' <<<"$ASSETS_JSON")
run_step
check "fails" exited_fail
check "writes no manifest" no_manifest
check "names the missing signature" output_has "::error::$NSIS.sig is not an uploaded asset of $TAG"

echo "MSI signature is empty"
new_scenario msi-sig-empty
full_release
: >"$REL/assets/$(asset_id_of "$MSI.sig")"
run_step
check "fails" exited_fail
check "writes no manifest" no_manifest
check "says the signature is empty" output_has "::error::$MSI.sig on $TAG is empty"

echo "MSI signature fails once, then downloads"
new_scenario msi-sig-504-once
full_release
FAIL_RULES="assets/$(asset_id_of "$MSI.sig"):1"
run_step
check "exits 0" exited_ok
check "all five platforms" platforms_are "$ALL5"
check "msi signature is the .sig's content" mf_eq '.platforms["windows-x86_64-msi"].signature' SIG-MSI
check "one back-off of 10 s" sleeps_are "sleep 10,"
check "warns about the failed attempt" output_has "::warning::GitHub API request repos/$REPO/releases/assets/$(asset_id_of "$MSI.sig") failed (attempt 1 of 3)"

echo "MSI signature cannot be downloaded (3 failures)"
new_scenario msi-sig-504
full_release
FAIL_RULES="assets/$(asset_id_of "$MSI.sig"):3"
run_step
check "fails" exited_fail
check "writes no manifest" no_manifest

echo "release metadata fails once, then answers"
new_scenario release-504-once
full_release
FAIL_RULES="releases/tags/$TAG:1"
run_step
check "exits 0" exited_ok
check "all five platforms" platforms_are "$ALL5"
check "notes are the release body" mf_eq .notes "$NOTES_BODY"
check "one back-off of 10 s" sleeps_are "sleep 10,"

echo "release metadata cannot be read (3 failures)"
new_scenario release-504
full_release
FAIL_RULES="releases/tags/$TAG:3"
run_step
check "fails" exited_fail
check "writes no manifest" no_manifest
check "downloads nothing" calls_eq "^api repos/$REPO/releases/assets/" 0
check "backs off 10 s then 20 s, no sleep after the last attempt" sleeps_are "sleep 10,sleep 20,"

echo
echo "release-channel-manifest: $PASSED passed, $FAILED failed"
[[ "$FAILED" == 0 && "$PASSED" == "$EXPECTED_CHECKS" ]]
