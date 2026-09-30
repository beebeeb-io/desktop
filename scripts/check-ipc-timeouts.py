#!/usr/bin/env python3
"""Source-level guard: XPCBridge.swift call sites keep their long IPC timeouts.

Task 1670 issue 3. XPCBridge.swift is NOT compiled into the swiftc test harness
(BeebeebFileProviderTests), so reverting a call site to the default 30 s
timeout leaves the Swift suite green while a large Finder copy would time out
again. The harness only proves the IPCFraming helpers; this script proves the
call sites still use them:

  QueueFinderCreate / QueueFinderModify -> timeoutSeconds:
      IPCFraming.writeQueueTimeoutSeconds(hasContents: contentsURL != nil)
  HydrateFile                           -> timeoutSeconds:
      IPCFraming.hydrateTimeoutSeconds

Each op must appear in exactly one sendRequest call (a renamed/duplicated call
site is RED, not silently unchecked).

Truth line: `ipc-timeout guard: N/N call sites correct` and exit 0. Anything
else, or a non-zero exit, is RED.

Usage:
  scripts/check-ipc-timeouts.py [path/to/XPCBridge.swift]
  scripts/check-ipc-timeouts.py --self-test   # mutates in-memory copies; each must go RED
"""
import re
import sys
from pathlib import Path

DEFAULT = Path(__file__).resolve().parents[1] / "BeebeebFileProvider" / "XPCBridge.swift"

EXPECTED = {
    "QueueFinderCreate": "IPCFraming.writeQueueTimeoutSeconds(hasContents: contentsURL != nil)",
    "QueueFinderModify": "IPCFraming.writeQueueTimeoutSeconds(hasContents: contentsURL != nil)",
    "HydrateFile": "IPCFraming.hydrateTimeoutSeconds",
}


def call_spans(src):
    """Yield the argument text of every `sendRequest(...)` call (not the func decl)."""
    for m in re.finditer(r"\bsendRequest\(", src):
        if re.search(r"\bfunc\s+$", src[: m.start()]):
            continue
        depth, i, in_str = 1, m.end(), False
        while i < len(src) and depth:
            c = src[i]
            if in_str:
                if c == "\\":
                    i += 1
                elif c == '"':
                    in_str = False
            elif c == '"':
                in_str = True
            elif c == "(":
                depth += 1
            elif c == ")":
                depth -= 1
            i += 1
        yield src[m.end() : i - 1]


def check(src):
    """Return (ok_count, problems)."""
    seen = {op: [] for op in EXPECTED}
    for args in call_spans(src):
        op = re.search(r'\[\s*"(\w+)"\s*:', args)
        if op and op.group(1) in EXPECTED:
            seen[op.group(1)].append(args)
    problems, ok = [], 0
    for op, want in EXPECTED.items():
        calls = seen[op]
        if len(calls) != 1:
            problems.append(f"{op}: expected exactly 1 sendRequest call, found {len(calls)}")
            continue
        got = re.search(r"timeoutSeconds:\s*(.+?)\s*(?:,\s*\w+:|$)", calls[0], re.S)
        got = got.group(1).strip() if got else None
        if got != want:
            problems.append(f"{op}: timeoutSeconds is {got!r}, expected {want!r}")
        else:
            ok += 1
    return ok, problems


def report(src):
    ok, problems = check(src)
    for p in problems:
        print(f"FAIL {p}", file=sys.stderr)
    if problems:
        print(f"ipc-timeout guard: {ok}/{len(EXPECTED)} call sites correct (RED)", file=sys.stderr)
        return 1
    print(f"ipc-timeout guard: {ok}/{len(EXPECTED)} call sites correct")
    return 0


def self_test(src):
    if check(src)[1]:
        print("self-test: the real source is already RED", file=sys.stderr)
        return 1
    w = "timeoutSeconds: IPCFraming.writeQueueTimeoutSeconds(hasContents: contentsURL != nil)"
    h = "timeoutSeconds: IPCFraming.hydrateTimeoutSeconds"
    assert src.count(w) == 2 and src.count(h) == 1, "self-test: expected call-site text not found"

    def first(s, old, new):
        return s.replace(old, new, 1)

    mutants = {
        "Create call site reverted to the default timeout": first(
            src, '["QueueFinderCreate": payload],\n            ' + w, '["QueueFinderCreate": payload]'),
        "Modify call site reverted to the default timeout": first(
            src, '["QueueFinderModify": payload],\n            ' + w, '["QueueFinderModify": payload]'),
        "Hydrate call site reverted to the default timeout": first(
            src, '["HydrateFile": payload],\n            ' + h + ",\n", '["HydrateFile": payload],\n'),
        "Modify hasContents hard-coded false": src[::-1].replace(
            w[::-1], w.replace("contentsURL != nil", "false")[::-1], 1)[::-1],
        "Hydrate uses the metadata timeout": first(src, h, "timeoutSeconds: IPCFraming.metadataTimeoutSeconds"),
        "Create call site renamed (unchecked)": first(src, '"QueueFinderCreate"', '"QueueFinderCreateV2"'),
    }
    bad = 0
    for name, mutated in mutants.items():
        if mutated == src:
            print(f"self-test: mutation did not change the source: {name}", file=sys.stderr)
            bad += 1
        elif not check(mutated)[1]:
            print(f"self-test: ACCEPTED a bad source: {name}", file=sys.stderr)
            bad += 1
    if bad:
        return 1
    print(f"self-test OK: the guard rejected {len(mutants)} mutated sources and accepted the real one")
    return 0


if __name__ == "__main__":
    args = sys.argv[1:]
    if args[:1] == ["--self-test"]:
        sys.exit(self_test(DEFAULT.read_text()))
    sys.exit(report(Path(args[0] if args else DEFAULT).read_text()))
