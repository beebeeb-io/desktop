#!/usr/bin/env bash
# Prepare macOS provisioning profiles before Tauri assembles/signs the bundle.
# Real release builds should set:
#   MACOS_APP_PROVISION_PROFILE or MACOS_APP_PROVISION_PROFILE_BASE64
#   MACOS_FILE_PROVIDER_PROVISION_PROFILE or MACOS_FILE_PROVIDER_PROVISION_PROFILE_BASE64
#
# When absent, we create empty placeholders so local UI bundles can still be
# assembled, but macos-release-preflight.sh will reject them.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PROFILE_DIR="src-tauri/target/profiles"
APP_PROFILE_OUT="$PROFILE_DIR/BeebeebApp.provisionprofile"
EXTENSION_PROFILE_OUT="$PROFILE_DIR/BeebeebFileProvider.provisionprofile"

mkdir -p "$PROFILE_DIR"

write_profile() {
  local src_path="$1"
  local src_base64="$2"
  local dest="$3"
  local label="$4"

  if [[ "$src_path" != "" ]]; then
    [[ -f "$src_path" ]] || {
      printf '%s provisioning profile not found: %s\n' "$label" "$src_path" >&2
      exit 1
    }
    cp "$src_path" "$dest"
    return
  fi

  if [[ "$src_base64" != "" ]]; then
    printf '%s' "$src_base64" | base64 --decode >"$dest"
    return
  fi

  : >"$dest"
  printf 'warning: no %s provisioning profile supplied; release preflight will fail\n' "$label" >&2
}

write_profile \
  "${MACOS_APP_PROVISION_PROFILE:-}" \
  "${MACOS_APP_PROVISION_PROFILE_BASE64:-}" \
  "$APP_PROFILE_OUT" \
  "app"

write_profile \
  "${MACOS_FILE_PROVIDER_PROVISION_PROFILE:-}" \
  "${MACOS_FILE_PROVIDER_PROVISION_PROFILE_BASE64:-}" \
  "$EXTENSION_PROFILE_OUT" \
  "File Provider"

printf 'Prepared macOS provisioning profile inputs in %s\n' "$PROFILE_DIR"
