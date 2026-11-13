#!/usr/bin/env python3
"""Decide which CI areas a change touches, from a list of changed paths.

Prints `rust=true|false` and `macos=true|false` (one line per area). It fails OPEN: a path
this script does not positively know to be irrelevant to an area turns that area on. An empty
list, an unreadable list or any error also turns every area on, so a broken filter can only cost
time, never skip a suite.

`rust` is the Tauri engine suite (`src-tauri`). Its tests read files outside `src-tauri`
(`scripts/`, `BeebeebFileProvider/`, `tauri` configs, `crates/` fixtures), so only the
paths below are known NOT to reach it:

  docs/**, design/**   prose and design sources (no Rust test opens them)
  *.md, LICENSE*       at any depth outside src-tauri (src-tauri/** always counts)
  native/**            the native UI crates: their own jobs, not this suite
  .github/ISSUE_TEMPLATE/**, .github/*.md

`macos` is the macOS-specific suite: `Rust check (macOS)`, `File Provider Swift (macOS)` and
`Engine crate test (macOS)`. It shares the same irrelevant set as `rust`: a path that is
irrelevant to the Rust suite also cannot behave differently on macOS. No macOS job builds or
tests the native Rune crate (`native/**`), which has its own Linux and Windows jobs.

The React sources (`src/`) were removed for 0.9.0 (native app, no web view).

Usage:
  ci-changed-areas.py <file-with-one-path-per-line>     # prints rust=... macos=...
  ci-changed-areas.py --self-test
"""

import sys
from pathlib import Path, PurePosixPath

# Directories whose every file is irrelevant to both the src-tauri suite and the macOS suite.
# No macOS CI job builds or tests native/beebeeb-drive-rune (that crate has its own Linux and
# Windows jobs), so `native/` is irrelevant to both areas.
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
        return {"rust": True, "macos": True, "paths": 0, "relevant_paths": 0}
    hits = [p for p in paths if rust_relevant(p)]
    active = bool(hits)
    return {"rust": active, "macos": active, "paths": len(paths), "relevant_paths": len(hits)}


def self_test():
    # Each case is (name, paths, expected_rust, expected_macos). Today the two areas share a
    # single irrelevant set so their expected values always match, but they are separate
    # outputs with separate test columns: a future refinement that narrows one area must add a
    # red case here first.
    cases = [
        ("docs only", ["docs/ci/a.md", "docs/specs/x.png"], False, False),
        ("root markdown only", ["README.md", "RELEASE_NOTES.md"], False, False),
        ("design only", ["design/hifi/macos-two-surface.html"], False, False),
        ("native crate only", ["native/beebeeb-drive-rune/src/main.rs", "native/Cargo.lock"], False, False),
        ("native goldens and scripts", ["native/beebeeb-drive-rune/goldens/DW-0001.png", "native/beebeeb-drive-rune/scripts/capture.sh"], False, False),
        ("native docs", ["native/beebeeb-drive-rune/docs/frame-lanes.md"], False, False),
        ("rust source", ["src-tauri/src/lib.rs"], True, True),
        ("markdown inside src-tauri still counts", ["src-tauri/MAP.md"], True, True),
        ("docs plus one rust file", ["docs/a.md", "src-tauri/Cargo.toml"], True, True),
        ("removed frontend source (fails open)", ["src/main.tsx"], True, True),
        ("workflow", [".github/workflows/ci.yml"], True, True),
        ("composite action", [".github/actions/rust-suite/action.yml"], True, True),
        ("toolchain", ["rust-toolchain.toml"], True, True),
        ("scripts", ["scripts/assert-cargo-test-counts.py"], True, True),
        ("file provider swift", ["BeebeebFileProvider/XPCBridge.swift"], True, True),
        ("engine crate", ["crates/beebeeb-engine/src/lib.rs"], True, True),
        ("mixed native plus crates", ["native/beebeeb-drive-rune/src/main.rs", "crates/beebeeb-engine/Cargo.toml"], True, True),
        ("unknown top-level file", ["something-new.toml"], True, True),
        ("empty list fails open", [], True, True),
        ("blank lines only fail open", ["", "  "], True, True),
        ("windows separators", ["docs\\a.md"], False, False),
        ("a directory merely named like docs", ["docs-tools/build.rs"], True, True),
    ]
    failed = 0
    for name, paths, expect_rust, expect_macos in cases:
        result = areas(paths)
        got_rust = result["rust"]
        got_macos = result["macos"]
        rust_ok = got_rust == expect_rust
        macos_ok = got_macos == expect_macos
        ok = rust_ok and macos_ok
        failed += not ok
        tag = "PASS" if ok else "FAIL"
        print(f"{tag} {name}: rust={str(got_rust).lower()} macos={str(got_macos).lower()}")
    print(f"Self-test: {len(cases) - failed} passed; {failed} failed")
    return 1 if failed else 0


def main():
    if len(sys.argv) == 2 and sys.argv[1] == "--self-test":
        return self_test()
    try:
        result = areas(Path(sys.argv[1]).read_text(encoding="utf-8").splitlines())
    except (IndexError, OSError, UnicodeError) as error:
        print(f"ci-changed-areas: {error}; running every area", file=sys.stderr)
        result = {"rust": True, "macos": True, "paths": 0, "relevant_paths": 0}
    print(f"changed paths: {result['paths']}; relevant: {result['relevant_paths']}", file=sys.stderr)
    print(f"rust={'true' if result['rust'] else 'false'}")
    print(f"macos={'true' if result['macos'] else 'false'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
