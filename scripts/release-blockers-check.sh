#!/usr/bin/env bash
# release-blockers-check.sh <platform>
# platform: macos | windows | linux
#
# Reads RELEASE_BLOCKERS.md from the current directory and prints which items
# bind the given platform, then exits non-zero if any do.
# .github/workflows/release-native-windows.yml delegates to this script, so the
# parsing lives in one place (the 0.8.x release.yml and the macOS release
# script did too; both went with the Tauri app in 0.9.0).
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
# Comparison with main (LD-0014): a release branch could delete a blocker that main still lists.
# When RELEASE_BLOCKERS_MAIN_REF is set (a git ref in the current repository, for example origin/main),
# every item in that ref's RELEASE_BLOCKERS.md that the branch's file lacks (compared after trimming
# whitespace and normalising the bullet to "-") binds the platform by the same scope rules and is
# printed as "main: <item>". An unresolvable ref fails closed (exit 1). Unset: no comparison.
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

if [[ ! -f "$BLOCKERS_FILE" && -z "${RELEASE_BLOCKERS_MAIN_REF:-}" ]]; then
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
done < <(if [[ -f "$BLOCKERS_FILE" ]]; then cat "$BLOCKERS_FILE"; fi)

if [[ -n "${RELEASE_BLOCKERS_MAIN_REF:-}" ]]; then
  if ! git rev-parse --verify --quiet "${RELEASE_BLOCKERS_MAIN_REF}^{commit}" >/dev/null 2>&1; then
    printf 'main: RELEASE_BLOCKERS_MAIN_REF "%s" does not resolve to a commit; refusing to guess what main lists\n' \
      "$RELEASE_BLOCKERS_MAIN_REF"
    exit 1
  fi
  norm() { local l="$1"; l="${l#"${l%%[![:space:]]*}"}"; l="${l%"${l##*[![:space:]]}"}"; printf -- '- %s' "${l#[-*]}" | sed -E 's/^- [[:space:]]+/- /'; }
  branch_items=""
  if [[ -f "$BLOCKERS_FILE" ]]; then
    while IFS= read -r line || [[ -n "$line" ]]; do
      if [[ "$line" =~ ^[[:space:]]*[-*][[:space:]] ]]; then
        branch_items="${branch_items}$(norm "$line")"$'\n'
      fi
    done < "$BLOCKERS_FILE"
  fi
  main_file="$(git show "${RELEASE_BLOCKERS_MAIN_REF}:RELEASE_BLOCKERS.md" 2>/dev/null || true)"
  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ "$line" =~ ^[[:space:]]*[-*][[:space:]] ]] || continue
    n="$(norm "$line")"
    if printf '%s' "$branch_items" | grep -qxF -- "$n"; then continue; fi
    if [[ "$n" =~ ^-[[:space:]]\[macOS\][[:space:]] && "$PLATFORM" != "macos" ]]; then continue; fi
    binding="${binding}main: ${n}
"
  done <<<"$main_file"
fi

# strip trailing newline
binding="${binding%$'\n'}"

if [[ -n "$binding" ]]; then
  printf '%s\n' "$binding"
  exit 1
fi
exit 0
