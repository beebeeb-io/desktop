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
team_id = application_id.split(".", 1)[0] if "." in application_id else ""

if not application_id.endswith("." + bundle_id):
    raise SystemExit(
        f"{profile_path}: application-identifier {application_id!r} does not match {bundle_id!r}"
    )
if app_group not in groups and not app_group.startswith(team_id + "."):
    raise SystemExit(f"{profile_path}: missing app group {app_group!r}")
if not any(platform in ("OSX", "macOS") for platform in platforms):
    raise SystemExit(f"{profile_path}: profile is not a macOS profile: {platforms!r}")

print(f"{profile_path} ok")
PY
  rm -f /tmp/beebeeb-profile.plist
}

# Task 1524 issue 5 (P0 crash, 2026-09-28): an app extension binary must have
# NO main() of its own — the Mach-O entry point has to be Foundation's own
# exported `_NSExtensionMain` (set via the linker's `-e` flag, exactly as
# Xcode does for every extension target; Apple's own App Store validator
# enforces this — ITMS-90898: "Please make sure the build system passes
# '-e _NSExtensionMain' to the linker for the ... extension bundle, or the
# extension will not function"). A hand-written main.swift that calls
# NSExtensionMain() as an ordinary function (the pre-fix shape here) recurses
# forever on macOS 26: NSExtensionMain now delegates into ExtensionFoundation,
# which re-invokes the process's real entry point as part of its own
# bootstrap, landing back in our main.swift, which calls NSExtensionMain
# again. Guard both ends: no source file may reintroduce a custom entry, and
# (when a built .appex is available) the linked binary's actual entry point
# must resolve to the imported `_NSExtensionMain` stub, not a locally defined
# `_main`.
verify_fileprovider_no_custom_main_source() {
  if [[ -f "BeebeebFileProvider/main.swift" ]]; then
    fail "BeebeebFileProvider/main.swift must not exist — an app extension's entry point is set via the linker (-e _NSExtensionMain in scripts/build-fileprovider-extension.sh), never a hand-written main.swift (task 1524 issue 5)"
  fi
  if grep -rn "NSExtensionMain" BeebeebFileProvider/*.swift 2>/dev/null; then
    fail "a BeebeebFileProvider/*.swift file references NSExtensionMain directly — the extension entry point must be set purely via the -e _NSExtensionMain linker flag in scripts/build-fileprovider-extension.sh, never called/declared from Swift source (task 1524 issue 5)"
  fi
  printf 'ok: no custom NSExtensionMain wrapper in BeebeebFileProvider/*.swift\n'
}

verify_fileprovider_entry_point_binary() {
  local appex_bin="$1"
  [[ -x "$appex_bin" ]] || fail "File Provider extension binary not found or not executable: $appex_bin"

  if nm -m "$appex_bin" 2>/dev/null | grep -Eq '\bexternal _main$'; then
    fail "$appex_bin defines its own _main symbol — an app extension must have no main() of its own (task 1524 issue 5); found:
$(nm -m "$appex_bin" | grep -E '\bexternal _main$')"
  fi

  if ! nm -m "$appex_bin" 2>/dev/null | grep -q '(undefined) external _NSExtensionMain'; then
    fail "$appex_bin does not import _NSExtensionMain from Foundation — expected an undefined external symbol (task 1524 issue 5)"
  fi

  local entryoff
  entryoff="$(otool -l "$appex_bin" | awk '/cmd LC_MAIN/{f=1} f && /entryoff/{print $2; exit}')"
  [[ -n "$entryoff" ]] || fail "$appex_bin has no LC_MAIN load command — cannot verify entry point"

  local entry_hex
  entry_hex="$(printf '0x%x\n' "$entryoff")"
  # The indirect symbol table maps each imported-symbol stub's address to its
  # name. LC_MAIN's entryoff (relative to the Mach-O image base, conventionally
  # 0x100000000 for a non-PIE-disabled arm64/x86_64 executable slice) must land
  # exactly on the _NSExtensionMain stub — i.e. entryoff's low bits must match
  # that stub's address low bits (both offsets are within the same image).
  local stub_addr
  stub_addr="$(otool -Iv "$appex_bin" 2>/dev/null | awk '/_NSExtensionMain$/{print $1; exit}')"
  [[ -n "$stub_addr" ]] || fail "$appex_bin: could not find an _NSExtensionMain stub in the indirect symbol table"
  local stub_low="0x${stub_addr: -8}"
  if [[ "$(printf '%d' "$stub_low")" -ne "$entryoff" ]]; then
    fail "$appex_bin: LC_MAIN entryoff ($entry_hex) does not point at the _NSExtensionMain stub ($stub_addr) — entry point is not NSExtensionMain (task 1524 issue 5)"
  fi
  printf 'ok: %s entry point (entryoff %s) is the imported _NSExtensionMain stub (%s), no local _main\n' "$appex_bin" "$entry_hex" "$stub_addr"
}

note "validating macOS plist files"
require_cmd plutil
plutil -lint src-tauri/entitlements.plist
plutil -lint BeebeebFileProvider/Info.plist

note "checking File Provider extension has no custom entry point (task 1524 issue 5)"
verify_fileprovider_no_custom_main_source

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
      verify_fileprovider_entry_point_binary "$appex/Contents/MacOS/BeebeebFileProvider"
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
        # Task 1608: notarization needs hardened runtime on EVERY executable in
        # the bundle, not just the containing app — Apple's notary service
        # rejects a submission where any nested binary (the appex, its CTL
        # helper) lacks the runtime flag, even if the app itself has it. Check
        # this BEFORE the spctl execute assessment below: spctl legitimately
        # rejects an as-yet-unnotarized build (`set -euo pipefail` would exit
        # the script right there), so a check placed after it would never run
        # on the normal pre-notarization preflight pass.
        for bin in "$artifact" "$appex" "$helper"; do
          flags="$(codesign -dvvv "$bin" 2>&1 | grep -m1 '^CodeDirectory ' || true)"
          if [[ "$flags" != *"flags=0x10000(runtime)"* ]]; then
            fail "$bin is Developer ID signed but missing the hardened runtime flag (need 'flags=0x10000(runtime)', got: $flags) — notarization will reject this bundle"
          fi
        done
        printf 'ok: hardened runtime present on app, appex and CTL helper\n'
        # scripts/release-macos-local.sh sets this ONLY for the pre-notarization pass
        # (Gatekeeper cannot accept an app Apple has not notarized yet); every later
        # pass, on the stapled app, runs the assessment.
        if [[ -n "${BB_PREFLIGHT_SKIP_SPCTL_EXECUTE:-}" ]]; then
          printf 'skipped spctl execute assessment (BB_PREFLIGHT_SKIP_SPCTL_EXECUTE set: pre-notarization pass)\n'
        else
          spctl --assess --type execute -vv "$artifact"
        fi
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
