#!/usr/bin/env bash
# Builds a TEST-identity bundle ad hoc in a temp dir (a stub Mach-O stands in for beebeeb-drive; the bundle is never
# launched; another stands in for the Office helper, bundled as the app's XPC service) and checks Info.plist, the appex and helpers, codesign, preflight --local, and that no secret is in the repo.
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
# The Office helper ships as the app's XPC service, named by its identifier, and nowhere else.
OFFICE_ID="io.beebeeb.app.test.office-render"
OFFICE="$APP/Contents/XPCServices/$OFFICE_ID.xpc"
OP="$OFFICE/Contents/Info.plist"
ox() { plutil -extract "$1" raw "$OP" 2>/dev/null; }
check "office XPC service bundle inside" test -d "$OFFICE"
check "office XPC service is the only service" eq "$(ls "$APP/Contents/XPCServices")" "$OFFICE_ID.xpc"
check "office XPC service executable" test -x "$OFFICE/Contents/MacOS/beebeeb-office-render"
check "no office helper beside the app executable" test ! -e "$APP/Contents/MacOS/beebeeb-office-render"
check "office XPC Info.plist is valid" plutil -lint "$OP"
check "office XPC CFBundleIdentifier is its folder name" eq "$(ox CFBundleIdentifier)" "$OFFICE_ID"
check "office XPC CFBundlePackageType" eq "$(ox CFBundlePackageType)" 'XPC!'
check "office XPC CFBundleExecutable" eq "$(ox CFBundleExecutable)" beebeeb-office-render
check "office XPC ServiceType" eq "$(ox XPCService.ServiceType)" Application
check "office XPC version from env" eq "$(ox CFBundleShortVersionString)" 9.8.7
check "office XPC service signed" codesign --verify --strict "$OFFICE"
check "office XPC signature is the bundle's (sealed Info.plist)" bash -c 'codesign -dv "$0" 2>&1 | grep -q "Identifier=$1"' "$OFFICE" "$OFFICE_ID"
# Exactly the one key, true: the App Sandbox the system starts the service in. No inherit, nothing more.
keys() { plutil -convert json -o - "$1" 2>/dev/null | tr -d ' \n{}' | tr ',' '\n' | sort | tr '\n' ' '; } # every key with its value, sorted
ONE='"com.apple.security.app-sandbox":true '
codesign -d --entitlements - --xml "$OFFICE" 2>/dev/null > "$T/office-ent.plist"
check "office XPC service entitlements: exactly one key" eq "$(keys "$T/office-ent.plist")" "$ONE"
check "office entitlements file: the same one key" eq "$(keys packaging/macos/office-render.entitlements)" "$ONE"
check "sign-app.sh signs the office service hardened and timestamped with the app" bash -c 'grep -q -- "--options runtime --timestamp" scripts/macos/sign-app.sh && grep -q "OFFICE_ENT\" \"\$OFFICE\"" scripts/macos/sign-app.sh'
check "sign-app.sh refuses a bundle without the office service" bash -c 'cp -R "$0" "$1/nohelper.app" && rm -rf "$1/nohelper.app/Contents/XPCServices" && ! scripts/macos/sign-app.sh "$1/nohelper.app"' "$APP" "$T"
check "sign-app.sh refuses a helper beside the app executable" bash -c 'cp -R "$0" "$1/beside.app" && cp /usr/bin/true "$1/beside.app/Contents/MacOS/beebeeb-office-render" && ! scripts/macos/sign-app.sh "$1/beside.app"' "$APP" "$T"
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
