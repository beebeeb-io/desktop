#!/usr/bin/env bash
# Stand-in for the macOS/GitHub tools scripts/release-macos-local.sh calls, so
# scripts/test-release-macos-local.sh can run the whole release on Linux.
# One file, linked under many names (xcrun, codesign, spctl, hdiutil, ditto,
# plutil, gh, bunx, bun, security, uname, minisign, git, tar); it behaves like
# basename "$0". It is NOT a model of Apple's tools: it models only the
# ordering and trust rules the release script relies on:
#   - stapling needs a prior Accepted notarization of THAT artifact
#   - spctl accepts (as Notarized Developer ID) only a notarized artifact
#   - codesign --verify on a dmg needs a signature made with --timestamp
#   - a dmg built by `hdiutil create` records whether its source app was stapled
#   - the sign workflow is the only thing that creates the updater .sig, and
#     release.yml (publish_existing) only emits darwin-aarch64 when that .sig exists
# Every invocation is appended to $BB_TEST_LOG as one line, in order.
#
# Failure injection (env): BB_SHIM_NOTARY_INVALID="app dmg", BB_SHIM_SPCTL_REJECT="app dmg",
#   BB_SHIM_SPCTL_SOURCE=<text>, BB_SHIM_BUILD_VERSION=<v>, BB_SHIM_APP_UNSIGNED=1,
#   BB_SHIM_DMG_NOT_SIGNED=1, BB_SHIM_NO_RELEASE=1, BB_SHIM_CORRUPT_UPLOAD=1,
#   BB_SHIM_SIGN_RUN_FAIL=1, BB_SHIM_NO_SIG=1, BB_SHIM_MINISIGN_FAIL=1,
#   BB_SHIM_PUBLISH_RUN_FAIL=1, BB_SHIM_MANIFEST_VERSION=<v>, BB_SHIM_MANIFEST_TAG=<tag>,
#   BB_SHIM_FINDER_DENIED=1 (bundle_dmg.sh fails with -10006 whenever a dmg is bundled).

set -uo pipefail

name="$(basename "$0")"
STATE="${BB_TEST_STATE:?}"
LOG="${BB_TEST_LOG:?}"
REAL_GIT="${BB_REAL_GIT:-/usr/bin/git}"
REAL_TAR="${BB_REAL_TAR:-/usr/bin/tar}"

mkdir -p "$STATE/release" "$STATE/runs"
printf '%s %s\n' "$name" "$*" >>"$LOG"

has() { [[ " ${2:-} " == *" $1 "* ]]; }

label_of() { # path -> app | dmg
  case "$1" in
    *.dmg | *.zip) [[ "$1" == *.zip ]] && echo app || echo dmg ;;
    *) echo app ;;
  esac
}

case "$name" in
  git) exec "$REAL_GIT" "$@" ;;
  tar) exec "$REAL_TAR" "$@" ;;

  uname)
    case "${1:-}" in -s) echo Darwin ;; -m) echo arm64 ;; *) echo Darwin ;; esac
    ;;

  security)
    echo '  1) 0123456789ABCDEF0123456789ABCDEF01234567 "Developer ID Application: Devidee B.V. (R8352WDJJR)"'
    echo '     1 valid identities found'
    ;;

  bun) exit 0 ;;

  bunx)
    # bunx tauri build --target aarch64-apple-darwin [--bundles app] [--config JSON] -- --locked
    [[ "${1:-}" == tauri && "${2:-}" == build ]] || exit 0
    [[ "${BEEBEEB_MACOS_HARDENED_RUNTIME:-}" == 1 ]] || { echo "shim: hardened runtime not requested" >&2; exit 1; }
    # Model of the Tauri CLI facts the release script depends on (workspace evidence,
    # tasks 0341-0354 and 1524 gate 63): with bundle.createUpdaterArtifacts on and a
    # pubkey configured, the build writes the .app and the updater tarball and then
    # exits 1 unless TAURI_SIGNING_PRIVATE_KEY is set; and a default (targets "all")
    # build also runs bundle_dmg.sh, which drives Finder over AppleScript.
    bundles="" cfg="" cur=""
    shift 2
    while (($#)); do
      case "$1" in
        --) break ;;
        --bundles) cur=bundles ;;
        --config) cur=config ;;
        --*) cur="" ;;
        *)
          [[ "$cur" == bundles ]] && bundles="$bundles $1"
          [[ "$cur" == config ]] && cfg="$1"
          ;;
      esac
      shift
    done
    conf_updater="$(jq -r 'if (.bundle // {} | has("createUpdaterArtifacts")) then (.bundle.createUpdaterArtifacts | tostring) else "false" end' src-tauri/tauri.conf.json)"
    cfg_updater=""
    if [[ -n "$cfg" ]]; then
      cfg_updater="$(jq -r 'if (.bundle // {} | has("createUpdaterArtifacts")) then (.bundle.createUpdaterArtifacts | tostring) else "" end' <<<"$cfg")" ||
        { echo "shim: --config is not valid JSON" >&2; exit 1; }
    fi
    updater="${cfg_updater:-$conf_updater}"
    pubkey="$(jq -r '.plugins.updater.pubkey // empty' src-tauri/tauri.conf.json)"
    want_dmg=1
    [[ -z "${bundles// /}" ]] || { has dmg "$bundles" && want_dmg=1 || want_dmg=0; }
    ver="$(jq -r .version src-tauri/tauri.conf.json)"
    ver="${BB_SHIM_BUILD_VERSION:-$ver}"
    bdir="src-tauri/target/aarch64-apple-darwin/release/bundle"
    rm -rf "$bdir"
    mkdir -p "$bdir/macos/Beebeeb.app/Contents/MacOS"
    printf 'CFBundleShortVersionString=%s\n' "$ver" >"$bdir/macos/Beebeeb.app/Contents/Info.plist"
    echo binary >"$bdir/macos/Beebeeb.app/Contents/MacOS/Beebeeb"
    # vite build rewrites the tracked dist/index.html
    echo "<!-- rebuilt -->" >>dist/index.html
    echo "shim: build env identity=${APPLE_SIGNING_IDENTITY:-} version=${BEEBEEB_RELEASE_VERSION:-} hardened=${BEEBEEB_MACOS_HARDENED_RUNTIME:-} profiles=${MACOS_APP_PROVISION_PROFILE:-},${MACOS_FILE_PROVIDER_PROVISION_PROFILE:-}" >>"$LOG"
    echo "shim: tauri bundles=${bundles# } updater=$updater dmg_bundling=$want_dmg signing_key=$([[ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]] && echo set || echo unset)" >>"$LOG"
    [[ "${BB_SHIM_BUILD_FAIL:-}" != 1 ]] || { echo "shim: build failed" >&2; exit 1; }
    if ((want_dmg)); then
      mkdir -p "$bdir/dmg"
      if [[ "${BB_SHIM_FINDER_DENIED:-}" == 1 ]]; then
        echo "shim: bundle_dmg.sh failed: Finder got an error: AppleEvent handler failed (-10006)" >&2
        exit 1
      fi
      echo "unnotarized tauri dmg" >"$bdir/dmg/Beebeeb_${ver}_aarch64.dmg"
    fi
    if [[ "$updater" == true ]]; then
      "$REAL_TAR" -czf "$bdir/macos/Beebeeb.app.tar.gz" -C "$bdir/macos" Beebeeb.app
      if [[ -n "$pubkey" && -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
        echo "Error A public key has been found, but no private key. Make sure to set TAURI_SIGNING_PRIVATE_KEY environment variable." >&2
        exit 1
      fi
    fi
    ;;

  plutil)
    # plutil -extract CFBundleShortVersionString raw -o - <plist>
    sed -n 's/^CFBundleShortVersionString=//p' "${@: -1}"
    ;;

  ditto)
    if [[ "${1:-}" == -c ]]; then
      # ditto -c -k --keepParent SRC ZIP
      src="${*: -2:1}"
      zip="${*: -1}"
      "$REAL_TAR" -cf "$zip" -C "$(dirname "$src")" "$(basename "$src")"
    else
      mkdir -p "$2"
      cp -a "$1"/. "$2"/
    fi
    ;;

  xcrun)
    tool="${1:-}"
    sub="${2:-}"
    if [[ "$tool" == notarytool && "$sub" == submit ]]; then
      file="$3"
      label="$(label_of "$file")"
      id="submission-$label-0001"
      if has "$label" "${BB_SHIM_NOTARY_INVALID:-}"; then
        printf '{"id":"%s","status":"Invalid","message":"Processing complete"}\n' "$id"
        exit 1
      fi
      touch "$STATE/notarized-$label"
      printf '{"id":"%s","status":"Accepted","message":"Processing complete"}\n' "$id"
    elif [[ "$tool" == notarytool && "$sub" == log ]]; then
      printf '{"status":"Invalid","issues":[{"path":"Beebeeb.app/Contents/MacOS/x","message":"The binary is not signed with a valid Developer ID certificate."}]}\n' >"${*: -1}"
    elif [[ "$tool" == stapler && "$sub" == staple ]]; then
      target="$3"
      label="$(label_of "$target")"
      [[ -d "$target" ]] && label=app || label=dmg
      [[ -e "$STATE/notarized-$label" ]] || { echo "Could not find ticket for $target" >&2; exit 65; }
      if [[ -d "$target" ]]; then touch "$target/Contents/_stapled"; else echo STAPLED >>"$target"; fi
    elif [[ "$tool" == stapler && "$sub" == validate ]]; then
      target="$3"
      if [[ -d "$target" ]]; then
        [[ -e "$target/Contents/_stapled" ]] || { echo "validate failed" >&2; exit 65; }
      else
        tail -c 64 "$target" | grep -aq STAPLED || { echo "validate failed" >&2; exit 65; }
      fi
      echo "The validate action worked!"
    fi
    ;;

  codesign)
    if has --verify "$*"; then
      target="${*: -1}"
      if [[ -d "$target" ]]; then
        [[ "${BB_SHIM_APP_UNSIGNED:-}" != 1 ]] || { echo "$target: code object is not signed at all" >&2; exit 1; }
      else
        grep -aq SIGNED "$target" 2>/dev/null || { echo "$target: code object is not signed at all" >&2; exit 1; }
      fi
    elif has --sign "$*"; then
      target="${*: -1}"
      has --timestamp "$*" || { echo "shim: dmg signed without --timestamp" >&2; exit 1; }
      [[ "${BB_SHIM_DMG_NOT_SIGNED:-}" == 1 ]] || printf 'SIGNED\n' >>"$target"
    elif has -dvv "$*"; then
      target="${*: -1}"
      if grep -aq SIGNED "$target" 2>/dev/null; then
        echo "Authority=Developer ID Application: Devidee B.V. (R8352WDJJR)" >&2
      fi
    fi
    ;;

  spctl)
    target="${*: -1}"
    label=app
    has install "$*" && label=dmg
    if has "$label" "${BB_SHIM_SPCTL_REJECT:-}" || [[ ! -e "$STATE/notarized-$label" ]]; then
      echo "$target: rejected" >&2
      echo "source=Unnotarized Developer ID" >&2
      exit 3
    fi
    echo "$target: accepted" >&2
    echo "source=${BB_SHIM_SPCTL_SOURCE:-Notarized Developer ID}" >&2
    ;;

  hdiutil)
    case "${1:-}" in
      create)
        # hdiutil create -volname V -srcfolder SRC -fs HFS+ -format UDZO -ov OUT
        src=""
        out="${*: -1}"
        prev=""
        for a in "$@"; do
          [[ "$prev" == -srcfolder ]] && src="$a"
          prev="$a"
        done
        if [[ -e "$src/Beebeeb.app/Contents/_stapled" ]]; then
          echo "shim: dmg source app stapled=1" >>"$LOG"
        else
          echo "shim: dmg source app stapled=0" >>"$LOG"
        fi
        "$REAL_TAR" -cf "$out" -C "$src" .
        ;;
      attach)
        mnt=""
        prev=""
        for a in "$@"; do
          [[ "$prev" == -mountpoint ]] && mnt="$a"
          prev="$a"
        done
        dmg="${*: -1}"
        mkdir -p "$mnt"
        "$REAL_TAR" -xf "$dmg" -C "$mnt" 2>/dev/null
        ;;
      detach) rm -rf "${2:-}" ;;
      verify) echo "checksum is VALID" ;;
    esac
    ;;

  minisign)
    [[ "${BB_SHIM_MINISIGN_FAIL:-}" != 1 ]] || { echo "Signature verification failed" >&2; exit 1; }
    echo "Signature and comment signature verified"
    ;;

  gh)
    sub="${1:-}"
    case "$sub" in
      auth) exit 0 ;;
      release)
        act="${2:-}"
        case "$act" in
          view)
            [[ "${BB_SHIM_NO_RELEASE:-}" != 1 ]] || { echo "release not found" >&2; exit 1; }
            # initial Windows/Linux assets exist from the CI run
            for n in "Beebeeb_${BB_SHIM_VERSION:-0.8.7}_amd64.AppImage.sig" "Beebeeb_${BB_SHIM_VERSION:-0.8.7}_x64-setup.exe.sig" "Beebeeb_${BB_SHIM_VERSION:-0.8.7}_x64_en-US.msi.sig"; do
              [[ -e "$STATE/release/$n" ]] || echo sig >"$STATE/release/$n"
            done
            ( cd "$STATE/release" && ls -1 ) | jq -R . | jq -s '{isDraft:false, assets: map({name: .})}'
            ;;
          upload)
            shift 2
            shift # tag
            for f in "$@"; do
              case "$f" in --*) break ;; esac
              cp "$f" "$STATE/release/$(basename "$f")"
              [[ "${BB_SHIM_CORRUPT_UPLOAD:-}" != 1 ]] || echo corrupt >>"$STATE/release/$(basename "$f")"
              echo "shim: uploaded $(basename "$f")" >>"$LOG"
            done
            ;;
          delete-asset) rm -f "$STATE/release/$4" ;;
          download)
            pattern=""
            dir=""
            prev=""
            for a in "$@"; do
              [[ "$prev" == --pattern ]] && pattern="$a"
              [[ "$prev" == --dir ]] && dir="$a"
              prev="$a"
            done
            [[ -e "$STATE/release/$pattern" ]] || { echo "no assets match $pattern" >&2; exit 1; }
            mkdir -p "$dir"
            cp "$STATE/release/$pattern" "$dir/$pattern"
            ;;
        esac
        ;;
      workflow)
        # gh workflow run WF --repo R --ref REF -f k=v ...
        wf="$3"
        id=$(($(cat "$STATE/run-counter" 2>/dev/null || echo 1000) + 1))
        echo "$id" >"$STATE/run-counter"
        echo "$wf" >"$STATE/runs/$id.wf"
        echo 0 >"$STATE/runs/$id.views"
        concl=success
        if [[ "$wf" == sign-macos-updater-artifact.yml ]]; then
          [[ "${BB_SHIM_SIGN_RUN_FAIL:-}" != 1 ]] && concl=success || concl=failure
          if [[ "$concl" == success && "${BB_SHIM_NO_SIG:-}" != 1 ]]; then
            [[ -e "$STATE/release/Beebeeb.app.tar.gz" ]] || concl=failure
            [[ "$concl" == success ]] && printf 'untrusted comment: signature from tauri secret key\nsig-for-%s\n' "$(sha256sum "$STATE/release/Beebeeb.app.tar.gz" | cut -c1-16)" | base64 | tr -d '\n' >"$STATE/release/Beebeeb.app.tar.gz.sig"
          fi
        elif [[ "$wf" == release.yml ]]; then
          [[ "${BB_SHIM_PUBLISH_RUN_FAIL:-}" != 1 ]] || concl=failure
          ver="" chan=""
          for a in "$@"; do
            case "$a" in version=*) ver="${a#version=}" ;; channel=*) chan="${a#channel=}" ;; esac
          done
          has "publish_existing=true" "$*" || echo "shim: release.yml dispatched WITHOUT publish_existing=true" >>"$LOG"
          case "$chan" in stable) file=latest ;; *) file="$chan" ;; esac
          tag="${BB_SHIM_MANIFEST_TAG:-desktop-v$ver}"
          mver="${BB_SHIM_MANIFEST_VERSION:-$ver}"
          base="https://github.com/beebeeb-io/desktop/releases/download/$tag"
          if [[ -e "$STATE/release/Beebeeb.app.tar.gz.sig" ]]; then
            jq -n --arg v "$mver" --arg b "$base" --arg s "$(cat "$STATE/release/Beebeeb.app.tar.gz.sig")" \
              '{version:$v, platforms:{"linux-x86_64":{url:($b+"/x.AppImage"),signature:"l"},"windows-x86_64-nsis":{url:($b+"/x.exe"),signature:"w"},"darwin-aarch64":{url:($b+"/Beebeeb.app.tar.gz"),signature:$s}}}' >"$STATE/manifest-$file.json"
          else
            jq -n --arg v "$mver" --arg b "$base" \
              '{version:$v, platforms:{"linux-x86_64":{url:($b+"/x.AppImage"),signature:"l"},"windows-x86_64-nsis":{url:($b+"/x.exe"),signature:"w"}}}' >"$STATE/manifest-$file.json"
          fi
        fi
        echo "$concl" >"$STATE/runs/$id.conclusion"
        ;;
      run)
        act="${2:-}"
        if [[ "$act" == list ]]; then
          wf=""
          prev=""
          for a in "$@"; do
            [[ "$prev" == --workflow ]] && wf="$a"
            prev="$a"
          done
          ids=()
          for f in "$STATE"/runs/*.wf; do
            [[ -e "$f" ]] || continue
            [[ "$(cat "$f")" == "$wf" ]] && ids+=("$(basename "$f" .wf)")
          done
          if ((${#ids[@]})); then printf '%s\n' "${ids[@]}" | jq -s 'map({databaseId: (. | tonumber)})'; else echo '[]'; fi
        elif [[ "$act" == view ]]; then
          id="$3"
          n=$(($(cat "$STATE/runs/$id.views") + 1))
          echo "$n" >"$STATE/runs/$id.views"
          if ((n < 2)); then
            jq -n --arg u "https://example.test/runs/$id" '{status:"in_progress",conclusion:null,url:$u}'
          else
            jq -n --arg c "$(cat "$STATE/runs/$id.conclusion")" --arg u "https://example.test/runs/$id" '{status:"completed",conclusion:$c,url:$u}'
          fi
        fi
        ;;
      api)
        path="${*: -1}"
        file="$(basename "$path" .json)"
        [[ -e "$STATE/manifest-$file.json" ]] || { echo "Not Found" >&2; exit 1; }
        cat "$STATE/manifest-$file.json"
        ;;
    esac
    ;;

  *) echo "shim: unknown command $name" >&2; exit 127 ;;
esac
