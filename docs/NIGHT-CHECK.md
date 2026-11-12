# Night check — landed work on `v2-next`

Date: 2026-11-11 (times from `date -Is`, this PC's clock).
Worktree: `~/code/bb-worktrees/desktop-v2-night-3`, branch `night/2026-11-10-90-check-landed-work`.
Base: `c5d9b699` (merge of night wave 1; merge base `03c0d06c`).

Each file below was run in its **own** `bun test` process. Two files are never passed to one
`bun` process: `storagePaneReal` loads `react-dom` before `securityFactorsReal` can install its
input shim, and that combined run fails a test that passes alone.

Every line ends `0 fail` with at least `1 pass`. A `0 pass / 0 fail` line would be a failure.

| # | Command | Result |
|---|---|---|
| 1 | `bun test tests/settingsThemeMenu.test.tsx` | 1 pass / 0 fail |
| 2 | `bun test tests/appVersionHonesty.test.ts` | 3 pass / 0 fail |
| 3 | `bun test tests/guideBackdrop.test.tsx` | 2 pass / 0 fail |
| 4 | `bun test tests/filesActionsHonest.test.tsx` | 4 pass / 0 fail |
| 5 | `bun test tests/devicesPaneStates.test.tsx` | 3 pass / 0 fail |
| 6 | `bun test tests/notificationsAllowOff.test.tsx` | 5 pass / 0 fail |
| 7 | `bun test tests/featureMap.test.ts` | 47 pass / 0 fail |
| 8 | `bun test tests/settingsPaneSurfaces.test.tsx` | 18 pass / 0 fail |
| 9 | `bun test tests/thumbnailsRealPrefs.test.tsx` | 3 pass / 0 fail |
| 10 | `bun test tests/backupsCardReal.test.ts` | 4 pass / 0 fail |
| 11 | `bun test tests/storagePaneReal.test.tsx` | 3 pass / 0 fail |
| 12 | `bun test tests/bandwidthReal.test.ts` | 4 pass / 0 fail |
| 13 | `bun test tests/diagnosticsReal.test.tsx` | 3 pass / 0 fail |
| 14 | `bun test tests/insightsReal.test.ts` | 4 pass / 0 fail |
| 15 | `bun test tests/recentReal.test.tsx` | 3 pass / 0 fail |
| 16 | `bun test tests/syncPaneReal.test.tsx` | 5 pass / 0 fail |
| 17 | `bun test tests/securityFactorsReal.test.tsx` | 4 pass / 0 fail |
| 18 | `bun test tests/accountProfileReal.test.tsx` | 4 pass / 0 fail |
| 19 | `bun test tests/keepDialogReal.test.ts` | 4 pass / 0 fail |
| 20 | `bun test tests/speedtestHonest.test.ts` | 4 pass / 0 fail |
| 21 | `bun test tests/trashReal.test.tsx` | 4 pass / 0 fail |
| 22 | `bun test tests/searchReal.test.tsx` | 4 pass / 0 fail |
| 23 | `bun test tests/photosReal.test.tsx` | 4 pass / 0 fail |
| 24 | `bun test tests/sharedReal.test.tsx` | 3 pass / 0 fail |
| 25 | `bun test tests/offlineReal.test.tsx` | 3 pass / 0 fail |
| 26 | `bun test tests/v2NoFixturesInProduction.test.ts` | 6 pass / 0 fail |

Type check:

| Command | Result |
|---|---|
| `bunx tsc --noEmit` | exit 0 |

Rust: `src-tauri/src/config/types.rs` and `src-tauri/src/config/settings.rs` are unchanged from
`029f960a` (`git diff --stat 029f960a -- <both>` printed nothing), so the `cargo test config`
line does not apply to this check.

No file was red, so no production file was edited and no assertion was changed. The 26 counts
above are the whole result; the raw per-file logs and the `date`-stamped count lines are in
`~/code/beebeeb.io/.claude/tasks/verification-evidence/desktop-v2/night-check/`.
