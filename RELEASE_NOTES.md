# Beebeeb Desktop 0.8.21 - Windows: downloads straight from storage, one version per download, a lighter sync check

0.8.21 is the first release built from the main line of development since 0.8.12. Versions 0.8.13 to 0.8.20 were built from a separate branch that took only selected fixes, so this release brings Windows the work that waited on main. The largest changes: files download straight from encrypted storage, a download keeps to one version of a file, and the sync check does less work when nothing has changed. This release is for Windows and Linux; macOS follows in a later version.

**Known limitations:**
- **Windows, a file fetched in parts:** Windows can fetch a cloud-only file in several parts. If the file is replaced on another device between two parts, the copy on this PC can still hold parts of both versions. A whole-file download keeps to one version (see below). This case is tracked as a separate fix (task 2180).
- **Still as in 0.8.20:**
  - The files of the same name that 0.8.18 made (the 89 on the maintainer's PC) are still in Beebeeb. Deleting, renaming or saving one of them on the PC can act on the wrong copy. A delete moves that copy to the trash, where you can restore it. Sorting them out is a separate step, reviewed by hand first.
  - "Free up space" does not work on a read-only file yet. Backing up Documents, Desktop and Pictures copies a read-only file once, but does not update that copy when the original changes.
- **Linux:** the Linux app cannot sign in yet. It has no support for a system password store, and it does not keep a sign-in any other way, so its sign-in window says "This feature is not available on this device." This release does not change that.
- **Shared folders:** this build contains code for the new shared folders. They are switched off on our servers, so you will not see them yet.

### What's New

- **[Windows] Files download straight from storage (task 2173).**
  - When the server offers it, a whole-file download, for example opening a cloud-only file in Explorer, fetches the file's encrypted parts straight from Beebeeb's storage in Europe. It uses short-lived links made for that one file. The Beebeeb API only checks that you may read the file and signs the links.
  - What comes from storage is still encrypted. The app decrypts it on this PC and checks every part before it writes the file. Storage is sent no sign-in token and no account details, and a storage link never appears in the app's log.
  - If storage cannot serve a part, that part comes through the API instead, from the same version. Your download speed limit applies to both routes.
  - Still through the API in this version: uploads and thumbnails. To send everything through the API, set the environment variable `BB_TRANSPORT=api`.
  - Tests: after one test was corrected, each of 33 deliberate breakages of the new rules was caught by a test. Against local test servers, every file downloaded directly, through the API, or through a forced fallback was byte-identical to the file uploaded.
- **[Windows] Settings use plainer names.** "Advanced" is now "Appearance & storage" and shows the theme choice once. "Data residency" is now "Storage location". Notifications are split into "Account notifications" and "On this device". The encryption row reads "End-to-end encryption", always on. Four new tests failed against the previous Settings before they passed.

### Bug Fixes / Hardening

- **[Windows] A download keeps to one version of the file (task 1643).**
  - **Before:** each part of a file came from whichever version was newest at that moment. If the file was replaced on another device during the download, the result could be made of parts of two versions. Nothing caught it, because each part was valid on its own.
  - **Now:** every part comes from the version the download started on. If that version is gone, the download starts once more from the new version. If it changes again, the download stops with an error and nothing is written. A file with no recorded version (an old upload) downloads as before.
  - **Tests:** on the code before the change, 8 of 9 new tests failed, most with a file made of two versions; the ninth checks old uploads and passed, as it should. With the change, all 9 pass. The exception is a file Windows fetches in parts (see Known limitations).
- **[Windows] The idle sync check no longer downloads your whole file list (task 1909).** Every 30 seconds the app asks the server for new changes. When there were none, 0.8.20 also fetched the full list of your files to confirm its place. Now the server's answer says where its list of changes ends, so one small request is enough. A server that does not say so still gets the full check. Two new tests cover the idle check and a server whose list was reset; the 12 existing sync-check tests pass.
- **[Windows] The sync folder checks do less work (task 2240).**
  - The scan looked up each file in a database cache too small for the database, so every pass read it from disk again: about 4,760 reads (18.5 MB) every 18 seconds on the maintainer's PC. The cache now fits the database. On that PC, with the window closed, disk reads went from 4.90 MB/s before the change to 0.00 MB/s. The cost: the app uses about 35 MB more memory there, close to the size of its database (36.4 MB).
  - The 30-second check of what Windows shows for each file (on this PC, or online only) reads one listing per folder instead of asking Windows about each file. It records a change only after the file itself confirms it.
- **[Windows] An edit whose starting version is unknown is held, not sent (task 2190).** Before, a file the app saw for the first time was taken to match the newest version in Beebeeb, so a later edit was sent as if it were based on that version. Now the starting version stays unknown, and such an edit is held: it is never marked as synced, never replaced by a download, and both versions are kept.
- **[Windows] One log line per scan for held edits (task 2244).** Each start wrote one warning per held file: 28,224 lines in about 100 seconds on the maintainer's PC (0.8.17), which buried every other warning. A scan pass now writes one line with the counts.
- **[Windows] Marking an uploaded file as synced is safer in two rare cases (task 2284).**
  - A file in a folder this account cannot read was taken as deleted, and its upload record was closed. Now it is held for review, and the copy made for the upload is kept.
  - If "Free up space" or Storage Sense freed a file between the app's check and its read, that read downloaded the whole file again. Now the app refuses its own download at that moment and tries again on the next pass. Other programs that open the file are still served.
  - **Tests:** native Windows tests, each seen to fail on the old code or with the fix deliberately broken. The finalization tests: 24 passed, 0 failed.
- **[Windows] A new file uploads only under an id this app chose (SF-0001).** The app proposes the new file's id itself, and checks that the server's answer names the same id before it encrypts anything under it. If the answer names another id, the upload stops. A new-file upload that 0.8.20 started and did not finish starts again from the beginning, once. An unfinished new-file upload that the app gives up on is now cancelled on the server; before, it was moved to the Trash. Eleven tests (nine new, two changed), each seen to fail first; 7 deliberate breakages, each caught.
- **[Windows] Signing out ends every session it removes (task 1886).** A session saved while a sign-out was running is now ended on the server too. A failure to prepare the request that ends the session no longer fails the sign-out after this PC's data was already cleared: ending the session is best effort, and it never changes the sign-out's result. New tests, each seen to fail first.
- **[Windows] A connection speed test that fails says so.** A test with no answered ping, or no finished upload or download, showed 0.00 Mbps. It now shows an error. A failed ping no longer counts toward the latency.
- **[Windows] A failed update install stays visible.** The update check every 4 hours cleared the error of a failed install of the same version. The error now stays until a newer version is offered.
- **[Developer] The app's largest source file is split into modules (LD-0057).** `src-tauri/src/lib.rs` went from 41,344 lines to 497. Code was moved, not changed: before and after the last step, the Linux library tests gave 2304 passed, 0 failed, and the macOS library tests 2650 passed, 0 failed.

### Verification

- Release code: `main` at the merge of desktop PR #220, plus the commit that adds these notes. #220 changes only `RELEASE_BLOCKERS.md` and `docs/RELEASING.md`, so the app's code is that of `80adaa9`, the commit the counts below come from.
- Continuous integration at `80adaa9` (Desktop CI run 41990999154): 8 of 8 checks passed.
  - Linux, `cargo test --locked`: the library reported `test result: ok. 2304 passed; 0 failed; 7 ignored`, and three integration suites ran 33, 10 and 3 tests, all with 0 failed. 2,350 of 2,350 tests passed.
  - Windows (GitHub's Windows runner), `cargo test`: the library reported `test result: ok. 2126 passed; 0 failed; 6 ignored`, and the same three suites ran 33, 10 and 3, all with 0 failed. 2,172 of 2,172 tests passed.
  - macOS (not in this release), `cargo test`: the library reported `test result: ok. 2650 passed; 0 failed; 9 ignored`, and the three suites 33, 10 and 3. 2,696 of 2,696 tests passed.
  - Frontend: `bun test` 2088 pass / 0 fail across 117 files; the same job's `tsc --noEmit` and `eslint` steps passed.
- Native Windows 11, on the maintainer's PC. Neither run was at `80adaa9`, which merges both:
  - At `3453057`, the head of #218 (the last split step, without #219), with a no-fail-fast copy of `scripts/windows/native-gate.sh`: `cargo check` exit 0. The library reported 2125 passed, 1 failed, 6 ignored, and the three suites ran 33, 10 and 3. The one failure, `a_target_in_a_denied_folder_goes_to_review_and_keeps_its_staged_copy`, also failed on main before the split. Its cause was the test's own restore of a file's permissions; #219 fixed the test and changed no app code.
  - At `9ca6a59`, the head of #219 (without #218), with the gate's two cargo commands run directly rather than through the script: `cargo check --locked --all-targets` exit 0. `cargo test --locked`: the library reported 2126 passed, 0 failed, 6 ignored; 2,172 tests ran across 6 suites, all passed.
- Release workflow check: `node scripts/release-channel-checks.mjs notes 0.8.21 alpha --no-macos`, the form the release workflow uses while `[macOS]` items bind, passes on these notes, and so does the same check for stable. With the macOS line removed, it fails. This is the first release built with the workflow's updated build actions (tauri-action 1.0.0); those steps were not dry-run beforehand.
- Real hardware smoke test: not run before the alpha publish. The alpha is installed on the maintainer's Windows 11 PC before it is promoted to stable.
- Not verified:
  - this build on real hardware before the alpha publish (see above);
  - Linux on real hardware;
  - direct downloads from this build against production storage (the tests ran against local servers);
  - macOS: not in this release.

### Install / Update

This release is published to the alpha channel first. Windows installs following that channel receive it through the in-app updater; stable installs receive it only once it is promoted to stable.

macOS: not in this release; it follows in a later version.

For a fresh Windows install, download the NSIS `setup.exe` from the GitHub release assets.
