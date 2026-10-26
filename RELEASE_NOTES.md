# Beebeeb Desktop 0.8.13 — Windows edits are sent and kept

0.8.13 fixes Windows sync-folder bugs that could lose an edit. Before this release, an edit of
an already-synced file in the Beebeeb folder was never uploaded, and "Free up space" could then
discard it. Also, a new file saved under a just-renamed file's old name was sent as a new version
of the renamed file. The macOS app is unchanged from 0.8.12.

**Known limitation:** files that were already on this PC before 0.8.13 have no record of which
server version their bytes are. An edit of such a file is kept on this PC and not freed by the
app, but it is not sent until a later release can check those files against the server (task
2191). Explorer's own "Free up space" and Storage Sense can still free such a file if 0.8.12
had marked it in sync.

### What's New

- No new features. This is a Windows reliability release built from 0.8.12 with only the
  fixes below. The macOS redesign and the other work on main are not in it.

### Bug Fixes / Hardening

- **[Windows] An edit of a synced file is sent, on the version it is based on (PR #151).**
  - **Before:** an edit in the sync folder was never uploaded. The in-sync stamp marked it
    synced, so "Free up space" (the app's or Explorer's) could discard it, and the next open
    fetched the server's bytes.
  - **Now:** the app records which server version and content each file on this PC holds. An
    edit is queued as a new version on that base, and a download never writes over unsent
    changes. "Free up space" only frees bytes the server already has.
  - **When the base moved:** if someone changed the file elsewhere first, the edit is kept and
    shows as a review entry.
  - **A file deleted elsewhere** keeps its unsent changes as "<name> (this PC)", with a
    dismissable "Kept copy" entry in the review page.
  - Verified by the `regression_1640_*` and `task_2190_*` engine tests.
- **[Windows] A new file is picked up again once it has synced (PR #154).** The scan no longer
  skips a file it created earlier in the same session, so a later save of it is sent. Verified by
  `regression_1640_a_new_file_is_scanned_again_once_its_row_exists`.
- **[Windows] A rename followed by a new file at the old name gives two files (PR #155).**
  - **Before:** a file renamed in Explorer kept its old place in the app's records until the
    server confirmed the rename. A new file saved under the old name in that window was sent as
    a new version of the renamed file, and the renamed file stopped syncing.
  - **Now:** the record moves when the file is renamed. An edit queued before the rename is sent
    before it, and a delete waits for it.
  - **Safe saves:** a save by renaming the original away, writing the new file under its name,
    then deleting the renamed copy is folded back into one new version of the original. Excel
    saves this way.
  - Verified by the `task_2204_*` engine tests: create, rename and a new file at the old name;
    the same with a synced file, offline, and across a restart; an Excel-style save; the pull
    and landing holds.
- **[Windows] The review page counts a kept copy as changed elsewhere, not as a failed send,**
  and kept copies alone do not turn its status dot red. For 0.8.13 this is a hand port of
  main's version of that logic. Covered in `tests/versionCenterReviewAction.test.ts`.

### Verification

- Test suite (macOS, the release branch): `cargo test --locked --lib` completed with
  `test result: ok. 1967 passed; 0 failed; 8 ignored`.
- Test suite (native Windows, a clean clone of the release branch):
  - `cargo check --locked --all-targets` exit 0;
  - `cargo test --locked` lib `test result: ok. 1447 passed; 0 failed; 5 ignored`, and every
    other test binary 0 failed.
- Frontend: `bun test` 1167 pass / 0 fail across 69 files; `bunx tsc --noEmit` and `bunx eslint .`
  both exit 0.
- macOS: `git diff desktop-v0.8.12 release/0.8.13` over every macOS-only path is empty. Those
  paths are the File Provider extension and its tests and tools, `macos/`, and the Finder, IPC,
  Keychain and File Provider modules in `src-tauri/src`.
- Real hardware smoke test: **not run before this release.** The Windows device checks (an edit
  of a synced file arrives as a new version and "Free up space" keeps it until then; a rename then
  a new file at the old name gives two files on the server; an Excel save stays one file with
  nothing trashed) are run after release, at the maintainer's call: "3 will do later, just release
  it now". The macOS sign-out device checks that 0.8.12 also shipped without were waived again for
  this release (the macOS code is identical to 0.8.12).
- Not verified:
  - files from before 0.8.13 that hold an edit hidden by 0.8.12 (task 2191);
  - folder renames and moves into a folder that is not synced yet (task 2204's full design);
  - renames made while the app is not running.

### Install / Update

Existing desktop installs receive this release through the in-app updater automatically. For a
fresh Windows install, download the NSIS `setup.exe` from the GitHub release assets.
