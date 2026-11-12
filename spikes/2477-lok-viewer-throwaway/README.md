# THROWAWAY: LibreOfficeKit spike (task 2477)

**Not product code.** It answers one question for the contract in
`docs/superpowers/specs/2026-11-12-native-office-viewer.md`: can a native Windows Rust program load
LibreOfficeKit, open Word / Excel / PowerPoint files and get page pixels, with the network blocked?
Delete this folder when step 2 lands its own bridge.

| File | What it is |
|---|---|
| `make-fixtures.py` | writes the four fixtures in `fixtures/` (standard library only, deterministic) |
| `shim.cpp`, `build.rs`, `src/main.rs` | the spike program: official init header → C shim → Rust, PNG out |
| `run.ps1` | the three phases on the Windows build VM: `install`, `run`, `remove` |
| `evidence/` | what the run produced: PNGs, `metrics.jsonl`, logs |

The LibreOfficeKit headers (MPL-2.0) are not vendored: `run.ps1` downloads them from the release
branch and checks pinned SHA-256 values. It is its own Cargo workspace and touches no other manifest.
