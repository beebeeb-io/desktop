# Beebeeb Desktop 0.8.14 — Windows files from before 0.8.13 are never discarded unread

0.8.14 protects Windows files that were on this PC before 0.8.13. The app has no record of which
server version those files' bytes are, and 0.8.12 may have marked an edit of one of them as in
sync. In 0.8.13, Explorer's "Free up space" and Storage Sense could therefore still free such an
edit, and signing out deleted it with the rest of the sync folder. In 0.8.14 nothing frees,
removes or deletes those bytes without reading them first, and signing out asks what to do with
them. The macOS app is unchanged from 0.8.13.

**Known limitation:** an edit in such a file is kept on this PC but still not sent. Sending it
needs a check of the file against the server, which comes in a later release (task 2191). Until
then, the app keeps the file marked as not in sync.

### What's New

- **[Windows] Signing out asks about files from before the last update (PR #157).**
  - When such files hold bytes on this PC, Disconnect says how many there are. It then offers
    **Keep copies and sign out**, the default, or **Delete them and sign out**.
  - Copies go into a new folder named `Beebeeb kept files <date> <time>` beside the sync
    folder, or in the home folder when the sync folder is a drive root. The dialog says plainly
    that the copies are not encrypted.
  - After signing out, an alert names that folder. It does so even when the sign-out stops after
    the copies were made.

### Bug Fixes / Hardening

- **[Windows] Explorer's "Free up space" and Storage Sense no longer free these files (PR #157).**
  - **Before:** 0.8.12 could have marked an edit of such a file as in sync, so Windows treated
    the edit as safe to free.
  - **Now:** before the first sync of each session, every such file that holds bytes and reads as
    in sync is marked not in sync. A mark that fails is retried on every sync pass, as is a file
    that becomes one of these during the session.
  - Verified by `task_2191_the_guard_decides_from_the_platform_view`,
    `task_2191_conflict_and_error_rows_are_marked_and_a_failed_mark_is_retried_on_the_next_tick` and
    `task_2191_a_row_that_joins_the_guarded_set_mid_session_is_marked_at_the_next_tick`.
- **[Windows] The app never takes "in sync" as proof that such a file matches the server.**
  - A download never writes over one that holds bytes.
  - When the file is deleted elsewhere, its bytes are kept as a copy before the original is
    removed, and the retry of that step never removes one unread.
  - Verified by `task_2191_an_unknown_file_in_sync_is_never_agreed`,
    `task_2191_a_hydrate_never_writes_over_an_unknown_file`,
    `task_2191_a_remote_deletion_copies_an_unknown_file_the_platform_calls_in_sync` and
    `task_2191_the_retry_never_removes_an_unknown_file_unread`.
- **[Windows] A file renamed or moved on another device stays protected at its old place.** The
  old place can still hold the edit. It is tracked by the file's identity, not by its name.
  Verified by `task_2191_a_web_rename_records_both_places_for_the_guard` and
  `task_2191_an_orphan_is_found_by_its_identity_even_under_another_file_s_row`.
- **[Windows] Signing out copies before it deletes.**
  - Every copy is made before anything is deleted. Each original is read again through the
    handle that deletes it. It is deleted only if it still matches its copy; otherwise the
    sign-out stops.
  - A partly downloaded file is copied over the parts on this PC.
  - A file with unsent changes that the app knows about still stops the sign-out, as before.
  - Verified on native Windows by `task_2191_sign_out_keeps_an_unknown_file_as_a_copy`,
    `task_2191_sign_out_deletes_an_unknown_file_when_asked` and
    `task_2191_a_known_file_that_is_not_clean_still_refuses_sign_out`. Also covered by
    `task_2191_sign_out_copies_whole_and_partial_files_and_rehashes_what_it_copied`,
    `task_2191_a_sync_folder_at_a_drive_root_keeps_copies_in_the_home_folder`,
    `task_2191_a_failure_after_the_purge_names_the_kept_folder` and
    `tests/windowsDisconnectUnknownFiles.test.tsx`.

### Verification

- Test suite (macOS, the release branch):
  - `cargo test --locked --lib` completed with `test result: ok. 1985 passed; 0 failed; 8 ignored`;
  - the integration binaries: `keychain` 31, `windows_session_wiring` 8, `windows_signout_cleanup` 3, all 0 failed.
- Test suite (native Windows, a clean clone of the release branch):
  - `cargo check --locked --all-targets` exit 0;
  - `cargo test --locked` lib `test result: ok. 1467 passed; 0 failed; 5 ignored`, and every other test binary 0 failed
    (`keychain` 31, `windows_session_wiring` 8, `windows_signout_cleanup` 3).
- Frontend: `bun test` 1172 pass / 0 fail across 71 files; `bunx tsc --noEmit` and `bunx eslint .`
  both exit 0.
- macOS:
  - `git diff desktop-v0.8.13 release/0.8.14` over every macOS-only path is empty. Those paths are
    the File Provider extension and its tests and tools, `macos/`, and the Finder, IPC, Keychain
    and File Provider modules in `src-tauri/src`.
  - In the code a macOS build shares with Windows, the only change that runs on macOS is two new,
    empty tables in the local database. Only Windows writes to them.
- Real hardware smoke test: **not run before this release.** The Windows device checks run after
  release, at the maintainer's call ("nah for later. continue"), on a PC that synced files with
  0.8.12:
  - Explorer's "Free up space" leaves an edited file from before 0.8.13 alone;
  - deleting such a file on the web keeps its bytes as a copy;
  - signing out with "Keep copies" puts the bytes in the named folder, and "Delete them" removes
    them.
- The macOS sign-out device checks that 0.8.12 shipped without were waived again for this release,
  at the same call ("nah for later. continue"). The macOS code is identical to 0.8.13 apart from
  the two empty tables above.
- Not verified:
  - sending an edit in a file from before 0.8.13 (a later release, task 2191);
  - folder renames and moves into a folder that is not synced yet (task 2204's full design);
  - renames made while the app is not running.

### Install / Update

Existing desktop installs receive this release through the in-app updater automatically. For a
fresh Windows install, download the NSIS `setup.exe` from the GitHub release assets.
