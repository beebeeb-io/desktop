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
EXTENSION_PROVISION_PROFILE="${MACOS_FILE_PROVIDER_PROVISION_PROFILE:-src-tauri/target/profiles/BeebeebFileProvider.provisionprofile}"
# Task 1608: a notarized Developer ID release needs every nested executable —
# not just the containing app (which Tauri's own bundler already handles) —
# signed with hardened runtime AND a secure (network) timestamp; Apple's
# notarization service rejects a submission over "The signature does not
# include a secure timestamp" or a missing hardened-runtime flag on ANY
# binary inside the bundle, including this appex and its CTL helper. Gate
# this behind an explicit env var rather than turning it on whenever
# SIGNING_IDENTITY is set: the existing local dev-signing recipe (task 1521,
# 1524) already passes a real (non-ad-hoc) Apple Development identity here,
# and that flow doesn't notarize, doesn't need hardened runtime, and a
# network timestamp would slow it down / fail offline for no benefit.
MACOS_HARDENED_RUNTIME="${BEEBEEB_MACOS_HARDENED_RUNTIME:-}"

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
BUNDLE_ID="io.beebeeb.app.FileProvider"
CONTENTS_DIR="$OUT_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
INFO_PLIST="$CONTENTS_DIR/Info.plist"
ENTITLEMENTS="BeebeebFileProvider/BeebeebFileProvider.entitlements"

rm -rf "$OUT_DIR"
mkdir -p "$MACOS_DIR"

# Task 1524 issue 5 (P0 crash, 2026-09-28): an app extension has NO main() of
# its own — Xcode never emits one for an extension target, and Apple's own App
# Store validator says so explicitly (ITMS-90898: "Please make sure the build
# system passes '-e _NSExtensionMain' to the linker for the ... extension
# bundle, or the extension will not function"). This repo used to compile a
# hand-written BeebeebFileProvider/main.swift that called the real
# NSExtensionMain via @_silgen_name and then exit()'d. On macOS 26 that
# indirection recurses forever: NSExtensionMain now delegates into
# ExtensionFoundation's EXExtensionMain, which re-invokes the process's real
# Mach-O entry point as part of its own bootstrap — and since our entry point
# was our own `_main` (not Foundation's `_NSExtensionMain`), that re-invocation
# landed back in our main.swift, which called NSExtensionMain again, forever
# (confirmed via crash report + `nm -m`/`otool -l` inspection: entryoff
# pointed exactly at the locally-defined `_main` symbol, not the imported
# `_NSExtensionMain` one — verification-evidence/1524/issue5-*).
#
# `-parse-as-library`: none of BeebeebFileProvider/*.swift is a `main.swift`
# or `@main` type anymore, so nothing here should synthesize a `_main` at all.
# `-Xlinker -e -Xlinker _NSExtensionMain`: sets the Mach-O entry point
# directly to Foundation's exported `_NSExtensionMain`, exactly as Xcode does
# for every extension target and as the ITMS-90898 message above documents.
xcrun swiftc \
  -target "$TARGET_TRIPLE" \
  -module-name "$MODULE_NAME" \
  -parse-as-library \
  -framework FileProvider \
  -framework Foundation \
  -framework UniformTypeIdentifiers \
  BeebeebFileProvider/*.swift \
  -Xlinker -e -Xlinker _NSExtensionMain \
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

# Badge icons for NSFileProviderDecorations (task 1699). Generated by
# scripts/gen-badge-pngs.py and committed under BeebeebFileProvider/Resources;
# they must ship in Contents/Resources so the UTImportedTypeDeclarations'
# UTTypeIconFile entries (badge-*) resolve at render time.
RESOURCES_SRC_DIR="BeebeebFileProvider/Resources"
if [[ -d "$RESOURCES_SRC_DIR" ]]; then
  RESOURCES_DIR="$CONTENTS_DIR/Resources"
  mkdir -p "$RESOURCES_DIR"
  cp "$RESOURCES_SRC_DIR"/badge-*.png "$RESOURCES_DIR/"
fi

if [[ -f "$EXTENSION_PROVISION_PROFILE" ]]; then
  cp "$EXTENSION_PROVISION_PROFILE" "$CONTENTS_DIR/embedded.provisionprofile"
fi

if [[ -n "$SIGNING_IDENTITY" ]]; then
  if [[ -n "$MACOS_HARDENED_RUNTIME" ]]; then
    codesign --force --sign "$SIGNING_IDENTITY" --options runtime --timestamp --entitlements "$ENTITLEMENTS" "$OUT_DIR"
  else
    codesign --force --sign "$SIGNING_IDENTITY" --timestamp=none --entitlements "$ENTITLEMENTS" "$OUT_DIR"
  fi
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
  if [[ -n "$MACOS_HARDENED_RUNTIME" ]]; then
    codesign --force --sign "$SIGNING_IDENTITY" --options runtime --timestamp "$HELPER_OUT"
  else
    codesign --force --sign "$SIGNING_IDENTITY" --timestamp=none "$HELPER_OUT"
  fi
else
  codesign --force --sign - "$HELPER_OUT"
fi

codesign --verify --strict --verbose=2 "$HELPER_OUT"
printf 'Built File Provider extension: %s\n' "$OUT_DIR"
printf 'Built File Provider helper: %s\n' "$HELPER_OUT"
