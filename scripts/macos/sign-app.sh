#!/usr/bin/env bash
# Sign a bundled Beebeeb .app inside out: extension (its entitlements), the File Provider helper, the Office helper
# (its own two entitlements: sandboxed, inheriting the app's sandbox), then the app.
# Ad hoc without APPLE_SIGNING_IDENTITY (local bundle checks only). APPLE_SIGNING_IDENTITY may be a
# certificate name or its 40-hex SHA-1 (preferred: names can match several certificates).
# BEEBEEB_MACOS_HARDENED_RUNTIME=1 adds --options runtime and a secure timestamp (a notarised release).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
APP="${1:-}"
[[ -d "$APP" ]] || { printf 'usage: sign-app.sh <App.app>\n' >&2; exit 2; }
APP="$(cd "$APP" && pwd)"
cd "$ROOT_DIR"

IDENTITY="${APPLE_SIGNING_IDENTITY:--}"
APP_ENT="packaging/macos/entitlements.plist"
EXT_ENT="BeebeebFileProvider/BeebeebFileProvider.entitlements"
APPEX="$APP/Contents/PlugIns/BeebeebFileProvider.appex"
HELPER="$APP/Contents/MacOS/BeebeebFileProviderCtl"
[[ -d "$APPEX" ]] || { printf 'extension missing: %s\n' "$APPEX" >&2; exit 1; }
OFFICE="$APP/Contents/MacOS/beebeeb-office-render"
OFFICE_ENT="packaging/macos/office-render.entitlements"
[[ -f "$HELPER" ]] || { printf 'helper missing: %s\n' "$HELPER" >&2; exit 1; }
[[ -f "$OFFICE" ]] || { printf 'office helper missing: %s\n' "$OFFICE" >&2; exit 1; }

if [[ "$IDENTITY" != "-" && "$IDENTITY" =~ ^[0-9A-Fa-f]{40}$ ]]; then
  security find-identity -v -p codesigning | grep -qi "$IDENTITY" || { printf 'identity %s is not in the keychain\n' "$IDENTITY" >&2; exit 1; }
fi

if [[ -n "${BEEBEEB_MACOS_HARDENED_RUNTIME:-}" && "$IDENTITY" != "-" ]]; then
  OPTS=(--options runtime --timestamp)
elif [[ "$IDENTITY" == "-" ]]; then
  OPTS=()
else
  OPTS=(--timestamp=none)
fi

codesign --force --sign "$IDENTITY" ${OPTS[@]+"${OPTS[@]}"} --entitlements "$EXT_ENT" "$APPEX"
codesign --force --sign "$IDENTITY" ${OPTS[@]+"${OPTS[@]}"} "$HELPER"
codesign --force --sign "$IDENTITY" ${OPTS[@]+"${OPTS[@]}"} --entitlements "$OFFICE_ENT" "$OFFICE"
codesign --force --sign "$IDENTITY" ${OPTS[@]+"${OPTS[@]}"} --entitlements "$APP_ENT" "$APP"

codesign --verify --strict --verbose=2 "$APPEX"
codesign --verify --strict --verbose=2 "$HELPER"
codesign --verify --strict --verbose=2 "$OFFICE"
codesign --verify --deep --strict --verbose=2 "$APP"
printf 'Signed %s\n' "$APP"
