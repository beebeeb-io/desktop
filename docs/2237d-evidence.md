# Lane d2-2237: desktop shared folders

Owner: Codex, sequential run, no child agents.
Worktree: `/Users/guuslangelaar/code/bb-worktrees/d2-2237-desktop`
Branch: `feat/2237d-desktop-shared-folders`
Desktop base: `f2d42d4` (includes #169 / 2190 and #171 / 2240 slice 2).
Core pin: `a245a549c12415af6311bf4488deca5def65e983` in `src-tauri/Cargo.toml` (core, sync and types), plus the lockfile.

Contract: workspace origin/main plan `2026-10-28-shared-folders-p1d-desktop-cli.md`, tasks 3-6 only; approved shared-folder spec; plan D lead review and plan C L1-L7, L6a/L7a. Custody uses core L6 and child wraps use core L7 with ROOT + item + kind. No client crypto implementation.

Verification planned before code: task filters `sf2237d_t3` (6), `sf2237d_t4` (8), `sf2237d_t5` (7), `sf2237d_t6` (8), behavioral red then identical green on testrig desktop image; relevant regressions; final macOS `cargo test --locked` under cargo-build lock with owned target/scratch HOME and loopback-only API; frontend Bun, tsc and eslint. Windows native gate belongs to the lead. No installed app, real user config, production, or merge.

Evidence directory: `/Users/guuslangelaar/code/bb-gate-logs/fullspeed/d2-2237/`.

## Decisions

- Use the existing authenticated client and cooperative engine stop signal, with a DB account generation checked atomically at persistence boundaries.
- Keep keys out of persistent state; store only encrypted envelopes, ciphertext metadata and an opaque fingerprint.
- The plan names a superpowers execution skill unavailable in the installed skill catalog; follow its explicit sequential task/checklist protocol directly, as required by this lane.

## Progress

- 2026-10-28: task 3 red: 6 executed / 0 passed / 6 failed; green: 6 executed / 6 passed / 0 failed. Logs: `t3-red.log`, `t3-green.log`; same command: `cargo test --locked --lib sf2237d_t3 -- --test-threads=1` via testrig desktop image. Count guard negative fixture: 1 rejected / 0 accepted. Core pin lockfile regenerated on testrig. Additional purge RED: 5 passed / 1 failed, then GREEN: 6 passed / 0 failed. Existing account selection: 148 passed / 0 failed; purge selection: 45 passed / 0 failed. Log: `t3-purge-green-regressions.log`. Both account clear and sign-out local-state purge erase envelopes and invalidate outstanding requests.

## Deviations and residuals

- Native Finder/Explorer behavior remains Task 13 evidence; unit fixtures do not prove provider behavior.

- Task 4 first green: 8 executed / 8 passed / 0 failed after behavioral red 8 executed / 0 passed / 8 failed (`t4-red.log`, `t4-green-attempt1.log`). Enrichment, L6/L7 custody resolution, adoption restart, safe blocked rows, CAS repair and app-only activity DTOs are covered. Normal-tick maintenance wiring and required enrichment mutation remain before Task 4 completion.

- Task 4 maintenance green: 8 passed / 0 failed (`t4-green-wiring-counted.log`). Required enrichment mutation: 1 executed / 0 passed / 1 failed (`t4-mutation-red.log`), wrong guest path with enrichment bypassed. Restored green and regressions follow.

## Task 5 premise blocker (2026-10-28)

STOP under the lane's explicit wrong-premise rule. Task 5 assumes existing authenticated chunk-format pin readers and asks to preserve them while routing keys. At desktop base `f2d42d4`, `engine_bridge.rs::download_and_decrypt_chunk_range` calls `decrypt_downloaded_chunk` (current line 4991); that helper (current line 7029) only tries raw AES-GCM and legacy JSON frames. It accepts no item id, chunk index, authenticated `ExpectedChunkFormat`, frame count or finish contract. `direct_download.rs:422` calls the same helper; its Request at line 91 carries an object-version manifest, not an authenticated chunk-format pin. `download_pin.rs:21` describes only object identity/size. There is no `ChunkDecryptor` or `ExpectedChunkFormat` consumer in either desktop download driver. These facts were checked in source, not inferred from a failed compiler/test.

The required v2-pinned bad-frame/no-fallback and authenticated finish tests cannot be met by swapping the key resolver into these existing interfaces. Correcting the premise requires a documented decoder/format-pin integration covering API, direct and range downloads (including v2 range AAD/frame-zero handling), beyond the plan's stated existing-reader contract and Task 5 file map. No alternative protocol or cryptography was invented. Task 5 has no red/green count and is unimplemented; Task 6 is blocked by Task 5. The new metadata/adoption work is not a working foreign-file open/edit feature and is not release-ready by itself.

- Task 4 content-only dirty-marker guard: RED 8 executed / 7 passed / 1 failed (`t4-content-dirty-red.log`), GREEN 8 executed / 8 passed / 0 failed (`t4-content-dirty-green.log`). Existing sync-tick regressions: 12 passed / 0 failed (`t4-sync-regressions.log`). Dirty roots are marked before applying version or parent changes even when opaque metadata is unchanged.

- Initial macOS final gate: build passed; 1,397 test rows passed before a nonblocking accepted socket panicked in the owned HTTP fixture, followed by a destructor panic/abort. This was a fixture failure, not a successful gate (`final-macos-rust-attempt1.log`). Fix: force accepted sockets into blocking mode and avoid a second panic while unwinding. Testrig recheck: 14 executed / 14 passed / 0 failed (`fixture-portability-green.log`). Frontend gate: 1,833 passed / 0 failed across 106 files; TypeScript and ESLint exit 0 with 0 diagnostic lines.
