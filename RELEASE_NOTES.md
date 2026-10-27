# Beebeeb Desktop 0.8.17 — Windows: the app stops re-checking files that have not changed

0.8.16 stopped the Windows app from reading its whole database for every file it scanned. It still checked every
file in the sync folder on every pass, a few seconds apart, and its windows kept reading the database every few
seconds, even while hidden. 0.8.17 fixes both. The scan now checks only files whose listing or record has
changed, and the windows read only what they show, only while they are on screen. On the maintainer's PC
processor use went from 86 % of one core to 25–30 %, and disk reads from about 28 MB to about 5 MB a second.
This release changes nothing else.

**Known limitations:**
- The Windows scan still walks the whole sync folder on every pass. It no longer re-checks unchanged files, but
  listing 37,000 files every few seconds is most of the processor and disk use that is left (tasks 2240 to
  2242).
- An edit in a file from before 0.8.13 is still kept on this PC and not sent (task 2191).
- 0.8.15's notes said 0.8.16 would resume changes held since a sign-in expired, and would repair folders this
  PC created but never managed to create on the server. Neither is in 0.8.16 or 0.8.17. Both move to a later
  release, and the app keeps counting those changes as not in sync until then.

### What's New

- Nothing new to use. This release only makes the Windows app do less work.

### Bug Fixes / Hardening

- **[Windows] The scan skips a file that has not changed (desktop #167, task 2229).**
  - **Before:** every pass classified all 37,381 files on the maintainer's PC. That took about five file-handle
    opens each, at 40 µs per open, for 10 to 13 seconds of one or two cores per pass.
  - **Now:**
    - A file that needs nothing is remembered with two stamps: its listing (size, modification time and, on
      Windows, its attributes) and a hash of its database record.
    - Later passes skip it while both stamps match, and classify it again when either changes.
    - Files of unknown content are held the same way until their bytes or their record change, and edits
      with no known base until their record holds one.
    - Every 10 minutes every file is classified again. That covers bytes rewritten with the same size and
      modification time.
    - The walk reads each file's size and time from the directory listing instead of opening it.
  - **Device:** on that PC, after the first pass, each pass classified 3 files and skipped 37,537.
  - **Verified by:**
    - `regression_2229_a_settled_file_is_classified_again_only_when_it_or_its_row_changes`
    - `regression_2229_a_file_passed_over_for_now_is_asked_about_every_pass`
    - `regression_2229_a_file_of_unknown_content_is_skipped_until_its_bytes_or_its_row_change`
    - `regression_2229_an_edit_with_no_known_base_is_skipped_until_its_row_records_one`
    - `regression_2229_an_equal_length_edit_that_restores_the_mtime_is_sent`
    - `a_row_stamp_changes_with_any_write_to_the_row`

    Each test was seen to fail when the behaviour it checks was removed.
- **The sync folder's cached file hash is not reused for a rewrite that keeps the same size and restores the
  modification time (task 2232).**
  - The cache key now includes the file's change time and identity, so such a rewrite is hashed again and its
    edit is sent.
  - Verified by `task_2232_a_cached_hash_is_not_reused_for_an_equal_length_rewrite_that_restores_the_mtime`.
- **[Windows] The windows read the database only for what they show, and only while they are on screen
  (desktop #168, task 2243).**
  - **Before:** each read opened the database afresh, and three of them loaded whole sets of records to show a
    count or a short list:
    - the status read loaded 5,969 records to count them;
    - the tray's recent files read all 43,000 records every 5 seconds;
    - the hold summary looked up every held change.

    The tray, the main window and the first-run window stay loaded while hidden, so these reads ran almost all
    the time. Every window also read the full status once a second to notice a sign-out.
  - **Now:**
    - The counts come from an index.
    - The tray's recent list reads only the newest records.
    - The hold summary reads one index.
    - The once-a-second sign-out check reads no file at all.
    - Every other poll in the Windows app pauses while its window is hidden, and reads at once when the window
      is shown again.
  - **Cold-start cost** on a copy of the maintainer's database (43,146 records, a fresh connection per read):
    one round of these reads went from 7,172 page reads to 99, not counting the cost of opening it. The
    sign-out check no longer opens it at all.
  - **Verified by:**
    - `task_2243_sync_status_counts_without_reading_the_rows`
    - `task_2243_the_tray_recent_list_reads_only_the_newest_rows`
    - `task_2243_the_hold_summary_reads_an_index_not_the_held_operations`
    - `task_2243_the_session_poll_reads_no_file`
    - 24 frontend tests: `tests/visiblePoll.test.ts` and the session poll in `tests/accountSession.test.ts`

    Each was seen to fail first.

### Verification

- Test suite (Linux, release/0.8.17 at `aab7ab6`):
  - `cargo test` lib reported `test result: ok. 1701 passed; 0 failed; 7 ignored`, 11 more than 0.8.16: the
    tests named above. `keychain` 31, `windows_session_wiring` 8 and `windows_signout_cleanup` 3, all with 0
    failed.
  - `bun test` 1237 pass / 0 fail (0.8.16: 1213), `tsc --noEmit` 0 errors, `eslint` clean.
- Test suite (native Windows, a clean clone at `aab7ab6`):
  - `cargo check --locked --all-targets` exit 0.
  - `cargo test --locked` lib reported `test result: ok. 1503 passed; 0 failed; 6 ignored`. `keychain` 31,
    `windows_session_wiring` 8 and `windows_signout_cleanup` 3, all with 0 failed.
- Test suite (macOS, at `aab7ab6`):
  - `cargo test --locked` lib reported `test result: ok. 2020 passed; 0 failed; 9 ignored`. `keychain` 31,
    `windows_session_wiring` 8 and `windows_signout_cleanup` 3, all with 0 failed.
  - `bun test` 1237 pass / 0 fail.
- Real hardware smoke test on the maintainer's Windows 11 PC, the one with the 43,000-record database.
  - **Builds:** local release builds of main with these two changes, unsigned, run in place against the real
    sync folder and state on 2026-10-27. Main carries other changes that this release does not, so the release
    build itself is checked again on that PC once 0.8.17 is installed.
  - **Scan (desktop #167):** processor 86 % of one core in 0.8.16, then 24 to 31 % in three 30-second samples.
  - **Windows (desktop #168):** measured back to back with the main window shown, then closed to the tray.
    - Disk reads: 25.3 to 26.1 MB a second without #168, 5.0 to 5.6 MB a second with it.
    - Reads in a quiet second, the median: 3,300 to 3,356 without #168, 15 to 42 with it.
    - Processor: 24.6 to 29.7 % of one core.
    - What remains is the scan's walk (first known limitation above).
- The macOS sign-out device checks that 0.8.12 shipped without were waived again for this release, by the
  maintainer on 2026-10-27:
  - to the Windows session: "Waive for 0.8.17";
  - to the macOS session, after first typing "waive for 0.8.16": "Yes, waive for 0.8.17", then "17".

  The blocker stays listed on main for later releases.
- Not verified:
  - sending an edit in a file from before 0.8.13 (task 2191);
  - the 0.8.17 build itself on the maintainer's PC (it runs after install).

### Install / Update

Existing desktop installs receive this release through the in-app updater automatically. For a
fresh Windows install, download the NSIS `setup.exe` from the GitHub release assets.
