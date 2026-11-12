#!/usr/bin/env bash
# THROWAWAY (task 2477 editor spike). Runs on the LINUX build VM through remote-build; never on a developer machine.
#   run.sh build      clone OUR subtree repository (github.com/beebeeb-io/desktop-office, main), check each subtree's
#                     tree against the tree of the upstream release commit, copy the spike crates in, apply the
#                     font-store patch, build our own binaries, measure closures
#   run.sh run        network block proven per binary, then round trips, frame commands and timings (no network)
#   run.sh remove     delete everything this spike put on the machine and count what is left
# The sources that are built are the ones in our repository. Upstream is cloned only to read the tree hash of each
# pinned release commit: same tree hash = the same files, byte for byte.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
OUT=${BB_OUT:?run through remote-build}
WORK=${CARGO_TARGET_DIR:?}/ed-2477
PHASE=${1:?phase}
NET='reqwest ureq hyper h2 curl isahc attohttpc tiny_http tungstenite rustls native-tls openssl quinn socket2 mio tokio async-std smol async-io'
ALLOW='MIT|MIT-0|Apache-2.0|Apache-2.0 WITH LLVM-exception|BSD-2-Clause|BSD-3-Clause|ISC|Zlib|Unicode-3.0|Unicode-DFS-2016|CC0-1.0|BSL-1.0|0BSD|OFL-1.1|Unlicense'
SPECS='wordcraft:v0.18.1:079f478bd85df7bcc567e3bafa7d7219096f9c3c:5ca7ae4aea4c80b3847de5926f87acedc61ae80c gridcraft:v0.19.1:476448d7d9f3a5f8e93fcf7d98b47fd52293fe7f:242d37604a83032b95c684a32b547ba9b2a059d2 deckcraft:v0.18.1:7cc73ab1895ed2e30c8afbcada20c70b9ea0a440:5aa3ce6744e927f694dc694acdba2deaac10ca91'
BIN=$WORK/target/release
scrub() { sed "s#$WORK#<work>#g; s#$HERE#<spike>#g"; }

count_left() {
  local files=0 procs
  [ -d "$WORK" ] && files=$(find "$WORK" -type f 2>/dev/null | wc -l)
  procs=$(pgrep -c -f 'zz-bb-(wc|gc|dc)-' || true)
  echo "leftovers $1: files under the spike's work directory=$files; spike processes=${procs:-0}; firewall rules added by this spike=0 (the block is a network namespace per command, nothing persists)"
}

if [ "$PHASE" = remove ]; then
  count_left before-remove
  if [ -d "$WORK" ]; then find "$WORK" -depth -delete; fi
  count_left after-remove
  exit 0
fi

if [ "$PHASE" = build ]; then
  mkdir -p "$WORK" && cd "$WORK" || exit 1
  export CARGO_TARGET_DIR="$WORK/target"
  echo "toolchain: $(rustc --version) / $(cargo --version) / $(uname -sm)"
  OFFICE=$WORK/office
  [ -d "$OFFICE/.git" ] || git clone --quiet --branch main https://github.com/beebeeb-io/desktop-office "$OFFICE" || exit 1
  echo "our repository: desktop-office main at $(git -C "$OFFICE" rev-parse HEAD)"
  for spec in $SPECS; do
    IFS=: read -r name tag commit tree <<<"$spec"
    up="$WORK/upstream/$name"
    [ -d "$up/.git" ] || git clone --quiet --depth 1 --branch "$tag" "https://github.com/storytold/$name" "$up" 2>/dev/null || exit 1
    upc=$(git -C "$up" rev-parse HEAD); upt=$(git -C "$up" rev-parse 'HEAD^{tree}'); ours=$(git -C "$OFFICE" rev-parse "HEAD:$name")
    [ "$upc" = "$commit" ] && [ "$upt" = "$ours" ] && [ "$ours" = "$tree" ] || { echo "== $name: upstream commit $upc tree $upt, our subtree $ours: DIFFERENT, STOP"; exit 1; }
    # the comparison must be able to fail: the same test against a tree hash with one digit changed
    wrong=${upt%?}0; [ "$wrong" = "$upt" ] && wrong=${upt%?}1
    echo "== $name: our subtree tree=$ours EQUALS upstream $tag (commit $upc) tree=$upt (same test against a wrong hash: $([ "$ours" = "$wrong" ] && echo EQUALS || echo DIFFERS))"
  done
  for pair in wc-edit:wordcraft wc-engine:wordcraft gc-edit:gridcraft dc-edit:deckcraft; do
    mkdir -p "$OFFICE/${pair#*:}/crates/zz-bb-${pair%%:*}" && cp -r "$HERE/${pair%%:*}/." "$OFFICE/${pair#*:}/crates/zz-bb-${pair%%:*}/"
  done

  closure() { # <label> <dir> <target> <cargo tree args…>
    local label=$1 dir=$2 target=$3; shift 3
    (cd "$dir" && cargo tree -e normal --prefix none --target "$target" -f '{p}|{l}' "$@" 2>"$WORK/tree.err" | sort -u >"$WORK/tree.txt")
    [ -s "$WORK/tree.txt" ] || { echo "== closure $label [$target]: cargo tree FAILED: $(tail -1 "$WORK/tree.err" | scrub)"; return; }
    python3 - "$WORK/tree.txt" "$WORK/$(basename "$dir")-audit.json" "$label [$target]" "$NET" "$ALLOW" <<'P'
import json, re, sys
rows = [l.rstrip("\n") for l in open(sys.argv[1]) if l.strip()]
au = json.load(open(sys.argv[2])); label = sys.argv[3]; net = set(sys.argv[4].split()); allow = set(sys.argv[5].split("|"))
ps = []
for r in rows:
    p, lic = r.replace(" (*)", "").rsplit("|", 1)
    m = re.match(r"(\S+) v(\S+)(?: \((.*)\))?", p.replace(" (proc-macro)", ""))
    ps.append((m.group(1), m.group(2), m.group(3) or "", lic))
ps = sorted(set(ps))
ws = [p for p in ps if p[2].startswith("/")]
git = sorted({f"{p[0]} ({p[2].split('?')[0].split('#')[0].replace('https://github.com/', '')})" for p in ps if p[2].startswith("http")})
adv = {}
for x in au.get("vulnerabilities", {}).get("list", []): adv.setdefault((x["package"]["name"], x["package"]["version"]), []).append("VULN " + x["advisory"]["id"])
for k, xs in au.get("warnings", {}).items():
    for x in xs: adv.setdefault((x["package"]["name"], x["package"]["version"]), []).append(k + " " + ((x.get("advisory") or {}).get("id") or "-"))
hits = sorted(f"{p[0]} {p[1]}: {', '.join(sorted(adv[(p[0], p[1])]))}" for p in ps if (p[0], p[1]) in adv)
def ok(lic):
    if not lic: return False
    parts = [x.strip("() ") for x in re.split(r"\s+OR\s+|/", lic)]
    return any(all(t.strip("() ") in allow for t in re.split(r"\s+AND\s+", part)) for part in parts)
lic = sorted(f"{p[0]} {p[1]}: {p[3] or 'NO LICENCE FIELD'}" for p in ps if not ok(p[3]) and p not in ws)
netc = sorted({p[0] for p in ps} & net)
print(f"== closure {label}: packages={len(ps)} (workspace {len(ws)}) network_crates={len(netc)} {netc} git_packages={len(git)} advisory_packages={len(hits)} licence_outside_allow_list={len(lic)}")
for g in git: print("   git:", g)
for h in hits: print("   advisory:", h)
for l in lic: print("   licence:", l)
P
  }
  TOOLS=$(ls -d "$(dirname "$WORK")"/audit-2477/tools "$(dirname "$WORK")"/../*/audit-2477/tools "$(dirname "$WORK")"/../../*/audit-2477/tools 2>/dev/null | head -1)
  if [ -z "$TOOLS" ] || [ ! -x "$TOOLS/bin/cargo-audit" ]; then
    TOOLS=$WORK/tools; echo "installing cargo-audit under the work directory"
    cargo install --quiet --locked --root "$TOOLS" cargo-audit || { echo "cargo-audit install failed"; exit 1; }
  fi
  export PATH="$TOOLS/bin:$PATH"; echo "tools: $(cargo audit --version)"
  # unpatched wordcraft first: what wordcraft-engine brings as upstream ships it (the spike crates add no package)
  for n in wordcraft gridcraft deckcraft; do
    (cd "$OFFICE/$n" && cargo audit --json >"$WORK/$n-audit.json" 2>/dev/null; echo "$n: cargo audit over the whole lockfile exit=$? vulnerabilities=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["vulnerabilities"]["count"])' "$WORK/$n-audit.json")")
  done
  for t in x86_64-pc-windows-msvc x86_64-unknown-linux-gnu; do
    closure "wordcraft-engine as upstream ships it" "$OFFICE/wordcraft" $t -p wordcraft-engine
  done
  (cd "$OFFICE" && git apply --directory=wordcraft --check "$HERE/patches/wordcraft-engine-font-store-optional.patch" && git apply --directory=wordcraft "$HERE/patches/wordcraft-engine-font-store-optional.patch" && echo "patch: applied ($(git diff --shortstat -- wordcraft/crates/engine))") || { echo "patch does not apply: STOP"; exit 1; }
  for t in x86_64-pc-windows-msvc x86_64-unknown-linux-gnu; do
    closure "wordcraft below the engine (zz-bb-wc-edit: doc+docx+docbin+layout+render)" "$OFFICE/wordcraft" $t -p zz-bb-wc-edit
    closure "wordcraft-engine PATCHED, default features (must equal upstream)" "$OFFICE/wordcraft" $t -p wordcraft-engine
    closure "wordcraft-engine PATCHED, no font store (zz-bb-wc-engine)" "$OFFICE/wordcraft" $t -p zz-bb-wc-engine
    closure "gridcraft-engine (zz-bb-gc-edit)" "$OFFICE/gridcraft" $t -p zz-bb-gc-edit
    closure "deckcraft-engine (zz-bb-dc-edit)" "$OFFICE/deckcraft" $t -p zz-bb-dc-edit
    closure "deckcraft below the engine (model+pptx+render)" "$OFFICE/deckcraft" $t -p deckcraft-model -p deckcraft-pptx -p deckcraft-render
  done
  for spec in "wordcraft:zz-bb-wc-edit zz-bb-wc-engine" "gridcraft:zz-bb-gc-edit" "deckcraft:zz-bb-dc-edit"; do
    name=${spec%%:*}; t0=$(date +%s)
    for p in ${spec#*:}; do
      (cd "$OFFICE/$name" && cargo build --release -p "$p" >"$WORK/build-$p.log" 2>&1); rc=$?
      echo "build $p: exit=$rc $(grep -E '^(warning: .*craft-fonts|error)' "$WORK/build-$p.log" | scrub | head -3 | tr '\n' ' ')"
      [ $rc -eq 0 ] || { tail -30 "$WORK/build-$p.log" | scrub; exit 1; }
    done
    echo "built $name in $(( $(date +%s) - t0 )) s; Cargo.lock lines changed by adding the spike crates: $(git -C "$OFFICE" diff --numstat -- "$name/Cargo.lock" | awk '{print "+"$1" -"$2}')"
  done
  for b in zz-bb-wc-edit zz-bb-wc-engine zz-bb-gc-edit zz-bb-dc-edit; do echo "binary $b: $(stat -c %s "$BIN/$b") bytes"; done
  exit 0
fi

# ---- run ----
[ -x "$BIN/zz-bb-wc-edit" ] || { echo "run 'build' first"; exit 1; }
RES=$WORK/res; mkdir -p "$RES"
# the network block: every measured command runs in its own empty network namespace (no interface but a down loopback)
NONET="unshare -rn"
$NONET true 2>/dev/null || { echo "unshare -rn is not allowed here: NO NETWORK BLOCK, STOP"; exit 1; }
python3 -c 'import socket
s=socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind(("127.0.0.1",47811)); s.listen(8)
while True:
    c,_=s.accept(); c.close()' & LISTENER=$!
sleep 1
for b in zz-bb-wc-edit zz-bb-wc-engine zz-bb-gc-edit zz-bb-dc-edit; do
  echo "network proof $b: open: [$("$BIN/$b" probe 127.0.0.1:47811)] [$("$BIN/$b" probe github.com:443)]  blocked: [$($NONET "$BIN/$b" probe 127.0.0.1:47811)] [$($NONET "$BIN/$b" probe github.com:443)]"
done
kill "$LISTENER"
one() { # <label> <cmd…>   one measured command: no network, 120 s limit, its one JSON line into the log
  local label=$1 line rc; shift
  line=$(cd "$RES" && $NONET timeout 120 "$@" 2>"$WORK/stderr.txt"); rc=$?
  if [ $rc -eq 0 ] && [ -n "$line" ]; then echo "$line" | scrub
  else echo "{\"file\":\"$label\",\"program\":\"$(basename "$1")\",\"FAILED\":\"exit=$rc\",\"stdout\":\"$(echo "$line" | tr -d '"\\' | scrub | cut -c1-200)\",\"stderr\":\"$(tail -3 "$WORK/stderr.txt" | tr -d '"\\' | tr '\n' ' ' | scrub | cut -c1-400)\"}"; fi
}
for dir in fixtures fixtures-other-producer; do
  [ -d "$HERE/$dir" ] || continue
  mkdir -p "$RES/$dir"
  for f in "$HERE/$dir"/*; do
    case "$f" in
      *.docx|*.docm|*.doc) one "$(basename "$f")" "$BIN/zz-bb-wc-edit" roundtrip "$f" "$RES/$dir"; one "$(basename "$f")" "$BIN/zz-bb-wc-engine" roundtrip "$f" "$RES/$dir";;
      *.xlsx|*.xlsm) one "$(basename "$f")" "$BIN/zz-bb-gc-edit" roundtrip "$f" "$RES/$dir";;
      *.pptx|*.pptm) one "$(basename "$f")" "$BIN/zz-bb-dc-edit" roundtrip "$f" "$RES/$dir";;
    esac
  done
done
one frames-wc "$BIN/zz-bb-wc-engine" frames
one frames-gc "$BIN/zz-bb-gc-edit" frames
one frames-dc "$BIN/zz-bb-dc-edit" frames
one keystroke "$BIN/zz-bb-wc-edit" keystroke "$HERE/fixtures/big-30-pages.docx" 200
one keystroke-engine "$BIN/zz-bb-wc-engine" keystroke "$HERE/fixtures/big-30-pages.docx" 200
one latency "$BIN/zz-bb-wc-edit" latency "$HERE/fixtures/big-30-pages.docx" 200
one recalc-1000 "$BIN/zz-bb-gc-edit" recalc 1000
one recalc-20000 "$BIN/zz-bb-gc-edit" recalc 20000
(cd "$RES" && tar czf "$OUT/roundtrip-files.tgz" .) && echo "saved files: $(find "$RES" -type f | wc -l) (returned as roundtrip-files.tgz)"
echo "run: done"
