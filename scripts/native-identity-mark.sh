#!/usr/bin/env bash
# native-identity-mark — which app is this built native binary?
#
#   scripts/native-identity-mark.sh <binary> <identifier>
#
# The native app carries ONE mark, `beebeeb-app-identity=<identifier>;` (native/beebeeb-drive-rune/build.rs and
# src/shell/identity.rs, BUILD_MARK). This prints every mark in <binary> and exits 0 only when there is exactly
# one and it names <identifier>. The bare identifiers are not searched for: both apps' names are in every
# binary, because the engine names both. A binary with no mark, two marks, or the other app's mark is refused.
set -euo pipefail
[ $# -eq 2 ] || { sed -n '2,9p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }
exe=$1 want=$2
[ -f "$exe" ] || { echo "native-identity-mark: no file at $exe" >&2; exit 2; }
case $want in io.beebeeb.app | io.beebeeb.app.test) ;; *) echo "native-identity-mark: '$want' is not an app" >&2; exit 2 ;; esac
marks=$(grep -a -o 'beebeeb-app-identity=[0-9a-z.]*;' "$exe" || true)
count=$(printf '%s' "$marks" | grep -c . || true)
echo "identity marks in $exe: $count"
[ -z "$marks" ] || printf '%s\n' "$marks" | sed 's/^/  /'
if [ "$count" != 1 ] || [ "$marks" != "beebeeb-app-identity=$want;" ]; then
  echo "native-identity-mark: FAILED: expected exactly one mark, beebeeb-app-identity=$want;" >&2
  exit 1
fi
echo "native-identity-mark: ok ($want)"
