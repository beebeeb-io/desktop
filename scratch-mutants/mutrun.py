#!/usr/bin/env python3
"""O-CP3 mutant runner. Tables: mutants/tables/*.py, each defining MUT = [dict(...), ...]:
    id      unique id (e.g. "H2-R4a")
    crate   one of CRATES below (picks the dir and the default command)
    files   [(path from the repo root, old, new[, count=1])]: `old` must occur exactly `count` times, replaced all
    filt    the test-name filter(s) after `--` (space separated; '' = none)
    expect  [test-name substrings that must FAIL]
    cmd     optional: the full cargo command, run in the crate dir (overrides crate default + filt)
    note    optional: the lane's row text / why
    where   optional: 'linux' (default) | 'win' | 'none' (cannot go red: listed, not run)
Modes:
    verify <repo-root> [ids…]          every anchor count checked against the tree (no edits)
    run <repo-root> <out-dir> [ids…]   controls first (one per distinct command), then each mutant: apply, run, restore
    list                               id, crate, where
Verdicts: KILLED (every expect FAILED), PARTIAL (some), SURVIVED (none), COMPILE-ERROR (no test ran: not a kill),
NOT-IN-CONTROL (an expected name did not pass in the control run: the mutant is not trusted)."""
import glob, hashlib, json, os, re, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
CRATES = {
    "native":   ("native/beebeeb-drive-rune",      "cargo test --locked --bin beebeeb-drive -- {filt}"),
    "native-all": ("native/beebeeb-drive-rune",    "cargo test --locked -- {filt}"),
    "engine":   ("crates/beebeeb-engine",          "cargo test --locked --lib -- {filt}"),
    "helper":   ("native/beebeeb-office-render",   "cargo test --locked -- {filt}"),
    "protocol": ("crates/beebeeb-office-protocol", "cargo test --locked -- {filt}"),
    "sandbox":  ("crates/beebeeb-office-sandbox",  "cargo test --locked --lib -- {filt}"),
    "office":   ("beebeeb-office",                 "cargo test --locked -- {filt}"),
}

def load(only=None):
    muts = []
    for f in sorted(glob.glob(os.path.join(HERE, "tables", "*.py"))):
        g = {}
        exec(compile(open(f).read(), f, "exec"), g)
        for m in g["MUT"]:
            m.setdefault("where", "linux"); m.setdefault("note", ""); m["table"] = os.path.basename(f)
            muts.append(m)
    ids = [m["id"] for m in muts]
    dup = {i for i in ids if ids.count(i) > 1}
    if dup: sys.exit(f"duplicate ids: {sorted(dup)}")
    if only: muts = [m for m in muts if m["id"] in only or m["table"] in only or any(m["id"].startswith(p.rstrip('*')) for p in only if p.endswith('*'))]
    return muts

def command(m):
    d, c = CRATES[m["crate"]]
    return d, (m.get("cmd") or c.format(filt=m.get("filt", ""))).strip()

def check_anchors(root, m):
    errs = []
    for t in m["files"]:
        path, old, new = t[0], t[1], t[2]; count = t[3] if len(t) > 3 else 1
        p = os.path.join(root, path)
        if not os.path.exists(p): errs.append(f"{path}: no such file"); continue
        n = open(p, encoding="utf-8").read().count(old)
        if n != count: errs.append(f"{path}: anchor found {n} times, expected {count}: {old[:70]!r}")
        if old == new: errs.append(f"{path}: old == new")
    return errs

def apply(root, m):
    saved = {}
    for t in m["files"]:
        path, old, new = t[0], t[1], t[2]
        p = os.path.join(root, path)
        if p not in saved: saved[p] = open(p, encoding="utf-8").read()
        s = open(p, encoding="utf-8").read()
        open(p, "w", encoding="utf-8").write(s.replace(old, new))
    return saved

def restore(saved):
    for p, s in saved.items(): open(p, "w", encoding="utf-8").write(s)
    return all(open(p, encoding="utf-8").read() == s for p, s in saved.items())

def run_cmd(root, d, cmd, log, limit):
    """Runs `cmd`; past `limit` seconds its whole process group is killed and rc is 124 (verdict TIMEOUT)."""
    t0 = time.time()
    with open(log, "w") as f:
        p = subprocess.Popen(cmd, shell=True, cwd=os.path.join(root, d), stdout=f, stderr=subprocess.STDOUT,
                             start_new_session=True)
        try:
            rc = p.wait(timeout=limit)
        except subprocess.TimeoutExpired:
            os.killpg(p.pid, 9); p.wait(); rc = 124
            f.write(f"\nMUTRUN TIMEOUT after {limit} s\n")
    return rc, int(time.time() - t0)

def parse(log):
    s = open(log, errors="replace").read()
    failed = set(re.findall(r"^test (\S+) \.\.\. FAILED", s, re.M))
    ok = set(re.findall(r"^test (\S+) \.\.\. ok", s, re.M))
    results = re.findall(r"^test result: .*$", s, re.M)
    passed = sum(int(x) for x in re.findall(r"test result: \w+\. (\d+) passed", s))
    nfail = sum(int(x) for x in re.findall(r"(\d+) failed", " ".join(results)))
    compile_err = bool(re.search(r"^error(\[E\d+\])?:", s, re.M)) and not results
    return failed, ok, results, passed, nfail, compile_err

def main():
    mode = sys.argv[1]
    if mode == "list":
        for m in load(): print(m["id"], m["crate"], m["where"], m["table"])
        return
    root = os.path.abspath(sys.argv[2])
    if mode == "verify":
        muts = load(sys.argv[3:] or None); bad = 0
        for m in muts:
            e = check_anchors(root, m)
            if m["crate"] not in CRATES: e.append(f"crate {m['crate']} unknown")
            if not m.get("expect") and m["where"] != "none": e.append("no expect")
            print(("OK   " if not e else "FAIL ") + m["id"] + ("" if not e else "  " + " | ".join(e)))
            bad += bool(e)
        print(f"verify: {len(muts)} mutants, {len(muts)-bad} ok, {bad} failing"); sys.exit(1 if bad else 0)
    out = os.path.abspath(sys.argv[3]); os.makedirs(out, exist_ok=True)
    muts = [m for m in load(sys.argv[4:] or None) if m["where"] in ("linux", os.environ.get("MUT_WHERE", "linux"))]
    summary = open(os.path.join(out, "summary.tsv"), "a")
    head = subprocess.run("git rev-parse --short=8 HEAD", shell=True, cwd=root, capture_output=True, text=True).stdout.strip()
    print(f"head {head} mutants {len(muts)}", flush=True)
    for m in muts:
        e = check_anchors(root, m)
        if e: print(f"ANCHOR {m['id']}: {e}", flush=True)
    # controls: one per distinct (dir, command)
    controls = {}
    for m in muts:
        key = command(m)
        if key in controls: continue
        n = len(controls) + 1; log = os.path.join(out, f"control-{n}.log")
        rc, secs = run_cmd(root, key[0], key[1], log, 3600)
        failed, ok, results, passed, nfail, ce = parse(log)
        controls[key] = (n, ok)
        line = f"CONTROL-{n}\t{key[0]}\t{key[1]}\trc={rc}\tpassed={passed}\tfailed={nfail}\t{secs}s"
        print(line, flush=True); summary.write(line + "\n"); summary.flush()
    for m in muts:
        key = command(m); cn, cok = controls[key]
        e = check_anchors(root, m)
        if e:
            line = f"{m['id']}\tANCHOR-MISMATCH\t{' | '.join(e)}"; print(line, flush=True); summary.write(line + "\n"); continue
        missing = [x for x in m["expect"] if not any(x in t for t in cok)]
        saved = apply(root, m)
        log = os.path.join(out, f"{m['id']}.log")
        try:
            rc, secs = run_cmd(root, key[0], key[1], log, int(os.environ.get("MUT_LIMIT", "1200")))
        finally:
            back = restore(saved)
        failed, ok, results, passed, nfail, ce = parse(log)
        hit = [x for x in m["expect"] if any(x in t for t in failed)]
        if rc == 124: v = "TIMEOUT"
        elif ce: v = "COMPILE-ERROR"
        elif missing: v = "NOT-IN-CONTROL(" + ",".join(missing) + ")"
        elif len(hit) == len(m["expect"]): v = "KILLED"
        elif hit: v = "PARTIAL(" + ",".join(x for x in m["expect"] if x not in hit) + ")"
        else: v = "SURVIVED"
        others = sorted(t for t in failed if not any(x in t for x in m["expect"]))
        line = (f"{m['id']}\t{v}\trc={rc}\tcontrol={cn}\tpassed={passed}\tfailed={nfail}\t{secs}s\trestored={back}"
                f"\tred={','.join(sorted(failed))[:400]}")
        print(line, flush=True); summary.write(line + "\n"); summary.flush()
        if not back: print("RESTORE FAILED: stopping", flush=True); sys.exit(3)
    st = subprocess.run("git status --porcelain --untracked-files=no", shell=True, cwd=root, capture_output=True, text=True).stdout
    print(f"done; tracked files changed after restore: {len([l for l in st.splitlines() if l.strip()])}", flush=True)

if __name__ == "__main__":
    main()
