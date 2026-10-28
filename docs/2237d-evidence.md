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
