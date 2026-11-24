#!/usr/bin/env bash
# THROWAWAY (task 2477). Runs on the LINUX build VM through remote-build. For the three upstream release
# tags that `desktop-office` pins: cargo audit, cargo deny (policy: deny.toml next to this file), and a map
# of which workspace crate reaches a network client, TLS stack, socket layer or async runtime.
# Tools are installed under the run's own target directory, never into the machine's cargo home.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
OUT=${BB_OUT:?run through remote-build}
WORK=${CARGO_TARGET_DIR:?}/audit-2477
TOOLS=$WORK/tools
mkdir -p "$WORK" "$TOOLS"
export CARGO_TARGET_DIR="$WORK/target"
if [ ! -x "$TOOLS/bin/cargo-deny" ] || [ ! -x "$TOOLS/bin/cargo-audit" ]; then
  cargo install --quiet --locked --root "$TOOLS" cargo-audit cargo-deny || { echo "tool install failed"; exit 1; }
fi
export PATH="$TOOLS/bin:$PATH"
echo "tools: $(cargo audit --version) / $(cargo deny --version)"
NET='reqwest ureq hyper h2 curl isahc attohttpc tiny_http tungstenite rustls native-tls openssl quinn socket2 mio tokio async-std smol async-io'

for spec in wordcraft:v0.18.1:079f478bd85df7bcc567e3bafa7d7219096f9c3c gridcraft:v0.19.1:476448d7d9f3a5f8e93fcf7d98b47fd52293fe7f deckcraft:v0.18.1:7cc73ab1895ed2e30c8afbcada20c70b9ea0a440; do
  IFS=: read -r name tag want <<<"$spec"
  dir="$WORK/$name"
  [ -d "$dir/.git" ] || git clone --quiet --depth 1 --branch "$tag" "https://github.com/storytold/$name" "$dir"
  got=$(git -C "$dir" rev-parse HEAD)
  echo "== $name $tag commit=$got $([ "$got" = "$want" ] && echo MATCHES-the-pin || echo DIFFERS-from-the-pin)"
  cd "$dir" || exit 1

  cargo audit --json >"$OUT/$name-audit.json" 2>"$OUT/$name-audit.err"; echo "$name audit exit=$?"
  python3 - "$OUT/$name-audit.json" "$name" <<'P'
import json, sys
d = json.load(open(sys.argv[1])); n = sys.argv[2]
v = d.get("vulnerabilities", {}); w = d.get("warnings", {})
print(f"{n} audit: lockfile_packages={d.get('lockfile', {}).get('dependency-count')} vulnerabilities={v.get('count')} "
      + " ".join(f"{k}={len(x)}" for k, x in sorted(w.items())))
for x in v.get("list", []):
    a = x["advisory"]; print(f"  VULN {a['id']} {x['package']['name']} {x['package']['version']}: {a['title']}")
for k, xs in sorted(w.items()):
    for x in xs:
        a = x.get("advisory") or {}; print(f"  {k} {a.get('id', '-')} {x['package']['name']} {x['package']['version']}: {a.get('title', '')}")
P

  for check in advisories bans licenses sources; do
    cargo deny --config "$HERE/deny.toml" check "$check" >"$OUT/$name-deny-$check.txt" 2>&1; code=$?
    echo "$name deny $check: exit=$code errors=$(grep -c '^error\[' "$OUT/$name-deny-$check.txt") warnings=$(grep -c '^warning\[' "$OUT/$name-deny-$check.txt")"
  done
  grep -h -A1 '^error\[banned\]' "$OUT/$name-deny-bans.txt" | grep -o "crate '[^']*'" | sort | uniq -c | sed "s/^/  $name banned /"
  grep -h '^error\[' "$OUT/$name-deny-licenses.txt" "$OUT/$name-deny-sources.txt" | sort | uniq -c | sed "s/^/  $name /"
  grep -h -A3 '^error\[rejected\]\|^error\[unlicensed\]' "$OUT/$name-deny-licenses.txt" | grep -o '^ *[0-9]* *│ *[a-zA-Z0-9_-]* [0-9][^ ]*\|name = "[^"]*"' | sort | uniq -c | head -20 | sed "s/^/  $name licence-detail /"
  grep -h -B1 -A6 '^error\[source-not-allowed\]\|^error\[git-source' "$OUT/$name-deny-sources.txt" | grep -o 'git+[^ "#]*' | sort | uniq -c | sed "s/^/  $name git-source /"

  cargo metadata --format-version 1 --locked >"$WORK/$name-meta.json" 2>"$OUT/$name-metadata.err" || { echo "$name: cargo metadata failed"; continue; }
  python3 - "$WORK/$name-meta.json" "$name" "$NET" <<'P'
import json, sys
m = json.load(open(sys.argv[1])); name = sys.argv[2]; net = set(sys.argv[3].split())
pk = {p["id"]: p for p in m["packages"]}
members = set(m["workspace_members"])
deps = {}
for n in m["resolve"]["nodes"]:
    # normal dependencies only (what ships in a build of that crate), default features of the workspace
    deps[n["id"]] = [d["pkg"] for d in n["deps"] if any(k["kind"] is None for k in d["dep_kinds"])]
def closure(i):
    seen = set(); stack = [i]
    while stack:
        for d in deps.get(stack.pop(), []):
            if d not in seen: seen.add(d); stack.append(d)
    return seen
print(f"{name} map: workspace_members={len(members)} packages_resolved={len(pk)}")
clean, dirty = [], []
for i in sorted(members, key=lambda i: pk[i]["name"]):
    c = closure(i)
    hit = sorted({pk[d]["name"] for d in c} & net)
    if not hit:
        clean.append(pk[i]["name"]); continue
    # who brings each one: the member's own direct dependencies whose closure (or self) contains it
    via = {}
    for h in hit:
        via[h] = sorted({pk[d]["name"] for d in deps[i] if pk[d]["name"] == h or h in {pk[x]["name"] for x in closure(d)}})
    dirty.append(pk[i]["name"])
    print(f"  {name} REACHES {pk[i]['name']}: " + "; ".join(f"{h} via {','.join(v)}" for h, v in via.items()))
print(f"  {name} CLEAN ({len(clean)} of {len(members)}): {' '.join(clean)}")
print(f"  {name} REACHING ({len(dirty)} of {len(members)}): {' '.join(dirty)}")
P
done
echo "audit-upstreams: done"
