#!/usr/bin/env bash
# Local preflight for macOS release packaging. This does not notarize; it
# validates repo config and, when an artifact path is provided, prints and runs
# the local trust checks that do not require Apple credentials.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

fail() {
  printf 'preflight failed: %s\n' "$*" >&2
  exit 1
}

note() {
  printf '==> %s\n' "$*"
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

verify_provision_profile() {
  local bundle_path="$1"
  local bundle_id="$2"
  local app_group="$3"
  local profile="$bundle_path/Contents/embedded.provisionprofile"

  [[ -f "$profile" ]] || fail "missing provisioning profile: $profile"
  security cms -D -i "$profile" >/tmp/beebeeb-profile.plist 2>/dev/null ||
    fail "could not decode provisioning profile: $profile"
  python3 - "$profile" "$bundle_id" "$app_group" <<'PY'
import plistlib
import sys
from pathlib import Path

profile_path, bundle_id, app_group = sys.argv[1:]
profile = plistlib.loads(Path("/tmp/beebeeb-profile.plist").read_bytes())
entitlements = profile.get("Entitlements", {})
application_id = (
    entitlements.get("application-identifier")
    or entitlements.get("com.apple.application-identifier")
    or ""
)
groups = entitlements.get("com.apple.security.application-groups") or []
platforms = profile.get("Platform") or []

if not application_id.endswith("." + bundle_id):
    raise SystemExit(
        f"{profile_path}: application-identifier {application_id!r} does not match {bundle_id!r}"
    )
if app_group not in groups:
    raise SystemExit(f"{profile_path}: missing app group {app_group!r}")
if not any(platform in ("OSX", "macOS") for platform in platforms):
    raise SystemExit(f"{profile_path}: profile is not a macOS profile: {platforms!r}")

print(f"{profile_path} ok")
PY
  rm -f /tmp/beebeeb-profile.plist
}

note "validating macOS plist files"
require_cmd plutil
plutil -lint src-tauri/entitlements.plist
plutil -lint BeebeebFileProvider/Info.plist

note "validating Tauri JSON config"
python3 - <<'PY'
import json
from pathlib import Path

config = json.loads(Path("src-tauri/tauri.conf.json").read_text())
assert config["identifier"] == "io.beebeeb.app"
assert config["bundle"]["macOS"]["entitlements"] == "entitlements.plist"
assert config["bundle"]["createUpdaterArtifacts"] is True
assert config["plugins"]["updater"]["pubkey"].strip()
print("tauri.conf.json ok")
PY

note "validating GitHub workflow YAML syntax"
ruby -e 'require "yaml"; YAML.load_file(".github/workflows/release.yml"); puts "release.yml ok"'

note "checking expected updater signing env names in workflow"
grep -q 'TAURI_SIGNING_PRIVATE_KEY' .github/workflows/release.yml
grep -q 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD' .github/workflows/release.yml

note "checking local Developer ID identity availability"
if security find-identity -v -p codesigning | grep -q "Developer ID Application"; then
  security find-identity -v -p codesigning | grep "Developer ID Application"
else
  printf 'No Developer ID Application identity found locally. This is expected on non-release machines.\n'
fi

if [[ "${1:-}" != "" ]]; then
  artifact="$1"
  [[ -e "$artifact" ]] || fail "artifact not found: $artifact"

  note "verifying macOS artifact: $artifact"
  case "$artifact" in
    *.dmg)
      hdiutil verify "$artifact"
      spctl --assess --type install -vv "$artifact"
      ;;
    *.app)
      appex="$artifact/Contents/PlugIns/BeebeebFileProvider.appex"
      helper="$artifact/Contents/MacOS/BeebeebFileProviderCtl"
      [[ -d "$appex" ]] || fail "File Provider extension missing from app bundle: $appex"
      [[ -x "$helper" ]] || fail "File Provider helper missing from app bundle: $helper"
      verify_provision_profile "$artifact" "io.beebeeb.app" "R8352WDJJR.io.beebeeb.app.fileprovider"
      verify_provision_profile "$appex" "io.beebeeb.app.FileProvider" "R8352WDJJR.io.beebeeb.app.fileprovider"
      codesign --verify --strict --verbose=2 "$appex"
      codesign --verify --strict --verbose=2 "$helper"
      codesign -dvvv --entitlements :- "$appex"
      codesign --verify --deep --strict --verbose=2 "$artifact"
      codesign -dvvv --entitlements :- "$artifact"
      codesign_details="$(codesign -dvvv "$artifact" 2>&1 || true)"
      if grep -q 'TeamIdentifier=not set' <<<"$codesign_details"; then
        printf 'Ad-hoc signed app detected; skipped spctl execute assessment. Use Developer ID signing for release Gatekeeper verification.\n'
      elif ! grep -q 'Authority=Developer ID Application:' <<<"$codesign_details"; then
        printf 'Non-Developer ID app signature detected; skipped spctl execute assessment. Use Developer ID signing for release Gatekeeper verification.\n'
      else
        spctl --assess --type execute -vv "$artifact"
      fi
      ;;
    *)
      fail "unsupported artifact type; pass a .dmg or .app"
      ;;
  esac
else
  note "no artifact argument supplied; skipped hdiutil/codesign/spctl artifact checks"
fi

note "running staged secret scan"
./check-secrets.sh

note "macOS release preflight complete"
