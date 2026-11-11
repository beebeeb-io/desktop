# NIGHT-GOAL — the Drive window check

The goal file `/home/guus/code/bb-legion-native/opencode-night/queue/desktop-v2/00-goal.md` is not mounted in this night sandbox (only `inbox/` is visible), so its two lines cannot be read from the file here. The two goal lines as supplied in the slice brief are:

> The Drive window tells the truth: the Files toolbar has a `Search names` field (`Ctrl+F`, Enter opens search, no Filter), the settings caption reads `Settings - Beebeeb Drive`, both window titles read `Beebeeb Drive`, the sidebar shows the drive mark plus `Beebeeb Drive`, and a listed text file previews its own bytes.
> The file-loss note is on v2-next.

| fact | command | count | date |
|---|---|---|---|
| Files toolbar field (`Search names`, `Ctrl+F`, Enter opens search, no Filter) | `bun test tests/filesToolbar.test.tsx` | 3 pass / 0 fail | 2026-11-11T01:08:14+01:00 |
| Settings caption `Settings - Beebeeb Drive` | `bun test tests/settingsChrome.test.tsx` | 3 pass / 0 fail | 2026-11-11T01:08:14+01:00 |
| Both window titles `Beebeeb Drive` | `bun test tests/driveMark.test.tsx` | 3 pass / 0 fail | 2026-11-11T01:08:15+01:00 |
| Sidebar drive mark plus `Beebeeb Drive` | `bun test tests/driveMark.test.tsx` | 3 pass / 0 fail | 2026-11-11T01:08:15+01:00 |
| Listed text file previews its own bytes | `bun test tests/previewRealFile.test.tsx` | 2 pass / 0 fail | 2026-11-11T01:08:15+01:00 |
| `bunx tsc --noEmit` | `bunx tsc --noEmit` | 0 errors, exit 0 | 2026-11-11T01:08:19+01:00 |
| `cargo check --all-targets -j 6 --manifest-path src-tauri/Cargo.toml` | `cargo check --all-targets -j 6 --manifest-path src-tauri/Cargo.toml` | `Finished \`dev\` profile [unoptimized + debuginfo] target(s) in 6.12s`, exit 0 | 2026-11-11T01:08:19+01:00 |
| Whole suite | `bun test` | 2390 pass / 310 fail / 1 error (2700 tests across 194 files) | 2026-11-11T01:10:32+01:00 |

The Files toolbar row was 0 tests on the first run of this report: the field and its guard were not on `v2-next`. The work is on `night/2026-11-10-61-sort-and-search-match-the-files-frame` (the `night/2026-11-10-66-files-toolbar-search-field` branch is at the base commit `30a339ec`), so that branch was merged into this branch before the run above; the guard then returned 3 pass / 0 fail.

The whole-suite line is one fail above the goal's floor (at least 2380 pass, at most 309 fail): 2390 pass and 310 fail. The one failing file that is not on the known sandbox baseline (Finder setup, speed test, trash, sign-in) is `tests/filesToolbar.test.tsx`: its `Enter on the field calls onFind once; another key does not` test passes 3 pass / 0 fail in its own process but fails inside the whole-suite process with `TypeError: null is not an object (evaluating 'inst.tag')` from `react-dom` at `tests/filesToolbar.test.tsx:65`, i.e. shared React/mini-DOM state across the suite. It reproduced in both whole-suite runs (310 fail each). No test was edited to change the line.

NOT MET: the file-loss note is not on v2-next. Slice 65 is parked for Guus (/home/guus/code/bb-legion-native/opencode-night/parked/desktop-v2/65-the-file-row-survives-the-key-walk.needs): the fix is proven red->green (lost_row 0/1 then 1/0) but the pre-commit rustfmt + 1,200-line file-size guards refuse any .rs commit (owner_keys.rs formats to 1,217 lines; engine_bridge.rs needs a 564-line reformat). No owner_keys.rs edit landed and no docs/night-notes/65-file-loss.md is on the branch.

Verified against the tree: `ls docs/night-notes/` lists `56-sign-out.md`, `57-files-list.md`, `58-stale-row.md`, `61-toolbar.md`, `63-preview.md`, `64-drive-mark.md` — no `65-file-loss.md`, so the last sentence matches. The parked path is not mounted in this sandbox, so the park file itself was not read.

0 new tests, so no red-first applies; the counts are existing guards run today.
