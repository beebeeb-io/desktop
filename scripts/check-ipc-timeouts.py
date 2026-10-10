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

Task 1684 adds the write-queue idempotency key. The key must be STABLE across
the system's retries (a random UUID per call dedups nothing), so it is derived
inside IPCWriteRequest / IPCWriteKey (IPCFraming.swift, unit-tested by
BeebeebFileProviderTests). What the Swift tests cannot see is whether
XPCBridge.swift still goes through that builder, so this script also requires,
inside queueCreateItem / queueModifyItem:

  * exactly one IPCWriteRequest.create( / .modify( call, given the staged file's
    path AND its fingerprint (`IPCContentFingerprint.ofFile(at:)`), and for a
    modify the changed-fields mask -- the inputs the key is derived from;
  * the request built from it is what sendRequest sends;
  * no hand-built `"QueueFinderCreate"` / `"QueueFinderModify"` dictionary
    anywhere in the file, no `"request_id"` literal and no `UUID()` in those
    functions (each would bypass or randomise the key).

The app cannot read the system's contents URL (it is outside the app's
sandbox), so the path the request carries must be the App Group copy, not the
system's URL. Inside each write function the guard also requires:

  * exactly one `let stagedContents = try stageUploadContents(contentsURL,
    kind: kind)` and `contentsPath: stagedContents?.path` in the builder call
    (the key's fingerprint still comes from `contentsURL`, the system's file);
  * a `defer` that discards the copy (`UploadStaging.discard(stagedContents)`),
    so it never outlives the exchange.

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

# Write-queue call sites: function -> (op, builder, argument patterns the builder call must contain).
FINGERPRINT_ARG = r"contents:\s*contentsURL\.flatMap\s*\{\s*IPCContentFingerprint\.ofFile\(at:\s*\$0\)\s*\}"
PATH_ARG = r"contentsPath:\s*stagedContents\?\.path"
STAGE_CALL = r"let\s+stagedContents\s*=\s*try\s+stageUploadContents\(\s*contentsURL\s*,\s*kind:\s*kind\s*\)"
DISCARD = r"defer\s*\{\s*if\s+let\s+stagedContents\s*\{\s*UploadStaging\.discard\(stagedContents\)\s*\}\s*\}"
WRITE_SITES = {
    "queueCreateItem": ("QueueFinderCreate", "IPCWriteRequest.create", [PATH_ARG, FINGERPRINT_ARG]),
    "queueModifyItem": (
        "QueueFinderModify",
        "IPCWriteRequest.modify",
        [PATH_ARG, FINGERPRINT_ARG, r"changedFields:\s*UInt64\(truncatingIfNeeded:\s*changedFields\.rawValue\)"],
    ),
}


def matching_close(src, open_idx, open_ch, close_ch):
    """Index just past the delimiter closing the one at `open_idx` (strings skipped)."""
    depth, i, in_str = 0, open_idx, False
    while i < len(src):
        c = src[i]
        if in_str:
            if c == "\\":
                i += 1
            elif c == '"':
                in_str = False
        elif c == '"':
            in_str = True
        elif c == open_ch:
            depth += 1
        elif c == close_ch:
            depth -= 1
            if depth == 0:
                return i + 1
        i += 1
    return None


def function_body(src, name):
    """Text of `func <name>(...) ... { body }`, or None."""
    m = re.search(r"\bfunc\s+" + re.escape(name) + r"\(", src)
    if not m:
        return None
    params_end = matching_close(src, m.end() - 1, "(", ")")
    if params_end is None:
        return None
    brace = src.find("{", params_end)
    if brace < 0:
        return None
    end = matching_close(src, brace, "{", "}")
    return src[brace:end] if end else None


def call_spans(src):
    """Yield the argument text of every `sendRequest(...)` call (not the func decl)."""
    for m in re.finditer(r"\bsendRequest\(", src):
        if re.search(r"\bfunc\s+$", src[: m.start()]):
            continue
        end = matching_close(src, m.end() - 1, "(", ")")
        if end is None:
            continue
        yield src[m.end() : end - 1]


def timeout_of(args):
    got = re.search(r"timeoutSeconds:\s*(.+?)\s*(?:,\s*\w+:|$)", args, re.S)
    return got.group(1).strip() if got else None


def check(src):
    """Return (ok_count, problems)."""
    problems, ok = [], 0
    ok_ops = set()

    # Hydrate: the literal-dictionary call site, as before.
    hydrate = []
    for args in call_spans(src):
        op = re.search(r'\[\s*"(\w+)"\s*:', args)
        if op and op.group(1) == "HydrateFile":
            hydrate.append(args)
    if len(hydrate) != 1:
        problems.append(f"HydrateFile: expected exactly 1 sendRequest call, found {len(hydrate)}")
    else:
        got = timeout_of(hydrate[0])
        if got != EXPECTED["HydrateFile"]:
            problems.append(f"HydrateFile: timeoutSeconds is {got!r}, expected {EXPECTED['HydrateFile']!r}")
        else:
            ok_ops.add("HydrateFile")

    # Write-queue sites: built by IPCWriteRequest, sent with the long timeout.
    for func, (op, builder, arg_patterns) in WRITE_SITES.items():
        before = len(problems)
        body = function_body(src, func)
        if body is None:
            problems.append(f"{op}: func {func} not found")
            continue
        builds = re.findall(re.escape(builder) + r"\(", body)
        if len(builds) != 1:
            problems.append(f"{op}: {func} must build its request with exactly one {builder}( call, found {len(builds)}")
        else:
            build_args = body[body.index(builder + "(") :]
            end = matching_close(build_args, build_args.index("("), "(", ")")
            build_args = build_args[:end] if end else build_args
            for pattern in arg_patterns:
                if not re.search(pattern, build_args):
                    problems.append(f"{op}: the {builder}( call no longer passes {pattern!r} (the request_id inputs)")
        if len(re.findall(STAGE_CALL, body)) != 1:
            problems.append(f"{op}: {func} must stage the contents for the app exactly once ({STAGE_CALL!r})")
        if len(re.findall(DISCARD, body)) != 1:
            problems.append(f"{op}: {func} must discard the staged copy in a defer ({DISCARD!r})")
        if re.search(r"\bUUID\(\)", body):
            problems.append(f"{op}: {func} uses UUID(); a random id per call cannot dedup a retry")
        if '"request_id"' in body:
            problems.append(f"{op}: {func} sets request_id by hand; it must come from {builder}")
        sends = list(call_spans(body))
        if len(sends) != 1:
            problems.append(f"{op}: {func} must contain exactly 1 sendRequest call, found {len(sends)}")
        else:
            if not re.match(r"\s*request\s*,", sends[0]):
                problems.append(f"{op}: {func} must send the request built by {builder}, found {sends[0].split(',')[0].strip()!r}")
            got = timeout_of(sends[0])
            if got != EXPECTED[op]:
                problems.append(f"{op}: timeoutSeconds is {got!r}, expected {EXPECTED[op]!r}")
        if len(problems) == before:
            ok_ops.add(op)

    # A hand-built request dictionary anywhere bypasses the builder (and its key).
    for op in ("QueueFinderCreate", "QueueFinderModify"):
        if re.search(r'\[\s*"' + op + r'"\s*:', src):
            problems.append(f"{op}: hand-built request dictionary found; requests must come from IPCWriteRequest")

    ok = len(ok_ops)
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
    fp = "contents: contentsURL.flatMap { IPCContentFingerprint.ofFile(at: $0) }"
    assert src.count(w) == 2 and src.count(h) == 1, "self-test: expected call-site text not found"
    assert src.count(fp) == 2, "self-test: expected fingerprint argument not found"
    staged_path = "contentsPath: stagedContents?.path"
    stage = "let stagedContents = try stageUploadContents(contentsURL, kind: kind)"
    discard = "                UploadStaging.discard(stagedContents)\n"
    assert src.count(staged_path) == 2 and src.count(stage) == 2 and src.count(discard) == 2, \
        "self-test: expected staging text not found"
    assert src.count("IPCWriteRequest.create(") == 1 and src.count("IPCWriteRequest.modify(") == 1
    # The create and modify send lines the request_id mutations anchor on. Asserted like every
    # anchor above, so a reshaped call site fails here, loudly, instead of leaving a mutation
    # that changes nothing (or only half of what it means to change).
    send = "        return try Self.decodeWriteResponse(sendRequest(\n            request,"
    assert src.count(send) == 2, "self-test: expected write send text not found"
    let_create = "        let request = IPCWriteRequest.create("
    assert src.count(let_create) == 1, "self-test: expected create builder text not found"

    def first(s, old, new):
        return s.replace(old, new, 1)

    def last(s, old, new):
        return s[::-1].replace(old[::-1], new[::-1], 1)[::-1]

    mutants = {
        "Create call site reverted to the default timeout": first(src, "            request,\n            " + w, "            request"),
        "Modify call site reverted to the default timeout": last(src, ",\n            " + w, ""),
        "Hydrate call site reverted to the default timeout": first(
            src, '["HydrateFile": payload],\n            ' + h + ",\n", '["HydrateFile": payload],\n'),
        "Modify hasContents hard-coded false": last(src, w, w.replace("contentsURL != nil", "false")),
        "Hydrate uses the metadata timeout": first(src, h, "timeoutSeconds: IPCFraming.metadataTimeoutSeconds"),
        "Create call site renamed (unchecked)": first(src, "func queueCreateItem(", "func queueCreateItemV2("),
        "Create no longer passes the staged file fingerprint": first(src, fp, "contents: nil"),
        "Modify no longer passes the staged file fingerprint": last(src, fp, "contents: nil"),
        "Modify no longer passes the changed-fields mask": first(
            src, "changedFields: UInt64(truncatingIfNeeded: changedFields.rawValue)", "changedFields: 0"),
        "Create request_id replaced by a random UUID": first(
            first(src, let_create, let_create.replace("let request", "var request")),
            send, "        _ = UUID().uuidString\n" + send),
        "Create request_id set by hand": first(
            first(src, let_create, let_create.replace("let request", "var request")),
            send, '        request["request_id"] = "x"\n' + send),
        "Create request hand-built instead of via IPCWriteRequest": first(
            src, "IPCWriteRequest.create(", '["QueueFinderCreate": payload] ?? IPCWriteRequest.create('),
        "Modify builder call removed": first(src, "IPCWriteRequest.modify(", "IPCWriteRequestModifyRemoved("),
        "Modify sends something other than the built request": last(
            src, "sendRequest(\n            request,", "sendRequest(\n            [:],"),
        "Create sends the system's contents URL instead of the App Group copy": first(
            src, staged_path, "contentsPath: contentsURL?.path"),
        "Modify sends the system's contents URL instead of the App Group copy": last(
            src, staged_path, "contentsPath: contentsURL?.path"),
        "Create no longer stages the contents": first(src, stage, "let stagedContents: URL? = contentsURL"),
        "Modify no longer stages the contents": last(src, stage, "let stagedContents: URL? = contentsURL"),
        "Create no longer discards the staged copy": first(src, discard, ""),
        "Modify no longer discards the staged copy": last(src, discard, ""),
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
