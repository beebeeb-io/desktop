#!/usr/bin/env bash
# Trust checks (packaging-signing-update.md §7) on a bundled Beebeeb .app.
#   scripts/macos/preflight.sh [--local] <App.app>
# --local skips the Developer ID ones: real provisioning profiles, team id, hardened runtime, spctl, identity.
set -uo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LOCAL=0
[[ "${1:-}" == "--local" ]] && { LOCAL=1; shift; }
APP="${1:-}"
[[ -d "$APP" ]] || { printf 'usage: preflight.sh [--local] <App.app>\n' >&2; exit 2; }
APP="$(cd "$APP" && pwd)"
cd "$ROOT_DIR"
APPEX="$APP/Contents/PlugIns/BeebeebFileProvider.appex"
HELPER="$APP/Contents/MacOS/BeebeebFileProviderCtl"
FAIL=0
ok() { printf 'ok   %s\n' "$1"; }
bad() { printf 'FAIL %s\n' "$1" >&2; FAIL=1; }
chk() { local msg="$1"; shift; if "$@" >/dev/null 2>&1; then ok "$msg"; else bad "$msg"; fi; }

BID="$(plutil -extract CFBundleIdentifier raw "$APP/Contents/Info.plist" 2>/dev/null || true)"

# 1. Profiles
profile_check() { # <bundle dir> <bundle id>
  local prof="$1/Contents/embedded.provisionprofile" dec
  [[ -s "$prof" ]] || { bad "$2: embedded.provisionprofile missing or empty"; return; }
  dec="$(security cms -D -i "$prof" 2>/dev/null)" || { bad "$2: profile does not decode"; return; }
  local appid platform groups
  appid="$(printf '%s' "$dec" | plutil -extract Entitlements.com\\.apple\\.application-identifier raw - 2>/dev/null)"
  [[ "$appid" == *".$2" ]] && ok "$2: profile application identifier" || bad "$2: profile application identifier ($appid)"
  groups="$(printf '%s' "$dec" | plutil -extract Entitlements.com\\.apple\\.security\\.application-groups json -o - - 2>/dev/null)"
  [[ "$groups" == *"io.beebeeb.app.fileprovider"* || "$groups" == *'"*"'* ]] && ok "$2: profile app group" || bad "$2: profile app group"
  platform="$(printf '%s' "$dec" | plutil -extract Platform json -o - - 2>/dev/null)"
  [[ "$platform" == *OSX* || "$platform" == *macOS* ]] && ok "$2: profile platform" || bad "$2: profile platform"
}
if [[ $LOCAL == 0 ]]; then
  profile_check "$APP" "$BID"
  profile_check "$APPEX" "$BID.FileProvider"
fi

# 2. Extension entry point
[[ ! -e BeebeebFileProvider/main.swift ]] && ok "no BeebeebFileProvider/main.swift" || bad "BeebeebFileProvider/main.swift exists"
if grep -rq NSExtensionMain BeebeebFileProvider --include=*.swift; then bad "a Swift file names NSExtensionMain"; else ok "no Swift file names NSExtensionMain"; fi
EXTBIN="$APPEX/Contents/MacOS/BeebeebFileProvider"
if [[ -f "$EXTBIN" ]]; then
  SYMS="$(nm -m "$EXTBIN" 2>/dev/null)"; LOADS="$(otool -l "$EXTBIN" 2>/dev/null)"
  grep -Eq ' _main$' <<<"$SYMS" && bad "extension defines its own _main" || ok "extension defines no _main"
  grep -Eq '\(undefined\) external _NSExtensionMain ' <<<"$SYMS" && ok "extension imports _NSExtensionMain" || bad "extension does not import _NSExtensionMain"
  grep -q 'LC_MAIN' <<<"$LOADS" && ok "extension has LC_MAIN" || bad "extension has no LC_MAIN"
else bad "extension binary missing"; fi

# 3. Files lint
chk "plutil -lint entitlements" plutil -lint packaging/macos/entitlements.plist
chk "plutil -lint extension Info.plist" plutil -lint "$APPEX/Contents/Info.plist"
chk "plutil -lint app Info.plist" plutil -lint "$APP/Contents/Info.plist"

# 4. Config
case "$BID" in io.beebeeb.app|io.beebeeb.app.test) ok "bundle identifier $BID" ;; *) bad "bundle identifier $BID" ;; esac

# 5/6. Signature
chk "codesign extension (strict)" codesign --verify --strict "$APPEX"
chk "codesign helper (strict)" codesign --verify --strict "$HELPER"
chk "codesign app (deep, strict)" codesign --verify --deep --strict "$APP"
if [[ $LOCAL == 0 ]]; then
  security find-identity -v -p codesigning | grep -q 'Developer ID Application' && ok "Developer ID identity in keychain" || bad "no Developer ID Application identity"
  for b in "$APPEX" "$HELPER" "$APP"; do
    info="$(codesign -dvv "$b" 2>&1)"
    [[ "$info" == *"TeamIdentifier="* && "$info" != *"TeamIdentifier=not set"* ]] && ok "TeamIdentifier set: ${b##*/}" || bad "TeamIdentifier not set: ${b##*/}"
    [[ "$info" == *"flags=0x10000(runtime)"* ]] && ok "hardened runtime: ${b##*/}" || bad "no hardened runtime: ${b##*/}"
  done
  spctl --assess --type execute --verbose=2 "$APP" 2>&1 | grep -q 'Notarized Developer ID' && ok "spctl: Notarized Developer ID" || bad "spctl does not say Notarized Developer ID"
fi
exit $FAIL
