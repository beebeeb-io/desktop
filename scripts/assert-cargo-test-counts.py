#!/usr/bin/env python3
"""Validate saved, unfiltered `cargo test` output, reporting each test binary.

Only src/main.rs (the entry-point shim) and doctests may be empty. The library
and every integration test binary must execute tests. Cargo's exit code is
required: successful earlier binaries must not conceal a later build failure.
"""

import argparse
from pathlib import Path
import re
import subprocess
import sys
import tempfile


ACCEPTANCE = "content_v2::storage_tests::slice1_acceptance_twenty_three_gib_cycles_and_three_gib_manifest"


def check(log, cargo_exit_code, acceptance=False):
    current = None
    announced = None
    binaries = 0
    total = 0
    library_seen = False
    acceptance_passes = 0
    cycles = []
    full_campaign = False
    for raw in log.splitlines():
        line = re.sub(r"\x1b\[[0-9;]*m", "", raw).strip()
        cycle = re.match(r"cycle=(\d+) allocated_db_peak=", line)
        if cycle:
            cycles.append(int(cycle.group(1)))
        full_campaign |= line.startswith("cycles=20 payload_per_worker=1073741824 final_validation=3221225472 ")
        if line == f"test {ACCEPTANCE} ... ok":
            acceptance_passes += 1
        if line.startswith(("Running ", "Doc-tests ")):
            if current is not None:
                raise ValueError(f"missing result for {current}")
            current = line.replace("\\", "/")
            announced = None
        elif re.fullmatch(r"running \d+ tests?", line):
            if current is None or announced is not None:
                raise ValueError("test count without a unique binary header")
            announced = int(line.split()[1])
        elif line.startswith("test result:"):
            match = re.fullmatch(
                r"test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; "
                r"(\d+) ignored; (\d+) measured; (\d+) filtered out; finished in .+",
                line,
            )
            if match is None or current is None or announced is None:
                raise ValueError("malformed or orphaned test result")
            result, passed, failed, ignored, measured, filtered = match.groups()
            passed, failed, ignored, measured, filtered = map(
                int, (passed, failed, ignored, measured, filtered)
            )
            executed = passed + failed
            print(f"{current}: {passed} passed; {failed} failed; {ignored} ignored; "
                  f"{filtered} filtered; {executed} executed")
            if result != "ok" or failed:
                raise ValueError(f"failed tests in {current}")
            if announced != executed + ignored + measured or (filtered and not acceptance):
                raise ValueError(f"inconsistent or filtered count in {current}")
            empty_allowed = current.startswith(("Running unittests src/main.rs (", "Doc-tests "))
            if executed == 0 and not empty_allowed:
                raise ValueError(f"zero executed tests in {current}")
            library_seen |= current.startswith("Running unittests src/lib.rs (")
            binaries += 1
            total += executed
            current = None
    if current is not None:
        raise ValueError(f"missing result for {current}")
    if not library_seen or total == 0:
        raise ValueError("missing library results or zero total executed tests")
    if cargo_exit_code != 0:
        raise ValueError(f"cargo exited {cargo_exit_code}")
    if acceptance and (binaries != 1 or total != 1 or acceptance_passes != 1):
        raise ValueError("acceptance requires exactly one executed named campaign")
    if acceptance and (cycles != list(range(1, 21)) or not full_campaign):
        raise ValueError("acceptance requires all 20 cycles and final 3 GiB validation")
    if not acceptance and acceptance_passes:
        raise ValueError("full acceptance campaign ran in default suite")
    print(f"Total: {binaries} harnesses; {total} executed tests; cargo exit 0")


def self_test():
    # Exercise the actual CLI in a throwaway directory, including its exit code.
    def fixture(header, passed, ignored=0, failed=0):
        result = "FAILED" if failed else "ok"
        return (f"{header}\nrunning {passed + ignored + failed} tests\n"
                f"test result: {result}. {passed} passed; {failed} failed; "
                f"{ignored} ignored; 0 measured; 0 filtered out; finished in 0.01s\n")

    lib = "Running unittests src/lib.rs (target/debug/deps/beebeeb_desktop_lib-123.exe)"
    main = "Running unittests src/main.rs (target/debug/deps/beebeeb_desktop-123.exe)"
    integration = "Running tests/keychain.rs (target/debug/deps/keychain-123.exe)"
    good = fixture(lib, 3) + fixture(main, 0) + fixture(integration, 2) + fixture("Doc-tests beebeeb_desktop_lib", 0)
    cases = [
        ("positive per-binary counts", good, 0, None),
        ("Windows CRLF, paths and ANSI", good.replace("/", "\\").replace("\n", "\r\n").replace("Running", "\x1b[32mRunning\x1b[0m"), 0, None),
        ("zero-test log", fixture(lib, 0), 0, "zero executed tests"),
        ("ignored-only library", fixture(lib, 0, ignored=3), 0, "zero executed tests"),
        ("empty integration after passing library", fixture(lib, 3) + fixture(integration, 0), 0, "zero executed tests"),
        ("empty log", "", 0, "missing library results"),
        ("summary without header", good.split("\n", 1)[1], 0, "test count without"),
        ("truncated next binary", fixture(lib, 3) + integration, 0, "missing result"),
        ("missing library", fixture(integration, 2), 0, "missing library results"),
        ("failed test", fixture(lib, 2, failed=1), 0, "failed tests"),
        ("nonzero cargo exit after passing tests", good, 101, "cargo exited 101"),
        ("inconsistent count", fixture(lib, 3).replace("running 3", "running 4"), 0, "inconsistent or filtered count"),
        ("filtered run", fixture(lib, 3).replace("0 filtered", "2 filtered"), 0, "inconsistent or filtered count"),
    ]
    acceptance_good = fixture(lib, 1).replace("0 filtered", "535 filtered") + f"test {ACCEPTANCE} ... ok\n"
    acceptance_good += "".join(f"cycle={n} allocated_db_peak=1\n" for n in range(1, 21))
    acceptance_good += "cycles=20 payload_per_worker=1073741824 final_validation=3221225472 peak_db=1\n"
    cases += [
        ("named acceptance", acceptance_good, 0, None),
        ("acceptance missing cycle", acceptance_good.replace("cycle=20 allocated_db_peak=1\n", ""), 0, "all 20 cycles"),
        ("acceptance wrong final size", acceptance_good.replace("final_validation=3221225472", "final_validation=1"), 0, "all 20 cycles"),
        ("acceptance empty", fixture(lib, 0), 0, "zero executed tests"),
        ("acceptance wrong name", fixture(lib, 1), 0, "exactly one executed named campaign"),
        ("acceptance extra test", acceptance_good.replace("running 1", "running 2").replace("1 passed", "2 passed"), 0, "exactly one executed named campaign"),
        ("acceptance nonzero exit", acceptance_good, 101, "cargo exited 101"),
        ("acceptance in default suite", fixture(lib, 1) + f"test {ACCEPTANCE} ... ok\n", 0, "full acceptance campaign ran"),
    ]
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "cargo-test.log"
        for name, log, cargo_exit, error in cases:
            path.write_text(log, encoding="utf-8")
            run = subprocess.run(
                [sys.executable, str(Path(__file__).resolve()), str(path),
                 "--cargo-exit-code", str(cargo_exit)] + (["--acceptance"] if name.startswith(("named acceptance", "acceptance ")) and name != "acceptance in default suite" else []), capture_output=True, text=True,
            )
            expected = 1 if error else 0
            if run.returncode != expected or (error and error not in run.stderr):
                raise AssertionError(f"{name}: expected exit {expected} / {error!r}, "
                                     f"got {run.returncode}\n{run.stdout}\n{run.stderr}")
            if error is None and ("1 harnesses; 1 executed tests" if name == "named acceptance" else "4 harnesses; 5 executed tests") not in run.stdout:
                raise AssertionError(f"{name}: per-binary aggregation failed: {run.stdout}")
            print(f"PASS {name}: guard exit {run.returncode}" + (f"; {run.stderr.strip()}" if error else ""))
    print(f"Self-test: {len(cases)} passed; 0 failed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", nargs="?", type=Path)
    parser.add_argument("--cargo-exit-code", type=int)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--acceptance", action="store_true", help="require exactly the named full storage campaign")
    args = parser.parse_args()
    if args.self_test:
        self_test()
    else:
        if args.log is None or args.cargo_exit_code is None:
            parser.error("log and --cargo-exit-code are required")
        try:
            check(args.log.read_text(encoding="utf-8"), args.cargo_exit_code, args.acceptance)
        except (ValueError, OSError) as error:
            print(f"FAIL: {error}", file=sys.stderr)
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
