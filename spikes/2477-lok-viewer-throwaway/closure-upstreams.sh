#!/usr/bin/env bash
# THROWAWAY (task 2477). After audit-upstreams.sh, on the same Linux build VM: for the crates a viewer
# would link from each upstream (the "roots"), what comes with them: package count, packages from git,
# packages with an advisory, packages whose licence is outside deny.toml's allow-list, network crates.
set -u
WORK=${CARGO_TARGET_DIR:?}/audit-2477
export PATH="$WORK/tools/bin:$PATH"
NET='reqwest ureq hyper h2 curl isahc attohttpc tiny_http tungstenite rustls native-tls openssl quinn socket2 mio tokio async-std smol async-io'
ALLOW='MIT|MIT-0|Apache-2.0|Apache-2.0 WITH LLVM-exception|BSD-2-Clause|BSD-3-Clause|ISC|Zlib|Unicode-3.0|Unicode-DFS-2016|CC0-1.0|BSL-1.0|0BSD|OFL-1.1|Unlicense'
for spec in "wordcraft:wordcraft-render wordcraft-docx wordcraft-docbin" "gridcraft:gridcraft-engine" "deckcraft:deckcraft-render deckcraft-pptx" "deckcraft:deckcraft-engine" "wordcraft:wordcraft-engine"; do
  name=${spec%%:*}; roots=${spec#*:}
  cd "$WORK/$name" || exit 1
  [ -f "$WORK/$name-audit.json" ] || cargo audit --json >"$WORK/$name-audit.json" 2>/dev/null
  python3 - "$WORK/$name-meta.json" "$WORK/$name-audit.json" "$name" "$roots" "$NET" "$ALLOW" <<'P'
import json, re, sys
m = json.load(open(sys.argv[1])); au = json.load(open(sys.argv[2]))
name, roots, net, allow = sys.argv[3], sys.argv[4].split(), set(sys.argv[5].split()), set(sys.argv[6].split("|"))
pk = {p["id"]: p for p in m["packages"]}
byname = {p["name"]: p["id"] for p in m["packages"] if p["id"] in set(m["workspace_members"])}
deps = {n["id"]: [d["pkg"] for d in n["deps"] if any(k["kind"] is None for k in d["dep_kinds"])] for n in m["resolve"]["nodes"]}
seen = set(); stack = [byname[r] for r in roots]; seen.update(stack)
while stack:
    for d in deps.get(stack.pop(), []):
        if d not in seen: seen.add(d); stack.append(d)
ps = [pk[i] for i in seen]
git = sorted({f"{p['name']} ({p['source'].split('?')[0].replace('git+https://github.com/', '')})" for p in ps if (p.get("source") or "").startswith("git+")})
adv = {}
for x in au.get("vulnerabilities", {}).get("list", []): adv.setdefault((x["package"]["name"], x["package"]["version"]), []).append("VULN " + x["advisory"]["id"])
for k, xs in au.get("warnings", {}).items():
    for x in xs: adv.setdefault((x["package"]["name"], x["package"]["version"]), []).append(k + " " + ((x.get("advisory") or {}).get("id") or "-"))
hits = sorted(f"{p['name']} {p['version']}: {', '.join(sorted(adv[(p['name'], p['version'])]))}" for p in ps if (p["name"], p["version"]) in adv)
def ok(lic):
    if not lic: return False
    parts = [x.strip("() ") for x in re.split(r"\s+OR\s+|/", lic)]
    return any(all(t.strip("() ") in allow for t in re.split(r"\s+AND\s+", part)) for part in parts)
lic = sorted(f"{p['name']} {p['version']}: {p.get('license') or 'NO LICENCE FIELD'}" for p in ps if not ok(p.get("license")) and p["id"] not in set(m["workspace_members"]))
netc = sorted({p["name"] for p in ps} & net)
print(f"== {name} roots={'+'.join(roots)}: packages={len(ps)} (workspace {sum(1 for i in seen if i in set(m['workspace_members']))}) network_crates={len(netc)} {netc} git_packages={len(git)} advisory_packages={len(hits)} licence_outside_allow_list={len(lic)}")
for g in git: print("   git:", g)
for h in hits: print("   advisory:", h)
for l in lic: print("   licence:", l)
P
done
echo "closure-upstreams: done"
