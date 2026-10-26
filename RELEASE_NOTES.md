# Beebeeb Desktop 0.8.15 — Windows: sync no longer waits for the start-up check, and the check downloads nothing

In 0.8.14 every start of the Windows app began with a check of each file synced before 0.8.13, and sync
waited until that check was done. No edit was sent and no new file was picked up in the meantime. On a PC
with tens of thousands of such files the check ran for hours. It also opened files in a way that made
Windows download every cloud-only file it looked at, which could fill the disk. 0.8.15 runs the check
beside sync, reads each file's state without downloading it, and shows its progress. Settings › Sync gets
Pause and Resume, and the tray and Settings now say plainly when changes are not in sync. On macOS the only
change is that Selective sync no longer lists a folder twice.

**Known limitations** (unchanged from 0.8.14, and now shown in the app):
- An edit in a file from before 0.8.13 is still kept on this PC and not sent. Explorer keeps showing it as
  "sync pending" until the check against the server lands in a later release (task 2191).
- On Windows, changes that were held when a sign-in expired stay held. Home, the tray and Settings count them
  as not in sync. They are repaired in 0.8.16, which first matches them to what the server already has.
- A new file saved into a folder that this PC created back then, but never managed to create on the server,
  waits as well. The server answers "not found" for that folder until 0.8.16 repairs it. The app counts such
  a file as waiting, then as stopped after repeated errors.

### What's New

- **[Windows] Pause and Resume in Settings › Sync.**
  - The button sits beside the live sync state and sets the same pause as the tray and the app menu.
  - Covered by `tests/syncPauseControl.test.ts`.
- **[Windows] Home, the tray and Settings › Sync say what sync is doing.**
  - During the start-up check they show the check and its progress. Uploads made during the check are the
    headline.
  - Changes waiting to be sent, changes held back and changes that ran out of retries are counted, with
    their dates.
  - Pending work is never shown as "Up to date". Held changes read "Not in sync".
  - Home's strip used to say "Up to date · Syncing 0" next to thousands of unsent changes. It now shows the
    same state as Settings, next to a count of the changes not sent yet.
  - Covered by `tests/syncActivityModel.test.ts`, `tests/homeSyncMetrics.test.tsx` and
    `desktop_0815_changes_on_hold_are_counted_and_are_never_syncing`.

### Bug Fixes / Hardening

- **[Windows] Edits and new files are sent while the start-up check runs.**
  - **Before (0.8.14):** the app checked every file from before 0.8.13 first, then started sending changes.
    On a PC with 37,169 such files the check ran for more than two hours without finishing, and no edit
    was sent in that time.
  - **Now:** the check runs beside sync and reports its progress.
  - Verified by `desktop_0815_uploads_start_without_waiting_for_the_guard`,
    `desktop_0815_the_guard_reports_progress_after_every_row` and
    `desktop_0815_the_guard_stops_between_rows_and_keeps_what_it_found`.
- **[Windows] The start-up check never downloads a file.**
  - **Before (0.8.14):** the check opened files with read access, so Windows fetched each cloud-only file
    in full to answer it. On one PC that added up to about 42 GB, and the disk filled.
  - **Now:** the check reads a file's attributes only, and marks a file "not in sync" through a handle that
    asks for no data. A file that is not wholly on this PC is never opened to check it.
  - A native Windows probe counts the downloads: 0 after the check's reads, 0 after promotion, 0 after the
    mark. 0.8.14's way of opening the same file triggers 1.
  - Verified by `desktop_0815_probe_the_guard_reads_a_cloud_only_file_without_fetching_it`.
- **[Windows] 0.8.14's protection still holds while the check runs beside sync.**
  - Nothing that can free or remove a file's bytes runs until the check has been through every file:
    cache eviction, the in-sync stamping, and the retry of kept originals.
  - A remote deletion trusts a stored hash only when its version is known.
  - Verified by `desktop_0815_freeing_waits_for_the_start_up_pass`,
    `desktop_0815_cache_eviction_waits_for_the_start_up_pass`,
    `desktop_0815_the_tick_stamps_nothing_until_the_start_up_pass_is_done`,
    `desktop_0815_the_guard_never_clears_a_flag_set_while_it_ran` and
    `desktop_0815_a_remote_deletion_never_trusts_a_guarded_row_s_fetched_hash`.
- **[Windows] Ctrl+Shift+P no longer prints.**
  - WebView2's browser shortcuts are off in every app window: print, reload, find and DevTools.
  - Text editing keys are unchanged, and zooming still works through the View menu.
  - The Pause/Resume keyboard shortcut is now macOS-only. On Windows, pause from the tray, the menu or
    Settings.
  - Verified by `the_browser_keys_plugin_is_registered_before_any_window_is_built`,
    `no_windows_or_linux_menu_shortcut_is_a_web_view_browser_key_or_an_altgr_chord` and
    `pause_and_resume_keeps_its_shortcut_on_macos_only`.
- **Selective sync lists each folder once.**
  - A folder this PC created but never managed to create on the server used to be listed next to the
    server's folder at the same path, so the folder appeared twice, once empty.
  - The list now shows the server's folder only.
  - This is display only. Nothing is deleted, moved or resumed.
  - Verified by `selective_sync_shows_one_folder_per_path_preferring_the_server_row` and
    `per_path_merge_keeps_two_server_items_and_never_mixes_kinds`.
- **[Windows] Diagnostics.** When Windows asks the app for a file's bytes, the debug log records which
  program asked, by its file name only, never its path. Verified by
  `desktop_0815_a_requester_is_named_by_its_image_file_name_only`.

### Verification

- Test suite (Linux, release/0.8.15 at `9e5298f`):
  - `cargo test --locked` lib reported `test result: ok. 1689 passed; 0 failed; 7 ignored`. The integration
    binaries reported `keychain` 31, `windows_session_wiring` 8 and `windows_signout_cleanup` 3, all with 0 failed.
  - Frontend: `bun test` 1213 pass / 0 fail across 74 files. `bunx tsc --noEmit` and `bunx eslint .` both
    exit 0.
- Test suite (native Windows, a clean clone at `9e5298f`):
  - `cargo check --locked --all-targets` exit 0.
  - `cargo test --locked` lib reported `test result: ok. 1491 passed; 0 failed; 6 ignored`. `keychain` 31,
    `windows_session_wiring` 8 and `windows_signout_cleanup` 3, all with 0 failed.
  - The download probe above, run on Windows with output captured.
- Test suite (macOS, Apple Silicon, a fresh worktree at `9c98818`):
  - `cargo test --locked` lib reported `test result: ok. 2008 passed; 0 failed; 9 ignored`. `keychain` 31,
    `windows_session_wiring` 8 and `windows_signout_cleanup` 3, all with 0 failed.
  - `bun test` 1203 pass / 0 fail.
  - The only change after `9c98818` is the Windows Home fix: the Windows main window's frontend and its
    tests, with no Rust.
- macOS:
  - No macOS-only path changed since 0.8.14.
  - Of the shared code, three changes run on macOS:
    - `sync_status` carries two new fields, which only the Windows views read;
    - the Selective sync list shows one folder per path (see above);
    - the menu keeps its Pause/Resume shortcut (Cmd+Shift+P).
  - The start-up check never runs on macOS, so nothing there waits for it.
- Real hardware smoke test, run on the maintainer's Windows 11 PC, the one where 0.8.14 stalled.
  - **Build:** a local release build of `9c98818`, unsigned, run in place against the real sync folder and
    state on 2026-10-26 from 17:30 CET. 0.8.14 had stalled on this PC: more than two hours without
    finishing, about 42 GB downloaded, and a full disk.
  - **Start-up check:** it went through 31,809 files in 7 seconds. In 45 minutes the debug log shows 0
    fetch requests, 0 downloads and 0 errors.
  - **Edits and new files:** the first scan reached an edited Word document and a new spreadsheet in the
    sync folder 4.5 minutes after start. Both were sent within seconds: the edit as version 5, the new file
    as version 1.
  - **Disk:** free space on C: dropped 106 MB in 45 minutes. In the same time the debug log alone grew by
    350 MB.
  - **Settings › Sync:** it showed "5 changes waiting to sync", the 2,924 held changes and the 43 that
    stopped. Pause and Resume switched the state both ways.
  - **Home:** re-checked on the same PC with a build of `9e5298f`. The strip read "Starting · Not sent 2,972"
    (2,924 held, 43 stopped, 5 waiting). The `9c98818` build had shown "Up to date · Syncing 0" for the same
    queue, which is why the Home fix is in this release.
  - **Not checked on the device:**
    - Ctrl+Shift+P: synthetic keystrokes could not reach the window, and the maintainer did not try it by
      hand. It is covered by the tests above.
    - the tray popover.
- The macOS sign-out device checks that 0.8.12 shipped without were waived again for this release, by the
  maintainer (2026-10-26: "Waive for 0.8.15"). The blocker stays listed on main for later releases.
- Not verified:
  - sending an edit in a file from before 0.8.13 (a later release, task 2191);
  - resuming changes held since a sign-in expired (0.8.16).

### Install / Update

Existing desktop installs receive this release through the in-app updater automatically. For a
fresh Windows install, download the NSIS `setup.exe` from the GitHub release assets.
