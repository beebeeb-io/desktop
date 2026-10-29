#!/usr/bin/env bash
# Fake `gh` for scripts/test-release-branch-point.sh. Installed on PATH as `gh`.
#
#   gh api repos/<repo>/commits/<sha>/pulls
#                                   -> $BB_FAKE_GH_DIR/pulls/<sha>.json (or `[]` when absent), or
#                                      a 500-style failure (exit 1) when
#                                      $BB_FAKE_GH_DIR/pull-errors/<sha> exists
#   gh api repos/<repo>/git/matching-refs/tags/desktop-v
#                                   -> $BB_FAKE_GH_DIR/tags.json
#   gh api repos/<repo>/git/tags/<sha>
#                                   -> $BB_FAKE_GH_DIR/tag-objects/<sha>.json
#   gh api repos/<repo>/releases/tags/<tag>
#                                   -> $BB_FAKE_GH_DIR/releases/<tag>.json (HTTP 200), or
#                                      HTTP 404 when that file is absent, or HTTP 500 when
#                                      $BB_FAKE_GH_DIR/release-errors/<tag> exists
#   any other call                  -> logged as "unsupported", exit 1
#
# `--paginate`, `--slurp` and `-H <header>` are accepted and ignored. `--include` makes the
# response start with an `HTTP/2.0 <status>` line and a blank line, as `gh api --include` does.
# Every call is appended to $BB_FAKE_GH_LOG, so a test can prove which endpoint was asked for.
set -uo pipefail

log() { printf '%s\n' "$*" >>"$BB_FAKE_GH_LOG"; }

if [[ "${1:-}" != "api" || -z "${2:-}" ]]; then
  log "unsupported gh $*"
  echo "fake gh: unsupported call: gh $*" >&2
  exit 1
fi

endpoint="$2"
shift 2
include=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --paginate | --slurp) shift ;;
    --include) include=1; shift ;;
    -H)
      shift
      [[ $# -gt 0 ]] && shift
      ;;
    *) shift ;;
  esac
done

log "api $endpoint"

# emit <status line> <body> <exit code>: mirrors `gh api` output, with or without --include.
emit() {
  if [[ "$include" == 1 ]]; then
    printf '%s\n\n%s\n' "$1" "$2"
  else
    printf '%s\n' "$2"
  fi
  exit "$3"
}

case "$endpoint" in
  repos/*/commits/*/pulls)
    sha="${endpoint%/pulls}"
    sha="${sha##*/}"
    if [[ -f "$BB_FAKE_GH_DIR/pull-errors/$sha" ]]; then
      echo "fake gh: HTTP 500 for the pulls of $sha" >&2
      exit 1
    fi
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
  repos/*/releases/tags/*)
    tag="${endpoint##*/tags/}"
    if [[ -f "$BB_FAKE_GH_DIR/release-errors/$tag" ]]; then
      emit "HTTP/2.0 500 Internal Server Error" '{"message":"Server Error"}' 1
    elif [[ -f "$BB_FAKE_GH_DIR/releases/$tag.json" ]]; then
      emit "HTTP/2.0 200 OK" "$(cat "$BB_FAKE_GH_DIR/releases/$tag.json")" 0
    else
      emit "HTTP/2.0 404 Not Found" '{"message":"Not Found"}' 1
    fi
    ;;
  *)
    log "unsupported gh api $endpoint"
    echo "fake gh: unsupported endpoint $endpoint" >&2
    exit 1
    ;;
esac
