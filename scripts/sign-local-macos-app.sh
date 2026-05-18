#!/usr/bin/env bash
# Sign a local Beebeeb.app after the File Provider extension has been embedded.
# Use APPLE_SIGNING_IDENTITY for Developer ID/Apple Development signing; when it
# is unset we use ad-hoc signing for local bundle verification only.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

APP_PATH="${1:-src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Beebeeb.app}"
SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:--}"
APP_ENTITLEMENTS="src-tauri/entitlements.plist"
EXTENSION_ENTITLEMENTS="BeebeebFileProvider/BeebeebFileProvider.entitlements"
APPEX="$APP_PATH/Contents/PlugIns/BeebeebFileProvider.appex"
HELPER="$APP_PATH/Contents/MacOS/BeebeebFileProviderCtl"

[[ -d "$APP_PATH" ]] || {
  printf 'app bundle not found: %s\n' "$APP_PATH" >&2
  exit 1
}
[[ -d "$APPEX" ]] || {
  printf 'File Provider extension missing: %s\n' "$APPEX" >&2
  exit 1
}
[[ -f "$HELPER" ]] || {
  printf 'File Provider helper missing: %s\n' "$HELPER" >&2
  exit 1
}

codesign --force --sign "$SIGNING_IDENTITY" --timestamp=none --entitlements "$EXTENSION_ENTITLEMENTS" "$APPEX"
codesign --force --sign "$SIGNING_IDENTITY" --timestamp=none "$HELPER"
codesign --force --sign "$SIGNING_IDENTITY" --timestamp=none --entitlements "$APP_ENTITLEMENTS" "$APP_PATH"

codesign --verify --strict --verbose=2 "$APPEX"
codesign --verify --strict --verbose=2 "$HELPER"
codesign --verify --deep --strict --verbose=2 "$APP_PATH"
printf 'Signed local macOS app: %s\n' "$APP_PATH"
