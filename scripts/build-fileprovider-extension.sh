#!/usr/bin/env bash
# Build the Beebeeb macOS File Provider extension bundle that Tauri embeds at
# Beebeeb.app/Contents/PlugIns/BeebeebFileProvider.appex.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

OUT_DIR="src-tauri/target/fileprovider/BeebeebFileProvider.appex"
HELPER_OUT="src-tauri/target/fileprovider/BeebeebFileProviderCtl"
TARGET_TRIPLE="${BEEBEEB_FILE_PROVIDER_TARGET:-}"
SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:-}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --out)
      OUT_DIR="$2"
      shift 2
      ;;
    --target)
      TARGET_TRIPLE="$2"
      shift 2
      ;;
    --helper-out)
      HELPER_OUT="$2"
      shift 2
      ;;
    --signing-identity)
      SIGNING_IDENTITY="$2"
      shift 2
      ;;
    *)
      printf 'unknown argument: %s\n' "$1" >&2
      exit 2
      ;;
  esac
done

if [[ -z "$TARGET_TRIPLE" ]]; then
  arch="${TAURI_ENV_ARCH:-$(uname -m)}"
  case "$arch" in
    arm64|aarch64) TARGET_TRIPLE="aarch64-apple-macos14.0" ;;
    x86_64) TARGET_TRIPLE="x86_64-apple-macos14.0" ;;
    *)
      printf 'unsupported macOS File Provider arch: %s\n' "$arch" >&2
      exit 1
      ;;
  esac
fi

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    printf 'missing required command: %s\n' "$1" >&2
    exit 1
  }
}

require_cmd xcrun
require_cmd python3
require_cmd plutil
require_cmd codesign

VERSION="$(python3 - <<'PY'
import json
from pathlib import Path
print(json.loads(Path("package.json").read_text())["version"])
PY
)"

APP_EXE="BeebeebFileProvider"
MODULE_NAME="BeebeebFileProvider"
BUNDLE_ID="io.beebeeb.desktop.FileProvider"
CONTENTS_DIR="$OUT_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
INFO_PLIST="$CONTENTS_DIR/Info.plist"
ENTITLEMENTS="BeebeebFileProvider/BeebeebFileProvider.entitlements"

rm -rf "$OUT_DIR"
mkdir -p "$MACOS_DIR"

xcrun swiftc \
  -target "$TARGET_TRIPLE" \
  -module-name "$MODULE_NAME" \
  -framework FileProvider \
  -framework Foundation \
  -framework UniformTypeIdentifiers \
  BeebeebFileProvider/*.swift \
  -o "$MACOS_DIR/$APP_EXE"

python3 - <<PY
from pathlib import Path

template = Path("BeebeebFileProvider/Info.plist").read_text()
replacements = {
    "\$(DEVELOPMENT_LANGUAGE)": "en",
    "\$(EXECUTABLE_NAME)": "$APP_EXE",
    "\$(PRODUCT_BUNDLE_IDENTIFIER)": "$BUNDLE_ID",
    "\$(PRODUCT_BUNDLE_PACKAGE_TYPE)": "XPC!",
    "\$(PRODUCT_NAME)": "$MODULE_NAME",
    "\$(MARKETING_VERSION)": "$VERSION",
    "\$(CURRENT_PROJECT_VERSION)": "$VERSION",
    "\$(PRODUCT_MODULE_NAME)": "$MODULE_NAME",
}
for old, new in replacements.items():
    template = template.replace(old, new)
Path("$INFO_PLIST").write_text(template)
PY

plutil -lint "$INFO_PLIST" "$ENTITLEMENTS"

if [[ -n "$SIGNING_IDENTITY" ]]; then
  codesign --force --sign "$SIGNING_IDENTITY" --timestamp=none --entitlements "$ENTITLEMENTS" "$OUT_DIR"
else
  codesign --force --sign - --entitlements "$ENTITLEMENTS" "$OUT_DIR"
fi

codesign --verify --strict --verbose=2 "$OUT_DIR"

xcrun swiftc \
  -target "$TARGET_TRIPLE" \
  -parse-as-library \
  -framework FileProvider \
  -framework Foundation \
  BeebeebFileProviderTools/DomainControlTool.swift \
  -o "$HELPER_OUT"

if [[ -n "$SIGNING_IDENTITY" ]]; then
  codesign --force --sign "$SIGNING_IDENTITY" --timestamp=none "$HELPER_OUT"
else
  codesign --force --sign - "$HELPER_OUT"
fi

codesign --verify --strict --verbose=2 "$HELPER_OUT"
printf 'Built File Provider extension: %s\n' "$OUT_DIR"
printf 'Built File Provider helper: %s\n' "$HELPER_OUT"
