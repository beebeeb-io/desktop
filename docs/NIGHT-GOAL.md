# NIGHT-GOAL — the Drive window check

The goal file `/home/guus/code/bb-legion-native/opencode-night/queue/desktop-v2/00-goal.md` is not mounted in this night sandbox (only `inbox/` is visible), so its two lines cannot be read from the file here. The two goal lines below are quoted from the slice brief and are **unverified against `00-goal.md`**:

> The Drive window tells the truth: the Files toolbar has a `Search names` field (`Ctrl+F`, Enter opens search, no Filter), the settings caption reads `Settings - Beebeeb Drive`, both window titles read `Beebeeb Drive`, the sidebar shows the drive mark plus `Beebeeb Drive`, and a listed text file previews its own bytes.
> The file-loss note is on v2-next.

| fact | command | count | date |
|---|---|---|---|
| Files toolbar field (`Search names`, `Ctrl+F`, Enter opens search, no Filter) | `bun test tests/filesToolbar.test.tsx` | 0 tests, exit 1 — `The following filters did not match any test files` (the field and its guard are absent on this branch) | 2026-11-11T01:26:58+01:00 |
| Settings caption `Settings - Beebeeb Drive` | `bun test tests/settingsChrome.test.tsx` | 3 pass / 0 fail | 2026-11-11T01:26:58+01:00 |
| Both window titles `Beebeeb Drive` | `bun test tests/driveMark.test.tsx` | 3 pass / 0 fail | 2026-11-11T01:26:58+01:00 |
| Sidebar drive mark plus `Beebeeb Drive` | `bun test tests/driveMark.test.tsx` | 3 pass / 0 fail | 2026-11-11T01:26:58+01:00 |
| Listed text file previews its own bytes | `bun test tests/previewRealFile.test.tsx` | 2 pass / 0 fail | 2026-11-11T01:26:59+01:00 |
| `bunx tsc --noEmit` | `bunx tsc --noEmit` | 0 errors, exit 0 | 2026-11-11T01:27:07+01:00 |
| `cargo check --all-targets -j 6 --manifest-path src-tauri/Cargo.toml` | `cargo check --all-targets -j 6 --manifest-path src-tauri/Cargo.toml` | `Finished \`dev\` profile [unoptimized + debuginfo] target(s) in 0.28s`, exit 0 | 2026-11-11T01:27:07+01:00 |
| Whole suite | `bun test` | `2388 pass`<br>`309 fail`<br>`1 error`<br>`12360 expect() calls`<br>`Ran 2697 tests across 193 files. [110.60s]` | 2026-11-11T01:27:19+01:00 |

The Files toolbar fact is **not met on this branch**: `src/v2/sections/files/FilesPane.tsx:342-354` draws a `<button>` reading `Search names Ctrl+F`, not a field, and `tests/filesToolbar.test.tsx` does not exist here. The work is on `night/2026-11-10-66-files-toolbar-search-field` (tip `a5847d62`, committed 2026-11-11T01:16:24+01:00), which is not merged into `v2-next` (`30a339ec`). This report was run on the branch as it stands, with no dependency branch merged in, so the diff is `docs/NIGHT-GOAL.md` only.

The whole-suite line is within the goal's floor (at least 2380 pass, at most 309 fail): 2388 pass and 309 fail. The `1 error` is the known sandbox baseline: `# Unhandled error between tests` in `tests/finderInstallOneSurface.test.tsx:474` (`reset_macos_integration` expected 1, received 0), part of the Finder-setup baseline. No test was edited to change the line.

NOT MET: the file-loss note is not on v2-next. Slice 65 is parked for Guus (/home/guus/code/bb-legion-native/opencode-night/parked/desktop-v2/65-the-file-row-survives-the-key-walk.needs): the fix is proven red->green (lost_row 0/1 then 1/0) but the pre-commit rustfmt + 1,200-line file-size guards refuse any .rs commit (owner_keys.rs formats to 1,217 lines; engine_bridge.rs needs a 564-line reformat). No owner_keys.rs edit landed and no docs/night-notes/65-file-loss.md is on the branch.

Verified against the tree: `ls docs/night-notes/` lists `56-sign-out.md`, `57-files-list.md`, `58-stale-row.md`, `63-preview.md`, `64-drive-mark.md` — no `65-file-loss.md`, so the last sentence matches. The parked path is not mounted in this sandbox, so the park file itself was not read.

0 new tests, so no red-first applies; the counts are existing guards run today.
