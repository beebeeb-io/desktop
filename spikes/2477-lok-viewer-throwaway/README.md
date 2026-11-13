# THROWAWAY: LibreOfficeKit spike (task 2477)

**Not product code.** It answers one question for the contract in
`docs/superpowers/specs/2026-11-12-native-office-viewer.md`: can a native Windows Rust program load
LibreOfficeKit, open Word / Excel / PowerPoint files and get page pixels, with the network blocked? And how do
the Rust office projects (WordCraft, GridCraft, DeckCraft) do on the same files?
Delete this folder when step 2 lands its own bridge.

| File | What it is |
|---|---|
| `make-fixtures.py`, `make-hard-fixtures.py` | write the fixtures in `fixtures/` (standard library only, deterministic); `fixtures/legacy/` was written by `soffice --convert-to` on the VM |
| `shim.cpp`, `build.rs`, `src/main.rs` | the spike program: official init header → C shim → Rust, PNG out |
| `run.ps1` | the phases on the Windows build VM: `install`, `run`, `lokdiag`, `artcraft`, `rustnet` (do the Rust tools open any connection?), `remove` |
| `evidence/` | what the runs produced: PNGs, `metrics.jsonl`, logs (machine paths removed) |

The LibreOfficeKit headers (MPL-2.0) are not vendored: `run.ps1` downloads them from the release
branch and checks pinned SHA-256 values. It is its own Cargo workspace and touches no other manifest.
