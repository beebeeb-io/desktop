#!/usr/bin/env bash
# release-blockers-check.sh <platform>
# platform: macos | windows | linux
#
# Reads RELEASE_BLOCKERS.md from the current directory and prints which items
# bind the given platform, then exits non-zero if any do.  Both
# .github/workflows/release.yml and scripts/release-macos-local.sh delegate to
# this script so the parsing lives in one place.
#
# Scope syntax (fail-closed):
#   - [macOS] <text>  - scoped to macOS only; silently skipped for windows / linux
#   Any other item    - unscoped; binds every platform
#
# Only the EXACT token "[macOS]" (case-sensitive, no extra whitespace inside
# the brackets, one ASCII space after the closing bracket) scopes an item.
# "[macos]", "[ macOS ]", "[MacOS]", "[MACOS]" and similar variants are NOT
# valid scopes; they are treated as unscoped and bind every platform (fail-closed).
#
# An item is any line that starts with optional whitespace followed by - or *
# and then one space (the same pattern used elsewhere in the release gate).
# Items detected by either a hyphen (-) or an asterisk (*) bullet.
# Leading whitespace is allowed (indented list items are matched).
# A tab is a valid separator between the bullet and the text.
#
# Usage:
#   scripts/release-blockers-check.sh macos     # list items that block macOS
#   scripts/release-blockers-check.sh linux     # list items that block Linux
#   scripts/release-blockers-check.sh windows   # list items that block Windows
#
# Output: "N: <line>" for each binding item (N is the 1-based line number in
#   the file), one per line.  Nothing when no items bind.
#
# Exit codes:
#   0 - no items bind this platform (or file absent)
#   1 - one or more items bind this platform (printed to stdout)
#   2 - usage error

set -euo pipefail

usage() {
  printf 'usage: %s <macos|windows|linux>\n' "$0" >&2
  exit 2
}

[[ $# -eq 1 ]] || usage
PLATFORM="$1"
case "$PLATFORM" in
  macos|windows|linux) ;;
  *) usage ;;
esac

BLOCKERS_FILE="${RELEASE_BLOCKERS_FILE:-RELEASE_BLOCKERS.md}"

if [[ ! -f "$BLOCKERS_FILE" ]]; then
  exit 0
fi

binding=""
lineno=0
# "|| [[ -n "$line" ]]" handles files that do not end with a newline:
# the last line is non-empty after EOF, so it is processed too.
while IFS= read -r line || [[ -n "$line" ]]; do
  lineno=$((lineno + 1))
  # Match a list item: optional leading whitespace, then - or *, then one space
  # or tab (IFS-collapsed separators are still valid bullets in Markdown).
  if [[ "$line" =~ ^[[:space:]]*[-*][[:space:]] ]]; then
    # Exact macOS scope: "- [macOS] " or "* [macOS] " with optional leading whitespace.
    # The bracket content must be exactly "macOS" (case-sensitive) with no extra whitespace.
    if [[ "$line" =~ ^[[:space:]]*[-*][[:space:]]\[macOS\][[:space:]] ]]; then
      # macOS-scoped: binds macOS only; skip for every other platform.
      if [[ "$PLATFORM" == "macos" ]]; then
        binding="${binding}${lineno}: ${line}
"
      fi
    else
      # Unscoped (or any non-exact bracket pattern): binds every platform.
      binding="${binding}${lineno}: ${line}
"
    fi
  fi
done < "$BLOCKERS_FILE"

# strip trailing newline
binding="${binding%$'\n'}"

if [[ -n "$binding" ]]; then
  printf '%s\n' "$binding"
  exit 1
fi
exit 0
