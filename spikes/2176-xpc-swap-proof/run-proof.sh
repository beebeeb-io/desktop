#!/usr/bin/env bash
# spike(2176): the XPC swap device proof, P1-P5 and the negative cases, run by hand on a Mac.
# THROWAWAY: branch spike/2176-xpc-swap-proof, never merged. README.md next to this file has the
# exact order, the pass line of each step and what to send back.
#
#   run-proof.sh <step>            run one step
#   DRY_RUN=1 run-proof.sh <step>  print every command of that step, run none
#   DRY_RUN=1 run-proof.sh all     print every command of every step, in order
#
# Steps: preflight build p1 p2-admin p2-share p2-standard p3-running p3-quit-first
#        p4-start p4-swap p4-check p5 negative restore status
#
# Environment (no secret is read or passed; signing uses the login keychain):
#   APPLE_SIGNING_IDENTITY   Developer ID Application identity of team R8352WDJJR (name or SHA-1)
#   MACOS_APP_PROVISION_PROFILE, MACOS_FILE_PROVIDER_PROVISION_PROFILE
#                            [~/.private_keys/macos-developer-id-profiles/{BeebeebApp,BeebeebFileProvider}.provisionprofile]
#   BB_SPIKE_N, BB_SPIKE_N1  the two versions [0.0.2176, 0.0.2177]
#   BB_WITH_LOCK             optional: the workspace's scripts/coord/with-lock.sh; the build then
#                            runs under its cargo-build lock
#   BB_SPIKE_P4_MB           size of the P4 upload [1024]
#   BB_SPIKE_FP_ROOT         the Beebeeb folder in Finder, if the script cannot find it
#
# Nothing here notarizes, uploads or talks to Apple: notarization is not needed for a local proof
# (the builds are never quarantined, so Gatekeeper does not assess them), and every APPLE_*/notary
# variable is removed from the build's environment.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." 2>/dev/null && pwd)"
DRY="${DRY_RUN:-0}"
[[ "$DRY" == "1" ]] || DRY=0

N="${BB_SPIKE_N:-0.0.2176}"
N1="${BB_SPIKE_N1:-0.0.2177}"
TEAM="R8352WDJJR"
IDENTITY="${APPLE_SIGNING_IDENTITY:-}"
PROFILE_DIR="$HOME/.private_keys/macos-developer-id-profiles"
APP_PROFILE="${MACOS_APP_PROVISION_PROFILE:-$PROFILE_DIR/BeebeebApp.provisionprofile}"
EXT_PROFILE="${MACOS_FILE_PROVIDER_PROVISION_PROFILE:-$PROFILE_DIR/BeebeebFileProvider.provisionprofile}"
P4_MB="${BB_SPIKE_P4_MB:-1024}"

APP="/Applications/Beebeeb.app"
PROC="beebeeb-desktop"
WORK="$HOME/beebeeb-spike-2176"             # builds, backup, logs; also the service's probe dir
SHARED="/Users/Shared/beebeeb-spike-2176"   # what the standard user of P2 reads
CONTAINER="$HOME/Library/Containers/io.beebeeb.app/Data"
SPIKE_C="$CONTAINER/Library/Application Support/BeebeebSpike2176"
STAGE="$SPIKE_C/stage"
RESULTS="$SPIKE_C/results"
TS="$(date +%Y%m%d-%H%M%S)"
LOGS="$WORK/logs"

# ---------------------------------------------------------------- plumbing
say() { printf '    %s\n' "$*"; }
banner() { printf '\n==> %s\n' "$*"; }
die() {
  printf '\nSTOP: %s\n' "$*" >&2
  exit 1
}
is_dry() { ((DRY)); }

quote() {
  case "$1" in
    '' | *[!A-Za-z0-9_@%+=:,./-]*) printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")" ;;
    *) printf '%s' "$1" ;;
  esac
}
show() {
  local out="" a
  for a in "$@"; do out="$out $(quote "$a")"; done
  printf '    +%s\n' "$out"
}
# run <cmd...>: print it; run it unless DRY_RUN.
run() {
  show "$@"
  is_dry && return 0
  "$@"
}

STEP=""
CHECKS=0
PASSED=0
step_begin() {
  STEP="$1"
  CHECKS=0
  PASSED=0
  banner "$1: $2"
  is_dry || mkdir -p "$LOGS"
}
# check <label> <cmd...>: a pass/fail line; the command decides.
check() {
  local label="$1"
  shift
  CHECKS=$((CHECKS + 1))
  if is_dry; then
    say "would check: $label"
    return 0
  fi
  if "$@"; then
    PASSED=$((PASSED + 1))
    printf 'PASS %s: %s\n' "$STEP" "$label"
  else
    printf 'FAIL %s: %s\n' "$STEP" "$label"
  fi
}
step_end() {
  local line
  if is_dry; then
    say "(dry run: $CHECKS checks would run)"
    return 0
  fi
  if ((PASSED == CHECKS && CHECKS > 0)); then
    line="RESULT $STEP: PASS ($PASSED of $CHECKS checks) $TS"
  else
    line="RESULT $STEP: FAIL ($PASSED of $CHECKS checks) $TS"
  fi
  printf '\n%s\n' "$line"
  printf '%s\n' "$line" >>"$LOGS/summary.txt"
  say "send back: $LOGS/summary.txt and $LOGS/$STEP-$TS/"
}
step_log_dir() {
  printf '%s' "$LOGS/$STEP-$TS"
}
save() { # <file> : copy into this step's log dir
  is_dry && return 0
  mkdir -p "$(step_log_dir)"
  cp "$1" "$(step_log_dir)/" 2>/dev/null || true
}
pause() { # <question>
  if is_dry; then
    say "would ask: $1 [Enter]"
    return 0
  fi
  printf '\n>>> %s\n    Press Enter to continue. ' "$1"
  read -r _
}
ask_yes() { # <question> -> 0 for y
  if is_dry; then
    say "would ask: $1 [y/n]"
    return 0
  fi
  local answer=""
  printf '\n>>> %s [y/n] ' "$1"
  read -r answer
  printf '%s %s: %s\n' "$STEP" "$1" "$answer" >>"$LOGS/answers.txt"
  [[ "$answer" == y* || "$answer" == Y* ]]
}

need_cmd() {
  is_dry && return 0
  command -v "$1" >/dev/null 2>&1 || die "missing command: $1"
}
need_file() {
  if is_dry; then
    say "would require $2: $1"
    return 0
  fi
  [[ -e "$1" ]] || die "$2 not found: $1"
}

# ---------------------------------------------------------------- facts about the Mac
running_pids() { pgrep -x "$PROC" 2>/dev/null || true; }
version_of() { plutil -extract CFBundleShortVersionString raw -o - "$1/Contents/Info.plist" 2>/dev/null || printf 'none'; }
installed_version() { version_of "$APP"; }
# CDHash of a bundle's main executable, or of a running process given its pid.
cdhash_of() { codesign -dv --verbose=4 "$1" 2>&1 | awk -F= '/^CDHash=/{print $2; exit}'; }
sibling_count() { find /Applications -maxdepth 1 -name '.beebeeb-update-*' 2>/dev/null | wc -l | tr -d ' '; }
fingerprint() { printf 'version=%s cdhash=%s siblings=%s' "$(installed_version)" "$(cdhash_of "$APP")" "$(sibling_count)"; }
is_spike_version() { [[ "$1" == "$N" || "$1" == "$N1" ]]; }
build_app() { printf '%s/builds/%s/Beebeeb.app' "$WORK" "$1"; }

require_not_running() {
  if is_dry; then
    say "would require that Beebeeb is not running (pgrep -x $PROC)"
    return 0
  fi
  local p
  p="$(running_pids | tr '\n' ' ')"
  [[ -z "$p" ]] || die "Beebeeb is running (pid $p). Quit it first: osascript -e 'tell application \"Beebeeb\" to quit'"
}

wait_exit() { # <seconds>
  if is_dry; then
    say "would wait up to $1 s until 'pgrep -x $PROC' finds nothing"
    return 0
  fi
  local i
  for ((i = 0; i < $1; i++)); do
    [[ -z "$(running_pids)" ]] && return 0
    sleep 1
  done
  return 1
}

quit_app() {
  if ! is_dry && [[ -z "$(running_pids)" ]]; then return 0; fi
  run osascript -e 'tell application "Beebeeb" to quit'
  wait_exit 30 || die "Beebeeb did not quit within 30 s; quit it from the menu bar icon, then re-run this step"
}

print_restore() {
  if is_dry || [[ -e "$WORK/backup/Beebeeb.app" ]]; then
    say "RESTORE (one line): mkdir -p \"$WORK/discard\" && mv /Applications/Beebeeb.app \"$WORK/discard/Beebeeb-\$(date +%s).app\" && mv \"$WORK/backup/Beebeeb.app\" /Applications/Beebeeb.app"
    say "   or: $HERE/run-proof.sh restore"
  else
    say "RESTORE: there was no Beebeeb.app in /Applications before the proof. Remove the spike build with:"
    say "   mkdir -p \"$WORK/discard\" && mv /Applications/Beebeeb.app \"$WORK/discard/Beebeeb-\$(date +%s).app\""
  fi
}

# install <version>: put build <version> at /Applications/Beebeeb.app. The first time a non-spike
# Beebeeb is found there it is MOVED (not copied: signature, ticket and xattrs stay as they are) to
# $WORK/backup, and the restore line is printed BEFORE anything new goes in.
install_build() {
  local v="$1" src cur
  src="$(build_app "$v")"
  require_not_running
  need_file "$src" "build $v (run: run-proof.sh build)"
  run mkdir -p "$WORK/discard" "$WORK/backup"
  if is_dry; then
    say "if $APP is not a spike build and no backup exists: it is moved to $WORK/backup/Beebeeb.app; if it is a spike build: it is moved to $WORK/discard/"
    show mv "$APP" "$WORK/backup/Beebeeb.app"
  elif [[ -e "$APP" ]]; then
    cur="$(installed_version)"
    if is_spike_version "$cur"; then
      run mv "$APP" "$WORK/discard/Beebeeb-$cur-$TS-$RANDOM.app"
    elif [[ -e "$WORK/backup/Beebeeb.app" ]]; then
      die "$APP is version $cur (not a spike build) and $WORK/backup/Beebeeb.app already exists. Decide which one to keep and move the other away by hand; nothing was changed."
    else
      run mv "$APP" "$WORK/backup/Beebeeb.app"
      printf '%s\n' "$cur" >"$WORK/backup/VERSION"
      say "saved the installed Beebeeb $cur to $WORK/backup/Beebeeb.app"
    fi
  fi
  print_restore
  run ditto "$src" "$APP"
  if ! is_dry; then
    [[ "$(installed_version)" == "$v" ]] || die "installed version is $(installed_version), expected $v"
    say "installed spike build $v at $APP"
  fi
}

require_spike_installed() {
  is_dry && return 0
  is_spike_version "$(installed_version)" || die "$APP is not a spike build (version $(installed_version))"
}

new_run() { printf '%s-%s-%s' "$1" "$(date +%H%M%S)" "$RANDOM"; }

# launch <command> <run>: start the installed app with the spike trigger in its environment.
launch() {
  require_not_running
  run open -n --env "BEEBEEB_SPIKE_2176=$1" --env "BEEBEEB_SPIKE_2176_RUN=$2" "$APP"
}

# send <run> <command>: hand a command to an app started with `watch`.
send() {
  show "write '$1 $2' to \"$SPIKE_C/command\" (via command.tmp, then mv)"
  is_dry && return 0
  mkdir -p "$SPIKE_C"
  printf '%s %s\n' "$1" "$2" >"$SPIKE_C/command.tmp"
  mv "$SPIKE_C/command.tmp" "$SPIKE_C/command"
}

RESULT=""
# wait_result <run> <seconds>: wait for the app's JSON result, print it, keep a copy.
wait_result() {
  RESULT="$RESULTS/$1.json"
  if is_dry; then
    say "would wait up to $2 s for \"$RESULT\", print it and copy it to the step's log dir"
    return 0
  fi
  local i
  for ((i = 0; i < $2; i++)); do
    [[ -f "$RESULT" ]] && break
    sleep 1
  done
  if [[ ! -f "$RESULT" ]]; then
    printf 'FAIL %s: no result for run %s within %s s (%s)\n' "$STEP" "$1" "$2" "$RESULT"
    RESULT=""
    return 1
  fi
  sleep 1
  sed 's/^/      /' "$RESULT"
  save "$RESULT"
}
field() { # <name> of the last result
  [[ -n "$RESULT" && -f "$RESULT" ]] || return 0
  plutil -extract "$1" raw -o - "$RESULT" 2>/dev/null || true
}
code_is() { [[ "$(field code)" == "$1" ]]; }

# ensure_container: the sandbox container must exist before the script stages into it, and only
# macOS may create it (a hand-made directory there breaks the app's launch). A first launch does.
ensure_container() {
  if is_dry; then
    say "if \"$CONTAINER\" does not exist yet: launch Beebeeb once with BEEBEEB_SPIKE_2176=ping, wait for its result, quit it"
    return 0
  fi
  [[ -d "$CONTAINER" ]] && return 0
  say "no container yet for this user: one launch creates it"
  local r
  r="$(new_run ping)"
  launch ping "$r"
  wait_result "$r" 90 || true
  quit_app
  [[ -d "$CONTAINER" ]] || die "the container was not created at $CONTAINER"
}

STAGED=""
# stage <source .app> <name>: copy a bundle into the container's staging area.
stage() {
  STAGED="$STAGE/$2/Beebeeb.app"
  run rm -rf "$STAGE/$2"
  run mkdir -p "$STAGE/$2"
  run ditto "$1" "$STAGED"
}

NEWPID=""
# wait_new_process <old pid> <expected cdhash> <seconds>: a Beebeeb process other than <old pid>
# whose running code has the expected CDHash.
wait_new_process() {
  NEWPID=""
  if is_dry; then
    say "would wait up to $3 s for a $PROC process (not pid $1) whose running CDHash is the new build's ($2); codesign -dv --verbose=4 <pid>"
    return 0
  fi
  local i p h
  for ((i = 0; i < $3; i++)); do
    for p in $(running_pids); do
      [[ "$p" == "$1" ]] && continue
      h="$(cdhash_of "$p")"
      if [[ -n "$h" && "$h" == "$2" ]]; then
        NEWPID="$p"
        say "new process $p runs CDHash $h"
        return 0
      fi
    done
    sleep 1
  done
  say "no new process with CDHash $2 within $3 s; running: $(running_pids | tr '\n' ' ')"
  for p in $(running_pids); do say "  pid $p CDHash $(cdhash_of "$p")"; done
  return 1
}

# remove the sibling directories a successful swap leaves, through the service itself.
cleanup_siblings() {
  if ! is_dry && [[ "$(sibling_count)" == "0" ]]; then return 0; fi
  quit_app
  local r
  r="$(new_run cleanup)"
  launch cleanup "$r"
  wait_result "$r" 60 || true
  quit_app
  if ! is_dry && [[ "$(sibling_count)" != "0" ]]; then
    say "the service left $(sibling_count) sibling dir(s); removing them by hand"
    run find /Applications -maxdepth 1 -name '.beebeeb-update-*' -user "$(id -un)" -exec rm -rf {} +
  fi
}

verified() { codesign --verify --deep --strict "$APP" >/dev/null 2>&1; }

# ---------------------------------------------------------------- steps
step_preflight() {
  step_begin preflight "tools, identity, profiles, current state"
  local t
  for t in git bun bunx jq xcrun codesign plutil ditto security pgrep osascript open log pluginkit strings perl; do need_cmd "$t"; done
  if ! is_dry; then
    [[ "$(uname -s) $(uname -m)" == "Darwin arm64" ]] || die "this proof runs on an Apple Silicon Mac"
  fi
  require_identity
  need_file "$APP_PROFILE" "app provisioning profile"
  need_file "$EXT_PROFILE" "File Provider provisioning profile"
  if ! is_dry; then
    say "installed: $APP version $(installed_version)"
    say "running: $(running_pids | tr '\n' ' ')"
    say "container: $([[ -d "$CONTAINER" ]] && echo exists || echo 'not yet created')"
    say "backup: $([[ -e "$WORK/backup/Beebeeb.app" ]] && echo "$WORK/backup/Beebeeb.app ($(cat "$WORK/backup/VERSION" 2>/dev/null))" || echo none)"
    say "siblings in /Applications: $(sibling_count)"
  fi
  say "Isolation: this proof runs unreleased code against THIS user's Beebeeb container, session and Finder location."
  say "Recommended: a separate admin macOS user for every step (README, 'Isolation')."
}

require_identity() {
  if is_dry; then
    say "would require APPLE_SIGNING_IDENTITY (now: '${IDENTITY:-unset}') in 'security find-identity -v -p codesigning', a Developer ID Application identity of team $TEAM"
    return 0
  fi
  [[ -n "$IDENTITY" ]] || die "set APPLE_SIGNING_IDENTITY to your Developer ID Application identity, e.g. export APPLE_SIGNING_IDENTITY='Developer ID Application: Devidee B.V. ($TEAM)'"
  local line
  line="$(security find-identity -v -p codesigning | grep -F -- "$IDENTITY" | head -1 || true)"
  [[ -n "$line" ]] || die "identity not in the keychain: $IDENTITY"
  [[ "$line" == *"Developer ID Application"* && "$line" == *"($TEAM)"* ]] ||
    die "not a Developer ID Application identity of team $TEAM: $line"
}

BUILD_BACKUP=""
restore_tracked() {
  if [[ -n "$BUILD_BACKUP" && -d "$BUILD_BACKUP" ]]; then
    cp "$BUILD_BACKUP/package.json" "$REPO/package.json"
    cp "$BUILD_BACKUP/index.html" "$REPO/dist/index.html"
    BUILD_BACKUP=""
  fi
}

build_one() { # <version>
  local v="$1" out bundle
  out="$(build_app "$v")"
  bundle="$REPO/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Beebeeb.app"
  banner "build $v"
  # The appex takes its version from package.json (scripts/build-fileprovider-extension.sh), so it
  # is patched for the build (and restored) to make N's and N+1's appex differ for P4.
  if ! is_dry; then
    BUILD_BACKUP="$WORK/build-tmp-$v"
    rm -rf "$BUILD_BACKUP"
    mkdir -p "$BUILD_BACKUP"
    cp "$REPO/package.json" "$REPO/dist/index.html" "$BUILD_BACKUP/"
    trap restore_tracked EXIT
  fi
  show jq --arg v "$v" '.version = $v' package.json '>' package.json.tmp '&&' mv package.json.tmp package.json
  if ! is_dry; then
    jq --arg v "$v" '.version = $v' "$REPO/package.json" >"$REPO/package.json.tmp"
    mv "$REPO/package.json.tmp" "$REPO/package.json"
  fi
  run "$HERE/build-swap-probe.sh" --version "$v" --identity "$IDENTITY" --out "$REPO/src-tauri/target/spike-2176/BeebeebSwapProbe.xpc"
  run rm -rf "$REPO/src-tauri/target/aarch64-apple-darwin/release/bundle"
  local lock=()
  [[ -n "${BB_WITH_LOCK:-}" ]] && lock=("$BB_WITH_LOCK" cargo-build --)
  # Same build as scripts/release-macos-local.sh step 2, plus the spike feature and the overlay that
  # embeds the service. Notary and updater-key variables are removed: nothing here talks to Apple.
  run ${lock[@]+"${lock[@]}"} env \
    -u APPLE_ID -u APPLE_PASSWORD -u APPLE_TEAM_ID -u APPLE_API_KEY -u APPLE_API_ISSUER -u APPLE_API_KEY_PATH \
    -u APPLE_CERTIFICATE -u APPLE_CERTIFICATE_PASSWORD \
    -u TAURI_SIGNING_PRIVATE_KEY -u TAURI_SIGNING_PRIVATE_KEY_PASSWORD -u TAURI_SIGNING_PRIVATE_KEY_PATH \
    "APPLE_SIGNING_IDENTITY=$IDENTITY" \
    "MACOS_APP_PROVISION_PROFILE=$APP_PROFILE" \
    "MACOS_FILE_PROVIDER_PROVISION_PROFILE=$EXT_PROFILE" \
    "BEEBEEB_RELEASE_VERSION=$v" \
    "BEEBEEB_MACOS_HARDENED_RUNTIME=1" \
    bunx tauri build --target aarch64-apple-darwin --bundles app --features spike-2176 \
    --config "$HERE/tauri.spike-2176.conf.json" \
    --config "{\"version\":\"$v\",\"bundle\":{\"createUpdaterArtifacts\":false}}" -- --locked
  if ! is_dry; then
    restore_tracked
    trap - EXIT
    (cd "$REPO" && git diff --quiet) || die "the tree is not back to its committed content after the build: $(cd "$REPO" && git status --porcelain | head -5 | tr '\n' ' ')"
  fi
  run rm -rf "$WORK/builds/$v"
  run mkdir -p "$WORK/builds/$v"
  run ditto "$bundle" "$out"
  verify_build "$out" "$v"
}

has_runtime_and_devid() { # <path>
  local d
  d="$(codesign -dvvv "$1" 2>&1)"
  grep -q 'flags=0x10000(runtime)' <<<"$d" && grep -q 'Authority=Developer ID Application:' <<<"$d" &&
    grep -q "TeamIdentifier=$TEAM" <<<"$d"
}
no_sandbox_entitlement() { ! codesign -d --entitlements - --xml "$1" 2>/dev/null | grep -q 'com.apple.security.app-sandbox'; }
has_sandbox_entitlement() { codesign -d --entitlements - --xml "$1" 2>/dev/null | grep -q 'com.apple.security.app-sandbox'; }
plist_is() { [[ "$(plutil -extract "$2" raw -o - "$1/Contents/Info.plist" 2>/dev/null)" == "$3" ]]; }
client_linked() { strings -a "$1/Contents/MacOS/$PROC" | grep -qx 'io.beebeeb.app.SwapProbe'; }

verify_build() { # <app> <version>
  local app="$1" v="$2" x="$1/Contents/XPCServices/BeebeebSwapProbe.xpc" appex="$1/Contents/PlugIns/BeebeebFileProvider.appex"
  check "build $v: app version is $v" plist_is "$app" CFBundleShortVersionString "$v"
  check "build $v: appex version is $v (P4 can tell N from N+1)" plist_is "$appex" CFBundleShortVersionString "$v"
  check "build $v: the service is embedded as an XPC bundle" plist_is "$x" CFBundlePackageType 'XPC!'
  check "build $v: service identifier io.beebeeb.app.SwapProbe" plist_is "$x" CFBundleIdentifier io.beebeeb.app.SwapProbe
  check "build $v: the service has NO app-sandbox entitlement" no_sandbox_entitlement "$x"
  check "build $v: the app IS sandboxed" has_sandbox_entitlement "$app"
  check "build $v: service hardened runtime, Developer ID, team $TEAM" has_runtime_and_devid "$x"
  check "build $v: app hardened runtime, Developer ID, team $TEAM" has_runtime_and_devid "$app"
  check "build $v: codesign --verify --deep --strict" codesign --verify --deep --strict "$app"
  check "build $v: the spike client is linked into the app binary" client_linked "$app"
  check "build $v: release preflight (unnotarized pass)" env BB_PREFLIGHT_SKIP_SPCTL_EXECUTE=1 "$REPO/scripts/macos-release-preflight.sh" "$app"
  if ! is_dry; then
    {
      printf 'app=%s\n' "$(cdhash_of "$app")"
      printf 'appex=%s\n' "$(cdhash_of "$appex")"
      printf 'xpc=%s\n' "$(cdhash_of "$x")"
    } >"$WORK/builds/$v/cdhashes"
    sed 's/^/    cdhash /' "$WORK/builds/$v/cdhashes"
  fi
}

build_cdhash() { # <version> <app|appex|xpc>
  is_dry && { printf '<cdhash %s %s>' "$1" "$2"; return 0; }
  awk -F= -v k="$2" '$1 == k {print $2}' "$WORK/builds/$1/cdhashes"
}

step_build() {
  step_begin build "N=$N and N+1=$N1, signed with the Developer ID identity, not notarized"
  require_identity
  need_file "$APP_PROFILE" "app provisioning profile"
  need_file "$EXT_PROFILE" "File Provider provisioning profile"
  [[ -f "$REPO/src-tauri/tauri.conf.json" ]] || die "run 'build' from the repo checkout (not from $SHARED)"
  if ! is_dry; then
    [[ -z "$(cd "$REPO" && git status --porcelain)" ]] || die "the tree at $REPO is not clean"
  fi
  run mkdir -p "$WORK/builds"
  (cd "$REPO" && run bun install --frozen-lockfile)
  if is_dry; then show cd "$REPO"; fi
  cd "$REPO"
  build_one "$N"
  build_one "$N1"
  step_end
}

step_p1() {
  step_begin p1 "the sandboxed app calls the bundled non-sandboxed service, which writes outside the container"
  install_build "$N"
  ensure_container
  local r start probe="$WORK/p1-probe.txt" log
  run rm -f "$probe"
  start="$(date '+%Y-%m-%d %H:%M:%S')"
  r="$(new_run p1)"
  launch p1 "$r"
  wait_result "$r" 90 || true
  quit_app
  log="$(step_log_dir)/sandbox-denials.log"
  show log show --start "$start" --predicate 'eventMessage CONTAINS "Sandbox"' --style compact '>' "$log"
  if ! is_dry; then
    mkdir -p "$(step_log_dir)"
    log show --start "$start" --predicate 'eventMessage CONTAINS "Sandbox"' --style compact >"$log" 2>&1 || true
    save "$probe"
  fi
  check "the app got 'ok' from writeProbe" code_is ok
  check "the service answered ping" eval '[[ "$(field ping_code)" == ok ]]'
  check "the app is sandboxed (its home is its container)" eval '[[ "$(field client_home_is_container)" == true || "$(field client_home_is_container)" == 1 ]]'
  check "the service is NOT sandboxed (its home is the real home)" eval '[[ "$(field service_info)" == *"home_is_container=0"* ]]'
  check "the probe file exists outside the container and names the service pid" eval 'grep -q "^service_pid=" "$probe"'
  check "no sandbox denial for $PROC" eval '! grep -E "$PROC\([0-9]+\) deny" "$log"'
  check "no sandbox denial for BeebeebSwapProbe" eval '! grep -E "BeebeebSwapProbe\([0-9]+\) deny" "$log"'
  if ! is_dry && grep -qE "($PROC|BeebeebSwapProbe)\([0-9]+\) deny" "$log"; then
    say "denials (all of them are in $log):"
    grep -E "($PROC|BeebeebSwapProbe)\([0-9]+\) deny" "$log" | head -20 | sed 's/^/      /'
  fi
  step_end
}

step_p2_admin() {
  step_begin p2-admin "as an ADMIN user: swap /Applications/Beebeeb.app $N -> $N1 while $N runs"
  install_build "$N"
  ensure_container
  local before r
  is_dry || before="$(fingerprint)"
  is_dry || say "before: $before"
  stage "$(build_app "$N1")" p2
  r="$(new_run p2a)"
  launch "swap:$STAGED" "$r"
  wait_result "$r" 300 || true
  check "the service answered ok" code_is ok
  check "/Applications/Beebeeb.app is now $N1" eval '[[ "$(installed_version)" == "$N1" ]]'
  check "its CDHash is the $N1 build's" eval '[[ "$(cdhash_of "$APP")" == "$(build_cdhash "$N1" app)" ]]'
  check "codesign --verify --deep --strict passes on the swapped bundle" verified
  check "the previous bundle waits in one 0700 sibling directory" eval '[[ "$(sibling_count)" == 1 ]] && [[ "$(stat -f %Lp /Applications/.beebeeb-update-* 2>/dev/null)" == 700 ]]'
  is_dry || say "after: $(fingerprint)"
  quit_app
  cleanup_siblings
  check "the service's cleanup removed the sibling directory" eval '[[ "$(sibling_count)" == 0 ]]'
  step_end
}

step_p2_share() {
  step_begin p2-share "as the ADMIN user: install $N and share the $N1 build and this script with the standard user"
  install_build "$N"
  run mkdir -p "$SHARED/builds/$N1"
  run rm -rf "$SHARED/builds/$N1/Beebeeb.app"
  run ditto "$(build_app "$N1")" "$SHARED/builds/$N1/Beebeeb.app"
  run cp "$HERE/run-proof.sh" "$SHARED/run-proof.sh"
  run chmod -R a+rX "$SHARED"
  say "Now switch to the STANDARD user (fast user switching) and run in Terminal:"
  say "   bash $SHARED/run-proof.sh p2-standard"
  CHECKS=1
  PASSED=1
  step_end
}

step_p2_standard() {
  step_begin p2-standard "as a STANDARD user: the same swap must be refused with not_writable and change nothing"
  if ! is_dry && id -Gn | tr ' ' '\n' | grep -qx admin; then
    die "$(id -un) is an administrator; run this step as a standard user"
  fi
  local src="$SHARED/builds/$N1/Beebeeb.app" before r
  need_file "$src" "the shared $N1 build (run p2-share as the admin user first)"
  if ! is_dry; then
    [[ "$(installed_version)" == "$N" ]] || die "/Applications/Beebeeb.app is $(installed_version); the admin user must run p2-share first"
    before="$(fingerprint)"
    say "before: $before"
  fi
  ensure_container
  stage "$src" p2
  r="$(new_run p2s)"
  launch "swap:$STAGED" "$r"
  wait_result "$r" 120 || true
  quit_app
  check "the service refused with not_writable" code_is not_writable
  check "/Applications/Beebeeb.app is unchanged (version, CDHash) and no sibling directory exists" eval '[[ "$(fingerprint)" == "$before" ]]'
  is_dry || say "after: $(fingerprint)"
  say "Switch back to the admin user; P3 continues there."
  step_end
}

step_p3_running() {
  step_begin p3-running "swap $N -> $N1 while $N runs, then the app relaunches itself (NSWorkspace, new instance) and quits"
  install_build "$N"
  ensure_container
  local r old
  stage "$(build_app "$N1")" p3
  r="$(new_run p3r)"
  launch "swap-relaunch:$STAGED" "$r"
  wait_result "$r" 300 || true
  old="$(field client_pid)"
  check "the swap answered ok" code_is ok
  check "the relaunch request answered ok" eval '[[ "$(field relaunch)" == ok ]]'
  check "the old process ($N) exits" wait_exit_pid "$old" 30
  check "a new process runs the $N1 code (CDHash)" wait_new_process "$old" "$(build_cdhash "$N1" app)" 45
  check "codesign --verify --deep --strict passes on /Applications/Beebeeb.app" verified
  check "the relaunched app looks normal (menu-bar icon, popover opens, no error)" ask_yes "Does Beebeeb $N1 look normal: menu-bar icon present, popover opens, no error shown?"
  quit_app
  cleanup_siblings
  step_end
}

wait_exit_pid() { # <pid> <seconds>
  is_dry && return 0
  [[ -n "$1" ]] || return 1
  local i
  for ((i = 0; i < $2; i++)); do
    kill -0 "$1" 2>/dev/null || return 0
    sleep 1
  done
  return 1
}

step_p3_quit_first() {
  step_begin p3-quit-first "the app arms the service and quits; the service waits for the exit, swaps, relaunches"
  install_build "$N"
  ensure_container
  local r marker i
  stage "$(build_app "$N1")" p3q
  r="$(new_run p3q)"
  marker="$WORK/after-exit-$r.log"
  launch "swap-after-quit:$STAGED" "$r"
  wait_result "$r" 90 || true
  check "the service armed (ok/armed)" eval '[[ "$(field code)" == ok && "$(field stage)" == armed ]]'
  check "the app quit" wait_exit 30
  if is_dry; then
    say "would wait up to 90 s for a 'done' line in $marker (written by the service), then print it"
  else
    for ((i = 0; i < 90; i++)); do
      grep -q ' done$' "$marker" 2>/dev/null && break
      sleep 1
    done
    say "service marker ($marker):"
    sed 's/^/      /' "$marker" 2>/dev/null || say "      (no marker file: the service wrote nothing after arming)"
    save "$marker"
  fi
  check "the service saw the app exit" eval 'grep -q " client_exited$" "$marker"'
  check "the service survived and swapped (swap code=ok)" eval 'grep -q " swap code=ok " "$marker"'
  check "the service launched the new app" eval 'grep -q " launch ok" "$marker"'
  check "a process runs the $N1 code (CDHash)" wait_new_process "" "$(build_cdhash "$N1" app)" 45
  check "codesign --verify --deep --strict passes on /Applications/Beebeeb.app" verified
  if ! is_dry && ! grep -q ' swap code=' "$marker" 2>/dev/null; then
    say "The marker stops before the swap: the service did not outlive its app. Quit-first then needs"
    say "the SMAppService agent of the spec's section 4.8; swap-while-running (p3-running) is the order that works."
  fi
  quit_app
  cleanup_siblings
  step_end
}

fp_root() {
  if [[ -n "${BB_SPIKE_FP_ROOT:-}" ]]; then
    printf '%s' "$BB_SPIKE_FP_ROOT"
    return 0
  fi
  local found
  found="$(find "$HOME/Library/CloudStorage" -maxdepth 1 -name 'Beebeeb*' 2>/dev/null)"
  [[ "$(grep -c . <<<"$found")" == 1 ]] || die "found $(grep -c . <<<"$found") Beebeeb folders in ~/Library/CloudStorage; set BB_SPIKE_FP_ROOT to the right one"
  printf '%s' "$found"
}

step_p4_start() {
  step_begin p4-start "install $N, start it listening for commands; you sign in and wait for the Finder location"
  install_build "$N"
  launch watch "watch-$(date +%H%M%S)"
  say "In the Beebeeb window: sign in with a TEST account (never your own vault for unreleased code)."
  say "Wait until Beebeeb shows in Finder's sidebar, then run: $HERE/run-proof.sh p4-swap"
  CHECKS=1
  PASSED=1
  step_end
}

step_p4_swap() {
  step_begin p4-swap "start a ${P4_MB} MB upload through Finder, then swap $N -> $N1 while it is in flight"
  local root name file="$WORK/p4-upload.bin" r old state="$WORK/p4-state"
  if ! is_dry; then
    [[ -n "$(running_pids)" ]] || die "Beebeeb is not running; run p4-start first"
    root="$(fp_root)"
    say "Finder location: $root"
    [[ "$(installed_version)" == "$N" ]] || die "/Applications/Beebeeb.app is $(installed_version), expected $N (run p4-start)"
  else
    root="<~/Library/CloudStorage/Beebeeb-...>"
  fi
  name="spike-2176-p4-$TS.bin"
  run "$APP/Contents/MacOS/BeebeebFileProviderCtl" status
  run dd if=/dev/urandom of="$file" bs=1m count="$P4_MB"
  run cp "$file" "$root/$name"
  if ! is_dry; then
    {
      printf 'name=%s\n' "$name"
      printf 'bytes=%s\n' "$(stat -f %z "$file")"
      printf 'sha256=%s\n' "$(shasum -a 256 "$file" | awk '{print $1}')"
      printf 'root=%s\n' "$root"
    } >"$state"
    sed 's/^/    /' "$state"
  fi
  pause "Open the Beebeeb popover: is the upload of $name IN PROGRESS (not finished)?"
  stage "$(build_app "$N1")" p4
  r="$(new_run p4)"
  is_dry || old="$(running_pids | head -1)"
  is_dry || printf 'swap_at=%s\n' "$(date '+%Y-%m-%d %H:%M:%S')" >>"$state"
  send "$r" "swap-relaunch:$STAGED"
  wait_result "$r" 300 || true
  check "the swap answered ok during the upload" code_is ok
  check "a new process runs the $N1 code (CDHash)" wait_new_process "${old:-}" "$(build_cdhash "$N1" app)" 45
  save "$state"
  say "Let the upload finish in the relaunched app (popover: up to date), then run: $HERE/run-proof.sh p4-check"
  step_end
}

appex_is_new() {
  local p h ok=1 want
  want="$(build_cdhash "$N1" appex)"
  for p in $(pgrep -x BeebeebFileProvider 2>/dev/null || true); do
    h="$(cdhash_of "$p")"
    say "File Provider extension pid $p started $(ps -o lstart= -p "$p") runs CDHash $h"
    [[ "$h" == "$want" ]] && ok=0
  done
  [[ $ok == 0 ]] || say "expected the $N1 appex CDHash $want (the $N appex is $(build_cdhash "$N" appex))"
  return $ok
}

step_p4_check() {
  step_begin p4-check "after the upload: no lost write, the domain stays registered, the new appex is loaded"
  local state="$WORK/p4-state" name="<file>" bytes="<bytes>" sha="" root="<root>"
  if ! is_dry; then
    [[ -f "$state" ]] || die "no $state: run p4-swap first"
    name="$(awk -F= '$1=="name"{print $2}' "$state")"
    bytes="$(awk -F= '$1=="bytes"{print $2}' "$state")"
    sha="$(awk -F= '$1=="sha256"{print $2}' "$state")"
    root="$(awk -F= '$1=="root"{sub(/^root=/,""); print}' "$state")"
  fi
  check "the File Provider domain is still registered" eval '[[ "$("$APP/Contents/MacOS/BeebeebFileProviderCtl" status 2>&1)" == installed ]]'
  run pluginkit -m -A -v -i io.beebeeb.app.FileProvider
  check "the running File Provider extension is the $N1 appex (CDHash)" appex_is_new
  check "the local copy in Finder still has the same bytes" eval '[[ "$(shasum -a 256 "$root/$name" | awk "{print \$1}")" == "$sha" ]]'
  check "the server has it: the web app lists $name with exactly ${bytes:-<bytes>} bytes" ask_yes "In the web app, does ${name:-<file>} exist with exactly ${bytes:-<bytes>} bytes?"
  check "Finder shows no pending/error badge on it" ask_yes "Does Finder show ${name:-<file>} as uploaded (no pending or error badge)?"
  save "$state"
  quit_app
  cleanup_siblings
  step_end
}

step_p5() {
  step_begin p5 "osascript 'tell application \"Beebeeb\" to quit', with and without the window"
  require_spike_installed
  require_not_running
  run open "$APP"
  pause "Is the Beebeeb window open?"
  run osascript -e 'tell application "Beebeeb" to quit'
  check "with the window: the process exits within 20 s" wait_exit 20
  if ! is_dry && [[ -n "$(running_pids)" ]]; then quit_app; fi
  run open "$APP"
  pause "Close the Beebeeb window with Cmd-W (the menu-bar icon stays). Is it closed and the icon still there?"
  run osascript -e 'tell application "Beebeeb" to quit'
  check "without the window: the process exits within 20 s" wait_exit 20
  if ! is_dry && [[ -n "$(running_pids)" ]]; then quit_app; fi
  step_end
}

NEG_BEFORE=""
neg_case() { # <label> <staged path or swap-dest argument> <expected code> [verb]
  local label="$1" arg="$2" want="$3" verb="${4:-swap}" r
  r="$(new_run neg)"
  send "$r" "$verb:$arg"
  wait_result "$r" 120 || true
  check "$label: refused with $want" code_is "$want"
  check "$label: /Applications untouched" eval '[[ "$(fingerprint)" == "$NEG_BEFORE" ]]'
}

step_negative() {
  step_begin negative "each must give its refusal code and leave /Applications untouched"
  install_build "$N"
  ensure_container
  is_dry || NEG_BEFORE="$(fingerprint)"
  is_dry || say "installed: $NEG_BEFORE"
  # Fixtures first, then one running app takes every case through the command file.
  stage "$(build_app "$N1")" neg-good
  local good="$STAGED"
  stage "$(build_app "$N1")" neg-inside
  run ln -s /etc/hosts "$STAGED/Contents/Resources/spike-link"
  local inside="$STAGED"
  run rm -f "$STAGE/neg-link.app" "$STAGE/neg-dirlink"
  run ln -s "$(build_app "$N1")" "$STAGE/neg-link.app"
  run ln -s "$WORK/builds/$N1" "$STAGE/neg-dirlink"
  stage "$(build_app "$N1")" neg-adhoc
  run codesign --force --deep --sign - "$STAGED"
  local adhoc="$STAGED"
  stage "$(build_app "$N")" neg-equal
  local equal="$STAGED"
  run rm -rf "$WORK/decoy"
  run mkdir -p "$WORK/decoy"
  run ditto "$(build_app "$N")" "$WORK/decoy/Beebeeb.app"
  local decoy_before=""
  is_dry || decoy_before="$(cdhash_of "$WORK/decoy/Beebeeb.app")"

  launch watch "watch-$(date +%H%M%S)"
  is_dry || sleep 5
  neg_case "symlink inside the staged bundle" "$inside" symlink_refused
  neg_case "staged path is a symlink" "$STAGE/neg-link.app" symlink_refused
  neg_case "a directory on the staged path is a symlink" "$STAGE/neg-dirlink/Beebeeb.app" symlink_refused
  neg_case "staged path outside the container" "$(build_app "$N1")" path_outside_container
  neg_case "staged path climbing out with .." "$CONTAINER/Library/../../../../../../..$(build_app "$N1")" path_outside_container
  neg_case "ad-hoc signed bundle" "$adhoc" invalid_signature
  neg_case "equal version ($N over $N)" "$equal" not_newer
  neg_case "destination other than this app" "$good|$WORK/decoy/Beebeeb.app" destination_refused swap-dest
  check "the decoy destination is unchanged" eval '[[ "$(cdhash_of "$WORK/decoy/Beebeeb.app")" == "$decoy_before" ]]'
  quit_app

  # Older: N1 installed, N staged.
  install_build "$N1"
  is_dry || NEG_BEFORE="$(fingerprint)"
  stage "$(build_app "$N")" neg-older
  local older="$STAGED"
  launch watch "watch-$(date +%H%M%S)"
  is_dry || sleep 5
  neg_case "older version ($N over $N1)" "$older" not_newer
  quit_app
  step_end
}

step_restore() {
  step_begin restore "put back the Beebeeb that was installed before the proof"
  require_not_running
  run mkdir -p "$WORK/discard"
  if is_dry || [[ -e "$WORK/backup/Beebeeb.app" ]]; then
    [[ -e "$APP" ]] || is_dry || say "no app at $APP"
    if is_dry || [[ -e "$APP" ]]; then run mv "$APP" "$WORK/discard/Beebeeb-$TS-$RANDOM.app"; fi
    run mv "$WORK/backup/Beebeeb.app" "$APP"
    is_dry || rm -f "$WORK/backup/VERSION"
  elif is_spike_version "$(installed_version)"; then
    run mv "$APP" "$WORK/discard/Beebeeb-$TS-$RANDOM.app"
  fi
  if is_dry || [[ "$(sibling_count)" != "0" ]]; then
    run find /Applications -maxdepth 1 -name '.beebeeb-update-*' -user "$(id -un)" -exec rm -rf {} +
  fi
  check "no spike build left at /Applications/Beebeeb.app" eval '! is_spike_version "$(installed_version)"'
  check "no sibling directory left in /Applications" eval '[[ "$(sibling_count)" == 0 ]]'
  if ! is_dry && [[ -e "$APP" ]]; then
    check "the restored app verifies" verified
    say "restored: $APP version $(installed_version)"
  fi
  say "Leftovers you may delete: \"$SPIKE_C\" (spike files in the container), $WORK (builds, logs: send logs first), $SHARED"
  step_end
}

step_status() {
  banner "status"
  say "installed: $(installed_version)  running: $(running_pids | tr '\n' ' ')  siblings: $(sibling_count)"
  say "backup: $([[ -e "$WORK/backup/Beebeeb.app" ]] && cat "$WORK/backup/VERSION" 2>/dev/null || echo none)"
  [[ -f "$LOGS/summary.txt" ]] && sed 's/^/    /' "$LOGS/summary.txt"
  return 0
}

main() {
  local s="${1:-}"
  case "$s" in
    preflight) step_preflight ;;
    build) step_build ;;
    p1) step_p1 ;;
    p2-admin) step_p2_admin ;;
    p2-share) step_p2_share ;;
    p2-standard) step_p2_standard ;;
    p3-running) step_p3_running ;;
    p3-quit-first) step_p3_quit_first ;;
    p4-start) step_p4_start ;;
    p4-swap) step_p4_swap ;;
    p4-check) step_p4_check ;;
    p5) step_p5 ;;
    negative) step_negative ;;
    restore) step_restore ;;
    status) step_status ;;
    all)
      is_dry || die "'all' exists only with DRY_RUN=1: the steps need you between them (README)"
      step_preflight
      step_build
      step_p1
      step_p2_admin
      step_p2_share
      step_p2_standard
      step_p3_running
      step_p3_quit_first
      step_p4_start
      step_p4_swap
      step_p4_check
      step_p5
      step_negative
      step_restore
      ;;
    *)
      sed -n '2,/^set -euo pipefail$/p' "$0" | sed -e '$d' -e 's/^# \{0,1\}//'
      exit 2
      ;;
  esac
}

main "$@"
