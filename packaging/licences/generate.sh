#!/usr/bin/env bash
# generate.sh: writes packaging/licences/THIRD-PARTY-LICENSES.txt, the notice the installer ships beside the exes.
#
#   packaging/licences/generate.sh            regenerate the committed file
#   packaging/licences/generate.sh --check    regenerate to a temp file and fail when the committed one differs
#
# What goes in (all three binaries: the app, beebeeb-office-render, beebeeb-media-helper): `cargo about` lists every
# crate of each Cargo.lock graph with its licence text; render.py merges the three lists (one text once, with every
# crate under it) and adds our own AGPL-3.0 text and source locations, the fonts, the bundled data (THIRD-PARTY-NOTICES.md),
# the Ubuntu Font Licence and the MPL-2.0 notice. Needs network (crate sources) and python3. Release: VERSION=0.9.0.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
VERSION="${VERSION:-0.9.0}"
MODE=write; [ "${1:-}" = "--check" ] && MODE=check
OUT="$HERE/THIRD-PARTY-LICENSES.txt"

if ! cargo about --version >/dev/null 2>&1; then
  echo "[licences] installing cargo-about"
  cargo install --locked cargo-about --features cli
fi
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
n=0
for m in native/beebeeb-drive-rune native/beebeeb-office-render native/beebeeb-media-helper; do
  n=$((n+1))
  echo "[licences] cargo about: $m"
  ( cd "$ROOT/$m" && cargo about generate --locked --fail -c "$HERE/about.toml" --format json -o "$TMP/$n.json" )
done
python3 -I "$HERE/render.py" --root "$ROOT" --version "$VERSION" --out "$TMP/out.txt" "$TMP/1.json" "$TMP/2.json" "$TMP/3.json"
if [ "$MODE" = check ]; then
  diff -u "$OUT" "$TMP/out.txt" >/dev/null || { echo "[licences] STALE: $OUT differs from a fresh generate; run packaging/licences/generate.sh" >&2; exit 1; }
  echo "[licences] up to date"
else
  cp "$TMP/out.txt" "$OUT"; echo "[licences] wrote $OUT ($(wc -c <"$OUT") bytes)"
fi
