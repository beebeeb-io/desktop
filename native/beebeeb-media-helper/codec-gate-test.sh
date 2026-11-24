#!/usr/bin/env bash
# codec-gate-test.sh: the codec gate's own test. It runs `codec-gate.sh` against made-up inputs: a stand-in `cargo`
# that prints a fixed dependency tree, made-up binaries holding crate paths and symbol names, and a licence directory
# with or without the H.264 record. No real cargo, no build. Each case names the mutant it plants and asserts the exit
# code AND the number of RED rows, so a gate that passes everything (or fails everything) fails here.
#
# Truth line: `codec-gate-test: <n> passed, 0 failed` with n = EXPECTED. Exit 0 only then.
set -uo pipefail

EXPECTED=21
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
gate="${CODEC_GATE_SUT:-$here/codec-gate.sh}"
work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
passed=0; failed=0

# The stand-in cargo: logs its arguments, then prints $FAKE_TREE (or fails like cargo does).
mkdir -p "$work/bin"
cat > "$work/bin/cargo" <<'EOF'
#!/usr/bin/env bash
echo "$*" >> "$FAKE_ARGS_LOG"
if [ "${FAKE_CARGO_FAIL:-0}" = 1 ]; then echo "error: the lock file needs to be updated but --locked was passed" >&2; exit 101; fi
cat "$FAKE_TREE"
EOF
chmod +x "$work/bin/cargo"
# Never the real cargo: refuse to run if the stand-in is not the one found.
if [ "$(PATH="$work/bin:$PATH" command -v cargo)" != "$work/bin/cargo" ]; then
  echo "codec-gate-test: the stand-in cargo is not first on PATH; refusing to run"; exit 2
fi

src='(https://github.com/beebeeb-io/desktop-office?rev=d4cc7039#d4cc7039)'
clean_tree() {
  printf '%s\n' "beebeeb-media-helper v0.0.1 (/w/native/beebeeb-media-helper)" \
    "beebeeb-media-protocol v0.0.1 (/w/crates/beebeeb-media-protocol)" \
    "filmcraft-codecs v0.9.0 $src" "filmcraft-h264 v0.9.0 $src" "filmcraft-vp8 v0.9.0 $src" \
    "filmcraft-vp9 v0.9.0 $src" "filmcraft-av1 v0.9.0 $src" "filmcraft-media v0.9.0 $src" \
    "filmcraft-isobmff v0.9.0 $src" "filmcraft-matroska v0.9.0 $src" "filmcraft-media v0.9.0 $src (*)" \
    "symphonia v0.5.5" "symphonia-core v0.5.5" "symphonia-bundle-mp3 v0.5.5" "rayon v1.10.0" "log v0.4.22"
}
path_of() { printf '/home/runner/.cargo/git/checkouts/desktop-office-1a2b3c/d4cc703/filmcraft/%s/src/lib.rs' "$1"; }
make_bin() {  # make_bin <file> <text>...: binary junk around each text
  : > "$1"; local f="$1"; shift
  for t in "$@"; do printf 'MZ\220\000\003\000%s\000\377\376' "$t" >> "$f"; done
}

clean_tree > "$work/tree.clean"
clean_tree | grep -v '^filmcraft-h264 ' > "$work/tree.no-h264"
{ clean_tree; echo "filmcraft-aac v0.9.0 $src"; echo "symphonia-codec-aac v0.5.5"; } > "$work/tree.aac"
{ clean_tree; echo "filmcraft-hevc v0.9.0 $src"; } > "$work/tree.hevc"
{ clean_tree; echo "filmcraft-dng v0.9.0 $src"; } > "$work/tree.dng"
clean_tree | grep -v '^filmcraft-codecs ' > "$work/tree.no-control"
make_bin "$work/clean.exe" "$(path_of filmcraft-codecs)" "$(path_of filmcraft-h264)" "$(path_of filmcraft-vp9)"
make_bin "$work/no-h264.exe" "$(path_of filmcraft-codecs)" "$(path_of filmcraft-vp9)"
make_bin "$work/hevc.exe" "$(path_of filmcraft-codecs)" "$(path_of filmcraft-h264)" "$(path_of filmcraft-hevc)"
make_bin "$work/aac-symbol.bin" "$(path_of filmcraft-codecs)" "$(path_of filmcraft-h264)" \
  "_ZN13filmcraft_aac7Decoder3new17h0123456789abcdefE"
make_bin "$work/stripped.exe" "no crate names here" "just code"
mkdir -p "$work/lic-yes" "$work/lic-no"
echo "Via LA AVC licence, signed (test fixture)" > "$work/lic-yes/via-la-avc.md"

# run <name> <want exit> <want RED rows> <tree file> [gate args...]
run() {
  local name="$1" want_rc="$2" want_red="$3" tree="$4"; shift 4
  local out="$work/out.$passed.$failed"
  FAKE_TREE="$tree" FAKE_ARGS_LOG="$work/args.log" PATH="$work/bin:$PATH" bash "$gate" "$@" > "$out" 2>&1
  local rc=$? red
  red=$(grep -c '^RED' "$out")
  if [ "$rc" -eq "$want_rc" ] && [ "$red" -eq "$want_red" ] && tail -1 "$out" | grep -q '^codec-gate: '; then
    passed=$((passed + 1)); echo "pass  $name (exit $rc, $red RED)"
  else
    failed=$((failed + 1)); echo "FAIL  $name: exit $rc (want $want_rc), $red RED (want $want_red)"; sed 's/^/      /' "$out"
  fi
}

run "clean build, record present: green"            0 0 "$work/tree.clean"      --exe "$work/clean.exe" --licences "$work/lic-yes"
run "clean build, record absent: licence red"       1 1 "$work/tree.clean"      --exe "$work/clean.exe" --licences "$work/lic-no"
run "mutant tree: aac on (2 crates x 2 targets)"    1 4 "$work/tree.aac"        --exe "$work/clean.exe" --licences "$work/lic-yes"
run "mutant tree: hevc linked"                      1 2 "$work/tree.hevc"       --exe "$work/clean.exe" --licences "$work/lic-yes"
run "mutant tree: dng linked (today's real state)"  1 2 "$work/tree.dng"        --exe "$work/clean.exe" --licences "$work/lic-yes"
# A red row prints its reason as written: shell syntax inside a reason (a backquoted word once ran as a command,
# "dng: command not found", and dropped the word from the row) fails here.
FAKE_TREE="$work/tree.dng" FAKE_ARGS_LOG="$work/args.log" PATH="$work/bin:$PATH" \
  bash "$gate" --exe "$work/clean.exe" --licences "$work/lic-yes" > "$work/out.reason" 2>&1
n_reason=$(grep -c "^RED   tree .*: filmcraft-dng 1 (must be 0) - .* its 'dng' feature, see " "$work/out.reason")
if ! grep -q 'command not found' "$work/out.reason" && [ "$n_reason" -eq 2 ]; then
  passed=$((passed + 1)); echo "pass  a red row's reason prints as written (dng row x 2 targets)"
else
  failed=$((failed + 1)); echo "FAIL  a red row's reason: $n_reason dng rows with the whole reason (want 2):"
  sed 's/^/      /' "$work/out.reason"
fi
run "mutant binary: hevc source path"               1 1 "$work/tree.clean"      --exe "$work/hevc.exe" --licences "$work/lic-yes"
run "mutant binary: aac symbol name"                1 1 "$work/tree.clean"      --exe "$work/aac-symbol.bin" --licences "$work/lic-yes"
run "blind binary: control red"                     1 1 "$work/tree.clean"      --exe "$work/stripped.exe" --licences "$work/lic-yes"
run "wrong tree: control red per target"            1 2 "$work/tree.no-control" --exe "$work/clean.exe" --licences "$work/lic-yes"
FAKE_CARGO_FAIL=1 run "cargo fails: red per target" 1 2 "$work/tree.clean"      --exe "$work/clean.exe" --licences "$work/lic-yes"
FAKE_CARGO_FAIL=1 run "cargo fails, no binary sight of h264: licence cannot tell" \
                                                    1 3 "$work/tree.clean"      --exe "$work/no-h264.exe" --licences "$work/lic-no"
run "h264 off everywhere, no record: green"         0 0 "$work/tree.no-h264"    --exe "$work/no-h264.exe" --licences "$work/lic-no"
run "h264 only in the binary, no record: red"       1 1 "$work/tree.no-h264"    --exe "$work/clean.exe" --licences "$work/lic-no"
run "missing exe: red"                              1 1 "$work/tree.clean"      --exe "$work/nope.exe" --licences "$work/lic-yes"
run "no --exe: partial, never a pass"               3 0 "$work/tree.clean"      --licences "$work/lic-yes"
BB_LICENCE_DIR="$work/lic-yes" run "record found through BB_LICENCE_DIR" \
                                                    0 0 "$work/tree.clean"      --exe "$work/clean.exe"

# --no-tree: only the binary half, no cargo at all (the stand-in cargo fails, and logs, if it is called).
: > "$work/args.log"
FAKE_CARGO_FAIL=1 run "no_tree_runs_only_the_binary_half" 0 0 "$work/tree.clean" --no-tree --exe "$work/clean.exe" --licences "$work/lic-yes"
if [ ! -s "$work/args.log" ]; then
  passed=$((passed + 1)); echo "pass  no_tree_runs_only_the_binary_half: cargo never called"
else
  failed=$((failed + 1)); echo "FAIL  no_tree_runs_only_the_binary_half: cargo was called:"; sed 's/^/      /' "$work/args.log"
fi
run "no_tree_without_exe_is_refused" 2 0 "$work/tree.clean" --no-tree --licences "$work/lic-yes"

# The arguments the gate gave cargo: both default targets, --locked, normal edges, and the pass-through.
: > "$work/args.log"
FAKE_TREE="$work/tree.clean" FAKE_ARGS_LOG="$work/args.log" PATH="$work/bin:$PATH" \
  bash "$gate" --exe "$work/clean.exe" --licences "$work/lic-yes" -- --features aac > /dev/null 2>&1
n_win=$(grep -c -- '^tree -e normal --locked --target x86_64-pc-windows-msvc .*--features aac$' "$work/args.log")
n_mac=$(grep -c -- '^tree -e normal --locked --target aarch64-apple-darwin .*--features aac$' "$work/args.log")
if [ "$n_win" -eq 1 ] && [ "$n_mac" -eq 1 ] && [ "$(wc -l < "$work/args.log")" -eq 2 ]; then
  passed=$((passed + 1)); echo "pass  cargo asked once per target, locked, normal edges, features passed through"
else
  failed=$((failed + 1)); echo "FAIL  cargo arguments:"; sed 's/^/      /' "$work/args.log"
fi

echo "codec-gate-test: $passed passed, $failed failed"
[ "$failed" -eq 0 ] && [ "$passed" -eq "$EXPECTED" ]
