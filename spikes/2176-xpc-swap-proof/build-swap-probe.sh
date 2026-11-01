#!/usr/bin/env bash
# spike(2176): build BeebeebSwapProbe.xpc, the non-sandboxed XPC swap service, as a bundle Tauri
# embeds at Beebeeb.app/Contents/XPCServices/ (tauri.spike-2176.conf.json). THROWAWAY, never merged.
#
#   build-swap-probe.sh --version <V> --out <path/BeebeebSwapProbe.xpc> [--identity <keychain identity>]
#
# With --identity (or APPLE_SIGNING_IDENTITY) it signs the bundle the way
# scripts/build-fileprovider-extension.sh signs the appex for a release: hardened runtime, a secure
# timestamp, and NO entitlements (no com.apple.security.app-sandbox: that absence is the point).
# Without an identity it only compiles; the linker's own ad-hoc signature is left as it is.
# The identity is a keychain identity NAME or SHA-1; no key file is read and no secret is passed.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC="$HERE/BeebeebSwapProbe"
VERSION=""
OUT=""
IDENTITY="${APPLE_SIGNING_IDENTITY:-}"
TARGET="arm64-apple-macos14.0"

while (($#)); do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    --identity) IDENTITY="$2"; shift 2 ;;
    --target) TARGET="$2"; shift 2 ;;
    *) printf 'unknown argument: %s\n' "$1" >&2; exit 2 ;;
  esac
done
[[ "$VERSION" =~ ^[0-9]+(\.[0-9]+){0,3}$ ]] || { echo "--version must be numeric, like 0.0.2176 (got '$VERSION')" >&2; exit 2; }
[[ "$OUT" == *.xpc ]] || { echo "--out must end in .xpc (got '$OUT')" >&2; exit 2; }

rm -rf "$OUT"
mkdir -p "$OUT/Contents/MacOS"

# -swift-version 5: spike code, not written for Swift 6 strict concurrency.
xcrun swiftc \
  -target "$TARGET" \
  -swift-version 5 \
  -O \
  -module-name BeebeebSwapProbe \
  -framework Foundation \
  -framework Security \
  -framework AppKit \
  "$SRC/SwapCore.swift" \
  "$SRC/SwapProbeService.swift" \
  "$SRC/main.swift" \
  -o "$OUT/Contents/MacOS/BeebeebSwapProbe"

sed "s/__VERSION__/$VERSION/g" "$SRC/Info.plist" >"$OUT/Contents/Info.plist"
plutil -lint "$OUT/Contents/Info.plist" >/dev/null
[[ "$(plutil -extract CFBundlePackageType raw -o - "$OUT/Contents/Info.plist")" == "XPC!" ]] ||
  { echo "Info.plist CFBundlePackageType is not XPC!" >&2; exit 1; }

if [[ -n "$IDENTITY" ]]; then
  codesign --force --sign "$IDENTITY" --options runtime --timestamp "$OUT"
  codesign --verify --strict --verbose=2 "$OUT"
  # The service must carry no entitlements at all, above all not the App Sandbox.
  ents="$(codesign -d --entitlements - --xml "$OUT" 2>/dev/null || true)"
  if grep -q 'com.apple.security.app-sandbox' <<<"$ents"; then
    echo "BeebeebSwapProbe.xpc carries the app-sandbox entitlement; it must not" >&2
    exit 1
  fi
  printf 'Built and signed %s (version %s)\n' "$OUT" "$VERSION"
else
  printf 'Built %s (version %s), NOT signed (no identity given)\n' "$OUT" "$VERSION"
fi
