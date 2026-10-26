# Beebeeb Desktop 0.8.16 — Windows: the sync folder scan no longer reads the whole database for every file

Since 0.8.13 the Windows app has scanned the sync folder every few seconds to pick up edits and new files.
For each file and folder it found, it looked up that path in its local database. That column had no index,
so every lookup read the entire table. On a PC with 43,000 files and folders that came to about 1.4 GB of disk
reads a second and more than one processor core, for as long as the app ran. 0.8.16 adds the index. Each lookup now
goes straight to its row. This release changes nothing else; on macOS the index is created once at start-up.

**Known limitations:**
- The Windows app still uses a lot of processor time while the sync folder holds many files from before 0.8.13:
  each scan still checks every one of them. A later release stops re-checking files that have not changed
  (task 2229).
- An edit in a file from before 0.8.13 is still kept on this PC and not sent (task 2191).
- 0.8.15's notes said 0.8.16 would resume changes held since a sign-in expired, and would repair folders this
  PC created but never managed to create on the server. Neither is in this release. Both move to a later
  release, and the app keeps counting those changes as not in sync until then.

### Bug Fixes / Hardening

- **[Windows] The sync folder scan finds each file's record through an index.**
  - **Before (0.8.13 to 0.8.15):** every lookup read the whole files table. On the maintainer's PC
    (43,146 records) a lookup took 2.6 to 2.8 ms, and the app read about 1,484 MB a second from disk.
  - **Now:** an index on the path. The same lookup takes 4 to 5 µs.
  - The index is built once at start-up, in about 30 ms on that database.
  - Verified by `a_path_lookup_reads_through_an_index_not_the_whole_files_table`. The test reads SQLite's own
    counters on the statement the app ran. It counts 0 full-table steps for a path that exists and 0 for
    one that does not. It fails when the index is missing, when the query cannot use it, and when the app
    runs a different query.

### Verification

- Test suite (Linux, release/0.8.16 at `837cf67`):
  - `cargo test` lib reported `test result: ok. 1690 passed; 0 failed; 7 ignored`, one more than 0.8.15: the
    index test. The integration binaries reported `keychain` 31, `windows_session_wiring` 8 and
    `windows_signout_cleanup` 3, all with 0 failed.
- Test suite (native Windows, a clean clone at `837cf67`):
  - `cargo check --locked --all-targets` exit 0.
  - `cargo test --locked` lib reported `test result: ok. 1492 passed; 0 failed; 6 ignored`. `keychain` 31,
    `windows_session_wiring` 8 and `windows_signout_cleanup` 3, all with 0 failed.
- Test suite (macOS, Apple Silicon, a fresh worktree at `837cf67`):
  - `cargo test --locked` lib reported `test result: ok. 2009 passed; 0 failed; 9 ignored`. `keychain` 31,
    `windows_session_wiring` 8 and `windows_signout_cleanup` 3, all with 0 failed.
  - `bun test` 1213 pass / 0 fail. The frontend is unchanged since 0.8.15.
- Real hardware smoke test on the maintainer's Windows 11 PC, the one with the 43,000-record database.
  - **Build:** a local release build of `837cf67`, unsigned, run in place against the real sync folder and
    state on 2026-10-26 from 20:58 CET.
  - **Upgrade:** the index had already been added to this PC's database by hand at 20:05, so creating it at
    start-up had to do nothing. After start-up the index was listed once, the path lookup's query plan used
    it, and SQLite's `quick_check` returned ok. Creating the index on a database without it is covered by the
    test above on all three platforms.
  - **Disk reads:** 0.8.15 as shipped read 1,484 MB a second on this PC. 0.8.16 read 26.9 to 29.0 MB a
    second in three 30-second samples.
  - **Processor:** 70 to 94 % of one core in the same samples, 86 % over 90 seconds. The scan still checks
    28,230 files on every pass. That is the first known limitation above.
- The macOS sign-out device checks that 0.8.12 shipped without were waived again for this release, by the
  maintainer (2026-10-26: "waive for 0.8.16"). The blocker stays listed on main for later releases.
- Not verified:
  - sending an edit in a file from before 0.8.13 (task 2191).

### Install / Update

Existing desktop installs receive this release through the in-app updater automatically. For a
fresh Windows install, download the NSIS `setup.exe` from the GitHub release assets.
