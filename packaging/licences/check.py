#!/usr/bin/env python3
"""Release check for the third-party licence notice. Fails (exit 1) when the notice is missing, stale or incomplete.

  check.py [--root <repo>] [--staged <dir>] [--version <x.y.z>]

Offline. Checks THIRD-PARTY-LICENSES.txt against the three Cargo.lock files (hash stamps; every non-path package of
every lock is in the crate index), that the sections a release must carry are present, that the release version
matches, that the installer script and build script wire it in, and, with --staged, that the installer's staging
folder holds a byte-identical copy beside the exes.
"""
import argparse
import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
from render import INDEX_BEGIN, INDEX_END, LOCKS, git_sources, lock_packages, lock_sha  # noqa: E402

NAME = "THIRD-PARTY-LICENSES.txt"
REQUIRED = [
    "GNU AFFERO GENERAL PUBLIC LICENSE",
    "SIL Open Font License",
    "UBUNTU FONT LICENCE Version 1.0",
    "Mozilla Public License",
    "Redistribution and use in source and binary forms",  # PDFium (Foxit) and Adobe CMaps, BSD-3-Clause
    "https://github.com/beebeeb-io/desktop",
    "release tag desktop-v",
]


def check(root: Path, staged: Path | None, version: str | None) -> list[str]:
    bad = []
    src = root / "packaging" / "licences" / NAME
    if not src.is_file() or src.stat().st_size < 10_000:
        return [f"{src} is missing or empty: run packaging/licences/generate.sh"]
    text = src.read_text(encoding="utf-8")
    for label, rel in LOCKS:
        m = re.search(rf"^cargo-lock-sha256 {re.escape(label)}: ([0-9a-f]{{64}})$", text, re.M)
        if not m:
            bad.append(f"no hash stamp for the {label} lock")
        elif m.group(1) != lock_sha(root / rel):
            bad.append(f"STALE: {rel} changed since the notice was generated (run packaging/licences/generate.sh)")
    b, e = text.find(INDEX_BEGIN + "\n"), text.find("\n" + INDEX_END)
    index = set(text[b + len(INDEX_BEGIN) + 1 : e].splitlines()) if 0 <= b < e else set()
    if not index:
        bad.append("no crate index")
    for label, rel in LOCKS:
        gone = sorted(f"{n} {v}" for n, v, _s in lock_packages(root / rel) if f"{n} {v}" not in index)
        if gone:
            bad.append(f"{len(gone)} crate(s) of {rel} are not in the notice, e.g. {gone[:5]}")
    for needle in REQUIRED:
        if needle not in text:
            bad.append(f"the notice lacks: {needle}")
    # The source offer names desktop-office at the commit each lock pins (the notice derives it from the locks).
    for _label, rel in LOCKS:
        for (url, rev) in git_sources(lock_packages(root / rel)):
            if "/beebeeb-io/" not in url:
                continue  # other people's git crates are credited in section 5
            if f"{url}/tree/{rev}" not in text:
                bad.append(f"the notice does not offer the source of {url} at {rev} ({rel})")
    if version:
        if f"release-version: {version}" not in text:
            bad.append(f"the notice is for another release than {version}")
    for rel, needle in (("packaging/windows/installer.nsi", NAME), ("packaging/windows/build-installer.ps1", NAME)):
        f = root / rel
        if not f.is_file() or needle not in f.read_text(encoding="utf-8"):
            bad.append(f"{rel} does not carry {needle}")
    if staged is not None:
        got = staged / NAME
        if not got.is_file():
            bad.append(f"{got} is missing from the installer's staged files")
        elif got.read_bytes() != src.read_bytes():
            bad.append(f"{got} differs from {src}")
    return bad


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", default=str(Path(__file__).resolve().parents[2]))
    ap.add_argument("--staged")
    ap.add_argument("--version")
    a = ap.parse_args()
    bad = check(Path(a.root), Path(a.staged) if a.staged else None, a.version)
    for line in bad:
        print(f"licence check FAILED: {line}", file=sys.stderr)
    if not bad:
        print("licence check ok")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
