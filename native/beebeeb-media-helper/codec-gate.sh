#!/usr/bin/env bash
# codec-gate.sh: the media helper's release gate for codecs that must NOT ship (the legal brief on media codec patents,
# 2026-11-15, finding F1: a route turned off at run time is not the same as a decoder left out of the binary).
#
# Usage:
#   native/beebeeb-media-helper/codec-gate.sh --exe <release helper> [--exe <another>]... [--pdb <file>]...
#                                             [--licences <dir>] [--targets "<triple> <triple>"] [-- <cargo tree args>]
#   native/beebeeb-media-helper/codec-gate.sh --no-tree --exe <file> [--exe <file>]... [--pdb <file>]... [--licences <dir>]
#   `--no-tree` runs only the binary half and the licence rows: no cargo at all (a Windows exe checked on a machine
#   without the workspace). It needs at least one `--exe`; `--targets ""` stays refused.
#   Everything after `--` goes to `cargo tree` unchanged, so the gate sees the feature set the release build uses
#   (the release builds the default features; pass `-- --features aac` to check a build that adds one).
#
# What it counts (every count is printed; a check that cannot count is RED, never a pass):
#   1. The dependency tree, per target (`cargo tree -e normal --locked --target <t>`, default: x86_64-pc-windows-msvc
#      and aarch64-apple-darwin). Every crate in OFF_CRATES below must appear 0 times. Control: `filmcraft-codecs` must
#      appear (a wrong package, an empty tree or a failed cargo would otherwise read as "0 found").
#   2. The release binary, per `--exe` (and per `--pdb`, the Windows symbols): the same crate names as text (`-` or `_`:
#      source paths in panic locations, symbol names) must appear 0 times. Control: `filmcraft-codecs` must appear (a
#      stripped or path-trimmed binary in which no crate name is visible would otherwise read as "0 found").
#   3. The licence records: a codec built in under a licence (LICENSED below; today H.264, the owner's ruling of
#      2026-11-15 18:04, the licence's free tier) needs its record. "Built in" is read from the build itself: the
#      codec's crate is in a tree of (1) or its name is in a binary of (2). The record must exist and be non-empty.
#      Where: `--licences <dir>`, else `$BB_LICENCE_DIR`, else `docs/legal/licences/` of the workspace this repository
#      is checked out in (two levels above the main checkout: the `<workspace>/repos/desktop` layout). Not found: RED.
#
# Truth line (last line): `codec-gate: <n> checks, <f> failing`. Exit 0 only when f = 0 and at least one `--exe` was
# checked. Without `--exe` the run is PARTIAL (exit 3 when nothing failed): it is never a release pass.
# The gate's own test, with a stand-in cargo and made-up binaries: `codec-gate-test.sh` next to this file.
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# The crates that must not be in the shipped helper, with the reason printed on a red row. Turning a codec on is a
# deliberate change in three places: the helper's `Cargo.toml` features, `deny.toml`'s bans, and this list.
OFF_CRATES=(
  "filmcraft-hevc|HEVC: the system's decoder only, never ours (D-MED-2)"
  "filmcraft-aac|AAC: compiled out until counsel clears it"
  "symphonia-codec-aac|AAC (symphonia's decoder): compiled out with AAC"
  "filmcraft-ac3|AC-3 and E-AC-3 (E-AC-3 is a module of this crate): compiled out until counsel clears them"
  "filmcraft-mpeg2v|MPEG-1/2 video: compiled out until counsel clears it"
  "filmcraft-prores|ProRes: compiled out until counsel clears it"
  "filmcraft-dnx|DNxHD / DNxHR: compiled out until counsel clears it"
  "filmcraft-apv|APV: not needed, compiled out"
  "filmcraft-mxf|MXF: not needed, compiled out"
  "filmcraft-dng|DNG: not needed (the brief: compiled out); filmcraft-media takes it only with its 'dng' feature, see the Office engine's filmcraft/PATCHES.md"
)
CONTROL_CRATE="filmcraft-codecs"
# A codec that may ship only with its signed licence record: crate|record file|what.
LICENSED=(
  "filmcraft-h264|via-la-avc.md|H.264 (Via LA AVC licence)"
)

exes=(); pdbs=(); licences="${BB_LICENCE_DIR:-}"; targets="x86_64-pc-windows-msvc aarch64-apple-darwin"; tree_args=(); no_tree=0
while [ $# -gt 0 ]; do
  case "$1" in
    --exe) exes+=("${2:?--exe needs a path}"); shift 2 ;;
    --pdb) pdbs+=("${2:?--pdb needs a path}"); shift 2 ;;
    --licences) licences="${2:?--licences needs a directory}"; shift 2 ;;
    --targets) targets="${2:?--targets needs a list}"; shift 2 ;;
    --no-tree) no_tree=1; shift ;;
    --) shift; tree_args=("$@"); break ;;
    -h|--help) sed -n '2,33p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "codec-gate: unknown argument: $1" >&2; exit 2 ;;
  esac
done
if [ "$no_tree" -eq 1 ]; then
  if [ "${#exes[@]}" -eq 0 ]; then echo "codec-gate: --no-tree needs at least one --exe (nothing would be checked)" >&2; exit 2; fi
  targets=""
fi
if [ -z "$licences" ]; then
  common="$(git -C "$here" rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)"
  [ -n "$common" ] && licences="$(cd "$common/../../.." 2>/dev/null && pwd)/docs/legal/licences"
fi

checks=0; failing=0; blind=0   # blind: a count above could not run
pass() { checks=$((checks + 1)); echo "ok    $*"; }
fail() { checks=$((checks + 1)); failing=$((failing + 1)); echo "RED   $*"; }
declare -A built_in=()   # crate -> where it was seen (a tree or a binary)

work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT

# 1. The dependency tree, per target.
for t in $targets; do
  tree="$work/tree-$t"
  if ! (cd "$here" && cargo tree -e normal --locked --target "$t" --prefix none --format '{p}' "${tree_args[@]}") \
      > "$tree" 2> "$tree.err"; then
    fail "tree $t: cargo tree failed ($(head -c 300 "$tree.err" | tr '\n' ' ')); nothing counted"
    blind=1; continue
  fi
  awk '{print $1}' "$tree" | sort -u > "$tree.names"
  n=$(grep -cx "$CONTROL_CRATE" "$tree.names")
  if [ "$n" -ge 1 ] && [ "$(wc -l < "$tree.names")" -ge 10 ]; then
    pass "tree $t: control $CONTROL_CRATE found ($(wc -l < "$tree.names") crates in the tree)"
  else
    fail "tree $t: control $CONTROL_CRATE found $n times in $(wc -l < "$tree.names") crates: the tree is not the helper's"
    blind=1; continue
  fi
  for row in "${OFF_CRATES[@]}"; do
    crate="${row%%|*}"; why="${row#*|}"
    n=$(grep -cx "$crate" "$tree.names")
    if [ "$n" -eq 0 ]; then pass "tree $t: $crate 0"; else fail "tree $t: $crate $n (must be 0) - $why"; fi
  done
  for row in "${LICENSED[@]}"; do
    crate="${row%%|*}"
    grep -qx "$crate" "$tree.names" && built_in[$crate]+="tree $t; "
  done
done

# 2. The release binaries (and the Windows symbol files).
count_in() { LC_ALL=C grep -a -o -E "$1" "$2" 2>/dev/null | wc -l | tr -d ' '; }
pattern_of() { printf '%s' "$1" | sed 's/-/[-_]/g'; }
for bin in "${exes[@]}" "${pdbs[@]}"; do
  if [ ! -s "$bin" ]; then fail "binary $bin: missing or empty; nothing counted"; blind=1; continue; fi
  n=$(count_in "$(pattern_of "$CONTROL_CRATE")" "$bin")
  if [ "$n" -ge 1 ]; then
    pass "binary $(basename "$bin"): control $CONTROL_CRATE seen $n times"
  else
    fail "binary $(basename "$bin"): control $CONTROL_CRATE seen 0 times: no crate name is visible in it, so a 0 below would prove nothing"
    blind=1; continue
  fi
  for row in "${OFF_CRATES[@]}"; do
    crate="${row%%|*}"; why="${row#*|}"
    n=$(count_in "$(pattern_of "$crate")" "$bin")
    if [ "$n" -eq 0 ]; then pass "binary $(basename "$bin"): $crate 0"; else fail "binary $(basename "$bin"): $crate $n (must be 0) - $why"; fi
  done
  for row in "${LICENSED[@]}"; do
    crate="${row%%|*}"
    [ "$(count_in "$(pattern_of "$crate")" "$bin")" -ge 1 ] && built_in[$crate]+="binary $(basename "$bin"); "
  done
done

# 3. The licence records of what is built in.
for row in "${LICENSED[@]}"; do
  crate="${row%%|*}"; rest="${row#*|}"; record="${rest%%|*}"; what="${rest#*|}"
  seen="${built_in[$crate]:-}"
  if [ -z "$seen" ] && [ "$blind" -eq 1 ]; then
    fail "licence $what: cannot tell whether $crate is built in (a count above could not run)"
  elif [ -z "$seen" ]; then
    pass "licence $what: $crate not built in, no record needed"
  elif [ -n "$licences" ] && [ -s "$licences/$record" ]; then
    pass "licence $what: built in (${seen%; }), record $licences/$record present"
  else
    fail "licence $what: built in (${seen%; }) and the record ${licences:-<no licence directory>}/$record is absent or empty"
  fi
done

if [ "${#exes[@]}" -eq 0 ]; then
  echo "codec-gate: PARTIAL (no --exe: the binary was not checked, so this is not a release pass): $checks checks, $failing failing"
  [ "$failing" -eq 0 ] && exit 3
  exit 1
fi
echo "codec-gate: $checks checks, $failing failing"
[ "$failing" -eq 0 ] && [ "$checks" -gt 0 ]
