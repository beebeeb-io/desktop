#!/usr/bin/env bash
# Test of scripts/macos/release-macos-local.sh against fake xcrun/codesign/spctl/hdiutil/gh/security/cargo/uname on PATH,
# inside a throwaway git repo (bundle-app.sh and preflight.sh are stubbed there). Touches no Keychain, network or app.
#   bash scripts/tests/test-release-macos-local.sh
set -uo pipefail
SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
PASS=0 FAILN=0
ok() { PASS=$((PASS + 1)); printf 'ok   %s\n' "$1"; }
bad() { FAILN=$((FAILN + 1)); printf 'FAIL %s\n' "$1"; }

# ---- fakes
FB="$T/bin"; mkdir -p "$FB"
cat >"$FB/xcrun" <<'F'
#!/usr/bin/env bash
echo "xcrun $*" >>"$FAKE_LOG"
case "$1 $2" in
  "notarytool submit") echo "{\"id\":\"sub-1\",\"status\":\"${FAKE_NOTARY_STATUS:-Accepted}\"}"; [[ "${FAKE_NOTARY_STATUS:-Accepted}" == Accepted ]] ;;
  "notarytool log") echo '{"issues":[{"path":"Beebeeb.app/x","message":"not signed with hardened runtime"}]}' >"${@: -1}" ;;
  "stapler staple"|"stapler validate") exit 0 ;;
esac
F
cat >"$FB/codesign" <<'F'
#!/usr/bin/env bash
echo "codesign $*" >>"$FAKE_LOG"
F
cat >"$FB/spctl" <<'F'
#!/usr/bin/env bash
echo "spctl $*" >>"$FAKE_LOG"
echo "${@: -1}: accepted"; echo "source=${FAKE_SPCTL_SOURCE:-Notarized Developer ID}"
[[ "${FAKE_SPCTL_REJECT:-0}" == 0 ]]
F
cat >"$FB/hdiutil" <<'F'
#!/usr/bin/env bash
echo "hdiutil $*" >>"$FAKE_LOG"
[[ "$1" == create ]] && : >"${@: -1}"
exit 0
F
cat >"$FB/gh" <<'F'
#!/usr/bin/env bash
echo "gh $*" >>"$FAKE_LOG"
case "$1 $2" in
  "auth status") ;;
  "release view") [[ "${FAKE_NO_RELEASE:-0}" == 0 ]] && echo '{"isDraft":false}' || exit 1 ;;
  "workflow run") echo "https://github.com/x/y/actions/runs/42" ;;
  "run view") echo '{"status":"completed","conclusion":"success"}' ;;
esac
F
cat >"$FB/security" <<'F'
#!/usr/bin/env bash
[[ "${FAKE_NO_IDENTITY:-0}" == 0 ]] && echo '  1) AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA "Developer ID Application: Test (T)"'
F
printf '#!/usr/bin/env bash\necho "cargo $*" >>"$FAKE_LOG"; mkdir -p "$CARGO_TARGET_DIR/release"; printf "beebeeb-updater-not-before=%%s;" "${BEEBEEB_BUILD_NOT_BEFORE:-unset}" >"$CARGO_TARGET_DIR/release/beebeeb-drive"\n' >"$FB/cargo"
printf '#!/usr/bin/env bash\n[[ "$1" == -s ]] && echo Darwin || echo arm64\n' >"$FB/uname"
chmod +x "$FB"/*

# ---- fixture repo: a clone of a bare "origin" with the tag, our scripts, stub bundle/preflight
new_repo() { # <dir>
  local d="$1"
  rm -rf "$d" "$d.git"; git init -q --bare "$d.git"
  git init -q "$d"; mkdir -p "$d/scripts/macos"
  cp "$SRC/scripts/macos/release-macos-local.sh" "$SRC/scripts/macos/notarise.sh" "$d/scripts/macos/"
  cp "$SRC/scripts/release-blockers-check.sh" "$d/scripts/"
  cat >"$d/scripts/macos/bundle-app.sh" <<'B'
#!/usr/bin/env bash
echo "bundle-app $* HR=${BEEBEEB_MACOS_HARDENED_RUNTIME:-} V=${BEEBEEB_RELEASE_VERSION:-} UPK=${TAURI_SIGNING_PRIVATE_KEY:-unset}" >>"$FAKE_LOG"
while [[ $# -gt 0 ]]; do [[ "$1" == --out ]] && OUT="$2"; shift; done
mkdir -p "$OUT/Beebeeb.app/Contents"
cat >"$OUT/Beebeeb.app/Contents/Info.plist" <<P
<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleShortVersionString</key><string>${FAKE_BUNDLE_VERSION:-$BEEBEEB_RELEASE_VERSION}</string></dict></plist>
P
B
  printf '#!/usr/bin/env bash\necho "preflight $*" >>"$FAKE_LOG"\n' >"$d/scripts/macos/preflight.sh"
  chmod +x "$d/scripts/macos/"*.sh
  cat >"$d/RELEASE_NOTES.md" <<'N'
# 0.9.0
Verification: 12 pass / 0 fail. test result: ok. 34 passed; 0 failed
N
  printf '# Release blockers\n' >"$d/RELEASE_BLOCKERS.md"
  printf 'target/\n' >"$d/.gitignore"
  git -C "$d" add -A; git -C "$d" -c user.email=t@t -c user.name=t commit -qm init
  git -C "$d" remote add origin "$d.git"; git -C "$d" tag desktop-v0.9.0
  git -C "$d" push -q origin HEAD:refs/heads/main --tags
}
# rel <name> [env...] -- [args...]: run the script in a fresh repo; sets OUT, RC, LOG
rel() {
  local name="$1"; shift
  local envs=() ; while [[ $# -gt 0 && "$1" != -- ]]; do envs+=("$1"); shift; done; shift
  R="$T/$name"; [[ -d "$R/.git" && "${KEEP:-0}" == 1 ]] || new_repo "$R"
  [[ -n "${PRE:-}" ]] && eval "$PRE"
  LOG="$T/$name.log"; : >"$LOG"; : >"$T/key"
  OUT="$(cd "$R" && env PATH="$FB:$PATH" FAKE_LOG="$LOG" HOME="$T/home" CARGO_TARGET_DIR="$T/$name.tgt" \
    BB_RELEASE_WORK="$T/$name.work" APPLE_SIGNING_IDENTITY=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA \
    MACOS_APP_PROVISION_PROFILE="$T/key" MACOS_FILE_PROVIDER_PROVISION_PROFILE="$T/key" BB_NOTARY_KEY_PATH="$T/key" \
    BB_NOTARY_KEY_ID=K BB_NOTARY_ISSUER=I BB_POLL_INTERVAL=0 TAURI_SIGNING_PRIVATE_KEY=secret "${envs[@]}" \
    scripts/macos/release-macos-local.sh "${ARGS[@]}" 2>&1)"; RC=$?
}
ARGS=(0.9.0 alpha)
expect_fail() { # <name> <needle>
  if [[ $RC -ne 0 && "$OUT" == *"$2"* ]]; then ok "$1"; else bad "$1 (rc=$RC) ${OUT: -300}"; fi
}
no_upload() { ! grep -qE 'gh (release upload|workflow run)' "$LOG"; }

# 1. happy path
rel happy --; [[ $RC -eq 0 ]] && ok "happy path exits 0" || bad "happy path rc=$RC: ${OUT: -400}"
grep -q 'bundle-app .*HR=1 V=0.9.0 UPK=unset' "$LOG" && ok "hardened runtime, version, updater key not in build env" || bad "build env"
grep -q 'notarytool submit .*\.zip' "$LOG" && grep -q 'notarytool submit .*\.dmg' "$LOG" && ok "app and dmg both notarised" || bad "notarise both"
[[ "$(grep -c 'stapler staple' "$LOG")" == 2 ]] && ok "app and dmg stapled" || bad "staple count"
grep -q 'spctl -a -vv -t install' "$LOG" && ok "dmg assessed with -t install" || bad "spctl install"
grep -q 'hdiutil create .*UDZO' "$LOG" && ok "dmg by hdiutil" || bad "hdiutil create"
grep -q 'codesign --force --sign AAAA.* --timestamp .*\.dmg' "$LOG" && ok "dmg signed with a timestamp" || bad "dmg sign"
grep -q 'gh release upload desktop-v0.9.0' "$LOG" && grep -q 'gh workflow run release-native-macos.yml' "$LOG" && ok "upload then publish workflow" || bad "upload/workflow"
tar -tzf "$T/happy.work/Beebeeb.app.tar.gz" >/dev/null 2>&1 && ok "updater tarball made" || bad "tarball"

# 2. dry run publishes nothing, runs no notarisation
ARGS=(0.9.0 alpha --dry-run); rel dry --
[[ $RC -eq 0 ]] && ok "--dry-run exits 0" || bad "dry rc=$RC ${OUT: -300}"
no_upload && ! grep -qE 'notarytool|hdiutil|stapler|^cargo|bundle-app' "$LOG" && ok "--dry-run runs nothing and publishes nothing" || bad "dry-run ran something"
ARGS=(0.9.0 alpha)

# 3. notary branches
rel inv FAKE_NOTARY_STATUS=Invalid --
expect_fail "notary Invalid on the app stops" "was not accepted"
[[ "$OUT" == *"hardened runtime"* ]] && ok "Invalid: the notary log issues are printed" || bad "issues not printed"
grep -q 'notarytool log' "$LOG" && ok "Invalid: log fetched" || bad "log not fetched"
no_upload && ! grep -q 'stapler' "$LOG" && ok "Invalid: nothing stapled or uploaded" || bad "Invalid went on"

# 4. spctl wording
rel spw FAKE_SPCTL_SOURCE="Developer ID" --
expect_fail "spctl accepting without 'Notarized Developer ID' refuses" "Notarized Developer ID"
no_upload && ok "spctl wording: nothing uploaded" || bad "uploaded after bad spctl"
rel spr FAKE_SPCTL_REJECT=1 --
expect_fail "spctl rejecting refuses" "spctl rejected"

# 5. version mismatch
rel ver FAKE_BUNDLE_VERSION=0.8.9 --
expect_fail "bundle version mismatch refused" "reports version"

# 6. step 1 refusals
rel nol FAKE_NO_RELEASE=1 --;      expect_fail "CI release missing" "not found"
rel noid FAKE_NO_IDENTITY=1 --;    expect_fail "identity not in keychain" "not in the keychain"
rel nokey BB_NOTARY_KEY_PATH="$T/none" --; expect_fail "notary key missing" "notary API key"
PRE='echo dirty >"$R/dirty.txt"' rel dirt --; expect_fail "dirty tree" "not clean"
PRE='git -C "$R" -c user.email=t@t -c user.name=t commit -q --allow-empty -m more' rel moved --; expect_fail "HEAD is not the tag commit" "not the commit tag"
PRE='git -C "$R" push -q origin :refs/tags/desktop-v0.9.0' rel notag --; expect_fail "tag not on origin" "does not exist on origin"
PRE='sed "s/0.9.0/0.8.0/" "$R/RELEASE_NOTES.md" >"$R/n.tmp"; mv "$R/n.tmp" "$R/RELEASE_NOTES.md"; git -C "$R" -c user.email=t@t -c user.name=t commit -qam n; git -C "$R" tag -f desktop-v0.9.0 >/dev/null; git -C "$R" push -q -f origin --tags' rel nover --
expect_fail "notes do not name the version" "does not mention"
PRE='echo "<!-- lead: fill -->" >>"$R/RELEASE_NOTES.md"; git -C "$R" -c user.email=t@t -c user.name=t commit -qam n; git -C "$R" tag -f desktop-v0.9.0 >/dev/null; git -C "$R" push -q -f origin --tags' rel lead --
expect_fail "lead marker in notes" "lead markers"
PRE='printf "# 0.9.0\nVerification: 0 pass / 0 fail\n" >"$R/RELEASE_NOTES.md"; git -C "$R" -c user.email=t@t -c user.name=t commit -qam n; git -C "$R" tag -f desktop-v0.9.0 >/dev/null; git -C "$R" push -q -f origin --tags' rel zero --
expect_fail "zero test counts" "no test counts"
PRE='echo "- [macOS] not yet" >>"$R/RELEASE_BLOCKERS.md"; git -C "$R" -c user.email=t@t -c user.name=t commit -qam n; git -C "$R" tag -f desktop-v0.9.0 >/dev/null; git -C "$R" push -q -f origin --tags' rel blk --
expect_fail "[macOS] blocker refuses" "release blockers"
ARGS=(0.9 alpha); rel arg2 --; [[ $RC -eq 2 ]] && ok "bad version refused" || bad "bad version rc=$RC"
ARGS=(0.9.0 nightly); rel arg3 --; [[ $RC -eq 2 ]] && ok "bad channel refused" || bad "bad channel rc=$RC"
ARGS=(0.9.0 alpha --from-step 5); rel stamp --; expect_fail "resume without build stamp refused" "does not exist"
no_upload && ok "refusals upload nothing" || bad "refusal uploaded"

# 7. hygiene
! git -C "$SRC" grep -nE 'BEGIN (PRIVATE|RSA)|AuthKey_[A-Z0-9]{10}\.p8' -- scripts/macos scripts/tests/test-release-macos-local.sh >/dev/null && ok "no key material in the scripts" || bad "key material in repo"

printf '\n%s passed, %s failed\n' "$PASS" "$FAILN"
[[ $FAILN -eq 0 ]]
