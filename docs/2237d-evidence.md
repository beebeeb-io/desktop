# Lane d2-2237: desktop shared folders

Owner: Codex, sequential run, no child agents.
Worktree: `/Users/guuslangelaar/code/bb-worktrees/d2-2237-desktop`
Branch: `feat/2237d-desktop-shared-folders`
Desktop base: `f2d42d4` (includes #169 / 2190 and #171 / 2240 slice 2).
Core pin: `a245a549c12415af6311bf4488deca5def65e983` in `src-tauri/Cargo.toml` (core, sync and types), plus the lockfile.

Contract: workspace origin/main plan `2026-10-28-shared-folders-p1d-desktop-cli.md`, tasks 3-6 only; approved shared-folder spec; plan D lead review and plan C L1-L7, L6a/L7a. Custody uses core L6 and child wraps use core L7 with ROOT + item + kind. No client crypto implementation.

Verification planned before code: task filters `sf2237d_t3` (6), `sf2237d_t4` (8), ~~`sf2237d_t5` (7), `sf2237d_t6` (8)~~, behavioral red then identical green on testrig desktop image; relevant regressions; final macOS `cargo test --locked` under cargo-build lock with owned target/scratch HOME and loopback-only API; frontend Bun, tsc and eslint. Amendment — Codex, 2026-10-28: Task 5 verification is replaced in this run by the source-premise audit below, under the explicit stop rule; Task 6 is blocked by Task 5. Neither is implemented or counted as complete. Windows native gate belongs to the lead. No installed app, real user config, production, or merge.

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

- Repository Graphify refresh ran successfully: 388 files, 8,486 nodes, 25,575 edges, 192 communities (`graphify-update.log`). Its repository-wide 386,343-line rewrite exceeded this lane's explicit plan file map. The refreshed graph/report are preserved under the external evidence directory `graphify-refresh/`; tracked graph artifacts retain their base version. No unrelated graph rewrite is included in this scoped PR.

## Final verification (2026-10-28, Codex)

Tasks 3 and 4 are complete within their interfaces. Task 5 is stopped and Task 6 is blocked. Source tested at `185d87ed768f52eff4b651c0bf18ecb85361f7ef`; subsequent changes only record these receipts.

| Gate | Passed | Failed | Ignored | Evidence |
| --- | ---: | ---: | ---: | --- |
| macOS Rust library | 2,220 | 0 | 9 | `final-macos-rust.log` |
| macOS Rust integration: keychain abstraction | 31 | 0 | 0 | same log; in-memory secret store |
| macOS Rust integration: portable Windows session wiring | 8 | 0 | 0 | same log |
| macOS Rust integration: portable Windows sign-out cleanup | 3 | 0 | 0 | same log |
| Rust total | 2,262 | 0 | 9 | `final-macos-counts.json`, process exit 0 |
| Frontend Bun | 1,833 | 0 | 0 | `final-macos-bun.log`, 106 files |
| TypeScript / ESLint | exit 0 / exit 0 | 0 diagnostic lines | — | `final-macos-tsc.log`, `final-macos-eslint.log` |

The final Rust command was `cargo test --locked --no-fail-fast -- --test-threads=1`, under workspace `with-lock.sh cargo-build`, with an owned target/cache configuration, fresh scratch HOME, `BB_API_BASE=http://127.0.0.1:9` and `BB_REFUSE_NON_LOOPBACK_API=1`. Six announced binary/doc-test totals agree with 2,271 completed test rows, including ignored rows with reasons. All 14 new tests passed on both Linux testrig and macOS; none is Windows-only. The nine existing ignored tests require a real local API/MinIO (5), a logged-in GUI session (2), a real DB copy (1), or a separate existing two-surface implementation (1); their exact names/reasons are retained in the log and JSON receipt. They were not forced on.

Native Windows tests inside `windows_cf`, including `windows_cf::reconcile_tests`, are gated by `cfg(target_os = "windows")` at `src-tauri/src/lib.rs:135` and were not run here. `tests/windows_session_wiring.rs` and `tests/windows_signout_cleanup.rs` are portable checks and passed on macOS; they do not prove Cloud Files behavior. `StateDb::finish_windows_signout` is available under `cfg(any(target_os = "windows", test))` for portable purge coverage. Lead glegion owns the native Windows gate.

Source compatibility was checked against server main `7dd40237b5abb50fae1b686a37246e773584e2fc` (#349 / #351), not production. Latest desktop main `64d39db` still has no `ChunkDecryptor` / `ExpectedChunkFormat` consumer; a read-only `git merge-tree --write-tree origin/main HEAD` completed with exit 0 (`merge-preview.log`). The implementation retains the original allocated base. Core remains pinned coherently to `a245a549`; no chunk-format rollout. `git diff --check` passes. No Finder/Explorer device proof, deployed-client interoperability, live server check, or release clearance is claimed.

Next required handoff: document/merge the authenticated API/direct/range chunk reader interface (including v2 format pins, finish validation and range AAD/frame-zero handling), amend Task 5's premise, then resume Tasks 5–6 and the Task 13 native rung.

## Resume under Guus's corrected ruling (2026-10-28)

Workspace origin/main's Task 5 correction (workspace #225) supersedes the struck #224 amendment and clears the previous wrong-premise stop. Task 1943 scope/verification and plan C L7b were read before editing. Resume remains in the same isolated branch/PR, sequential with no child agents. All previously used trees were checked: the lane tree was clean at 105ba298; shared-tree changes are unrelated and untouched.

Verification before code: seven `sf2237d_t5` tests, plus a separate truncated-v2 completion case and finish-bypass mutation; one L7b custody-versus-legacy reader guard; eight `sf2237d_t6` convergence tests. Each receives counted behavioral RED then GREEN on the testrig desktop image; related reader/hydration/conflict regressions follow. Final macOS full Rust gate uses the cargo-build lock, owned target, scratch HOME and loopback API. Frontend gate runs on testrig Bun image. Native Windows/device evidence remains the lead's.

Implementation decision: core a245a549 keeps `StreamOpener` crate-private; its public `ChunkDecryptor::for_push_with_file_key` delegates every `push_frame` to `StreamOpener::open_next` and `finish` to its finish check. Use that existing public core API for whole-file reads, and core `open_chunk` with an explicit index for ranges. No second reader or client crypto is introduced. JSON legacy wire envelopes are decoded to raw frame bytes without decrypting them. Authenticated metadata supplies the format pin; no server plaintext pin is trusted. Preserve object-version restart-once. Direct-download wiring is in scope under the explicit correction, despite the original Task 5 file map.

Task 5 reader step: red 7 executed (2 passed / 5 failed); initial reader/resolver green 7 passed / 0 failed (`t5-reader3.log`). Both transports now consume core ChunkDecryptor and finish before publication. Range opening authenticates frame zero for stream identity, then uses core open_chunk with explicit indexes. Next: strengthen actual replacement/version/request fixtures, custody legacy guard, recoverable errors, and full related regressions.

Custody guard RED: 1 executed / 0 passed / 1 failed (`l7b-red.log`); bound core dispatch GREEN with all lane tests: 23 passed / 0 failed (`t5-boundaries.log`, 14 prior + 7 Task 5 + truncation + L7b). Custody detection uses the authenticated custody endpoint; only HTTP 404 selects the named legacy invite reader. Transport/auth failures never downgrade. Purpose-specific core thumbnail/name readers replace local thumbnail AES and raw metadata decoding. Added an optional, default-false nonsecret `member_added` provenance bit to choose the specified recoverable error copy. Replacement v2 sessions cannot resume into a new core stream; fresh init is attempted without file-wide abandonment, preserving the old resume record on refusal.

Task 5 strengthened GREEN: 7 passed / 0 failed (`t5-green.log`): actual replacement upload-session PUTs decrypt under the original random key and V2 pin; API hydrate consumes published core v1 and v2 known-answer vectors; SharedWithMe uses its actual invite branch with zero owner adoption; existing request owner wraps require no custody. Object-version requests stay pinned and two 409s stop after one restart. Related selections: hydrate 36/0, conflict_content_preview 5/0, version_bound_downloads_1643 16/0. Direct range request-count expectation updated for authenticated frame zero; individual green 1/0 (`t5-green-final.log`).

Broader Linux instrument: first lib 1820/64/7 ignored, then corrected engine fixtures 402/6/2 ignored. Five remaining engine failures require chmod to deny writes and run as root in the container; final macOS gate runs as the normal user. One initial unrelated lib DNS fixture received HTTP 200 for nothing.invalid on the testrig. Neither broad run is claimed green. Production pin extraction refuses absent encrypted metadata; old test fixtures were corrected instead of adding a downgrade seam.

Task 5 finish mutation: bypassing the whole-reader finish call made the truncated-v2 publication guard fail (1 executed / 0 passed / 1 failed, `t5-finish-mutation-red.log`). Restored production code: all 23 lane tests passed / 0 failed (`t5-finish-restored-green.log`). Task 5 corrected ruling is complete; Task 6 starts next through the existing sync/held-edit interfaces.

Task 6 RED: 8 executed / 7 passed / 1 failed (`t6-red.log`); a folder rename left the guest child at A/guest.txt instead of Renamed/guest.txt. First fixture attempt also omitted the hydrate destination parent directory (6/2); correcting the fixture left the behavioral failure. GREEN: 8 passed / 0 failed (`t6-green.log`). Sparse folder events now reuse existing metadata/guarded DB writes for descendants and report their working-set IDs; no watcher or held-edit module was changed. The eight fixtures exercise actual sparse folder_public payloads, canonical enrichment, guest/member identity, version retention, owner Trash/restore, restart/replay, and 2190's held-edit API with preserved local bytes. Each checks three quiet ticks with zero upload requests, conflicts/errors, queued uploads and orphan entries (existing native verifier over the actual portable filesystem view). These are portable bridge fixtures, not native provider proof.

Dependencies: desktop base contains #169 (2190) and #171 (2240 slice 2). Remaining 2240–2242 scanner/device integration and signed Finder/Explorer Task 13 scenarios stay explicit in RELEASE_BLOCKERS.md; this lane uses merged documented interfaces and adds no alternate scanner.

Strengthened final lane fixtures: 31 passed / 0 failed (`lane-green.log`). Task 5 additionally exercises actual direct transport for owned and foreign keys with both unrecorded-v1 and V2 pins, including direct V2 truncation refusal and zero API fallback requests. Task 6 additionally checks a moved/renamed parent's descendants, two actors' interleaved sparse events in one catch-up batch, and both concurrent local/remote byte sequences surviving a held edit. New fixture files were rustfmt-formatted on the testrig. Per-task totals remain 6 + 8 + 7 + 8, plus two independent finish/L7b guards.

Final remote related selections: direct_download 32 passed / 0 failed / 1 existing ignored; sync_tick 12 passed / 0 failed (`final-linux-related.log`). Frontend on testrig Bun image: 1,833 passed / 0 failed across 106 files; frozen install, TypeScript build and ESLint all exit 0 (`final-testrig-frontend.log`). No JS suite ran on the Mac in this resume.
