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
| DW-1201 | The desktop: the taskbar, the b pressed, the flyout over the window on Files | linked, no scene and no golden of its own: Windows draws the taskbar and the b's button, the flyout is DW-1202, the window is DW-0401. It opens from the b (`shell/host.rs` `on_tray`, signed out: the window on sign-in) and from the window's status line in Files and in Search (`tray::show_from_window`, test `the_status_line_of_the_window_opens_the_flyout`). Left: see "DW-1201 and DW-1234: what is left" |
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
| DW-1230 | The b's six icons | OS-drawn, verified by a real-window capture (shell lane's icon code merged: `shell/tray_icon.rs`, `shell/tray.rs`; Windows draws the taskbar and notification area around them) |
| DW-1231 | The b's menu | OS-drawn, verified by a real-window capture (shell lane's menu code merged: `shell/tray.rs`; Windows draws the context menu) |
| DW-1231b | The b's menu while paused | OS-drawn, verified by a real-window capture (same) |
| DW-1232 | Commands and keyboard shortcuts: Beebeeb, File, Edit | built (`shortcuts.rs`, `registry::DIALOGS`); gate NOT at 0: light geometry 2, pixels 8, frame tiles 4; dark 2, 10, 1 (text, icons, fills 0). Every failing key is, or lies under, "New ▾" (2 rows): the board's ▾ is the capture machine's fallback font (see the PR, "Gate not at 0") |
| DW-1233 | Commands and keyboard shortcuts: View, Go, Window, Help | built (the dialog's second page: Page Down); gate NOT at 0: light geometry 1, pixels 7, frame tiles 8; dark 1, 10, 4. Every failing key is, or lies under, "Alt+←", "Alt+→" or "the □ caption button": the same fallback fonts |
| DW-1234 | Commands and keyboard shortcuts while a document is open | the dialog is built: the editor's four lists (`commands::EDITOR`), shown while the window is on an open file, a scene (`DW-1234`: the note open in the text editor, the dialog in front) held by the test `the_scene_of_dw_1234_is_the_editors_four_lists_over_an_open_document`. NOT gated and no golden: see "DW-1201 and DW-1234: what is left" |

Measured on 2026-11-12 at `b6df6207` (the base of the tray's engine wiring) and at its head, the same both times:
DW-1232 light geometry 12, pixels 19, frame tiles 44, dark 12, 20, 12; DW-1233 light 11, 18, 50, dark 11, 20, 13
(text, icons, fills 0). That is more than the two rows above say; the dialog is over the window, which other
lanes have changed since. The 19 flyout frames are at 0 in both themes, both times.

## DW-1201 and DW-1234: what is left

**DW-1201.** Nothing of it is Beebeeb's to draw beyond DW-1202 and DW-0401, so it has no gated scene.
- Opened from the status line, the flyout opens where it last opened: its place 12 px over the taskbar comes from
  the b's rectangle, which only a click on the b carries (`TrayEvent::Activate { anchor }`). Before the first click
  on the b it opens at the place its window was made with. Needs: the b's rectangle from the platform without a
  click (`Shell_NotifyIconGetRect`), a shell change.
- The status lines of Shared, Trash, Photos, Sync history and Available offline still only record the intent
  (`Intent::Frame("DW-1201")` in their own queues); each needs the one call `tray::show_from_window`.
- "The b pressed while the flyout is open" is Windows' drawing of the notification area button.

**DW-1234.** The dialog and its words are done; the frame is not gated.
- The board draws the dialog over the document editor (Docs, with its ribbon). No lane has built that editor, and
  whether it can be native is an open decision (the office editor needs a web view). A golden captured today would
  gate the dialog's keys at their place and fail on every key of the editor behind the scrim.
- When the editor exists: capture `golden/DW-1234*` (keymap-tray.js keys the dialog already, as for DW-1232), point
  the scene at the document the board has open instead of the note, and gate it light and dark.
- The dialog shares DW-1232's card, so its known reds apply here too once gated (the fallback font of "▾", "←", "→").
- Until then the commands are bound word for word by `the_three_contents_hold_the_boards_lists_and_counts`.

## The engine calls each action needs

`TraySource` (`source.rs`) is the seam; `BoardFixture` stands in for the frame gate and for a run without an
engine. In the running app the source is the engine's (next section). What the engine lane implements:

| Control | Today | Needs |
| --- | --- | --- |
| The status block, the rows | the running app: `engine_host::EngineTraySource` (next section); the frame gate and a run without an engine: `TraySource::snapshot()` on the fixture, following the shell's `TrayInputs` (`frame_for`) | what "Not wired" below lists |
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

## The running app: what the flyout reads from the engine

`on_engine` puts `engine_host::EngineTraySource` in place of the fixture; `live.rs` holds the words. Every read
first takes what the engine queued (`LiveEngine::drain`): its reports of itself fold into one
`EngineStatusView`, which the b reads too, so whichever of the two reads first moves it for both, also while the
main window is closed.

| What | Engine call |
| --- | --- |
| Signed out, locked, the recovery phrase needed, the keys unreadable, ready | `Engine::gate` |
| Signed out because Beebeeb refused the session again and again (DW-1219) | `Accounts::active_account` → `auth_health.is_expired` |
| Paused (DW-1204) | `Engine::is_paused` |
| Sync runs on this PC | `Engine::is_syncing` |
| Checking (DW-1212), up to date (DW-1202: an `idle` that carries the time of a finished check, `last_tick_ok_at`), syncing with files left, bytes left and the batch bar (DW-1203), can't reach Beebeeb (DW-1206), stopped by an error on this PC | the engine's last `engine-status` report (`EngineEvent::Status` through `EngineStatusView::apply`; `PauseChanged`) |
| Changes waiting while paused, files given up (DW-1207), changes held for the quota (DW-1209) | `StateDb::queue_hold_summary` |
| Files that need a decision (DW-1205), files on their way | `drive_v2::model::sync_counts` |
| Storage almost full (DW-1208) | #322's `EngineDriveSource::storage` (`Engine::storage_info`): on a thread of its own, at most once a minute, shown only for the account it was read for |
| Rows on their way | `StateDb::newest_file_rows`, its uploading and downloading files, only while sync runs and reaches Beebeeb |
| Rows given up | `StateDb::unsent_changes`, group `gave_up` (a backup's file is counted, not listed) |
| Rows uploaded from and downloaded to this PC | `StateDb::list_recent_transfer_activity` |
| Rows moved to Trash on this PC | `StateDb::list_recent_local_activity` |
| No device's manifest in any list (LD-0113) | `backup_protect::BackupsTree::is_hidden` |
| A row's click | #322's `EngineFilesSource::show_in_file_explorer`, with its guard: refused while sync does not run |

The database is read only while the account's keys are in memory; signed out, locked or without the keys the
list says why it is empty, and nothing is read. A hidden flyout is not read at all (`Shell::refresh`): the click
that opens it reads it first, and a change that came while it opened is read once it shows.

The b reads the same report (`LiveEngine::tray_inputs`, `shell::tray`): checking while the engine runs and has
not finished a check (no report yet, its first check, after a resume, Beebeeb answering 401), not syncing while
sync does not run on this PC (also without the account's keys), and up to date only after an `idle` that
carries a finished check.

## States the board does not draw

| State | Title, line | When |
| --- | --- | --- |
| Sync isn't running | "Sync isn't running", "Files on this PC are unchanged." | signed in and unlocked, and the engine does not run on this PC (before its first start, after a start that failed): "Up to date" would be a guess (DW-1212) |
| Locked, sync off | "{name} is locked", "Nothing syncs while it's locked." | DW-1211 says "Sync keeps running while it's locked"; this engine stops while the keys are not in memory |
| The recovery phrase needed | "Recovery phrase needed", "Enter it in {name} to sync this PC." | signed in, and this PC holds no key of the account (the window's DW-A23) |
| The keys unreadable | "Can't read your keys", "Windows didn't answer. Nothing syncs until it does." | the credential store did not answer |
| Stopped by an error on this PC | "Sync stopped", no line | the engine's `engine_error` with nothing given up: DW-1207's count would be a guess |
| Recent changes hidden | "Recent changes are hidden", "Sign in to see file names." / "File names show once your keys are on this PC." | DW-1211's rule (no names), for the other states without the keys in memory |
| A photo's tile | the design system's image glyph | the flyout has no thumbnail from the engine |

Checking comes after signed out, paused and "sync isn't running" here: those three are known at once; checking
waits only for what the engine checks. The b keeps the same order, with "not syncing" for "Sync isn't running".

## Not wired in the running app

- The upload speed, the time left and per-file bars: the engine reports no rate, and the start path
  (`engine_host::open`) does not keep the engine's transfer board (`ChannelHost::transfers`).
- The file count of "Up to date": the engine counts files and folders together (`sync_counts`); the files alone
  would read every row on each refresh.
- A new version (DW-1210) and too old to connect (DW-1214): no update check in the native app.
- Adding to and couldn't add to File Explorer (DW-1215, DW-1216): Cloud Files registration is not wired, so the
  running app never shows them and Open in File Explorer stays disabled.
- No plan yet (DW-1213): `Engine::gate` makes no plan check.
- Waiting on a metered connection (DW-1222).
- Buttons that would do nothing are not drawn: Sync anyway, Try again (offline and File Explorer), Add storage,
  Choose a plan (no `Platform::open_url`), Restart to update, Unlock with Windows Hello, Sign in after the
  session ended (no re-sign-in prompt). Pause, Resume, Review…, Show and Sign in (signed out) are drawn.
- Rows from other devices and from people, restored files, kept local copies: the database records this PC's
  transfers and moves to Trash only.
- A row's click while sync does not run on this PC: #322's guard refuses it (logged, no name), and the flyout
  closes without showing the file.
- The engine's startup check of the files on this PC (`EngineStatusView::startup_check`, done of total): the
  flyout says Checking, without the numbers.
- The open flyout reads the engine again when the b's inputs change (`Shell::refresh`); between two such changes
  (one large file, the same count) its byte numbers are those of the last one.
- The b shows no conflicts, no full storage and nothing given up: those need the database, and its inputs are
  read on every frame. On an `idle` after a finished check it says up to date over them, while the flyout
  names them.
