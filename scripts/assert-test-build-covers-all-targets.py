#!/usr/bin/env python3
"""Fail when `cargo test --no-run` would leave a target of the crate uncompiled.

CI compiles the crate once, with `cargo test --locked --no-run`, instead of once with
`cargo check --all-targets` and once more for the tests. The test build compiles the library,
every binary, every example and every test target. A bench target is the one kind
`--all-targets` covers and the test build does not, so this guard reads
`cargo metadata --no-deps --format-version 1` and fails when it finds one (or a kind it does
not know). It prints the COUNT of targets per kind: zero targets is a failure, not a pass.

Usage:
  assert-test-build-covers-all-targets.py <cargo-metadata.json>
  assert-test-build-covers-all-targets.py --self-test
"""

import json
from pathlib import Path
import subprocess
import sys
import tempfile

# Kinds `cargo test --no-run` compiles (a build script runs as part of any build).
COVERED = {"lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro", "bin", "example", "test", "custom-build"}


def check(metadata):
    packages = metadata.get("packages") or []
    if not packages:
        raise ValueError("no package in the metadata")
    counts = {}
    for package in packages:
        for target in package.get("targets") or []:
            for kind in target.get("kind") or []:
                counts[kind] = counts.get(kind, 0) + 1
                if kind not in COVERED:
                    raise ValueError(
                        f"target `{target.get('name')}` of `{package.get('name')}` has kind `{kind}`: "
                        "`cargo test --no-run` does not compile it; add a step that does"
                    )
    total = sum(counts.values())
    if total == 0 or not any(kind in counts for kind in ("lib", "rlib", "bin")):
        raise ValueError("no library or binary target found")
    listing = ", ".join(f"{count} {kind}" for kind, count in sorted(counts.items()))
    print(f"Targets: {total} target kinds in {len(packages)} package(s) ({listing}); 0 left out by the test build")


def self_test():
    def metadata(*kinds):
        return {"packages": [{"name": "x", "targets": [{"name": f"t{n}", "kind": [k]} for n, k in enumerate(kinds)]}]}

    cases = [
        ("library, binary, tests", metadata("lib", "bin", "test", "test", "custom-build"), None),
        ("tauri library kinds", {"packages": [{"name": "x", "targets": [{"name": "l", "kind": ["staticlib", "cdylib", "rlib"]}]}]}, None),
        ("an example is compiled by the test build", metadata("lib", "example"), None),
        ("a bench target", metadata("lib", "bench"), "does not compile it"),
        ("an unknown kind", metadata("lib", "something-new"), "does not compile it"),
        ("no package", {"packages": []}, "no package"),
        ("no targets", metadata(), "no library or binary"),
        ("tests only", metadata("test"), "no library or binary"),
    ]
    failed = 0
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "metadata.json"
        for name, data, error in cases:
            path.write_text(json.dumps(data), encoding="utf-8")
            run = subprocess.run([sys.executable, str(Path(__file__).resolve()), str(path)], capture_output=True, text=True)
            expected = 1 if error else 0
            ok = run.returncode == expected and (error is None or error in run.stderr)
            failed += not ok
            print(f"{'PASS' if ok else 'FAIL'} {name}: guard exit {run.returncode}" + (f"; {run.stderr.strip()}" if error else ""))
    print(f"Self-test: {len(cases) - failed} passed; {failed} failed")
    return 1 if failed else 0


def main():
    if len(sys.argv) == 2 and sys.argv[1] == "--self-test":
        return self_test()
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    try:
        check(json.loads(Path(sys.argv[1]).read_text(encoding="utf-8")))
    except (ValueError, OSError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
