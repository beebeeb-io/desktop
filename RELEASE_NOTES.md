# Beebeeb Desktop 0.8.20 - a read-only file is marked as synced after it uploads

This release fixes the problem 0.8.18 and 0.8.19 listed as a known limitation.

- **Windows:** a read-only file that uploads is now marked as synced. 0.8.19 tried again on every pass and failed every time, and an unfinished one blocked signing out.
- **Windows:** marking a file as synced never needs to download it, and never changes it. Its read-only setting, its date and its permissions stay as they were.
- **Windows:** a file this PC cannot mark as synced is no longer retried in the background for ever. It shows as one line in the app, with what you can do about it.

**Known limitations:**
- **Two files of the same name made by 0.8.18 are still in Beebeeb** (the 89 on the maintainer's PC; see 0.8.19's notes). Sorting them out is planned as a separate step. It will be reviewed by hand first and will never happen automatically.
- **Deleting, renaming or saving one of those 89 files on the PC can still act on the wrong copy.** 0.8.19's notes said a fix was planned for 0.8.20. It is not in this release; it moves to that separate step. A delete moves that copy to the trash, where you can restore it.
- **Windows, read-only files:**
  - "Free up space" does not work on a read-only file yet. The same kind of fix is planned.
  - Backing up Documents, Desktop and Pictures copies a read-only file once, but does not update that copy when the original changes. We found this with a test on temporary files; a fix is planned.
- **macOS:** this release does not change macOS. The 0.8.19 changes still apply there.

### What's New

- **[Windows] The app says when it could not mark an uploaded file as synced.** The app's sync status shows one line for them: "1 file was uploaded, but this PC couldn't mark it as synced." (or "N files were uploaded, but this PC couldn't mark them as synced."), followed by "Contact support@beebeeb.io and we'll sort it out with you." Before, such a file was retried in the background for ever, and nothing said so.

### Bug Fixes / Hardening

- **[Windows] A read-only file was never marked as synced after it uploaded (tasks 2270, 2280).**
  - **Scenario:**
    1. A read-only file uploads, for example a backed-up copy of a read-only file in Documents.
    2. After the upload, the app gives the file on the PC its Beebeeb identity and marks it as synced. To do that, 0.8.19 opened the file for writing.
    3. Windows refuses to open a read-only file for writing. The app logged a warning, kept the file in its list, and tried again on every pass, with the same result. While that list was not empty, signing out was refused.

    On the maintainer's PC: 86 files stayed in that list. 82 were read-only copies. The other 4 had been edited after the upload, and those edits had already reached Beebeeb; 0.8.19 refused them with "Local bytes changed".
  - **Now:**
    - The app opens the file without write access. A file that is only in the cloud is not opened for its contents at all, so nothing is downloaded.
    - A file whose contents are the uploaded bytes, or the bytes this PC and Beebeeb last agreed on, is marked as synced.
    - A file with other contents keeps its new identity but is not marked as synced, so the next change is sent as usual.
    - A file the app cannot mark safely is held for review instead: for example, it was moved, another file has taken its place, or only part of it is on the PC. It is tried again each time the app starts. It does not block signing out.
    - A file that is open in another program is tried again on the next pass.
  - **Evidence:** on the 0.8.19 way of opening the file, 9 of the 10 new finalization tests failed, each with "Access is denied" on a read-only file. With the fix, all 10 pass.
  - **Tests:** 18 new automated tests on Windows, most of them on a temporary sync folder with a provider that counts every download.
    - Each one that marks a file checks that the file's read-only setting, its date and its permissions are unchanged, and that nothing was downloaded.
    - Each was seen to fail first: on the 0.8.19 code, or with the fix deliberately broken. For example, with the check that the file is wholly on the PC removed, opening a file that was only partly there downloaded the rest, and the test caught it.
- **[Developer] The state database is created under one write lock** (task 2278). Two parts of the app opening a new database at the same moment could make one of them fail. This made a required macOS check fail at random.
- **[Developer] A release branch must start at a reviewed commit** (task 2281). Before it builds anything, the release workflow checks a release branch. Every change on the branch must have come through a pull request into that branch. The branch must start either at a commit on `main` or at the commit of a published release. That release's tag must point at the merge commit of a pull request into `main` or a release branch; a published release on any other tagged commit no longer counts. A release built from `main` itself skips this check, because changes reach `main` only through pull requests.

### Verification

- Release code: `7ac1fd0` (the merge of the read-only fix) plus the commit that adds these notes, which changes nothing else. The fix's own commit, `6f7797d`, differs from `7ac1fd0` only in the release workflow, the continuous integration file, their scripts and the release guide; the app's code is the same.
- Test suite (native Windows 11, a clean checkout of `7ac1fd0` fetched from GitHub, rustc 1.98 MSVC):
  - `cargo check --locked --all-targets` exit 0.
  - `cargo test --locked`: the library reported `test result: ok. 1709 passed; 0 failed; 7 ignored`, and three integration suites ran 31, 8 and 3 tests, all with 0 failed. 1,751 tests ran in total. The run at `6f7797d` gave the same counts.
- Test suite (Linux, at `7ac1fd0`):
  - `cargo test --locked`: the library reported `test result: ok. 1881 passed; 0 failed; 8 ignored`, and three integration suites ran 31, 8 and 3 tests, all with 0 failed. The run at `6f7797d` gave the same counts.
  - `bun test` 1291 pass / 0 fail across 78 files, and `tsc --noEmit` exit 0.
- Test suite (macOS, at `6f7797d`): `cargo test --locked`: the library reported `test result: ok. 2190 passed; 0 failed; 10 ignored`, and three integration suites ran 31, 8 and 3 tests, all with 0 failed. This release does not change macOS code.
- The project's usual gate script was not on the build machine, so it was not run. The Windows and Linux test suites above ran in full at those exact commits instead, and the reviewer accepted those runs in its place.
- The release branch point: release.yml's verify-branch-point check ran before build and signing; its result is recorded in task 2281. Before the release was started, the same check, run by hand on the release branch (read-only), passed: the branch starts at the commit of the published desktop-v0.8.19 release, which is the merge commit of a reviewed pull request.
- Continuous integration: 7 of 7 checks passed on the read-only fix, and 7 of 7 on the release-check change. They cover the Linux tests, the Windows and macOS builds, the macOS File Provider build, the frontend tests, the macOS release script check and the release branch-point check.
- The maintainer waived the macOS sign-out device checks that 0.8.12 shipped without for this release on 2026-10-29: "Waive for 0.8.20". The blocker stays listed on main for later releases.
- Not verified:
  - the 0.8.20 build itself on real hardware: it was not run on a device before release, and it runs on the maintainer's Windows 11 PC after install;
  - Linux on real hardware (the Linux test suite above ran; there was no device run).

### Install / Update

Existing desktop installs receive this release through the in-app updater automatically. For a fresh Windows install, download the NSIS `setup.exe` from the GitHub release assets; for macOS, the `.dmg` from the same release.
