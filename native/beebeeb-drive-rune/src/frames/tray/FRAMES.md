# The b, its flyout and its menus: the frame checklist (lane f-tray)

The Windows frames of the board's groups "The notification area: the b and its flyout" (DW-1201 to DW-1222,
20 frames) and "The b, its menu, and the app menus" (DW-1230 to DW-1234, 6 frames): 26 frames.

The shortcuts dialog's golden is the whole window (1278 x 798): Files behind it, the shell's scrim, the card.

Built = drawn from `Ui` on a `TraySource`, every control working (a state change in the fixture or an `Intent`),
a scene in `frames::tray::FRAMES`, and gated with `scripts/compare.sh <frame>` in light and dark.
`0` = all six counts (geometry, pixels, text, icons, fills, frame tiles) at 0 failing in both themes.

The golden of a flyout frame is the flyout's own box (340 x 520): the wallpaper and the taskbar around it on the
board are Windows' (`scripts/capture-golden.mjs`, `OWN_WINDOW`).

| Frame | What | State |
| --- | --- | --- |
| DW-1201 | The desktop: the taskbar, the b pressed, the flyout over the window on Files | not a frame of its own to draw: Windows draws the taskbar, the flyout is DW-1202, the window is DW-0401; where the flyout opens is `shell/flyout.rs` (the shell lane's, merged) |
| DW-1202 | Up to date | built, 0 |
| DW-1203 | Syncing: the batch bar, the upload speed, rows in progress | built, 0 |
| DW-1204 | Paused | built, 0 |
| DW-1205 | Needs a decision | built, 0 |
| DW-1206 | Can't reach Beebeeb | built, 0 |
| DW-1207 | Sync stopped: the failed file first, its reason in red | built, 0 |
| DW-1208 | Storage almost full | built, 0 |
| DW-1209 | Storage full | built, 0 |
| DW-1210 | A new version | built, 0 |
| DW-1211 | Locked: no file names | built, 0 |
| DW-1212 | Checking: a skeleton, no words | built, 0 |
| DW-1213 | No plan yet | built, 0 |
| DW-1214 | Too old to connect | built, 0 |
| DW-1215 | Adding Beebeeb to File Explorer | built, 0 (the bar is drawn as the board draws it, still: see the PR, "Not done") |
| DW-1216 | Couldn't add Beebeeb to File Explorer | built, 0 |
| DW-1219 | Signed out after 90 days | built, 0 |
| DW-1220 | The end of the list: the last 50, View full history, the scroll thumb | built, 0 |
| DW-1221 | The longest copy and names | built, 0 |
| DW-1222 | Waiting on a metered connection | built, 0 |
| DW-1230 | The b's six icons | the shell lane's, merged (`shell/tray_icon.rs`, `shell/tray.rs`); Windows draws the taskbar around them. Not gated here: the board draws them inside Windows' own notification area |
| DW-1231 | The b's menu | the shell lane's, merged (`shell/tray.rs`: the items; Windows draws the menu) |
| DW-1231b | The b's menu while paused | the same |
| DW-1232 | Commands and keyboard shortcuts: Beebeeb, File, Edit | built (`shortcuts.rs`, `registry::DIALOGS`); gate NOT at 0: light geometry 2, pixels 8, frame tiles 4; dark 2, 10, 1 (text, icons, fills 0). Every failing key is, or lies under, "New ▾" (2 rows): the board's ▾ is the capture machine's fallback font (see the PR, "Gate not at 0") |
| DW-1233 | Commands and keyboard shortcuts: View, Go, Window, Help | built (the dialog's second page: Page Down); gate NOT at 0: light geometry 1, pixels 7, frame tiles 8; dark 1, 10, 4. Every failing key is, or lies under, "Alt+←", "Alt+→" or "the □ caption button": the same fallback fonts |
| DW-1234 | Commands and keyboard shortcuts while a document is open | the lists are built and tested (`commands::EDITOR`, shown while the window is on a viewer); NOT gated and no golden: the frame behind it is the document editor, which no lane has built |

## The engine calls each action needs

`TraySource` (`source.rs`) is the seam; `BoardFixture` stands in. What the engine lane implements:

| Control | Today | Needs |
| --- | --- | --- |
| The status block, the rows | `TraySource::snapshot()` on the fixture, following the shell's `TrayInputs` (`frame_for`) | the sync engine's state and counts (files on this PC, bytes left, speed), the last 50 rows of its history (`sync_history`), storage use, the update check, the Cloud Files registration state and its error code |
| Pause / Resume | `ShellActions::pause_sync` / `resume_sync` (the shell's, wired) | nothing more |
| Sync anyway | `TraySource::act(SyncAnyway)` | "sync on this metered connection until the PC leaves it" |
| Review… | `host::request(Intent::ReviewConflict { file: None })` (wired) | the conflict dialog's lane (DW-0429) |
| Try again (offline) | `TraySource::act(TryAgain)` | a connection check now |
| Show | `host::request(Go(SyncHistory))` (wired) | Sync history's Problems filter as part of the route (DW-0428) |
| Add storage, Choose a plan on beebeeb.io | `TraySource::act(AddStorage / ChoosePlan)`, the flyout closes | open the plans page in the browser (a platform call: `Platform::open_url`, not there yet) |
| Restart to update | `TraySource::act(RestartToUpdate)`, the flyout closes | the updater (`crates/beebeeb-updater`): install and reopen |
| Unlock with Windows Hello | the window comes forward (`Go(Files)`), `TraySource::act(Unlock)` | the Windows Hello prompt of the app lock (DW-0009) |
| Try again (File Explorer) | `TraySource::act(RetryExplorer)` | register the sync root with Cloud Files again |
| Sign in | the window comes forward (`Go(Files)`), `TraySource::act(SignIn)` | the password prompt (DW-1109) |
| A row | `TraySource::reveal(id)`, the flyout closes; a trashed file: `Go(Trash)` (wired) | show the file in File Explorer, selected (`SHOpenFolderAndSelectItems`) |
| Commands and keyboard shortcuts: Close, Esc, Page Down / Page Up | `shortcuts::close`, the shell's Esc, `State::shortcuts.page` (wired) | nothing: the list is static; which lists show while a document is open follows the route (`FrameGroup::Viewer`) |
| Open Beebeeb, Settings, View full history | `host::request` (wired) | nothing more |
| Open in File Explorer | `ShellActions::open_in_file_explorer` (the shell's, wired) | nothing more |
