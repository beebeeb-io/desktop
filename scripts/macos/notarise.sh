#!/usr/bin/env bash
# Submit one file to Apple's notary service and wait; require status Accepted.
#   scripts/macos/notarise.sh <file.zip|.dmg|.pkg> <label> <log-dir>
# On anything but Accepted the notarytool log is fetched into <log-dir>/notary-<label>-log.json, the issues are
# printed, and the script exits 1. Credentials by env name only (the App Store Connect API key stays on this Mac):
#   BB_NOTARY_KEY_ID, BB_NOTARY_ISSUER, BB_NOTARY_KEY_PATH (path to the .p8), BB_NOTARY_TIMEOUT [45m]
set -euo pipefail

FILE="${1:-}" LABEL="${2:-}" DIR="${3:-}"
[[ -f "$FILE" && -n "$LABEL" && -d "$DIR" ]] || { printf 'usage: notarise.sh <file> <label> <log-dir>\n' >&2; exit 2; }
: "${BB_NOTARY_KEY_ID:?BB_NOTARY_KEY_ID is not set}" "${BB_NOTARY_ISSUER:?BB_NOTARY_ISSUER is not set}" "${BB_NOTARY_KEY_PATH:?BB_NOTARY_KEY_PATH is not set}"
[[ -f "$BB_NOTARY_KEY_PATH" ]] || { printf 'notary key file not found (BB_NOTARY_KEY_PATH)\n' >&2; exit 1; }

OUT="$DIR/notary-$LABEL.json" ERR="$DIR/notary-$LABEL.err" LOG="$DIR/notary-$LABEL-log.json"
CREDS=(--key "$BB_NOTARY_KEY_PATH" --key-id "$BB_NOTARY_KEY_ID" --issuer "$BB_NOTARY_ISSUER")

rc=0
xcrun notarytool submit "$FILE" "${CREDS[@]}" --wait --timeout "${BB_NOTARY_TIMEOUT:-45m}" --output-format json >"$OUT" 2>"$ERR" || rc=$?
id="$(jq -r '.id // empty' "$OUT" 2>/dev/null || true)"
status="$(jq -r '.status // empty' "$OUT" 2>/dev/null || true)"

if [[ "$status" != "Accepted" ]]; then
  issues=""
  if [[ -n "$id" ]]; then
    xcrun notarytool log "$id" "${CREDS[@]}" "$LOG" >/dev/null 2>&1 || true
    issues="$(jq -r '(.issues // [])[]? | "  - " + (.path // "?") + ": " + (.message // "?")' "$LOG" 2>/dev/null | head -20 || true)"
  fi
  printf 'notarisation of %s was not Accepted: status "%s", notarytool exit %s, submission "%s"\n' "$LABEL" "${status:-none}" "$rc" "${id:-none}" >&2
  [[ -z "$issues" ]] || printf '%s\n' "$issues" >&2
  exit 1
fi
[[ $rc -eq 0 ]] || { printf 'notarytool exited %s although the JSON said Accepted; not trusting it (see %s)\n' "$rc" "$ERR" >&2; exit 1; }
printf 'notarised %s: Accepted (submission %s)\n' "$LABEL" "$id"
