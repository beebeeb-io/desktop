#!/usr/bin/env bash
# Assemble the native macOS app: <out>/<Product>.app from a built beebeeb-drive binary.
# Spec: native/beebeeb-drive-rune/docs/macos/packaging-signing-update.md §1, §2, §6.
#   scripts/macos/bundle-app.sh --identity real|test --binary <path> --out <dir> [--sign]
# Nothing here launches the app. Signing is scripts/macos/sign-app.sh (run with --sign, or by hand).
# Provisioning: MACOS_APP_PROVISION_PROFILE / MACOS_FILE_PROVIDER_PROVISION_PROFILE (a path) or the
# *_BASE64 twin; with neither, an empty placeholder is embedded and preflight.sh rejects it.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

IDENTITY="" BINARY="" OUT="" SIGN=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --identity) IDENTITY="${2:-}"; shift 2 ;;
    --binary) BINARY="${2:-}"; shift 2 ;;
    --out) OUT="${2:-}"; shift 2 ;;
    --sign) SIGN=1; shift ;;
    *) printf 'unknown argument: %s\n' "$1" >&2; exit 2 ;;
  esac
done

# Names mirror AppIdentity in native/beebeeb-drive-rune/src/shell/identity.rs.
case "$IDENTITY" in
  real) BUNDLE_ID="io.beebeeb.app"; PRODUCT="Beebeeb"; SCHEME="beebeeb" ;;
  test) BUNDLE_ID="io.beebeeb.app.test"; PRODUCT="Beebeeb Test"; SCHEME="beebeeb-test" ;;
  *) printf 'usage: bundle-app.sh --identity real|test --binary <path> --out <dir> [--sign]\n' >&2; exit 2 ;;
esac
[[ -f "$BINARY" ]] || { printf 'binary not found: %s\n' "${BINARY:-<unset>}" >&2; exit 1; }
[[ -n "$OUT" ]] || { printf '--out is required\n' >&2; exit 2; }

if [[ -n "${BEEBEEB_RELEASE_VERSION:-}" ]]; then
  VERSION="$BEEBEEB_RELEASE_VERSION"
else
  VERSION="$(sed -n '/^\[package\]/,/^\[/{s/^version *= *"\([^"]*\)".*/\1/p;}' native/beebeeb-drive-rune/Cargo.toml | head -1)"
fi
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]] || { printf 'bad version: %s\n' "$VERSION" >&2; exit 1; }

EXE="beebeeb-drive"
APP="$OUT/$PRODUCT.app"
CONTENTS="$APP/Contents"
rm -rf "$APP"
mkdir -p "$CONTENTS/MacOS" "$CONTENTS/Resources" "$CONTENTS/PlugIns"

cp "$BINARY" "$CONTENTS/MacOS/$EXE"
chmod 755 "$CONTENTS/MacOS/$EXE"
cp packaging/macos/icon.icns "$CONTENTS/Resources/icon.icns"
cp packaging/macos/tray-template.png packaging/macos/tray-template@2x.png "$CONTENTS/Resources/"

sed -e "s|@EXECUTABLE@|$EXE|g" -e "s|@BUNDLE_ID@|$BUNDLE_ID|g" -e "s|@PRODUCT_NAME@|$PRODUCT|g" \
    -e "s|@VERSION@|$VERSION|g" -e "s|@URL_SCHEME@|$SCHEME|g" \
    packaging/macos/Info.plist.in > "$CONTENTS/Info.plist"
plutil -lint "$CONTENTS/Info.plist" >/dev/null

# File Provider extension + Ctl helper (the script signs them; sign-app.sh signs them again inside out).
FP_DIR="$(mktemp -d)"
trap 'rm -rf "$FP_DIR"' EXIT
scripts/build-fileprovider-extension.sh --out "$FP_DIR/BeebeebFileProvider.appex" --helper-out "$FP_DIR/BeebeebFileProviderCtl"
cp -R "$FP_DIR/BeebeebFileProvider.appex" "$CONTENTS/PlugIns/BeebeebFileProvider.appex"
cp "$FP_DIR/BeebeebFileProviderCtl" "$CONTENTS/MacOS/BeebeebFileProviderCtl"
if [[ "$IDENTITY" == test ]]; then
  # The test app's extension must not collide with the real one.
  plutil -replace CFBundleIdentifier -string "$BUNDLE_ID.FileProvider" "$CONTENTS/PlugIns/BeebeebFileProvider.appex/Contents/Info.plist"
fi

# Provisioning profiles: path, else base64, else an empty placeholder.
embed_profile() { # <path-var> <b64-var> <dest>
  local p="${!1:-}" b="${!2:-}"
  if [[ -n "$p" ]]; then
    [[ -f "$p" ]] || { printf 'provisioning profile not found (%s)\n' "$1" >&2; exit 1; }
    cp "$p" "$3"
  elif [[ -n "$b" ]]; then
    printf '%s' "$b" | base64 -D > "$3"
  else
    : > "$3"
  fi
}
embed_profile MACOS_APP_PROVISION_PROFILE MACOS_APP_PROVISION_PROFILE_BASE64 "$CONTENTS/embedded.provisionprofile"
embed_profile MACOS_FILE_PROVIDER_PROVISION_PROFILE MACOS_FILE_PROVIDER_PROVISION_PROFILE_BASE64 \
  "$CONTENTS/PlugIns/BeebeebFileProvider.appex/Contents/embedded.provisionprofile"

if [[ "$SIGN" == 1 ]]; then scripts/macos/sign-app.sh "$APP"; fi
printf 'Bundled %s (%s, %s)\n' "$APP" "$BUNDLE_ID" "$VERSION"
