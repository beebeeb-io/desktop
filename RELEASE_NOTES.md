# Beebeeb Desktop 0.8.19 - no second file of the same name, and an edit is never hidden behind "Up to date"

This release fixes a problem 0.8.18 introduced and one it exposed.

- **Windows and Linux:** when 0.8.18 sent changes held since a sign-in expired, some new files went into Beebeeb next to a file of the same name that was already there. This release holds such a file instead.
- **Windows and Linux:** on a PC where that happened, an edit to one of those files could stay on the PC while the app said "Up to date". This release sends it.

**A correction to 0.8.18's notes.** They said that "a file that meets a file of the same name" is held. That was not true for a new file whose own record named no folder, and most held new files were like that. On the maintainer's PC, 0.8.18 sent **89** such files as second files of the same name. No data was lost: both copies are in Beebeeb, and the files on the PC are intact. If you see two files with the same name in one folder after updating to 0.8.18, this is why. Contact support@beebeeb.io and we will help you sort them out. Nothing is deleted on its own.

**Known limitations:**
- **Two files of the same name made by 0.8.18 stay in Beebeeb.** This release stops new ones; it does not remove existing ones. Sorting them out is planned for a later release, and it will never happen automatically.
- **Deleting, renaming or saving one of those 89 files on the PC can act on the wrong copy.** It can act on the copy that was in Beebeeb first instead of the copy this PC sent. Some apps save by replacing the file, and that counts too. A delete moves that copy to the trash, where you can restore it. We found this by reading the code; no test shows it yet. A fix is planned for 0.8.20.
- **A read-only file is not marked as synced after it uploads** (as in 0.8.18). On the maintainer's PC, every such file was one of the 89. A fix is planned for 0.8.20.
- **macOS:** this release does not change how macOS handles either problem.

### Bug Fixes / Hardening

- **[Windows, Linux] A new file was sent next to a file of the same name (task 2271).**
  - **Scenario:**
    1. Changes were held since a sign-in expired. Among them were new files in a folder the PC made in that time.
    2. The same folder had reached Beebeeb another way, with a file of the same name in it.
    3. 0.8.18 merged the folder into the one in Beebeeb, then checked each new file for a file of the same name. It looked in the wrong folder: the one named in the file's own record, which for most new files was none. The upload goes to the folder named in the queued change.
    4. So each such file was sent as a second file of that name.

    On the maintainer's PC: 145 held new files. 128 of them named no folder in their own record, and 89 were sent next to a file of the same name.
  - **Now:** the check looks in the folder the file is sent to. When the file's own record names a different folder, it looks there too. A file that meets a file of the same name in either folder is held, never sent.
  - **Evidence:**
    - We ran the reconcile pass in dry-run mode on a read-only copy of the maintainer's database, taken before 0.8.18 ran. It holds exactly those 89 files: 0 missed, 0 extra. Everything else it does is unchanged; it still merges 2,828 folders.
    - The 0.8.18 code on the same copy, the same day, holds 0.
  - **Tests:** 2 new automated tests. Each was seen to fail, either on the 0.8.18 code or with the fix deliberately broken.
- **[Windows, Linux] An edit stayed on the PC while the app said "Up to date" (task 2271).**
  - **Scenario:**
    1. After the problem above, the PC had two records for the same place: the file that was in Beebeeb first, and the copy it sent.
    2. To decide whether the file had changed, the app compared it with the first record. This PC had never downloaded that copy, so there was nothing to compare, and the app counted the file as unchanged.
    3. An edit made after the upload was queued stayed on the PC. On the maintainer's PC this happened to 4 files.
  - **Now:** when two records share a place, the app compares the file with the record of what this PC and Beebeeb last agreed on, so a newer edit is sent. If several records could apply and the file matches none of them, the app does not guess and sends nothing. The log records this without the file's name.
  - **Tests:** 4 new automated tests. One of them, on Windows, checks that a file that is only in the cloud is never downloaded just to decide this. Each was seen to fail first.
- **[Developer] Continuous integration now runs the tests on macOS, not only a compile check.** 0.8.18's macOS test failure was found only on a maintainer's Mac.

### Verification

- Release code: `74db445` (the merge of the change) plus the commit that adds these notes, which changes nothing else. The tests below ran at `3566ef4`, whose files are identical to `74db445`.
- Test suite (Linux, at `3566ef4`):
  - `cargo test --locked`: the library reported `test result: ok. 1866 passed; 0 failed; 8 ignored`, and three integration suites ran 31, 8 and 3 tests, all with 0 failed.
  - `bun test` 1290 pass / 0 fail, `tsc --noEmit` 0 errors (both in continuous integration).
- Test suite (native Windows, a clean clone at `3566ef4`, rustc 1.98 MSVC):
  - `cargo check --locked --all-targets` exit 0.
  - `cargo test --locked`: the library reported `test result: ok. 1676 passed; 0 failed; 7 ignored`, and three integration suites ran 31, 8 and 3 tests, all with 0 failed. 1,718 tests ran in total.
- Test suite (the macOS runner in continuous integration, at `3566ef4`):
  - `cargo test --locked`: the library reported `test result: ok. 2175 passed; 0 failed; 10 ignored`, and three integration suites ran 31, 8 and 3 tests, all with 0 failed. 2,217 tests ran in total.
- Continuous integration on the change: 6 of 6 checks passed. They cover the Linux tests, the Windows tests, the macOS tests (new in this release), the macOS File Provider build, the frontend tests and the macOS release script check.
- Real data: the dry run above, on a read-only copy of the maintainer's database.
- The maintainer waived the macOS sign-out device checks that 0.8.12 shipped without for this release on 2026-10-29: "Waive for 0.8.19". The blocker stays listed on main for later releases.
- Not verified:
  - the 0.8.19 build itself on real hardware: it was not run on a device before release, and it runs on the maintainer's Windows 11 PC after install;
  - Linux on real hardware (the Linux test suite above ran; there was no device run).

### Install / Update

Existing desktop installs receive this release through the in-app updater automatically. For a fresh Windows install, download the NSIS `setup.exe` from the GitHub release assets; for macOS, the `.dmg` from the same release.
