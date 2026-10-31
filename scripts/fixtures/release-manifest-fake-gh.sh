#!/usr/bin/env bash
# Fake `gh`, `sleep` and `curl` for scripts/test-release-channel-manifest.sh. Installed on PATH
# under each of those three names; dispatches on the name it was called by.
#
#   gh api repos/<repo>/releases/tags/<tag>       -> $BB_FAKE_RELEASE_DIR/release.json
#   gh api repos/<repo>/releases/assets/<id> -H 'Accept: application/octet-stream'
#                                                  -> $BB_FAKE_RELEASE_DIR/assets/<id>
#     (without that Accept header GitHub answers with the asset's JSON metadata; so does this)
#   any other gh call                              -> logged as "unsupported", exit 1
#   sleep N                                        -> logged, returns at once
#   curl ...                                       -> logged, exit 22 (the step must not use it)
#
# BB_FAKE_GH_FAIL="<endpoint suffix>:<times> ..." makes the first <times> requests whose
# endpoint ends in <suffix> fail after writing a partial body (counters in $BB_FAKE_STATE_DIR).
# Every call is appended to $BB_FAKE_GH_LOG.
set -uo pipefail

log() { printf '%s\n' "$*" >>"$BB_FAKE_GH_LOG"; }

case "$(basename "$0")" in
  sleep)
    log "sleep $*"
    exit 0
    ;;
  curl)
    log "curl $*"
    echo "curl: (22) The requested URL returned error: 504 (fake)" >&2
    exit 22
    ;;
esac

if [[ "${1:-}" != "api" || -z "${2:-}" ]]; then
  log "unsupported gh $*"
  echo "fake gh: unsupported call: gh $*" >&2
  exit 1
fi

endpoint="$2"
shift 2
octet=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    -H)
      [[ "${2:-}" == "Accept: application/octet-stream" ]] && octet=1
      shift
      [[ $# -gt 0 ]] && shift
      ;;
    *) shift ;;
  esac
done

# Injected failures: fail the first <times> requests for an endpoint ending in <suffix>.
for rule in ${BB_FAKE_GH_FAIL:-}; do
  suffix="${rule%:*}" times="${rule##*:}"
  if [[ "$endpoint" == *"$suffix" ]]; then
    counter="$BB_FAKE_STATE_DIR/fail-$(printf '%s' "$suffix" | tr '/' '_')"
    seen=$(cat "$counter" 2>/dev/null || echo 0)
    if ((seen < times)); then
      echo $((seen + 1)) >"$counter"
      log "api $endpoint octet=$octet -> 504"
      # A dropped connection leaves part of a body behind: the step must not use it.
      printf 'PARTIAL'
      echo "HTTP 504: Gateway Timeout (fake)" >&2
      exit 1
    fi
  fi
done

case "$endpoint" in
  repos/*/releases/tags/*)
    log "api $endpoint"
    cat "$BB_FAKE_RELEASE_DIR/release.json"
    ;;
  repos/*/releases/assets/*)
    id="${endpoint##*/}"
    log "api $endpoint octet=$octet"
    [[ -f "$BB_FAKE_RELEASE_DIR/assets/$id" ]] || { echo "HTTP 404: Not Found (fake)" >&2; exit 1; }
    if [[ "$octet" == 1 ]]; then
      cat "$BB_FAKE_RELEASE_DIR/assets/$id"
    else
      printf '{"id":%s,"url":"https://api.github.com/%s"}\n' "$id" "$endpoint"
    fi
    ;;
  *)
    log "unsupported gh api $endpoint"
    echo "fake gh: unsupported endpoint $endpoint" >&2
    exit 1
    ;;
esac
