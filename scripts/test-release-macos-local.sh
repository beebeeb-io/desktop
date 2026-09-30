#!/usr/bin/env bash
# Linux-runnable test for scripts/release-macos-local.sh (task: 0.8.7 release prep).
#
# The release script drives macOS-only tools (xcrun notarytool/stapler, codesign,
# spctl, hdiutil, ditto, plutil) and GitHub (gh). This test puts shims for them on
# PATH (scripts/fixtures/release-macos-shim.sh), runs the REAL script end to end
# against a throwaway git repo with a real local "origin" and an annotated tag, and
# asserts:
#   1. the exact ORDER: notarize app -> staple app -> dmg built from the stapled app
#      -> notarize dmg -> staple dmg -> upload -> sign workflow -> publish_existing
#      workflow -> manifest read, with every step happening exactly once;
#   2. what the artifacts contain (the dmg and the updater tarball hold the STAPLED app);
#   3. fail-closed behaviour: each critical failure stops at the right step, prints a
#      remediation and a resume command, and leaves no upload / no publish behind;
#   4. --dry-run runs nothing, --from-step resumes without redoing earlier steps,
#      and tauri.conf.json / dist/index.html are restored on success AND failure.
# It cannot prove that Apple's tools behave like the shims; that is the Mac run.
#
# The truth line is a COUNT ("release-macos-local: N passed, 0 failed") asserted
# against EXPECTED_CHECKS: a run that executed fewer checks than declared, printed
# no count, or had a failure is RED. `--self-test` proves that assertion can go red.
#
# Usage:
#   scripts/test-release-macos-local.sh              # run every scenario
#   scripts/test-release-macos-local.sh --self-test  # red-proof of the count guard

# shellcheck disable=SC2016  # notes fixtures hold literal backticks
# shellcheck disable=SC2329  # predicates are invoked indirectly through `check`
set -uo pipefail

EXPECTED_CHECKS=397

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$HERE/.." && pwd)"
SUT="${BB_RELEASE_SCRIPT_UNDER_TEST:-$ROOT_DIR/scripts/release-macos-local.sh}"
SHIM_SRC="$ROOT_DIR/scripts/fixtures/release-macos-shim.sh"
REAL_GIT="$(command -v git)"
REAL_TAR="$(command -v tar)"
export BB_REAL_GIT="$REAL_GIT" BB_REAL_TAR="$REAL_TAR"

# assert_counts <log file> <exit code of the test run>
assert_counts() {
  local log="$1" code="$2" lines passed failed
  if [[ "$code" != "0" ]]; then
    echo "FAIL: test run exited $code" >&2
    return 1
  fi
  lines="$(grep -E '^release-macos-local: [0-9]+ passed, [0-9]+ failed$' "$log" || true)"
  if [[ "$(printf '%s' "$lines" | grep -c .)" != "1" ]]; then
    echo "FAIL: expected exactly one 'release-macos-local: N passed, M failed' line, found:" >&2
    printf '%s\n' "$lines" >&2
    return 1
  fi
  passed="$(printf '%s' "$lines" | sed -E 's/^release-macos-local: ([0-9]+) passed, ([0-9]+) failed$/\1/')"
  failed="$(printf '%s' "$lines" | sed -E 's/^release-macos-local: ([0-9]+) passed, ([0-9]+) failed$/\2/')"
  if [[ "$failed" != "0" ]]; then
    echo "FAIL: $failed check(s) failed" >&2
    return 1
  fi
  if [[ "$passed" != "$EXPECTED_CHECKS" ]]; then
    echo "FAIL: $passed check(s) passed, expected $EXPECTED_CHECKS" >&2
    return 1
  fi
  echo "OK: release-macos-local: $passed passed, 0 failed (expected $EXPECTED_CHECKS)"
}

self_test() {
  local dir
  dir="$(mktemp -d)"
  trap 'rm -rf "$dir"' RETURN

  printf 'release-macos-local: %s passed, 0 failed\n' "$EXPECTED_CHECKS" >"$dir/good.log"
  assert_counts "$dir/good.log" 0 >/dev/null 2>&1 || { echo "self-test: a correct log was rejected" >&2; return 1; }

  printf 'release-macos-local: %s passed, 0 failed\n' "$((EXPECTED_CHECKS - 1))" >"$dir/fewer.log"
  ! assert_counts "$dir/fewer.log" 0 >/dev/null 2>&1 || { echo "self-test: a run with too few checks was accepted" >&2; return 1; }

  printf 'release-macos-local: %s passed, 1 failed\n' "$EXPECTED_CHECKS" >"$dir/failed.log"
  ! assert_counts "$dir/failed.log" 1 >/dev/null 2>&1 || { echo "self-test: a run with a failure was accepted" >&2; return 1; }
  ! assert_counts "$dir/failed.log" 0 >/dev/null 2>&1 || { echo "self-test: a failed count with exit 0 was accepted" >&2; return 1; }

  : >"$dir/empty.log"
  ! assert_counts "$dir/empty.log" 0 >/dev/null 2>&1 || { echo "self-test: a log with no count line was accepted" >&2; return 1; }

  printf 'release-macos-local: %s passed, 0 failed\n' "$EXPECTED_CHECKS" >"$dir/nonzero.log"
  ! assert_counts "$dir/nonzero.log" 139 >/dev/null 2>&1 || { echo "self-test: a crashed run with a good count was accepted" >&2; return 1; }

  echo "self-test OK: the count guard rejected 5 bad logs and accepted the good one"
}

if [[ "${1:-}" == "--self-test" ]]; then
  self_test
  exit $?
fi

for tool in git tar jq sha256sum; do
  command -v "$tool" >/dev/null 2>&1 || { echo "missing $tool" >&2; exit 2; }
done

TMP="$(mktemp -d)"
# BB_TEST_KEEP_TMP=1 keeps every scenario's repo, state, call log (calls.log) and script output
# (out.log) so a failing check can be re-read instead of guessed at.
cleanup_tmp() { if [[ "${BB_TEST_KEEP_TMP:-}" == 1 ]]; then echo "kept scenario files in $TMP" >&2; else rm -rf "$TMP"; fi; }
trap cleanup_tmp EXIT
PASSED=0
FAILED=0
TOTAL_STEPS=13

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

# ---------------------------------------------------------------- fixture
FX="" REPO="" STATE="" LOGF="" OUT="" RC=0 HOME_FX="" SHIMS=""
SCEN_ENV=()

new_fixture() { # name
  FX="$TMP/$1"
  rm -rf "$FX"
  REPO="$FX/repo" STATE="$FX/state" LOGF="$FX/calls.log" OUT="$FX/out.log" HOME_FX="$FX/home" SHIMS="$FX/bin"
  SCEN_ENV=()
  mkdir -p "$REPO/scripts" "$REPO/src-tauri" "$REPO/dist" "$STATE" "$SHIMS" \
    "$HOME_FX/.private_keys/macos-developer-id-profiles"
  cp "$SUT" "$REPO/scripts/release-macos-local.sh"
  chmod +x "$REPO/scripts/release-macos-local.sh"
  # Stand-in for the real preflight (it needs plutil, security, ruby, nm, otool): record the call.
  cat >"$REPO/scripts/macos-release-preflight.sh" <<'STUB'
#!/usr/bin/env bash
echo "preflight ${1:-} skip_spctl=${BB_PREFLIGHT_SKIP_SPCTL_EXECUTE:-}" >>"$BB_TEST_LOG"
[[ "${BB_STUB_PREFLIGHT_FAIL:-}" != 1 ]] || { echo "preflight failed: stub" >&2; exit 1; }
STUB
  chmod +x "$REPO/scripts/macos-release-preflight.sh"
  printf 'untrusted comment: minisign public key\nRWSfixturepubkey\n' >"$FX/pub.txt"
  jq -n --arg pk "$(base64 <"$FX/pub.txt" | tr -d '\n')" \
    '{productName:"Beebeeb",version:"0.1.0",identifier:"io.beebeeb.app",bundle:{targets:"all",createUpdaterArtifacts:true},plugins:{updater:{pubkey:$pk}}}' >"$REPO/src-tauri/tauri.conf.json"
  echo '<!doctype html><html></html>' >"$REPO/dist/index.html"
  write_notes "$REPO/RELEASE_NOTES.md"
  printf 'src-tauri/target/\n' >"$REPO/.gitignore"
  echo fixture >"$HOME_FX/.private_keys/AuthKey_5KRWK96245.p8"
  echo profile >"$HOME_FX/.private_keys/macos-developer-id-profiles/BeebeebApp.provisionprofile"
  echo profile >"$HOME_FX/.private_keys/macos-developer-id-profiles/BeebeebFileProvider.provisionprofile"
  : >"$LOGF"
  local n
  for n in git tar uname security bun bunx plutil ditto xcrun codesign spctl hdiutil minisign gh base64; do
    ln -s "$SHIM_SRC" "$SHIMS/$n"
  done
  (
    cd "$REPO" || exit 1
    "$REAL_GIT" init -q -b main
    "$REAL_GIT" config user.email t@example.test
    "$REAL_GIT" config user.name test
    "$REAL_GIT" add -A
    "$REAL_GIT" commit -q -m "release commit"
    "$REAL_GIT" init -q --bare "$FX/origin.git"
    "$REAL_GIT" remote add origin "$FX/origin.git"
    "$REAL_GIT" push -q origin main
    "$REAL_GIT" tag -a desktop-v0.8.7 -m "desktop-v0.8.7"
    "$REAL_GIT" push -q origin desktop-v0.8.7
  ) >/dev/null 2>&1
}

# The fixture notes carry real counts, one of them wrapped over a line break (the gate must
# read them wrap-proof), as the lead fills them in on the release commit.
NOTES_GOOD_GATE='- Test gate: `bun test` 12 pass /
  0 fail, `cargo test --locked` per-binary `test result: ok.
  7 passed`.'
write_notes() { # path [the Verification gate bullet(s)]
  {
    printf '# Beebeeb Desktop 0.8.7 - fixture\n\nNotes.\n\n### Verification\n\n'
    printf '%s\n' "${2:-$NOTES_GOOD_GATE}"
  } >"$1"
}

# run_release <args...>: run the script inside the fixture repo with shims first on PATH.
run_release() {
  RC=0
  (
    cd "$REPO" || exit 99
    env -u TAURI_SIGNING_PRIVATE_KEY -u TAURI_SIGNING_PRIVATE_KEY_PASSWORD -u TAURI_SIGNING_PRIVATE_KEY_PATH \
      PATH="$SHIMS:$PATH" HOME="$HOME_FX" BB_TEST_STATE="$STATE" BB_TEST_LOG="$LOGF" \
      BB_POLL_INTERVAL=0 ${SCEN_ENV[@]+"${SCEN_ENV[@]}"} \
      scripts/release-macos-local.sh "$@"
  ) >"$OUT" 2>&1 || RC=$?
}

# ---------------------------------------------------------------- predicates
log_count() { grep -cE "$1" "$LOGF" || true; }
log_has() { grep -qE "$1" "$LOGF"; }
log_lacks() { ! grep -qE "$1" "$LOGF"; }
out_has() { grep -qF -- "$1" "$OUT"; }
line_of() { grep -nE "$1" "$LOGF" | head -1 | cut -d: -f1; }
rc_is() { [[ "$RC" -eq "$1" ]]; }
no_upload() { log_lacks '^gh release upload '; }
no_dispatch() { log_lacks '^gh workflow run '; }
no_publish() { log_lacks '^gh workflow run release.yml'; }
tree_clean() { [[ -z "$("$REAL_GIT" -C "$REPO" status --porcelain)" ]]; }
conf_unpatched() { [[ "$(jq -r .version "$REPO/src-tauri/tauri.conf.json")" == "0.1.0" ]]; }
in_order() { # line numbers, strictly increasing and all non-empty
  local prev=0 n
  for n in "$@"; do
    [[ -n "$n" && "$n" -gt "$prev" ]] || return 1
    prev="$n"
  done
}

# expect_stop <step> <message substring> <mode: pre-upload|pre-publish>
expect_stop() {
  local step="$1" msg="$2" mode="$3"
  check "fails closed: exit code 1" rc_is 1
  check "stops at step $step/$TOTAL_STEPS" out_has "FAILED at step $step/$TOTAL_STEPS"
  check "names the cause ($msg)" out_has "$msg"
  check "prints a remediation" out_has "Remediation:"
  check "prints the resume command --from-step $step" out_has "--from-step $step"
  case "$mode" in
    pre-upload)
      check "nothing was uploaded" no_upload
      check "no workflow was dispatched" no_dispatch
      check "says nothing was uploaded or published" out_has "nothing was uploaded to GitHub and nothing was published"
      ;;
    pre-publish)
      check "the manifest workflow (release.yml) was never dispatched" no_publish
      check "says the manifest was NOT published" out_has "the manifest was NOT published"
      ;;
  esac
}

echo "== scenario: full run with shims (happy path, order, artifact contents) =="
new_fixture happy
run_release 0.8.7 alpha
check "full run exits 0" rc_is 0
check "prints DONE" out_has "DONE: macOS 0.8.7 is published on the alpha channel"
check "all 13 step banners printed" bash -c "[[ \$(grep -c '^==> \[[0-9]*/13\]' '$OUT') -eq 13 ]]"
check "exactly 2 notarization submissions" bash -c "[[ \$(grep -cE '^xcrun notarytool submit ' '$LOGF') -eq 2 ]]"
check "exactly 1 upload" bash -c "[[ \$(grep -cE '^gh release upload ' '$LOGF') -eq 1 ]]"
check "exactly 1 sign-workflow dispatch" bash -c "[[ \$(grep -cE '^gh workflow run sign-macos-updater-artifact.yml ' '$LOGF') -eq 1 ]]"
check "exactly 1 release.yml dispatch" bash -c "[[ \$(grep -cE '^gh workflow run release.yml ' '$LOGF') -eq 1 ]]"
L_NOTARIZE_APP="$(line_of '^xcrun notarytool submit .*\.zip ')"
L_STAPLE_APP="$(line_of '^xcrun stapler staple .*/Beebeeb\.app$')"
L_DMG_CREATE="$(line_of '^hdiutil create ')"
L_NOTARIZE_DMG="$(line_of '^xcrun notarytool submit .*\.dmg ')"
L_STAPLE_DMG="$(line_of '^xcrun stapler staple .*\.dmg$')"
L_UPLOAD="$(line_of '^gh release upload ')"
L_SIGN="$(line_of '^gh workflow run sign-macos-updater-artifact.yml ')"
L_PUBLISH="$(line_of '^gh workflow run release.yml .*publish_existing=true')"
L_MANIFEST="$(line_of '^gh api .*desktop/alpha\.json')"
check "ORDER: notarize app -> staple app -> dmg built -> notarize dmg -> staple dmg -> upload -> sign -> publish_existing -> manifest read" \
  in_order "$L_NOTARIZE_APP" "$L_STAPLE_APP" "$L_DMG_CREATE" "$L_NOTARIZE_DMG" "$L_STAPLE_DMG" "$L_UPLOAD" "$L_SIGN" "$L_PUBLISH" "$L_MANIFEST"
check "the dmg was built from the STAPLED app" log_has '^shim: dmg source app stapled=1$'
check "no dmg was ever built from an unstapled app" log_lacks '^shim: dmg source app stapled=0$'
check "the dmg is signed with a timestamp before it is notarized" bash -c "[[ \$(grep -nE '^codesign --force --sign .* --timestamp ' '$LOGF' | head -1 | cut -d: -f1) -lt $L_NOTARIZE_DMG ]]"
check "release.yml is dispatched with version, channel and publish_existing=true" \
  log_has '^gh workflow run release.yml .*version=0\.8\.7 -f channel=alpha -f publish_existing=true'
check "the sign workflow gets the release tag" log_has '^gh workflow run sign-macos-updater-artifact.yml .*release_tag=desktop-v0\.8\.7'
check "the first preflight (unnotarized app) skips only the spctl execute assessment" \
  bash -c "grep -E '^preflight .*Beebeeb\.app skip_spctl=' '$LOGF' | head -1 | grep -q 'skip_spctl=1'"
check "the stapled-app preflight does NOT skip spctl" \
  bash -c "grep -E '^preflight .*Beebeeb\.app skip_spctl=' '$LOGF' | sed -n 2p | grep -q 'skip_spctl=\$'"
check "the dmg is preflighted" log_has '^preflight .*\.dmg '
check "spctl checked the app (execute) and the dmg (-t install)" bash -c "grep -qE '^spctl -a -vv .*Beebeeb\.app$' '$LOGF' && grep -qE '^spctl -a -vv -t install .*\.dmg$' '$LOGF'"
check "the build ran with hardened runtime, the release version, both profiles and the identity" \
  log_has '^shim: build env identity=Developer ID Application: Devidee B\.V\. \(R8352WDJJR\) version=0\.8\.7 hardened=1 profiles=.*BeebeebApp\.provisionprofile,.*BeebeebFileProvider\.provisionprofile$'
check "the build targets aarch64-apple-darwin, bundles ONLY the app, turns updater artifacts off by CLI override, --locked" \
  log_has '^bunx tauri build --target aarch64-apple-darwin --bundles app --config \{"bundle":\{"createUpdaterArtifacts":false\}\} -- --locked$'
check "the build saw updater artifacts off, no dmg bundling (bundle_dmg.sh/Finder never runs) and NO updater signing key" \
  log_has '^shim: tauri bundles=app updater=false dmg_bundling=0 signing_key=unset$'
check "tauri.conf.json itself still says createUpdaterArtifacts true (the preflight asserts it; the off switch is a CLI override)" \
  bash -c "[[ \$(jq -r .bundle.createUpdaterArtifacts '$REPO/src-tauri/tauri.conf.json') == true ]]"
check "the build left no Tauri updater tarball and no Tauri dmg behind" \
  bash -c "[[ ! -e '$REPO/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Beebeeb.app.tar.gz' && ! -d '$REPO/src-tauri/target/aarch64-apple-darwin/release/bundle/dmg' ]]"
check "the commit the artifacts were built from is recorded and is HEAD" \
  bash -c "[[ \$(cat '$REPO/src-tauri/target/release-macos-local/0.8.7/build-commit') == \$('$REAL_GIT' -C '$REPO' rev-parse HEAD) ]]"
check "the updater signature was verified locally with minisign before publishing" log_has '^minisign -Vm '
check "the build ran with the version patched (app reports 0.8.7)" \
  grep -q '^CFBundleShortVersionString=0.8.7$' "$REPO/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Beebeeb.app/Contents/Info.plist"
check "tauri.conf.json is back to 0.1.0 after the run" conf_unpatched
check "the working tree is clean after the run (dist/index.html restored too)" tree_clean
check "the release got the dmg" test -f "$STATE/release/Beebeeb_0.8.7_aarch64.dmg"
check "the release got the updater bundle" test -f "$STATE/release/Beebeeb.app.tar.gz"
check "the release got the updater signature (from the CI workflow)" test -s "$STATE/release/Beebeeb.app.tar.gz.sig"
check "the uploaded dmg contains the stapled app" bash -c "'$REAL_TAR' -tf '$STATE/release/Beebeeb_0.8.7_aarch64.dmg' | grep -q 'Beebeeb.app/Contents/_stapled'"
check "the uploaded dmg carries a signature and a ticket" bash -c "grep -aq SIGNED '$STATE/release/Beebeeb_0.8.7_aarch64.dmg' && grep -aq STAPLED '$STATE/release/Beebeeb_0.8.7_aarch64.dmg'"
check "the uploaded updater bundle contains the stapled app" bash -c "'$REAL_TAR' -tzf '$STATE/release/Beebeeb.app.tar.gz' | grep -q '^Beebeeb.app/Contents/_stapled$'"
check "the uploaded updater bundle has the single top-level Beebeeb.app" \
  bash -c "[[ \$('$REAL_TAR' -tzf '$STATE/release/Beebeeb.app.tar.gz' | cut -d/ -f1 | sort -u) == Beebeeb.app ]]"
check "the manifest (alpha) points darwin-aarch64 at the release" \
  bash -c "jq -e '.platforms[\"darwin-aarch64\"].url == \"https://github.com/beebeeb-io/desktop/releases/download/desktop-v0.8.7/Beebeeb.app.tar.gz\"' '$STATE/manifest-alpha.json'"
check "a summary with both notarization ids was written" \
  bash -c "grep -q 'submission-app-0001' '$REPO/src-tauri/target/release-macos-local/0.8.7/release-summary.txt' && grep -q 'submission-dmg-0001' '$REPO/src-tauri/target/release-macos-local/0.8.7/release-summary.txt'"
check "the ASC key path/id/issuer defaults were passed to notarytool" \
  log_has "^xcrun notarytool submit .* --key .*AuthKey_5KRWK96245\\.p8 --key-id 5KRWK96245 --issuer 8cacf7df-c877-47db-aab2-7c9f1f9f5fda --wait"
check "no key material is echoed (the .p8 content never appears in output)" bash -c "! grep -q '^fixture\$' '$OUT'"

echo "== scenario: resume with --from-step 12 =="
COUNT_BEFORE="$(log_count '^xcrun notarytool submit ')"
UPLOAD_BEFORE="$(log_count '^gh release upload ')"
run_release 0.8.7 alpha --from-step 12
check "resume exits 0" rc_is 0
check "steps 2-11 are reported as skipped" bash -c "[[ \$(grep -c 'skipped (resume)' '$OUT') -eq 10 ]]"
check "step 1 (the release-state gate) still ran on resume" out_has "[1/13] Verify release state"
check "no new notarization on resume" bash -c "[[ \$(grep -cE '^xcrun notarytool submit ' '$LOGF') -eq $COUNT_BEFORE ]]"
check "no new upload on resume" bash -c "[[ \$(grep -cE '^gh release upload ' '$LOGF') -eq $UPLOAD_BEFORE ]]"
check "release.yml dispatched a second time on resume" bash -c "[[ \$(grep -cE '^gh workflow run release.yml ' '$LOGF') -eq 2 ]]"
check "the resume did not touch tauri.conf.json" conf_unpatched

echo "== scenario: stable channel targets latest.json =="
new_fixture stable
run_release 0.8.7 stable
check "stable run exits 0" rc_is 0
check "the manifest read is desktop/latest.json" log_has '^gh api .*desktop/latest\.json$'
check "release.yml got channel=stable" log_has '^gh workflow run release.yml .*channel=stable'

echo "== scenario: --dry-run runs nothing =="
new_fixture dry
run_release 0.8.7 alpha --dry-run
check "dry run exits 0" rc_is 0
check "dry run prints the DRY RUN banner" out_has "DRY RUN complete: 13 steps listed, no command was executed."
check "dry run lists all 13 steps" bash -c "[[ \$(grep -c '^==> \[[0-9]*/13\]' '$OUT') -eq 13 ]]"
check "dry run invoked NO tool at all (shim log empty)" bash -c "[[ ! -s '$LOGF' ]]"
check "dry run created no build/work directory" bash -c "[[ ! -e '$REPO/src-tauri/target' ]]"
check "dry run left the tree clean" tree_clean
check "dry run prints the notarization command" out_has "xcrun notarytool submit"
check "dry run prints the upload command" out_has "gh release upload desktop-v0.8.7"
check "dry run prints the publish_existing dispatch" out_has "gh workflow run release.yml"
check "dry run prints the stapler, spctl and hdiutil commands" bash -c "grep -q 'xcrun stapler staple' '$OUT' && grep -q 'spctl -a -vv -t install' '$OUT' && grep -q 'hdiutil create' '$OUT'"
check "dry run prints no secret (the key path is shown, not its content)" bash -c "! grep -q 'BEGIN' '$OUT'"

echo "== scenario: argument validation =="
new_fixture args
run_release 0.8.7-alpha alpha
check "a suffixed version is refused with exit 2" rc_is 2
run_release 0.8.7 nightly
check "an unknown channel is refused with exit 2" rc_is 2
run_release 0.8.7 alpha --from-step 0
check "--from-step 0 is refused" rc_is 2
run_release 0.8.7 alpha --from-step 14
check "--from-step 14 is refused" rc_is 2
run_release 0.8.7
check "a missing channel is refused" rc_is 2
check "no argument error ran a single tool" bash -c "[[ ! -s '$LOGF' ]]"

echo "== scenario: notarytool says Invalid for the app (step 4) =="
new_fixture notary_app
SCEN_ENV=(BB_SHIM_NOTARY_INVALID=app)
run_release 0.8.7 alpha
expect_stop 4 "notarization of the app was not Accepted" pre-upload
check "reports the notary status Invalid" out_has "status 'Invalid'"
check "fetched and printed the notary log issue" out_has "not signed with a valid Developer ID certificate"
check "never stapled anything" log_lacks '^xcrun stapler staple '
check "never built a dmg" log_lacks '^hdiutil create '
check "tauri.conf.json restored" conf_unpatched

echo "== scenario: notarytool says Invalid for the dmg (step 7) =="
new_fixture notary_dmg
SCEN_ENV=(BB_SHIM_NOTARY_INVALID=dmg)
run_release 0.8.7 alpha
expect_stop 7 "notarization of the dmg was not Accepted" pre-upload
check "the app had been notarized and stapled before" log_has '^xcrun stapler staple .*/Beebeeb\.app$'
check "the dmg was never stapled" log_lacks '^xcrun stapler staple .*\.dmg$'

echo "== scenario: spctl rejects the stapled app (step 5) =="
new_fixture spctl_app
SCEN_ENV=(BB_SHIM_SPCTL_REJECT=app)
run_release 0.8.7 alpha
expect_stop 5 "spctl rejected" pre-upload
check "never built a dmg" log_lacks '^hdiutil create '

echo "== scenario: spctl rejects the dmg (step 8) =="
new_fixture spctl_dmg
SCEN_ENV=(BB_SHIM_SPCTL_REJECT=dmg)
run_release 0.8.7 alpha
expect_stop 8 "spctl rejected" pre-upload
check "the dmg had been notarized before the rejection" log_has '^xcrun notarytool submit .*\.dmg '

echo "== scenario: spctl accepts but not as Notarized Developer ID (step 5) =="
new_fixture spctl_source
SCEN_ENV=("BB_SHIM_SPCTL_SOURCE=Developer ID")
run_release 0.8.7 alpha
expect_stop 5 "not as 'source=Notarized Developer ID'" pre-upload

echo "== scenario: version mismatch, the conf patch did not reach the build (step 3) =="
new_fixture version
SCEN_ENV=(BB_SHIM_BUILD_VERSION=0.1.0)
run_release 0.8.7 alpha
expect_stop 3 "reports CFBundleShortVersionString '0.1.0', expected '0.8.7'" pre-upload
check "nothing was submitted to Apple" log_lacks '^xcrun notarytool '

echo "== scenario: the tag points at another commit than HEAD (step 1) =="
new_fixture tagmismatch
(
  cd "$REPO" || exit 1
  echo change >>RELEASE_NOTES.md
  "$REAL_GIT" commit -q -am "later commit"
) >/dev/null 2>&1
run_release 0.8.7 alpha
expect_stop 1 "is not the commit tag desktop-v0.8.7 points to" pre-upload
check "no build was attempted" log_lacks '^bunx '
check "no bun install was attempted" log_lacks '^bun '

echo "== scenario: the tag does not exist on origin (step 1) =="
new_fixture notag
"$REAL_GIT" -C "$FX/origin.git" tag -d desktop-v0.8.7 >/dev/null 2>&1
run_release 0.8.7 alpha
expect_stop 1 "does not exist on origin" pre-upload
check "no build was attempted" log_lacks '^bunx '

echo "== scenario: dirty working tree (step 1) =="
new_fixture dirty
echo x >"$REPO/stray.txt"
run_release 0.8.7 alpha
expect_stop 1 "working tree is not clean" pre-upload
check "names the stray file" out_has "stray.txt"
check "no build was attempted" log_lacks '^bunx '

echo "== scenario: unresolved lead markers in RELEASE_NOTES.md (step 1) =="
new_fixture markers
(
  cd "$REPO" || exit 1
  echo '- a fix <!-- lead: confirm merged before cut -->' >>RELEASE_NOTES.md
  "$REAL_GIT" commit -q -am "notes with marker"
  "$REAL_GIT" tag -f -a desktop-v0.8.7 -m x >/dev/null
  "$REAL_GIT" push -q -f origin desktop-v0.8.7
) >/dev/null 2>&1
run_release 0.8.7 alpha
expect_stop 1 "unresolved lead markers" pre-upload

echo "== scenario: RELEASE_NOTES.md does not mention the version (step 1) =="
new_fixture notesversion
(
  cd "$REPO" || exit 1
  printf '# Beebeeb Desktop 0.8.6\n' >RELEASE_NOTES.md
  "$REAL_GIT" commit -q -am "stale notes"
  "$REAL_GIT" tag -f -a desktop-v0.8.7 -m x >/dev/null
  "$REAL_GIT" push -q -f origin desktop-v0.8.7
) >/dev/null 2>&1
run_release 0.8.7 alpha
expect_stop 1 "does not mention 0.8.7" pre-upload

echo "== scenario: the CI release does not exist yet (step 1) =="
new_fixture norelease
SCEN_ENV=(BB_SHIM_NO_RELEASE=1)
run_release 0.8.7 alpha
expect_stop 1 "release desktop-v0.8.7 not found" pre-upload
check "no build was attempted" log_lacks '^bunx '

echo "== scenario: signing identity not in the keychain (step 1) =="
new_fixture identity
SCEN_ENV=("APPLE_SIGNING_IDENTITY=Developer ID Application: Someone Else (XXXXXXXXXX)")
run_release 0.8.7 alpha
expect_stop 1 "signing identity not found in the keychain" pre-upload

echo "== scenario: notary key file missing (step 1) =="
new_fixture nokey
rm -f "$HOME_FX/.private_keys/AuthKey_5KRWK96245.p8"
run_release 0.8.7 alpha
expect_stop 1 "App Store Connect API key (BB_NOTARY_KEY_PATH) not found" pre-upload

echo "== scenario: the build fails; tracked files are restored (step 2) =="
new_fixture buildfail
SCEN_ENV=(BB_SHIM_BUILD_FAIL=1)
run_release 0.8.7 alpha
expect_stop 2 "command failed" pre-upload
check "tauri.conf.json restored after a failed build" conf_unpatched
check "dist/index.html restored after a failed build" tree_clean

echo "== scenario: the app signature is missing (step 5) =="
new_fixture unsigned_app
SCEN_ENV=(BB_SHIM_APP_UNSIGNED=1)
run_release 0.8.7 alpha
expect_stop 5 "command failed" pre-upload

echo "== scenario: the rebuilt dmg carries no signature (step 6) =="
new_fixture unsigned_dmg
SCEN_ENV=(BB_SHIM_DMG_NOT_SIGNED=1)
run_release 0.8.7 alpha
expect_stop 6 "command failed" pre-upload
check "the unsigned dmg was never submitted to Apple" bash -c "[[ \$(grep -cE '^xcrun notarytool submit ' '$LOGF') -eq 1 ]]"

echo "== scenario: the preflight fails (step 3) =="
new_fixture preflight
SCEN_ENV=(BB_STUB_PREFLIGHT_FAIL=1)
run_release 0.8.7 alpha
expect_stop 3 "command failed" pre-upload
check "nothing was submitted to Apple" log_lacks '^xcrun notarytool '

echo "== scenario: upload is corrupted in transit (step 10) =="
new_fixture corrupt
SCEN_ENV=(BB_SHIM_CORRUPT_UPLOAD=1)
run_release 0.8.7 alpha
expect_stop 10 "downloaded from the release differs from the local file" pre-publish
check "the sign workflow was not dispatched" no_dispatch
check "says assets were uploaded" out_has "assets were uploaded to desktop-v0.8.7"

echo "== scenario: the signing workflow fails (step 11) =="
new_fixture signfail
SCEN_ENV=(BB_SHIM_SIGN_RUN_FAIL=1)
run_release 0.8.7 alpha
expect_stop 11 "finished with conclusion 'failure'" pre-publish

echo "== scenario: another dispatch of the same workflow is in flight and fails; we wait on OUR run (steps 11, 12) =="
new_fixture extrarun
SCEN_ENV=(BB_SHIM_EXTRA_RUN=1)
run_release 0.8.7 alpha
check "the run exits 0 (the other run's failure is not ours)" rc_is 0
check "it followed the run gh workflow run created (1002 for signing, 1004 for the manifest)" \
  bash -c "grep -q 'run 1002 started' '$OUT' && grep -q 'run 1004 started' '$OUT'"
check "it never polled the unrelated runs 1001 and 1003" bash -c "! grep -qE '^gh run view (1001|1003) ' '$LOGF'"

echo "== scenario: gh prints no run URL and two runs are candidates: refuse, do not guess (step 11) =="
new_fixture extrarun_nourl
SCEN_ENV=(BB_SHIM_EXTRA_RUN=1 BB_SHIM_NO_RUN_URL=1)
run_release 0.8.7 alpha
expect_stop 11 "cannot tell which one is ours" pre-publish

echo "== scenario: gh prints no run URL and exactly one new run exists: that one is ours =="
new_fixture nourl
SCEN_ENV=(BB_SHIM_NO_RUN_URL=1)
run_release 0.8.7 alpha
check "the run exits 0 on the fallback" rc_is 0
check "the output says the run was found by elimination" out_has "no run URL from gh"

echo "== scenario: the decoder on this Mac only knows -D (macOS base64), step 11 still verifies the signature =="
new_fixture macbase64
run_release 0.8.7 alpha
check "the run exits 0 with a base64 that rejects --decode" rc_is 0
check "base64 was asked to decode (-D), never with --decode" bash -c "grep -qE '^base64 -D' '$LOGF' && ! grep -qE '^base64 (--decode|-d)( |\$)' '$LOGF'"
check "minisign still ran against the decoded signature and key" log_has '^minisign -Vm '

echo "== scenario: the signing run succeeds but no .sig is on the release (step 11) =="
new_fixture nosig
SCEN_ENV=(BB_SHIM_NO_SIG=1)
run_release 0.8.7 alpha
expect_stop 11 "the release has no Beebeeb.app.tar.gz.sig asset" pre-publish

echo "== scenario: the signature does not verify against the baked pubkey (step 11) =="
new_fixture badsig
SCEN_ENV=(BB_SHIM_MINISIGN_FAIL=1)
run_release 0.8.7 alpha
expect_stop 11 "command failed" pre-publish
check "minisign was given the pubkey from tauri.conf.json" log_has '^minisign -Vm .* -x .* -P RWSfixturepubkey$'

echo "== scenario: the manifest workflow fails (step 12) =="
new_fixture publishfail
SCEN_ENV=(BB_SHIM_PUBLISH_RUN_FAIL=1)
run_release 0.8.7 alpha
check "fails closed: exit code 1" rc_is 1
check "stops at step 12/13" out_has "FAILED at step 12/$TOTAL_STEPS"
check "names the failed run" out_has "release.yml run"
check "says the manifest workflow was dispatched" out_has "the release workflow was dispatched"
check "never claims DONE" bash -c "! grep -q '^DONE' '$OUT'"

echo "== scenario: the manifest is stale after publishing (step 13) =="
new_fixture stale
SCEN_ENV=(BB_SHIM_MANIFEST_VERSION=0.8.6)
run_release 0.8.7 alpha
check "fails closed: exit code 1" rc_is 1
check "stops at step 13/13" out_has "FAILED at step 13/$TOTAL_STEPS"
check "names the stale manifest version" out_has "manifest version is '0.8.6', expected '0.8.7'"
check "never claims DONE" bash -c "! grep -q '^DONE' '$OUT'"

echo "== scenario: the manifest points darwin-aarch64 at another tag (step 13) =="
new_fixture wrongurl
SCEN_ENV=(BB_SHIM_MANIFEST_TAG=desktop-v0.8.6)
run_release 0.8.7 alpha
check "fails closed: exit code 1" rc_is 1
check "stops at step 13/13" out_has "FAILED at step 13/$TOTAL_STEPS"
check "names the wrong url" out_has "darwin-aarch64 url is"

echo "== scenario: the model of the Tauri build is honest (the shim itself can go red) =="
new_fixture model
TAURI_OVERRIDE='{"bundle":{"createUpdaterArtifacts":false}}'
# model_build [ENV=v ...] -- <bunx args...>: the shim's bunx run directly inside the fixture repo, no signing key.
model_build() {
  local envs=()
  while [[ "${1:-}" != "--" ]]; do envs+=("$1"); shift; done
  shift
  ( cd "$REPO" && env -u TAURI_SIGNING_PRIVATE_KEY PATH="$SHIMS:$PATH" BB_TEST_STATE="$STATE" BB_TEST_LOG="$LOGF" \
      BEEBEEB_MACOS_HARDENED_RUNTIME=1 ${envs[@]+"${envs[@]}"} bunx "$@" ) >"$FX/model.out" 2>&1
}
model_fails() { ! model_build "$@"; }
check "the OLD invocation (no override, no --bundles) fails like Tauri did: updater on, pubkey set, no private key" \
  model_fails -- tauri build --target aarch64-apple-darwin -- --locked
check "  ...and says why (public key found, no private key)" grep -q 'A public key has been found, but no private key' "$FX/model.out"
check "the NEW invocation builds without the key" \
  model_build -- tauri build --target aarch64-apple-darwin --bundles app --config "$TAURI_OVERRIDE" -- --locked
check "a default-bundles build with Finder automation denied fails with -10006 (bundle_dmg.sh)" \
  model_fails BB_SHIM_FINDER_DENIED=1 -- tauri build --target aarch64-apple-darwin --config "$TAURI_OVERRIDE" -- --locked
check "  ...and says -10006" grep -q -- '-10006' "$FX/model.out"

echo "== scenario: Finder automation is denied (-10006) and the release does not care =="
new_fixture finder
SCEN_ENV=(BB_SHIM_FINDER_DENIED=1)
run_release 0.8.7 alpha
check "a release run with bundle_dmg.sh broken exits 0 (Tauri's dmg is never built)" rc_is 0
check "prints DONE" out_has "DONE: macOS 0.8.7 is published on the alpha channel"
check "the uploaded dmg is ours (contains the stapled app), not Tauri's" \
  bash -c "'$REAL_TAR' -tf '$STATE/release/Beebeeb_0.8.7_aarch64.dmg' | grep -q 'Beebeeb.app/Contents/_stapled'"

echo "== scenario: a TAURI_SIGNING_PRIVATE_KEY in the operator's shell never reaches the build =="
new_fixture signkeyleak
SCEN_ENV=(TAURI_SIGNING_PRIVATE_KEY=dummy-not-a-key TAURI_SIGNING_PRIVATE_KEY_PASSWORD=dummy)
run_release 0.8.7 alpha
check "the run exits 0" rc_is 0
check "the build saw NO updater signing key" log_has '^shim: tauri bundles=app updater=false dmg_bundling=0 signing_key=unset$'
check "the key value is not echoed anywhere" bash -c "! grep -q 'dummy-not-a-key' '$OUT' '$LOGF'"

echo "== scenario: the tag was re-cut after a failed run; --from-step N must not reuse the old commit's artifacts (step 1) =="
new_fixture recut
SCEN_ENV=(BB_SHIM_NOTARY_INVALID=dmg)
run_release 0.8.7 alpha
expect_stop 7 "notarization of the dmg was not Accepted" pre-upload
check "the failed run recorded the commit its artifacts came from" test -s "$REPO/src-tauri/target/release-macos-local/0.8.7/build-commit"
NOTARIZE_BEFORE="$(log_count '^xcrun notarytool submit ')"
(
  cd "$REPO" || exit 1
  echo '- a note added when the release was re-cut' >>RELEASE_NOTES.md
  "$REAL_GIT" commit -q -am "re-cut: notes fixed"
  "$REAL_GIT" tag -f -a desktop-v0.8.7 -m x >/dev/null
  "$REAL_GIT" push -q -f origin desktop-v0.8.7
) >/dev/null 2>&1
SCEN_ENV=()
run_release 0.8.7 alpha --from-step 7
expect_stop 1 "were built from commit" pre-upload
check "names both commits and says to rebuild" bash -c "grep -q 'but HEAD is' '$OUT' && grep -qi 'Remediation:.*rebuild' '$OUT'"
check "nothing new was submitted to Apple on the refused resume" bash -c "[[ \$(grep -cE '^xcrun notarytool submit ' '$LOGF') -eq $NOTARIZE_BEFORE ]]"
run_release 0.8.7 alpha --from-step 2
check "resuming at step 2 (a rebuild from the new commit) is allowed and finishes" rc_is 0
check "  ...and re-stamps the artifacts with the new HEAD" \
  bash -c "[[ \$(cat '$REPO/src-tauri/target/release-macos-local/0.8.7/build-commit') == \$('$REAL_GIT' -C '$REPO' rev-parse HEAD) ]]"

echo "== scenario: the build-commit stamp is missing on a resume (step 1) =="
new_fixture nostamp
SCEN_ENV=(BB_SHIM_NOTARY_INVALID=dmg)
run_release 0.8.7 alpha
expect_stop 7 "notarization of the dmg was not Accepted" pre-upload
rm -f "$REPO/src-tauri/target/release-macos-local/0.8.7/build-commit"
SCEN_ENV=()
run_release 0.8.7 alpha --from-step 7
expect_stop 1 "build-commit (the commit they were built from) does not exist" pre-upload

echo "== scenario: a failed rebuild leaves no stamp and no stale app behind (step 2) =="
new_fixture stalebuild
run_release 0.8.7 alpha
check "first run exits 0" rc_is 0
SCEN_ENV=(BB_SHIM_BUILD_FAIL=1)
run_release 0.8.7 alpha --from-step 2
check "the failed rebuild stops at step 2 with exit 1" bash -c "[[ $RC -eq 1 ]] && grep -q 'FAILED at step 2/13' '$OUT'"
check "the old stamp was removed before the build started" test ! -e "$REPO/src-tauri/target/release-macos-local/0.8.7/build-commit"
SCEN_ENV=()
run_release 0.8.7 alpha --from-step 3
check "fails closed at step 1 (a stale app cannot be resumed from)" rc_is 1
check "names the missing stamp" out_has "build-commit (the commit they were built from) does not exist"
check "no further notarization happened after the failed rebuild" bash -c "[[ \$(grep -cE '^xcrun notarytool submit ' '$LOGF') -eq 2 ]]"
check "no second upload happened after the failed rebuild" bash -c "[[ \$(grep -cE '^gh release upload ' '$LOGF') -eq 1 ]]"

echo "== scenario: RELEASE_NOTES.md has no lead marker but the Verification counts were never filled in (step 1) =="
new_fixture nocounts
(
  cd "$REPO" || exit 1
  write_notes RELEASE_NOTES.md '- Test gate: `bun test` pass / 0 fail, `cargo test --locked` per-binary `test result: ok. N passed`.'
  "$REAL_GIT" commit -q -am "notes with placeholder counts"
  "$REAL_GIT" tag -f -a desktop-v0.8.7 -m x >/dev/null
  "$REAL_GIT" push -q -f origin desktop-v0.8.7
) >/dev/null 2>&1
run_release 0.8.7 alpha
expect_stop 1 "Verification section carries no test counts" pre-upload
check "no build was attempted" log_lacks '^bunx '

echo "== scenario: only one of the two counts is filled in (step 1) =="
new_fixture onecount
(
  cd "$REPO" || exit 1
  write_notes RELEASE_NOTES.md '- Test gate: `bun test` 512 pass / 0 fail, `cargo test --locked` per-binary `test result: ok. N passed`.'
  "$REAL_GIT" commit -q -am "notes with one count"
  "$REAL_GIT" tag -f -a desktop-v0.8.7 -m x >/dev/null
  "$REAL_GIT" push -q -f origin desktop-v0.8.7
) >/dev/null 2>&1
run_release 0.8.7 alpha
expect_stop 1 "Verification section carries no test counts" pre-upload
check "names which count is missing" out_has "cargo test"

echo "== scenario: the Verification counts are zero (step 1) =="
new_fixture zerocounts
(
  cd "$REPO" || exit 1
  write_notes RELEASE_NOTES.md '- Test gate: `bun test` 0 pass / 0 fail, `cargo test --locked` per-binary `test result: ok. 0 passed`.'
  "$REAL_GIT" commit -q -am "notes with zero counts"
  "$REAL_GIT" tag -f -a desktop-v0.8.7 -m x >/dev/null
  "$REAL_GIT" push -q -f origin desktop-v0.8.7
) >/dev/null 2>&1
run_release 0.8.7 alpha
expect_stop 1 "Verification section carries no test counts" pre-upload

echo "== scenario: minisign is required up front (step 1) =="
new_fixture nominisign
SCEN_ENV=(BB_MINISIGN=minisign-not-installed-xyz)
run_release 0.8.7 alpha
expect_stop 1 "missing required command: minisign-not-installed-xyz" pre-upload
check "no build was attempted" log_lacks '^bunx '

echo "== scenario: --skip-local-sig-check is the explicit opt-out from the minisign requirement =="
new_fixture skipsig
SCEN_ENV=(BB_MINISIGN=minisign-not-installed-xyz)
run_release 0.8.7 alpha --skip-local-sig-check
check "the run exits 0 without minisign" rc_is 0
check "no minisign call was made" log_lacks '^minisign '
check "the warning says the signature is NOT verified against the tarball" bash -c "grep -q 'WARNING: --skip-local-sig-check' '$OUT' && grep -q 'NOT verified against the tarball' '$OUT'"
check "step 13 still ran and says it is a consistency check" out_has "consistency check only"

echo "== scenario: BB_UPDATER_VERIFY_PUBKEY overrides the key for a transition release (step 11) =="
new_fixture pubkeyoverride
SCEN_ENV=(BB_UPDATER_VERIFY_PUBKEY=RWSoverrideOldKeyAAAA1234)
run_release 0.8.7 alpha
check "the run exits 0" rc_is 0
check "minisign verified against the override, not the baked key" log_has '^minisign -Vm .* -x .* -P RWSoverrideOldKeyAAAA1234$'
check "the baked fixture key was NOT used" log_lacks '-P RWSfixturepubkey'
check "the output says the override was used, and why that is only for a transition release" bash -c "grep -q 'BB_UPDATER_VERIFY_PUBKEY' '$OUT' && grep -qi 'transition' '$OUT'"

echo "== scenario: a malformed BB_UPDATER_VERIFY_PUBKEY is refused up front (step 1) =="
new_fixture pubkeybad
SCEN_ENV=("BB_UPDATER_VERIFY_PUBKEY=not a key")
run_release 0.8.7 alpha
expect_stop 1 "BB_UPDATER_VERIFY_PUBKEY is not a minisign public key" pre-upload

echo "== scenario: hygiene =="
check "the script holds no private key block" bash -c "! grep -qE 'BEGIN (EC |RSA |OPENSSH )?PRIVATE KEY' '$SUT'"
check "the script never reads the .p8 file itself" bash -c "! grep -nE 'cat .*NOTARY_KEY_PATH|<.*NOTARY_KEY_PATH' '$SUT'"
check "the script is executable" test -x "$SUT"
check "the script passes bash -n" bash -n "$SUT"
echo
echo "release-macos-local: $PASSED passed, $FAILED failed"
[[ "$FAILED" -eq 0 ]] || exit 1
[[ "$PASSED" -eq "$EXPECTED_CHECKS" ]] || { echo "FAIL: $PASSED checks ran, expected $EXPECTED_CHECKS" >&2; exit 1; }
exit 0
