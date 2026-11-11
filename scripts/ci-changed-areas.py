#!/usr/bin/env python3
"""Decide which CI areas a change touches, from a list of changed paths.

Prints `rust=true|false` (one line per area). It fails OPEN: a path this script does not
positively know to be irrelevant to an area turns that area on. An empty list, an unreadable
list or any error also turns every area on, so a broken filter can only cost time, never
skip a suite.

`rust` is the Tauri engine suite (`src-tauri`). Its tests read files outside `src-tauri`
(source pins on `src/*.tsx`, `scripts/`, `BeebeebFileProvider/`, `tauri` configs), so only
the paths below are known NOT to reach it:

  docs/**, design/**   prose and design sources (no Rust test opens them)
  *.md, LICENSE*       at any depth outside src-tauri (src-tauri/** always counts)
  native/**            the native UI crates: their own jobs, not this suite
  .github/ISSUE_TEMPLATE/**, .github/*.md

Usage:
  ci-changed-areas.py <file-with-one-path-per-line>     # prints rust=...
  ci-changed-areas.py --self-test
"""

import sys
from pathlib import Path, PurePosixPath

# Directories whose every file is irrelevant to the src-tauri suite.
IRRELEVANT_DIRS = ("docs/", "design/", "native/", ".github/ISSUE_TEMPLATE/")
IRRELEVANT_SUFFIXES = (".md",)
IRRELEVANT_NAMES = ("LICENSE", "LICENSE.md", "LICENSE.txt", "NOTICE", ".gitattributes")


def rust_relevant(path):
    path = path.strip().replace("\\", "/")
    if not path:
        return False
    if path.startswith("src-tauri/"):
        return True
    if path.startswith(IRRELEVANT_DIRS):
        return False
    if PurePosixPath(path).name in IRRELEVANT_NAMES:
        return False
    if path.endswith(IRRELEVANT_SUFFIXES):
        return False
    return True


def areas(paths):
    paths = [p for p in (line.strip() for line in paths) if p]
    if not paths:
        # No list is not "nothing changed": run everything.
        return {"rust": True, "paths": 0, "rust_paths": 0}
    hits = [p for p in paths if rust_relevant(p)]
    return {"rust": bool(hits), "paths": len(paths), "rust_paths": len(hits)}


def self_test():
    cases = [
        ("docs only", ["docs/ci/a.md", "docs/specs/x.png"], False),
        ("root markdown only", ["README.md", "RELEASE_NOTES.md"], False),
        ("design only", ["design/hifi/macos-two-surface.html"], False),
        ("native crate only", ["native/beebeeb-drive-rune/src/main.rs", "native/Cargo.lock"], False),
        ("rust source", ["src-tauri/src/lib.rs"], True),
        ("markdown inside src-tauri still counts", ["src-tauri/MAP.md"], True),
        ("docs plus one rust file", ["docs/a.md", "src-tauri/Cargo.toml"], True),
        ("frontend source (Rust tests pin TSX)", ["src/main.tsx"], True),
        ("workflow", [".github/workflows/ci.yml"], True),
        ("composite action", [".github/actions/rust-suite/action.yml"], True),
        ("toolchain", ["rust-toolchain.toml"], True),
        ("scripts", ["scripts/assert-cargo-test-counts.py"], True),
        ("file provider swift", ["BeebeebFileProvider/XPCBridge.swift"], True),
        ("unknown top-level file", ["something-new.toml"], True),
        ("empty list fails open", [], True),
        ("blank lines only fail open", ["", "  "], True),
        ("windows separators", ["docs\\a.md"], False),
        ("a directory merely named like docs", ["docs-tools/build.rs"], True),
    ]
    failed = 0
    for name, paths, expected in cases:
        got = areas(paths)["rust"]
        ok = got == expected
        failed += not ok
        print(f"{'PASS' if ok else 'FAIL'} {name}: rust={str(got).lower()}")
    print(f"Self-test: {len(cases) - failed} passed; {failed} failed")
    return 1 if failed else 0


def main():
    if len(sys.argv) == 2 and sys.argv[1] == "--self-test":
        return self_test()
    try:
        result = areas(Path(sys.argv[1]).read_text(encoding="utf-8").splitlines())
    except (IndexError, OSError, UnicodeError) as error:
        print(f"ci-changed-areas: {error}; running every area", file=sys.stderr)
        result = {"rust": True, "paths": 0, "rust_paths": 0}
    print(f"changed paths: {result['paths']}; reaching the Rust suite: {result['rust_paths']}", file=sys.stderr)
    print(f"rust={'true' if result['rust'] else 'false'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
