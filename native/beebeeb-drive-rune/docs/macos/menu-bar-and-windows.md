# macOS: the menu bar, the popover and the windows (carry note C5)

The rules of the macOS menu bar, the menu-bar popover, the Beebeeb window and the launch
decision, whose only copy lives in `src-tauri/`, are carried here before the delete. This is the macOS phase's specification and
acceptance list for those surfaces.

- **Reference implementation:** the shipped tag `desktop-v0.8.25`, same paths. Seven carried files changed after the tag
  (`main_window_tests.rs`, `native_menu.rs`, `popover_data/snapshot.rs`, `popover_data/tests.rs`, `surfaces/policy.rs`,
  `surfaces/window_state.rs`, `tray_window.rs`); every test name of the tag is at the base, and the base has 13 more (named below).
- **Line numbers** are at `a57b9592` (v2-native, the delete batch's base). After the delete, read them with `git show a57b9592:<path>`.
- **Already in the engine:** the popover's phase reducer, `crates/beebeeb-engine/src/surfaces/phase.rs` (`PopoverPhase`, its
  precedence and tests), and the transfer board, `crates/beebeeb-engine/src/transfer_progress.rs`.
- **Windows counterparts** (not this note's): the tray `native/beebeeb-drive-rune/src/shell/tray.rs`, the flyout
  `shell/flyout.rs`, the window frame `shell/window_state.rs`, close-to-tray `shell/host.rs`, start at login `shell/autostart.rs`.
- **Specs** (dated records, they stay): `docs/specs/2026-10-24-macos-two-surface-app.md` (the two surfaces, F1–F15, §7, §9),
  `docs/specs/2026-09-30-macos-menubar-popover.md` (phases, anchoring, blur guard, storage), `docs/specs/2026-10-02-macos-settings-dialogs.md`.
- **Artefacts moved with this batch:** `packaging/macos/tray-template.png` and `packaging/macos/tray-template@2x.png` (the
  menu-bar template icon, shown as a template image so macOS tints it), `packaging/macos/icon.icns` (C6).

## 0. Rows carried here

| census row | section |
|---|---|
| §5 `src-tauri/src/native_menu.rs` (the macOS menu spec rows; the update loop and the Windows/Linux menu bar are not macOS) | 1 |
| §5 `src-tauri/src/main_window_tests.rs` | 1, 9 |
| §5 `src-tauri/src/menu_update_check_tests.rs` | 1, 9 |
| §5 `src-tauri/src/popover_data/constants.rs` | 2 |
| §5 `src-tauri/src/popover_data/snapshot.rs` | 2 |
| §5 `src-tauri/src/popover_data/storage.rs` | 2 |
| §5 `src-tauri/src/popover_data/tests.rs` | 2, 9 |
| `src-tauri/src/file_open_commands.rs` (its `popover_snapshot`; the rest of that row is C3) | 2 |
| §5 `src-tauri/src/surfaces/registry.rs` | 3, 9 |
| §5 `src-tauri/src/surfaces/policy.rs` (macOS rows; Windows rows moved at T0; Linux dropped by 2537) | 3, 9 |
| §5 `src-tauri/src/surfaces/anchor.rs` | 4, 9 |
| §5 `src-tauri/src/surfaces/blur_guard.rs` | 4, 9 |
| §5 `src-tauri/src/surfaces/window_state.rs` (mac points, mixed DPI) | 5, 9 |
| §5 `src-tauri/src/surfaces/failure.rs` | 6, 9 |
| §5 `src-tauri/src/surfaces/combined_tests.rs` | 6, 9 |
| §5 `src-tauri/src/surfaces/launch.rs` | 7, 9 |
| §5 `src-tauri/src/launch_wiring_tests.rs` | 7, 9 |
| §5 `src-tauri/src/tray_window.rs` (macOS openers; the tray rules moved at T0) | 3, 9 |
| §1 row 53 `src-tauri/src/tests.rs`: the 3 macOS window tests | 4, 9 |
| §5 git mv `src-tauri/icons/tray-template.png`, `src-tauri/icons/tray-template@2x.png` | header |

## 1. The macOS menu bar (two-surface spec §5.F F1–F15, §9.2)

Source: `MACOS_MENU_SPECS` and `MACOS_MENU_LAYOUT` in `src-tauri/src/native_menu.rs` (:300, :465). macOS title case, the one-character
ellipsis, one ⌘ per key. Each key is bound once; the legacy File › Settings and the duplicate items (F10, F11) are gone.

| menu | entries, top to bottom |
|---|---|
| Beebeeb | About Beebeeb · Settings… (⌘,) · Check for Updates… (⇧⌘U) · Pause Sync / Resume Sync (⇧⌘P) · — · Sign Out… (⇧⌘L) · — · Hide Beebeeb · Hide Others · Show All · — · Quit Beebeeb (⌘Q) |
| File | Open in Finder (⌘O) · Upload Files… (⌘U) · New Folder… (⇧⌘N) · — · Open Web App (⇧⌘O) |
| Edit | Cut · Copy · Paste · Select All (drawn by AppKit) |
| View | Home (⌘1) · — · Back (⌘[) · — · Zoom in (⌘=) · Zoom out (⌘−) · Actual size (⌘0) |
| Window | Minimize · Zoom · Close · — · Bring All to Front (drawn by AppKit) |
| Help | Documentation · Keyboard shortcuts (⌘/) · Report a problem · — · Service status |

Rules:
1. **F3, the pause label says what it will do:** "Pause Sync" while syncing, "Resume Sync" while paused, and it follows every writer
   of the pause (the menu, the tray, a settings write from the compact window's Network page). A pause sent with the settings pauses
   the engine as the menu's and the tray's do (the file, the engine flag, the status event).
2. **The window commands** (Settings…, Check for Updates…, Home, Back) open or ask the Beebeeb window: Settings… opens it on Settings;
   a menu update check opens it on About (`MainNav::settings(Some("about"))`) and is consumed once, by the Beebeeb window only
   (`main`, F2). View › Back only acts on an open window; it never opens one.
3. **Navigation requests:** one slot keeps the latest request (`Home`, `Back`, `Settings(section)`) and gives it to the Beebeeb window
   once; no other window can take it. The menu around the window follows §8.2 (`main_menu_state`).
4. **Quit Beebeeb is the menu's exit** (`app.exit(0)`); closing the last window does not quit (section 3).
5. The Windows and Linux menu bar (File/View/Help) has no native counterpart by design (the native Windows app has a tray menu and
   no menu bar); the update check loop (5 s after start, every 4 h) moved to `crates/beebeeb-updater` (`machine.rs:CHECK_INTERVAL_SECS`).

## 2. The menu-bar popover and its snapshot

Sources: `src-tauri/src/popover_data/{constants,snapshot,storage}.rs`, `popover_snapshot` (`src-tauri/src/file_open_commands.rs:506`).

1. **One snapshot call per frame** (`popover_snapshot`): read, in order, the session (locked / signed out / session ended), the live
   pause flag, the engine's last status (state, progress, last check that reached the server, reason), the Finder setup (the
   reconciler's published view; it never asks the OS here), the storage summary, and the state DB (queue, recent transfers,
   conflicts). File names are read only while the vault is unlocked. The storage wait and the DB read run side by side.
2. **One phase, one reducer:** the popover body is `popover_phase` (engine `surfaces/phase.rs`), first match wins: signed out,
   session ended, locked, Finder failed, Finder turned off in System Settings, Finder adding, Finder missing, paused, storage full,
   offline, error, syncing, synced. A file on the transfer board is syncing even when the last event said idle. A quota-paused upload
   is storage full even when the usage call failed; a zero or negative quota is unknown, not full. A 401 is not a connectivity phase.
   A failure shown on another surface does not paint state f2 here. The sync part alone (the Finder taken as ready) is the same
   reducer, so the Beebeeb window keeps no second copy of the precedence (task 2179).
3. **The Finder reason line** (state f2): `reason: <typed reason>`, at most 30 characters, one line, only for a failure with a reason;
   the reason is the reconciler's typed vocabulary, never OS or Rust free text.
4. **Activity rows:** active first, then failed, then done, each with a direction (failed rows have none); at most
   `DEFAULT_ACTIVITY_LIMIT` = 8 by default, clamped to 1..=`MAX_ACTIVITY_LIMIT` (20); an active row without board bytes shows no
   percentage, never a guess; a file moving again is not listed twice; a file uploaded twice is one finished row, the newest. A row
   splits into name, folder name (the last folder, `""` at the root) and the full path for a tooltip.
5. **Conflicts:** all are counted, `CONFLICT_FILES_LIMIT` = 5 are named, oldest first (nearest 24 h deadline); Review opens the first.
   Pending changes are counted from the queue as of now; an upload in retry backoff is not due.
6. **The storage summary cache** (spec §9, task 2179): trusted for `STORAGE_TTL_SECS` = 300 s and refetched after an unlock (a new
   in-memory session), never on every open; a snapshot waits for a fetch at most `STORAGE_WAIT` = 1 s, then answers from the cache
   (an older number marked stale, or nothing: the header shows the email only); a failed fetch is not tried again for
   `STORAGE_FAILURE_BACKOFF_SECS` = 30 s, dated when the failure came. One fetch per session at a time (by ticket); a late answer from
   an older session, or from before a sign-out's clear, answers into nothing; a panicking fetch frees the slot. The fetch is the
   billing usage call with the session token; a 401 is an error that does not carry the token. A sign-out clears the summary.
7. **Events:** `popover-shown` (the UI refreshes its snapshot) and `popover-hidden` (it stops reading); the popover is hidden, never
   destroyed.
8. **During a sign-out's send phase,** a failed read of the waiting changes leaves the count out of the snapshot instead of failing it
   (#160 review P3-5).

## 3. Exactly two surfaces, and how windows behave on a Mac

Sources: `src-tauri/src/surfaces/registry.rs`, `surfaces/policy.rs`, the macOS openers in `tray_window.rs` and `update_window.rs`
(`show_main_window` :825, `unhide_app` :862, `reopen_main_window` :870, `set_macos_dock_icon` :1075, `macos_window_lifecycle` :1165).

1. **Two kinds, one window each:** the popover under the status item and the Beebeeb window (`main`). Every other label is refused.
   Asking for a live kind shows it, never a second window.
2. **Visibility:** opening the Beebeeb window hides the popover (hidden, not destroyed); opening the popover leaves the window where
   it is. At most two are live, and the popover never hides the window.
3. **Close:** the Beebeeb window is destroyed on close (its web view freed, rebuilt on open; it keeps its frame, section 5); the
   popover and every other window hide. Closing something not open does nothing.
4. **The app outlives its windows:** a Mac keeps running when its last window closes and quits only when asked (an exit request with
   a code). Reopen (Dock, Finder, Spotlight) brings the window forward, or opens it on Home.
5. **The Dock icon follows the Beebeeb window** (§9.1): Regular (a Dock icon and the app menu) while it exists, Accessory (neither)
   otherwise; applied only when it changes; turned on before the window shows, the app activated after. A launch that opens no
   window switches to Accessory itself.
6. **Hide:** the tray's Hide hides the app on a Mac (⌘H too); every opener unhides the app first, because a window shown in a hidden
   app stays hidden.
7. **Sign-in happens in the Beebeeb window** on a Mac; the separate onboarding window is never built there.
8. **A conflict auto-opens a window** on a Mac (today's row).
9. Every opener of the old compact window on a Mac opens the Beebeeb window instead.

## 4. Where the popover and Settings open (anchoring, the blur guard)

Sources: `src-tauri/src/surfaces/anchor.rs`, `surfaces/blur_guard.rs`; the macOS window constants in `src-tauri/src/tests.rs`.

1. **Logical points only,** top-left origin, as AppKit reports `NSScreen.frame` / `visibleFrame` flipped; no scale factor appears in
   the placement (the caller converts).
2. **The icon's display:** the icon's CENTRE against each display's FULL frame (the menu bar is outside the visible frame), half-open,
   so a centre on a shared edge belongs to the display on the right or below; a centre on no display uses the display it overlaps
   most; no overlap is "unavailable". On two displays the external one is picked correctly (the rejected naive approach is documented
   by a test).
3. **Popover:** `POPOVER_WIDTH` 372 × `POPOVER_HEIGHT` 488, centred under the icon, `ICON_GAP` 6 below it, clamped into the visible
   frame with `SCREEN_MARGIN` 8 left, right and bottom; the top clamps to the visible frame itself (under the notch); height (and
   width) capped at visible size minus two margins; a screen narrower than the popover keeps its top-left on screen. A display left of
   the primary has a negative origin.
4. **Unavailable icon display:** the top right of the PRIMARY display's visible frame; with no primary flag, the first display;
   never centred, never a remembered position. No displays: no placement.
5. **Settings** (`SETTINGS_WIDTH` 560): the same function anchored top right on the icon's display, `ICON_GAP` under the menu bar, its
   top edge level with the popover's; its height capped to the visible frame. The 0.8.x macOS settings window constants were 680 × 540;
   with an anchor the window is a popover, without one a regular window.
6. **The blur guard** (spec §6 rule 4): a tray click within `BLUR_CLICK_GUARD_MS` = 250 ms (exclusive) after a blur-hide stays closed;
   one blur suppresses one click; a click on a visible popover hides it and clears a stale blur; a clock that steps backwards counts
   as the same gesture. The Windows flyout has its own stateless rule (`shell/flyout.rs`).

## 5. Where the Beebeeb window opens (§7.2)

Source: `src-tauri/src/surfaces/window_state.rs`; the app side in `update_window.rs` (`main_frame_path` :992, `save_main_window_frame`
:1022, `place_main_window` :1053, `macos_displays` :940).

1. **First open:** centred on the display that holds the menu-bar icon, at the window's own size (920 × 620, minimum 760 × 520,
   overlay title bar with a hidden title; C6 has the bundle values).
2. **After that:** where it was left, at the size it was left; never its visibility, never full screen or zoomed (a full-screen or
   zoomed window keeps no frame, so the last ordinary frame stays).
3. **Off-screen guard:** at least half of the kept frame on a connected display keeps it (exactly half counts); less is centred again
   at its kept size on the icon's display, else the primary; a frame under the menu bar counts as off the work area; a window larger
   than the work area keeps its title bar on screen.
4. **Logical points per display:** a display is its physical frame over its own scale; the icon names its display on a mixed-DPI desk
   whichever display holds it; an icon no display claims, or two claim, is unknown; a bad scale counts as 1. The frame is kept in
   `main-window-frame.json` (logical points) in the config directory, written when the window closes and when the app quits; anything
   that is not a frame is no frame. The window-state plugin is NOT used on a Mac (it stores physical numbers without their scale).

## 6. One failure, one surface (spec §6, §10)

Sources: `src-tauri/src/surfaces/failure.rs`, `surfaces/combined_tests.rs`.

1. A Finder failure is never a toast and never on two surfaces: it shows where it started while that surface is visible (the popover's
   state, or the Settings row), else on the one surface that is.
2. Gating failures are inline and never a toast or a notification; each has its one home (the whole engine: the popover state; a file:
   its activity row).
3. A transient failure that gates nothing is a toast in the surface where it started, fires once, is not raised again on the next
   render, and a stale Settings toggle failure does not come back as a toast in the popover. Taking toasts leaves every other failure.
4. A background event is a native notification, never a toast.
5. A new attempt or a success clears the failure; clearing an unknown id is harmless; distinct failures each get one surface.
6. Combined with the registry and the phase reducer: every reachable registry state shows a Finder failure exactly once, and the
   popover is f2 only when it is the one surface showing it.

## 7. How the app was launched (§7.3), and the relaunch flag

Sources: `src-tauri/src/surfaces/launch.rs`, `launch_wiring_tests.rs`, `update_window.rs` (`launch_source_now` :889,
`note_updater_relaunch` :906).

| launch | the Beebeeb window |
|---|---|
| manual (Finder, Dock, Spotlight), first launch after install | opens: sign-in with no account, else Home |
| login item | nothing: the menu-bar icon only |
| updater relaunch | what the person had open: the window comes back if it was open and they were signed in; sign-in if signed out |

1. **The source:** the login item's signal (one seam, `login_item_signal`; until task 1845 it says nothing, so every non-updater
   launch is manual) and the relaunch flag. A fresh flag makes an updater relaunch and wins over the login signal; a stale flag is not
   an update.
2. **The flag:** `relaunch.json` in the directory it is handed, holding exactly `signed_in`, `window_open`, `written_at` (nothing that
   identifies anyone); fresh for `RELAUNCH_FLAG_MAX_AGE_SECS` = 120 s to the second (a flag from the future is not fresh); read once
   and removed, stale or not; a missing directory or file, or a file that is not a flag, is no flag (and is removed); writing again
   replaces it. The updater leaves it just before every restart; a failure is logged and the restart goes on.
3. **Only two restarts exist,** both after leaving the flag; a third is a relaunch with no flag and must be decided on purpose.
4. **The login item mechanism** is the LaunchAgent the autostart plugin installs; task 1845 changes it on purpose.
5. Windows and Linux ignore the source (their start rows are their own; the native Windows start at login is `shell/autostart.rs`).

## 8. Not carried (and why)
- The popover's review event (`review:open` payload) and the shared JSON fixtures of the snapshot are the web view's wire shape
  (§5: the four `popover-snapshot.*.json` / `review-open.conflict.json` fixtures drop). Their tests are listed below as TAURI-ONLY.
- The Windows and Linux rows of `surfaces/policy.rs` and the Windows/Linux frame of `surfaces/window_state.rs` (the Tauri `main-app`
  shell). The native Windows app has its own window frame (`shell/window_state.rs`, e.g. `maximized_is_carried_and_no_displays_means_defaults`,
  `saving_keeps_the_normal_rectangle_while_maximized_or_minimized`) and close-to-tray (`shell/host.rs`). Linux is dropped by 2537.
- **Finding (Windows, not macOS; found while carrying):** the Tauri shell fitted the Beebeeb window's DEFAULT size to the display's
  work area minus a 24 pt margin (`FIT_MARGIN`), and the minimum followed (the four `fit_*` tests of 9.12). The native window opens
  at `MAIN_SIZE` 1280 × 800 points (`native/beebeeb-drive-rune/src/shell/surfaces.rs:39`, `shell/mod.rs:359`) and only a SAVED size
  is cut to its display (`shell/window_state.rs:80`, `size_for`; pinned by `a_window_larger_than_its_display_is_cut_to_it`). On a
  1366 × 768 display at 125 % the work area is about 1093 × 582 points, so a first open is larger than the screen. Not built here
  (outside this note's area); reported to the delete batch's checkpoint.

## 9. Acceptance list: every test of the carried files

Class per line: carried unless marked **TAURI-ONLY** (web-view or Tauri wiring), **MOVED** (with its native pin), **pinned** (the
engine or native test that already holds it), or **Linux, dropped (2537)**.

### 9.1 `src-tauri/src/launch_wiring_tests.rs` (6)
- `startup_asks_the_policy_with_how_the_app_was_launched_and_handles_every_answer`: the start asks the policy with the launch source and handles every answer.
- `a_launch_that_opens_nothing_takes_the_dock_icon_and_the_app_menu_away`: a launch that opens no window switches to Accessory itself.
- `how_the_app_was_launched_comes_from_the_one_seam_and_the_flag_and_from_nothing_else`: the source is the login seam plus the consumed flag only.
- `the_updater_leaves_its_flag_before_every_restart_and_there_are_only_these_restarts`: two restarts, each after the flag.
- `opening_the_app_again_asks_reopen_action`: reopen asks `reopen_action`; no window opens Home.
- `the_login_items_mechanism_is_the_one_it_was_and_task_1845_changes_it_on_purpose`: one LaunchAgent autostart, unchanged until task 1845.

### 9.2 `src-tauri/src/main_window_tests.rs` (18)
- `a_request_serialises_as_the_window_reads_it`: the navigation request's JSON (`{ "to": "home" }` …). Wire shape **TAURI-ONLY**; the request kinds are carried (1.3).
- `the_slot_keeps_the_latest_request_and_gives_it_to_the_beebeeb_window_once`: latest wins, given to `main` once, no other window takes it.
- `the_menu_around_the_window_follows_section_8_2`: the menu state around the window.
- `the_pause_item_says_what_it_will_do`: "Pause Sync" / "Resume Sync". Windows tray label **pinned** by `native/beebeeb-drive-rune/src/shell/tray.rs:paused_turns_pause_into_resume`.
- `the_pause_label_follows_every_writer_of_the_pause`: menu, tray and settings writes all move the label.
- `every_pause_writer_refreshes_the_menu_label`: each writer calls the refresh.
- `a_pause_sent_with_the_settings_pauses_the_engine`: **pinned** by `crates/beebeeb-engine/tests/host_contract.rs:a_host_restores_signs_in_unlocks_syncs_lists_and_restarts`.
- `a_bad_kept_frame_file_is_no_frame_and_the_window_opens_at_its_default_place`: **pinned** for Windows by `shell/window_state_tests.rs:nonsense_is_ignored`; on a Mac, section 5.4.
- `the_kept_frame_is_written_whole_or_not_at_all`: **pinned** for Windows by `shell/window_state_tests.rs:the_file_round_trips_and_survives_damage`; on a Mac, no temporary file is left.
- `the_menu_keeps_its_four_items_from_the_table_without_a_lookup_that_can_fail`: the items come from the table, no lookup that can fail.
- `a_route_named_opener_asks_for_settings_or_home`: a route name opens Settings or Home.
- `the_macos_items_have_the_words_keys_and_targets_of_f1_to_f11`: every macOS item's label, key and action (section 1).
- `the_zoom_and_help_items_keep_their_words_f12_f13`: zoom and Help words unchanged.
- `macos_binds_each_key_once_and_drops_the_legacy_items_f10_f11`: no key bound twice; F10/F11 gone.
- `the_macos_menu_bar_is_the_order_of_section_9_2_and_f15`: the menus and entries in the order of section 1.
- `every_macos_item_is_in_the_menu_bar_exactly_once_and_every_entry_has_an_item`: placement is a bijection with the table.
- `the_installed_table_is_the_platforms_own`: a Mac installs the macOS table ("Settings…").
- `the_window_commands_are_registered_once`: **TAURI-ONLY** (command registration).

### 9.3 `src-tauri/src/menu_update_check_tests.rs` (2)
- `pending_check_survives_other_windows_and_is_consumed_once`: other windows do not consume the pending check; it is consumed once.
- `menu_update_check_targets_the_beebeeb_window`: on a Mac the check targets `main` (F2).

### 9.4 `src-tauri/src/popover_data/tests.rs` (43; one, the last, is base-only)
- `every_phase_is_reachable_from_the_inputs_the_command_gathers`: each phase is reachable from the gathered inputs.
- `a_file_on_the_board_is_syncing_even_when_the_last_event_said_idle`: board beats the last event; paused still wins.
- `a_quota_paused_upload_is_storage_full_even_when_the_usage_call_failed`: storage full without the usage call.
- `a_zero_quota_is_unknown_not_full`: zero or negative quota is unknown.
- `a_failure_shown_elsewhere_does_not_paint_f2_here`: the popover shows Finder missing, not f2.
- `the_sync_phase_is_the_phase_with_the_finder_taken_as_ready`: one reducer for both (task 2179).
- `a_401_error_is_not_a_connectivity_phase`: a 401 does not make offline.
- `the_healthy_snapshot_serializes_to_exactly_the_shared_fixture`: **TAURI-ONLY** (wire fixture, dropped).
- `the_error_snapshot_serializes_to_exactly_the_shared_fixture`: **TAURI-ONLY** (wire fixture, dropped).
- `the_syncing_snapshot_serializes_to_exactly_the_shared_fixture`: **TAURI-ONLY** (wire fixture, dropped).
- `the_reason_is_omitted_when_the_link_is_fine`: no reason while the link is fine.
- `the_finder_failure_carries_the_reason_line`: setup failed, reason, `reason: timeout`.
- `pending_changes_and_conflicts_are_reported`: counts reach the snapshot.
- `only_a_handful_of_conflicts_are_named_but_all_are_counted`: 5 named, all counted.
- `rows_are_active_first_then_failed_then_done_with_direction_each`: the row order and directions.
- `an_active_row_without_board_bytes_has_no_percentage_to_show`: no guessed percentage.
- `a_finished_transfer_of_a_file_moving_again_is_not_listed_twice`: one row per file.
- `a_file_uploaded_twice_is_one_finished_row_the_newest`: the newest finished row.
- `the_list_is_capped_and_the_cap_is_clamped`: cap honoured; a zero limit still returns one row.
- `a_row_path_splits_into_name_folder_name_and_path`: name, last folder, full path; root rows have an empty folder.
- `the_finder_reason_line_adds_the_typed_reason_and_fits_30_characters`: the line format.
- `every_reason_line_fits_30_characters`: every reason fits.
- `a_fresh_summary_is_served_without_fetching`: served from the cache.
- `the_summary_is_refetched_after_the_ttl`: refetched at the TTL.
- `a_new_session_refetches_even_inside_the_ttl`: an unlock refetches.
- `a_failed_refetch_serves_the_old_number_marked_stale`: stale old number.
- `a_failed_fetch_with_nothing_cached_is_no_summary_and_another_sessions_is_not_reused`: no summary; never another session's.
- `a_slow_fetch_holds_a_snapshot_for_the_wait_at_most_and_serves_the_next_one`: waits exactly the wait; the late answer serves the next snapshot.
- `the_wait_is_the_callers_a_test_passes_its_own`: the wait is a parameter.
- `a_failed_fetch_is_not_tried_again_for_the_backoff`: no retry inside the backoff.
- `a_late_answer_from_an_older_session_does_not_replace_the_newer_ones_summary`: older answers land nowhere.
- `a_late_answer_from_an_older_session_keeps_the_newer_ones_backoff`: and do not clear the newer backoff.
- `a_fetch_still_running_at_sign_out_answers_into_nothing_even_after_a_new_fetch_started`: a sign-out's clear wins.
- `a_fetch_task_that_panics_frees_the_slot_for_the_next_snapshot`: a panic frees the slot.
- `a_failure_is_dated_when_it_comes_not_when_the_fetch_started`: the backoff starts at the failure.
- `the_storage_fetch_calls_billing_usage_with_the_bearer_token_and_parses_it`: the usage call with the bearer token, parsed.
- `a_401_from_the_usage_call_is_an_error_that_does_not_carry_the_token`: the error names 401, never the token.
- `the_review_event_is_camel_case_like_the_url_it_replaces`: **TAURI-ONLY** (web-view event).
- `a_review_event_with_no_file_id_is_refused_and_a_missing_name_gets_the_window_default`: **TAURI-ONLY** (web-view event).
- `the_snapshot_reads_the_queue_as_of_now_and_skips_an_upload_in_retry_backoff`: only due uploads count.
- `a_real_state_db_becomes_the_snapshot_the_popover_shows`: a seeded state DB becomes the snapshot (paused-for-quota is not due).
- `an_empty_state_db_is_an_empty_snapshot_not_an_error`: empty, not an error.
- `a_failed_count_while_sign_out_sends_leaves_the_count_out` (base only, #160): a failed read during the send phase leaves the count out.

### 9.5 `src-tauri/src/surfaces/anchor.rs` (22)
- `centres_under_the_icon_with_the_gap`: centred, 6 pt below, 372 × 488.
- `clamps_on_the_left_with_the_margin`: left margin 8.
- `clamps_on_the_right_with_the_margin`: right margin 8.
- `clamps_at_the_bottom_with_the_margin`: bottom margin 8.
- `clamps_at_the_top_to_the_visible_frame_under_the_notch`: top at the visible frame.
- `height_is_capped_at_the_visible_height_minus_two_margins`: min(488, visible − 16).
- `a_screen_narrower_than_the_popover_keeps_the_top_left_on_screen`: width capped, left at the margin.
- `icon_is_matched_against_the_full_frame_not_the_visible_frame`: full frame match.
- `ambiguous_two_display_case_picks_the_external_display`: the external display, not the naive answer.
- `a_centre_exactly_on_the_shared_edge_belongs_to_the_right_display`: half-open edges.
- `an_icon_straddling_two_displays_goes_with_its_centre`: the centre decides.
- `a_display_left_of_the_primary_has_a_negative_origin`: negative x placement.
- `a_centre_off_every_display_uses_the_largest_overlap`: largest overlap.
- `no_icon_falls_back_to_the_top_right_of_the_primary_display`: the fallback.
- `an_icon_on_no_display_falls_back_to_the_primary_not_the_centre`: never centred.
- `with_no_primary_flag_the_first_display_is_the_fallback`: the first display.
- `no_displays_means_no_placement`: none.
- `settings_opens_top_right_under_the_menu_bar_on_the_icons_display`: Settings on the icon's display.
- `settings_top_edge_lines_up_with_the_popover`: same top, same right edge.
- `settings_without_an_icon_display_falls_back_to_the_primary_top_right`: the fallback.
- `settings_height_is_capped_to_the_visible_frame`: capped.
- `sweep_every_placement_stays_inside_the_margins_and_never_centres`: a sweep of icon positions stays inside the margins.

### 9.6 `src-tauri/src/surfaces/blur_guard.rs` (6)
- `a_click_right_after_a_blur_hide_does_not_reopen`: stays hidden. The Windows core rule is **pinned** by `shell/flyout.rs:a_click_on_the_b_toggles_and_the_closing_click_does_not_reopen`.
- `the_window_is_250_ms_exclusive`: 250 ms, exclusive.
- `with_no_blur_a_click_opens`: shows.
- `one_blur_suppresses_one_click`: one click only.
- `a_click_on_a_visible_popover_hides_it_and_clears_a_stale_blur`: hide, then a later click shows.
- `a_clock_that_steps_backwards_counts_as_the_same_gesture`: stays hidden.

### 9.7 `src-tauri/src/surfaces/combined_tests.rs` (4)
- `the_popover_and_the_ledger_never_show_a_finder_failure_twice`: at most one, and at least one, surface.
- `a_settings_origin_failure_is_not_repeated_in_the_popover_when_both_are_visible`: the Settings row only.
- `the_popover_is_f2_when_it_is_the_one_surface_showing_the_failure`: f2 then.
- `every_reachable_registry_state_shows_a_finder_failure_exactly_once`: 31 reachable states, each exactly once.

### 9.8 `src-tauri/src/surfaces/failure.rs` (14)
- `screenshot_one_two_sources_for_one_finder_failure_render_one_surface`: two sources, one surface.
- `a_finder_failure_is_never_a_toast_and_never_in_two_surfaces`: never a toast, never two.
- `a_finder_failure_shows_where_it_started_while_that_surface_is_visible`: the origin surface.
- `gating_failures_are_inline_and_never_a_toast`: never a toast or a notification.
- `each_gating_failure_has_its_one_home`: the engine on the popover state, a file on its row.
- `a_transient_failure_that_gates_nothing_is_a_toast_in_the_surface_where_it_started`: a toast where it started.
- `a_stale_settings_toggle_failure_does_not_come_back_as_a_toast_in_the_popover`: not in the popover.
- `a_toast_fires_once_and_is_not_raised_again_on_the_next_render`: once.
- `taking_toasts_leaves_every_other_failure_alone`: the gating failure stays.
- `a_background_event_is_a_native_notification_never_a_toast`: a notification.
- `a_new_attempt_clears_the_failure`: cleared.
- `success_clears_the_failure_and_clearing_an_unknown_id_is_harmless`: cleared; unknown id harmless.
- `distinct_failures_each_get_one_surface`: one each.
- `exhaustive_no_failure_ever_has_two_surfaces_across_every_kind_and_visibility`: 6 × 3 × 8 combinations, never two.

### 9.9 `src-tauri/src/surfaces/launch.rs` (17)
- `with_no_signal_and_no_flag_a_launch_is_a_manual_one`: manual.
- `the_login_items_signal_makes_a_login_launch`: login item.
- `a_fresh_flag_makes_an_updater_relaunch_and_it_wins_over_the_login_signal`: updater relaunch wins.
- `a_stale_flag_is_not_an_update_and_the_launch_is_what_the_rest_says`: stale flag ignored.
- `freshness_is_the_age_limit_to_the_second_and_a_flag_from_the_future_is_not_fresh`: 120 s inclusive; future is stale.
- `a_manual_launch_opens_the_window_and_the_first_launch_opens_sign_in`: the manual rows.
- `a_login_launch_shows_nothing_set_up_or_not`: nothing.
- `an_updater_relaunch_shows_what_the_person_had_open_and_nothing_else`: what was open.
- `windows_and_linux_ignore_how_they_were_launched`: their rows. Windows: **MOVED** (native start is `shell/autostart.rs`, pinned by `this_programs_own_value_is_written_rewritten_and_deleted`); Linux, dropped (2537).
- `opening_the_app_again_brings_the_window_forward_or_opens_it`: forward or Home.
- `a_flag_written_is_the_flag_read_and_it_is_read_once`: round trip, read once.
- `a_stale_flag_is_consumed_too_so_it_cannot_surface_later`: consumed.
- `a_missing_directory_or_file_is_no_flag_and_no_panic`: no flag.
- `a_file_that_is_not_a_flag_is_no_flag_and_is_removed`: removed.
- `the_flag_holds_three_facts_and_nothing_that_identifies_anyone`: exactly the three keys.
- `writing_the_flag_again_replaces_it`: replaced.
- `until_task_1845_the_login_item_says_nothing`: no signal.

### 9.10 `src-tauri/src/surfaces/policy.rs` (35)
- `linux_still_opens_settings_after_startup`: Linux, dropped (2537).
- `linux_first_run_still_opens_onboarding`: Linux, dropped (2537).
- `windows_startup_is_unchanged`: **TAURI-ONLY** (Tauri window labels; the native Windows start is its own).
- `macos_manual_launch_keeps_todays_rows`: a manual launch opens the window, sign-in on first run.
- `macos_main_window_is_the_beebeeb_window`: the label is `main`.
- `the_compact_window_keeps_its_label_on_macos_and_linux`: **TAURI-ONLY** (label); the macOS rule is carried by `every_macos_opener_of_the_compact_app_now_opens_the_beebeeb_window`.
- `linux_conflict_still_opens_a_window`: Linux, dropped (2537).
- `windows_conflict_still_opens_a_window`: **TAURI-ONLY** (Windows window label rule).
- `macos_conflict_is_todays_behaviour_until_slice_6`: a conflict auto-opens a window on a Mac.
- `windows_and_linux_hide_every_window_on_close`: **MOVED** for Windows (close goes to the tray: `shell/host.rs:on_close`, pinned by `shell/host_tests.rs:a_tray_whose_icon_was_not_added_is_not_a_way_back`, `closing_the_window_without_a_b_has_nowhere_to_come_back_from`); Linux, dropped.
- `macos_destroys_the_beebeeb_window_on_close_and_hides_everything_else`: destroy `main`, hide the rest.
- `windows_and_linux_hide_every_label_even_when_the_macos_switch_is_on`: **MOVED** / dropped, as above.
- `with_the_switch_off_macos_hides_every_label_as_before`: with the label-aware close switch off, a Mac hides every window.
- `the_dock_icon_follows_the_beebeeb_window_and_the_legacy_decorated_windows`: the Dock icon rule.
- `a_mac_signs_in_in_the_beebeeb_window_and_windows_and_linux_keep_their_own`: sign-in surface per platform.
- `a_mac_keeps_running_when_its_last_window_closes_and_quits_when_asked`: keep running; quit on a coded request.
- `the_tray_hide_hides_the_app_on_a_mac_and_the_old_window_elsewhere`: Hide hides the app on a Mac.
- `current_platform_matches_the_build_target`: **TAURI-ONLY** (helper).
- `the_global_close_handler_asks_close_policy_and_no_longer_hides_unconditionally`: the close handler asks the policy.
- `the_compact_window_takes_its_label_from_compact_window_label`: **TAURI-ONLY** (label wiring).
- `the_beebeeb_window_takes_its_label_from_main_window_label`: the Beebeeb window's openers ask the policy.
- `every_macos_opener_of_the_compact_app_now_opens_the_beebeeb_window`: every compact opener on a Mac opens `main`.
- `the_run_loop_asks_keep_running_on_exit_request_and_prevents_only_then`: an exit request is prevented only when the policy says so.
- `a_reopen_shows_the_beebeeb_window`: reopen shows `main`.
- `opening_a_window_on_a_mac_unhides_the_app_first`: every show unhides first.
- `the_tray_hide_item_asks_tray_hide`: the tray's Hide asks the policy.
- `opening_the_window_turns_the_dock_icon_on_before_it_shows_and_activates_after`: Dock icon, show, activate, in that order.
- `the_global_window_handler_runs_the_macos_lifecycle_for_every_event`: every window event runs the macOS lifecycle.
- `the_windows_main_app_shell_takes_its_label_from_main_window_label`: **TAURI-ONLY** (the Windows web-view shell).
- `startup_and_the_conflict_handler_still_go_through_policy`: the start and the conflict handler ask the policy.
- `the_sign_in_openers_route_through_onboarding_surface_and_a_mac_never_builds_the_onboarding_window`: never builds onboarding on a Mac.
- `quit_beebeeb_is_the_menus_exit`: the menu's exit is `exit(0)`.
- `the_helper_finds_what_it_claims_to`: **TAURI-ONLY** (test helper).
- `the_settings_menu_action_opens_the_beebeeb_window_on_settings_on_macos`: Settings… opens `main` on Settings.
- `the_menu_update_check_opens_the_beebeeb_window_on_about_on_macos`: the menu check opens `main` on About.

### 9.11 `src-tauri/src/surfaces/registry.rs` (11)
- `exactly_two_labels_and_they_round_trip`: `["popover", "main"]`.
- `every_other_label_is_refused`: refused.
- `opening_a_kind_twice_yields_one_window`: create, then show.
- `a_hidden_popover_is_shown_again_not_rebuilt`: hidden, live, shown again.
- `the_beebeeb_window_is_destroyed_on_close_and_rebuilt_on_open`: destroyed, rebuilt.
- `closing_something_that_is_not_open_does_nothing`: nothing.
- `opening_the_beebeeb_window_hides_the_popover`: hidden, not destroyed.
- `opening_the_popover_leaves_the_beebeeb_window_where_it_is`: both visible.
- `at_most_two_surfaces_exist_and_the_window_is_never_hidden_by_the_popover`: over every sequence.
- `visible_reports_exactly_what_is_showing`: exact visibility.
- `close_policy_table`: popover hides, `main` is destroyed.

### 9.12 `src-tauri/src/surfaces/window_state.rs` (22; the last 8 are base-only, the Tauri Windows/Linux frame, not macOS)
- `a_display_is_its_physical_frame_over_its_own_scale`: per-display logical frames.
- `the_icon_names_its_display_on_a_mixed_dpi_desk_on_either_display`: mixed DPI.
- `one_display_holding_the_icon_names_it_whatever_the_icon_height`: one holder.
- `off_screen_recovery_keeps_the_kept_size`: centred at the kept size.
- `an_icon_no_display_claims_or_two_displays_claim_is_unknown`: unknown.
- `first_open_centres_on_the_icon_display_in_points_whichever_display_holds_it`: first open.
- `a_kept_frame_on_the_external_comes_back_exactly_as_it_was_left`: exact.
- `exactly_half_on_screen_is_kept_and_less_than_half_is_centred_again`: the half rule.
- `a_frame_on_a_display_that_was_unplugged_is_centred_on_the_icon_display`: unplugged display.
- `a_full_screen_or_zoomed_window_keeps_no_frame`: none kept.
- `under_the_menu_bar_counts_as_off_the_work_area`: off.
- `a_window_larger_than_the_work_area_keeps_its_title_bar_on_screen`: title bar on screen.
- `the_file_round_trips_and_anything_else_is_no_frame`: round trip; garbage is none.
- `no_displays_place_nothing_and_a_bad_scale_counts_as_one`: none; scale 1.
- `fit_1920x1080_at_100_keeps_the_board_size`: 1920 × 1080 at 100 %: the board size stays. **No native copy found** (finding, section 8); Linux dropped.
- `fit_1366x768_at_125_shrinks_and_the_minimum_follows`: 1366 × 768 at 125 %: the default shrinks to the work area minus the margin and the minimum follows. **No native copy found** (finding, section 8); Linux dropped.
- `fit_1280x720_at_150_fits_inside_the_margin`: 1280 × 720 at 150 %: the window and its minimum fit inside the 24 pt margin. **No native copy found** (finding, section 8); Linux dropped.
- `fit_portrait_1080x1920_shrinks_the_width_only`: portrait 1080 × 1920: only the width shrinks. **No native copy found** (finding, section 8); Linux dropped.
- `a_kept_frame_comes_back_and_a_maximized_flag_survives`: **MOVED**, native pin `shell/window_state_tests.rs:maximized_is_carried_and_no_displays_means_defaults`.
- `an_off_screen_kept_frame_falls_back_to_the_fitted_default`: **MOVED** for the off-screen fallback, native pin `shell/window_state_tests.rs:a_window_on_a_display_that_is_gone_is_centred_on_the_primary`; its "fitted default" half is the finding of section 8.
- `maximizing_keeps_the_last_ordinary_frame`: **MOVED**, native pin `shell/window_state_tests.rs:saving_keeps_the_normal_rectangle_while_maximized_or_minimized`.
- `the_wiring_reads_the_file_and_a_corrupt_one_is_the_default`: **MOVED**, native pin `shell/window_state_tests.rs:nonsense_is_ignored`.

### 9.13 `src-tauri/src/tray_window.rs` (4, base-only; the macOS openers are carried in section 3)
- `tray_menu_says_syncing_while_files_move`: **MOVED**, pinned by `shell/tray.rs:each_state_has_its_icon_and_tooltip`.
- `tray_menu_says_locked_and_offers_unlock_with_the_plain_b`: **MOVED**, pinned by `shell/tray.rs:each_state_has_its_icon_and_tooltip`.
- `tray_menu_says_needs_a_decision_with_a_waiting_conflict`: **MOVED**, pinned by `shell/tray.rs:the_first_state_in_the_boards_order_wins`.
- `tray_menu_says_up_to_date_when_nothing_is_happening`: **MOVED**, pinned by `shell/tray.rs:each_state_has_its_icon_and_tooltip`.

### 9.14 `src-tauri/src/tests.rs` (the 3 macOS window tests of §1 row 53)
- `macos_tray_flyout_geometry_matches_configured_window`: the 0.8.x macOS settings window is 680 × 540.
- `macos_window_mode_is_popover_when_anchor_present`: with an anchor, popover mode.
- `macos_window_mode_is_regular_when_anchor_absent`: without one, a regular window.
