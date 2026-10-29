#!/usr/bin/env bash
# Fake `gh` for scripts/test-release-branch-point.sh. Installed on PATH as `gh`.
#
#   gh api repos/<repo>/commits/<sha>/pulls
#                                   -> $BB_FAKE_GH_DIR/pulls/<sha>.json (or `[]` when absent)
#   gh api repos/<repo>/git/matching-refs/tags/desktop-v
#                                   -> $BB_FAKE_GH_DIR/tags.json
#   gh api repos/<repo>/git/tags/<sha>
#                                   -> $BB_FAKE_GH_DIR/tag-objects/<sha>.json
#   any other call                  -> logged as "unsupported", exit 1
#
# `--paginate`, `--slurp` and `-H <header>` are accepted and ignored. Every call is appended
# to $BB_FAKE_GH_LOG, so a test can prove which endpoint was asked for.
set -uo pipefail

log() { printf '%s\n' "$*" >>"$BB_FAKE_GH_LOG"; }

if [[ "${1:-}" != "api" || -z "${2:-}" ]]; then
  log "unsupported gh $*"
  echo "fake gh: unsupported call: gh $*" >&2
  exit 1
fi

endpoint="$2"
shift 2
while [[ $# -gt 0 ]]; do
  case "$1" in
    --paginate | --slurp) shift ;;
    -H)
      shift
      [[ $# -gt 0 ]] && shift
      ;;
    *) shift ;;
  esac
done

log "api $endpoint"

case "$endpoint" in
  repos/*/commits/*/pulls)
    sha="${endpoint%/pulls}"
    sha="${sha##*/}"
    if [[ -f "$BB_FAKE_GH_DIR/pulls/$sha.json" ]]; then
      cat "$BB_FAKE_GH_DIR/pulls/$sha.json"
    else
      printf '[]\n'
    fi
    ;;
  repos/*/git/matching-refs/tags/desktop-v)
    cat "$BB_FAKE_GH_DIR/tags.json"
    ;;
  repos/*/git/tags/*)
    sha="${endpoint##*/}"
    if [[ ! -f "$BB_FAKE_GH_DIR/tag-objects/$sha.json" ]]; then
      echo "fake gh: HTTP 404 for tag object $sha" >&2
      exit 1
    fi
    cat "$BB_FAKE_GH_DIR/tag-objects/$sha.json"
    ;;
  *)
    log "unsupported gh api $endpoint"
    echo "fake gh: unsupported endpoint $endpoint" >&2
    exit 1
    ;;
esac
