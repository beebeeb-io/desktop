#!/usr/bin/env bash
# Builds a TEST-identity bundle ad hoc in a temp dir (a stub Mach-O stands in for beebeeb-drive; the bundle is never
# launched; another stands in for the Office helper) and checks Info.plist, the appex and helpers, codesign, preflight --local, and that no secret is in the repo.
set -uo pipefail
[[ "$(uname)" == Darwin ]] || { echo "skip: macOS only"; exit 0; }
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
PASS=0 FAILN=0
check() { local m="$1"; shift; if "$@" >/dev/null 2>&1; then PASS=$((PASS+1)); echo "ok   $m"; else FAILN=$((FAILN+1)); echo "FAIL $m"; fi; }
eq() { [[ "$1" == "$2" ]]; }

cp /usr/bin/true "$T/beebeeb-drive"
cp /usr/bin/true "$T/beebeeb-office-render"
unset APPLE_SIGNING_IDENTITY MACOS_APP_PROVISION_PROFILE MACOS_FILE_PROVIDER_PROVISION_PROFILE \
  MACOS_APP_PROVISION_PROFILE_BASE64 MACOS_FILE_PROVIDER_PROVISION_PROFILE_BASE64
export BEEBEEB_RELEASE_VERSION=9.8.7
check "bundle-app.sh --identity test" scripts/macos/bundle-app.sh --identity test --binary "$T/beebeeb-drive" --office-helper "$T/beebeeb-office-render" --out "$T/out" --sign
APP="$T/out/Beebeeb Test.app"; P="$APP/Contents/Info.plist"
check "app dir named from identity" test -d "$APP"
ex() { plutil -extract "$1" raw "$P" 2>/dev/null; }
check "CFBundleIdentifier" eq "$(ex CFBundleIdentifier)" io.beebeeb.app.test
check "CFBundleName" eq "$(ex CFBundleName)" "Beebeeb Test"
check "version from env" eq "$(ex CFBundleShortVersionString)" 9.8.7
check "LSMinimumSystemVersion" eq "$(ex LSMinimumSystemVersion)" 14.0
check "category" eq "$(ex LSApplicationCategoryType)" public.app-category.productivity
check "URL scheme" eq "$(ex CFBundleURLTypes.0.CFBundleURLSchemes.0)" beebeeb-test
check "icon" test -f "$APP/Contents/Resources/icon.icns"
check "appex inside" test -x "$APP/Contents/PlugIns/BeebeebFileProvider.appex/Contents/MacOS/BeebeebFileProvider"
check "helper inside" test -x "$APP/Contents/MacOS/BeebeebFileProviderCtl"
OFFICE="$APP/Contents/MacOS/beebeeb-office-render"
check "office helper inside" test -x "$OFFICE"
check "office helper signed" codesign --verify --strict "$OFFICE"
# Exactly the two keys, both true: sandboxed, and the sandbox is the app's (inherit). Anything more and macOS refuses the start.
keys() { plutil -convert json -o - "$1" 2>/dev/null | tr -d ' \n{}' | tr ',' '\n' | sort | tr '\n' ' '; } # every key with its value, sorted
TWO='"com.apple.security.app-sandbox":true "com.apple.security.inherit":true '
codesign -d --entitlements - --xml "$OFFICE" 2>/dev/null > "$T/office-ent.plist"
check "office helper entitlements: two keys" eq "$(keys "$T/office-ent.plist")" "$TWO"
check "office entitlements file: the same two keys" eq "$(keys packaging/macos/office-render.entitlements)" "$TWO"
check "sign-app.sh signs the office helper hardened and timestamped with the app" bash -c 'grep -q -- "--options runtime --timestamp" scripts/macos/sign-app.sh && grep -q "OFFICE_ENT\" \"\$OFFICE\"" scripts/macos/sign-app.sh'
check "sign-app.sh refuses a bundle without the office helper" bash -c 'cp -R "$0" "$1/nohelper.app" && rm "$1/nohelper.app/Contents/MacOS/beebeeb-office-render" && ! scripts/macos/sign-app.sh "$1/nohelper.app"' "$APP" "$T"
check "appex bundle id (test)" eq "$(plutil -extract CFBundleIdentifier raw "$APP/Contents/PlugIns/BeebeebFileProvider.appex/Contents/Info.plist")" io.beebeeb.app.test.FileProvider
check "placeholder profile is empty" test ! -s "$APP/Contents/embedded.provisionprofile"
check "codesign --verify --deep --strict" codesign --verify --deep --strict "$APP"
check "preflight.sh --local" scripts/macos/preflight.sh --local "$APP"
check "preflight.sh (full) rejects the placeholder" bash -c '! scripts/macos/preflight.sh "$0"' "$APP"
check "preflight.sh accepts the team app-group wildcard" grep -qF '"R8352WDJJR.*"' scripts/macos/preflight.sh
check "unversioned env: bad version refused" bash -c '! BEEBEEB_RELEASE_VERSION=x scripts/macos/bundle-app.sh --identity real --binary "$0" --office-helper "$0" --out "$1/o2"' "$T/beebeeb-drive" "$T"
check "no real profile or key tracked" bash -c '! git ls-files | grep -Eq "\.(provisionprofile|mobileprovision|p12|p8|cer)$"'
check "no profile base64 in scripts" bash -c '! git grep -nE "(PROVISION_PROFILE_BASE64|SIGNING_IDENTITY)=[\"'"'"']?[A-Za-z0-9+/]{20,}" -- scripts packaging'
echo "macos bundle: $PASS pass / $FAILN fail"
[[ $FAILN == 0 ]]
