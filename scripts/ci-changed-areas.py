#!/usr/bin/env python3
"""Decide which CI areas a change touches, from a list of changed paths.

Prints `macos=true|false`. It fails OPEN: a path this script does not positively know to be
irrelevant turns the area on. An empty list, an unreadable list or any error also turns it on, so
a broken filter can only cost time, never skip a suite.

`macos` is the macOS-specific suite: `File Provider Swift (macOS)` and `Engine crate test (macOS)`.
Only the paths below are known NOT to reach it:

  docs/**, design/**   prose and design sources (no test opens them)
  *.md, LICENSE*       at any depth
  native/**            the native Rune app: its own Linux and Windows jobs; no macOS job builds or
                       tests it. One exception: native/beebeeb-drive-rune/Cargo.toml counts, because
                       the File Provider build reads the extension's version from it
  .github/ISSUE_TEMPLATE/**, .github/*.md

There is no Tauri suite any more (the Tauri app went in 0.9.0, decision 2537), so there is no
`rust` area: every other Rust job runs on every change.

Usage:
  ci-changed-areas.py <file-with-one-path-per-line>     # prints macos=...
  ci-changed-areas.py --self-test
"""

import sys
from pathlib import Path, PurePosixPath

# Directories whose every file is irrelevant to the macOS suite. No macOS CI job builds or tests
# native/beebeeb-drive-rune (that crate has its own Linux and Windows jobs), so `native/` is
# irrelevant to it.
IRRELEVANT_DIRS = ("docs/", "design/", "native/", ".github/ISSUE_TEMPLATE/")
IRRELEVANT_SUFFIXES = (".md",)
IRRELEVANT_NAMES = ("LICENSE", "LICENSE.md", "LICENSE.txt", "NOTICE", ".gitattributes")
# Files under an irrelevant directory that a macOS job reads: scripts/build-fileprovider-extension.sh
# takes the extension's version from the native app's manifest.
RELEVANT_FILES = ("native/beebeeb-drive-rune/Cargo.toml",)


def macos_relevant(path):
    path = path.strip().replace("\\", "/")
    if not path:
        return False
    if path in RELEVANT_FILES:
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
        return {"macos": True, "paths": 0, "relevant_paths": 0}
    hits = [p for p in paths if macos_relevant(p)]
    return {"macos": bool(hits), "paths": len(paths), "relevant_paths": len(hits)}


def self_test():
    # Each case is (name, paths, expected_macos).
    cases = [
        ("docs only", ["docs/ci/a.md", "docs/specs/x.png"], False),
        ("root markdown only", ["README.md", "RELEASE_NOTES.md"], False),
        ("design only", ["design/hifi/macos-two-surface.html"], False),
        ("native crate only", ["native/beebeeb-drive-rune/src/main.rs", "native/Cargo.lock"], False),
        ("native goldens and scripts", ["native/beebeeb-drive-rune/goldens/DW-0001.png", "native/beebeeb-drive-rune/scripts/capture.sh"], False),
        ("native docs", ["native/beebeeb-drive-rune/docs/frame-lanes.md"], False),
        ("native app manifest (the File Provider build reads its version)", ["native/beebeeb-drive-rune/Cargo.toml"], True),
        ("native app manifest, Windows separators", ["native\\beebeeb-drive-rune\\Cargo.toml"], True),
        ("markdown inside a crate", ["crates/beebeeb-engine/README.md"], False),
        ("docs plus one engine file", ["docs/a.md", "crates/beebeeb-engine/Cargo.toml"], True),
        ("removed frontend source (fails open)", ["src/main.tsx"], True),
        ("removed Tauri path (fails open)", ["windows/src/main.rs"], True),
        ("workflow", [".github/workflows/ci.yml"], True),
        ("toolchain", ["rust-toolchain.toml"], True),
        ("scripts", ["scripts/assert-cargo-test-counts.py"], True),
        ("file provider swift", ["BeebeebFileProvider/XPCBridge.swift"], True),
        ("file provider bridge", ["packaging/macos/FileProviderBridge.m"], True),
        ("engine crate", ["crates/beebeeb-engine/src/lib.rs"], True),
        ("updater crate", ["crates/beebeeb-updater/src/signature.rs"], True),
        ("mixed native plus crates", ["native/beebeeb-drive-rune/src/main.rs", "crates/beebeeb-engine/Cargo.toml"], True),
        ("unknown top-level file", ["something-new.toml"], True),
        ("empty list fails open", [], True),
        ("blank lines only fail open", ["", "  "], True),
        ("windows separators", ["docs\\a.md"], False),
        ("a directory merely named like docs", ["docs-tools/build.rs"], True),
    ]
    failed = 0
    for name, paths, expect_macos in cases:
        got_macos = areas(paths)["macos"]
        ok = got_macos == expect_macos
        failed += not ok
        tag = "PASS" if ok else "FAIL"
        print(f"{tag} {name}: macos={str(got_macos).lower()}")
    print(f"Self-test: {len(cases) - failed} passed; {failed} failed")
    return 1 if failed else 0


def main():
    if len(sys.argv) == 2 and sys.argv[1] == "--self-test":
        return self_test()
    try:
        result = areas(Path(sys.argv[1]).read_text(encoding="utf-8").splitlines())
    except (IndexError, OSError, UnicodeError) as error:
        print(f"ci-changed-areas: {error}; running every area", file=sys.stderr)
        result = {"macos": True, "paths": 0, "relevant_paths": 0}
    print(f"changed paths: {result['paths']}; relevant: {result['relevant_paths']}", file=sys.stderr)
    print(f"macos={'true' if result['macos'] else 'false'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
