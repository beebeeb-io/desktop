#!/usr/bin/env python3
"""Validate saved, unfiltered `cargo test` output, reporting each test binary.

Only src/main.rs (the entry-point shim) and doctests may be empty. The library
and every integration test binary must execute tests. Cargo's exit code is
required: successful earlier binaries must not conceal a later build failure.

A shrinking suite fails too. `scripts/cargo-test-floors.txt` (or `--floors`) gives a floor per test binary and
OS: a binary named there that executes fewer tests than its floor fails the run. `--expect <header part>` names a
binary that must be in the log at all (a crate with no library, such as the native app, has no other anchor).
"""

import argparse
import platform
from pathlib import Path
import re
import subprocess
import sys
import tempfile


def this_os():
    return {"Linux": "linux", "Windows": "windows", "Darwin": "macos"}.get(platform.system(), "other")


def read_floors(path, os_name):
    """`<os> <floor> <part of the binary's header>` per line; `#` starts a comment. Only this OS's lines count."""
    floors = []
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        parts = line.split(None, 2)
        if len(parts) != 3 or parts[0] not in ("linux", "windows", "macos") or not parts[1].isdigit():
            raise ValueError(f"{path.name}:{number}: expected '<linux|windows|macos> <floor> <header part>'")
        if parts[0] == os_name:
            floors.append((parts[2], int(parts[1])))
    return floors


def check(log, cargo_exit_code, floors=(), expected=(), retried_ok=False):
    seen_expected = set()
    current = None
    announced = None
    binaries = 0
    total = 0
    library_seen = False
    for raw in log.splitlines():
        line = re.sub(r"\x1b\[[0-9;]*m", "", raw).strip()
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
                if not retried_ok:
                    raise ValueError(f"failed tests in {current}")
                print(f"  (retried-ok: {failed} failure(s) accepted — retried and passed)")
            if announced != executed + ignored + measured or filtered:
                raise ValueError(f"inconsistent or filtered count in {current}")
            named = [part for part in expected if part in current]
            seen_expected.update(named)
            empty_allowed = current.startswith(("Running unittests src/main.rs (", "Doc-tests ")) and not named
            if executed == 0 and not empty_allowed:
                raise ValueError(f"zero executed tests in {current}")
            for part, floor in floors:
                if part in current and executed < floor:
                    raise ValueError(f"{executed} executed tests in {current}: below the floor of {floor} for '{part}'")
            library_seen |= current.startswith("Running unittests src/lib.rs (")
            binaries += 1
            total += executed
            current = None
    if current is not None:
        raise ValueError(f"missing result for {current}")
    missing = [part for part in expected if part not in seen_expected]
    if missing:
        raise ValueError(f"expected test binary not in the log: {', '.join(missing)}")
    # A crate with no library names its binaries with --expect instead.
    if (not library_seen and not expected) or total == 0:
        raise ValueError("missing library results or zero total executed tests")
    if cargo_exit_code != 0:
        if not retried_ok:
            raise ValueError(f"cargo exited {cargo_exit_code}")
        print(f"  (retried-ok: cargo exit {cargo_exit_code} accepted — retried tests passed)")
    print(f"Total: {binaries} harnesses; {total} executed tests; cargo exit {cargo_exit_code}"
          + (" (retried-ok)" if retried_ok else ""))


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
    floor_cases = [
        # (name, log, floors file text, --expect values, expected error)
        ("library at its floor", good, "linux 3 deps/beebeeb_desktop_lib-\n", [], None),
        ("library one below its floor", good, "linux 4 deps/beebeeb_desktop_lib-\n", [], "below the floor of 4"),
        ("a 300-test drop", fixture(lib, 2040), "linux 2300 deps/beebeeb_desktop_lib-\n", [], "below the floor of 2300"),
        ("another OS's floor is not this one's", good, "windows 400 deps/beebeeb_desktop_lib-\n", [], None),
        ("malformed floors file", good, "linux many deps/x\n", [], "expected '<linux|windows|macos> <floor> <header part>'"),
        ("expected binary present", good, "", ["src/lib.rs"], None),
        ("expected binary missing", good, "", ["deps/beebeeb_engine-"], "expected test binary not in the log"),
        ("binary-only crate, expected and run", fixture(main, 9), "", ["src/main.rs"], None),
        ("binary-only crate that ran nothing", fixture(main, 0), "", ["src/main.rs"], "zero executed tests"),
        ("binary-only crate without --expect", fixture(main, 9), "", [], "missing library results"),
    ]
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
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "cargo-test.log"
        # The repo's own floors are for real binaries; these fixtures run 3 tests under the same names.
        no_floors = Path(directory) / "no-floors.txt"
        no_floors.write_text("", encoding="utf-8")
        for name, log, cargo_exit, error in cases:
            path.write_text(log, encoding="utf-8")
            run = subprocess.run(
                [sys.executable, str(Path(__file__).resolve()), str(path),
                 "--cargo-exit-code", str(cargo_exit), "--floors", str(no_floors)], capture_output=True, text=True,
            )
            expected = 1 if error else 0
            if run.returncode != expected or (error and error not in run.stderr):
                raise AssertionError(f"{name}: expected exit {expected} / {error!r}, "
                                     f"got {run.returncode}\n{run.stdout}\n{run.stderr}")
            if error is None and "4 harnesses; 5 executed tests" not in run.stdout:
                raise AssertionError(f"{name}: per-binary aggregation failed: {run.stdout}")
            print(f"PASS {name}: guard exit {run.returncode}" + (f"; {run.stderr.strip()}" if error else ""))
        floors_path = Path(directory) / "floors.txt"
        for name, log, floors, expects, error in floor_cases:
            path.write_text(log, encoding="utf-8")
            floors_path.write_text(floors, encoding="utf-8")
            command = [sys.executable, str(Path(__file__).resolve()), str(path), "--cargo-exit-code", "0",
                       "--floors", str(floors_path), "--os", "linux"]
            for part in expects:
                command += ["--expect", part]
            run = subprocess.run(command, capture_output=True, text=True)
            expected = 1 if error else 0
            if run.returncode != expected or (error and error not in run.stderr):
                raise AssertionError(f"{name}: expected exit {expected} / {error!r}, "
                                     f"got {run.returncode}\n{run.stdout}\n{run.stderr}")
            print(f"PASS {name}: guard exit {run.returncode}" + (f"; {run.stderr.strip()}" if error else ""))
    print(f"Self-test: {len(cases) + len(floor_cases)} passed; 0 failed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", nargs="?", type=Path)
    parser.add_argument("--cargo-exit-code", type=int)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--floors", type=Path, default=Path(__file__).resolve().parent / "cargo-test-floors.txt",
                        help="floors per test binary and OS (default: cargo-test-floors.txt beside this script)")
    parser.add_argument("--os", default=this_os(), help="whose floors apply (default: this machine's)")
    parser.add_argument("--expect", action="append", default=[], metavar="HEADER_PART",
                        help="a test binary that must be in the log, by a part of its 'Running ...' header")
    parser.add_argument("--retried-ok", action="store_true",
                        help="accept failures and a non-zero cargo exit (task 2448: the CI retry script already "
                             "confirmed they pass); the binary, floor and count checks still apply")
    args = parser.parse_args()
    if args.self_test:
        self_test()
    else:
        if args.log is None or args.cargo_exit_code is None:
            parser.error("log and --cargo-exit-code are required")
        try:
            floors = read_floors(args.floors, args.os) if args.floors.exists() else []
            check(args.log.read_text(encoding="utf-8"), args.cargo_exit_code, floors, args.expect,
                  retried_ok=args.retried_ok)
        except (ValueError, OSError) as error:
            print(f"FAIL: {error}", file=sys.stderr)
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
