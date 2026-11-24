#!/usr/bin/env bash
# Builds a TEST-identity bundle ad hoc in a temp dir (a stub Mach-O stands in for beebeeb-drive; the bundle is never
# launched) and checks Info.plist, the appex and helper, codesign, preflight --local, and that no secret is in the repo.
set -uo pipefail
[[ "$(uname)" == Darwin ]] || { echo "skip: macOS only"; exit 0; }
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
PASS=0 FAILN=0
check() { local m="$1"; shift; if "$@" >/dev/null 2>&1; then PASS=$((PASS+1)); echo "ok   $m"; else FAILN=$((FAILN+1)); echo "FAIL $m"; fi; }
eq() { [[ "$1" == "$2" ]]; }

cp /usr/bin/true "$T/beebeeb-drive"
unset APPLE_SIGNING_IDENTITY MACOS_APP_PROVISION_PROFILE MACOS_FILE_PROVIDER_PROVISION_PROFILE \
  MACOS_APP_PROVISION_PROFILE_BASE64 MACOS_FILE_PROVIDER_PROVISION_PROFILE_BASE64
export BEEBEEB_RELEASE_VERSION=9.8.7
check "bundle-app.sh --identity test" scripts/macos/bundle-app.sh --identity test --binary "$T/beebeeb-drive" --out "$T/out" --sign
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
check "appex bundle id (test)" eq "$(plutil -extract CFBundleIdentifier raw "$APP/Contents/PlugIns/BeebeebFileProvider.appex/Contents/Info.plist")" io.beebeeb.app.test.FileProvider
check "placeholder profile is empty" test ! -s "$APP/Contents/embedded.provisionprofile"
check "codesign --verify --deep --strict" codesign --verify --deep --strict "$APP"
check "preflight.sh --local" scripts/macos/preflight.sh --local "$APP"
check "preflight.sh (full) rejects the placeholder" bash -c '! scripts/macos/preflight.sh "$0"' "$APP"
check "unversioned env: bad version refused" bash -c '! BEEBEEB_RELEASE_VERSION=x scripts/macos/bundle-app.sh --identity real --binary "$0" --out "$1/o2"' "$T/beebeeb-drive" "$T"
check "no real profile or key tracked" bash -c '! git ls-files | grep -Eq "\.(provisionprofile|mobileprovision|p12|p8|cer)$"'
check "no profile base64 in scripts" bash -c '! git grep -nE "(PROVISION_PROFILE_BASE64|SIGNING_IDENTITY)=[\"'"'"']?[A-Za-z0-9+/]{20,}" -- scripts packaging'
echo "macos bundle: $PASS pass / $FAILN fail"
[[ $FAILN == 0 ]]
