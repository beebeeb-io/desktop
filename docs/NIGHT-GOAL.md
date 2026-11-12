# NIGHT-GOAL — the Drive window check

These two lines were copied from 00-goal.md by the lead on 2026-11-11 and are carried in this brief because the queue directory is not mounted in the worker sandbox; they were not re-read from that file here.

> Goal: By morning v2-next's Drive window matches the board and opens a real file — the Files toolbar is a `Search names` field with `Ctrl+F` and no Filter, the settings caption reads `Settings - Beebeeb Drive`, both window titles say `Beebeeb Drive`, the sidebar carries the drive mark plus those words, and a listed text file previews its own bytes — and `docs/night-notes/65-file-loss.md` names the exact line in `src-tauri/src/owner_keys.rs` where a file row is lost on the way into the list, with a red test pinning it and a fix only where that test proves one. Every merge keeps `bunx tsc --noEmit` at 0 errors, `cargo check --all-targets` at 0 errors, and `bun test` no worse than 2380 pass / 309 fail (measured 2026-11-10 17:08 at `c1766222`; the standing red set at `03c0d06c` was 2369 pass / 312 fail, and the +8 tests are the merged 55/56/58 groups. The 1 unhandled error in `tests/finderInstallOneSurface.test.tsx` is pre-existing in the macOS Finder suite and is not tonight's).
> In short: Files matches the board; the lost file row gets a named cause.

Measured on `v2-next` at `929cde1c` (the merge of the Files toolbar field, `a5847d62`).

| fact | command | count | date |
|---|---|---|---|
| Files toolbar field — MET: one `input` with `aria-label` and `placeholder` `Search names`, `Ctrl+F` visible, Enter opens search, 0 Filter | `bun test tests/filesToolbar.test.tsx` | 3 pass / 0 fail | 2026-11-11T01:45:40+01:00 |
| Settings caption `Settings - Beebeeb Drive` | `bun test tests/settingsChrome.test.tsx` | 3 pass / 0 fail | 2026-11-11T01:45:40+01:00 |
| Both window titles `Beebeeb Drive` and the sidebar drive mark plus those words | `bun test tests/driveMark.test.tsx` | 3 pass / 0 fail | 2026-11-11T01:45:40+01:00 |
| Listed text file previews its own bytes | `bun test tests/previewRealFile.test.tsx` | 2 pass / 0 fail | 2026-11-11T01:45:41+01:00 |
| `bunx tsc --noEmit` | `bunx tsc --noEmit` | 0 errors, exit 0 | 2026-11-11T01:45:45+01:00 |
| `cargo check --all-targets -j 6 --manifest-path src-tauri/Cargo.toml` | `cargo check --all-targets -j 6 --manifest-path src-tauri/Cargo.toml` | `Finished \`dev\` profile [unoptimized + debuginfo] target(s) in 1.66s`, exit 0 | 2026-11-11T01:45:45+01:00 |
| Whole suite | `bun test` | `2391 pass`<br>`309 fail`<br>`1 error`<br>`12371 expect() calls`<br>`Ran 2700 tests across 194 files. [111.55s]` | 2026-11-11T01:43:38+01:00 |

NOT MET: the file-loss note is not on v2-next. Slice 65 is parked for Guus (~/code/bb-legion-native/opencode-night/parked/desktop-v2/65-the-file-row-survives-the-key-walk.needs): the fix is proven red->green (lost_row 0/1 then 1/0) but the pre-commit rustfmt + 1,200-line file-size guards refuse any .rs commit (owner_keys.rs formats to 1,217 lines; engine_bridge.rs needs a 564-line reformat). No owner_keys.rs edit landed and no docs/night-notes/65-file-loss.md is on the branch.

Verified against the tree: `ls docs/night-notes/` lists `56-sign-out.md`, `57-files-list.md`, `58-stale-row.md`, `61-toolbar.md`, `63-preview.md`, `64-drive-mark.md`, `evidence` — no `65-file-loss.md`, so the last sentence matches. The parked path is not mounted in this sandbox, so the park file itself was not read.

The whole-suite line is within the goal's floor (at least 2380 pass, at most 309 fail): 2391 pass and 309 fail. The `1 error` is the known sandbox baseline: `# Unhandled error between tests` in `tests/finderInstallOneSurface.test.tsx:474` (`reset_macos_integration` expected 1, received 0), part of the Finder-setup baseline.

0 new tests, so no red-first applies; the counts are existing guards run today. No test file was edited for isolation: the whole-suite run showed 2391 pass / 309 fail with no extra failure in `tests/filesToolbar.test.tsx`, so its three tests needed no change.
