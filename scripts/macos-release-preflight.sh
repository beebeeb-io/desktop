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

note "validating macOS plist files"
require_cmd plutil
plutil -lint src-tauri/entitlements.plist
plutil -lint BeebeebFileProvider/Info.plist

note "validating Tauri JSON config"
python3 - <<'PY'
import json
from pathlib import Path

config = json.loads(Path("src-tauri/tauri.conf.json").read_text())
assert config["identifier"] == "io.beebeeb.desktop"
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
      codesign --verify --deep --strict --verbose=2 "$artifact"
      codesign -dvvv --entitlements :- "$artifact"
      spctl --assess --type execute -vv "$artifact"
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
