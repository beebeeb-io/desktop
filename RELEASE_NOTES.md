# Beebeeb Desktop 0.8.18 - changes held since a sign-in expired are sent, and Windows sync starts again

This release fixes two ways the desktop app stopped sending your changes.

- **Windows:** a person with one account could be told the PC held another account's files, and sync never started.
- **Windows and Linux:** after a sign-in expired, changes made in the meantime stayed on hold for good, even after you signed in again.

0.8.15's notes promised the second fix for 0.8.16, and it did not ship in 0.8.16 or 0.8.17. It is in this release. Two older problems in the step after an upload are not fixed yet: see the known limitations.

**Known limitations:**
- **A read-only file is not marked as synced after it uploads.**
  - **What you see:** its upload reaches Beebeeb, but on the PC the file is not shown as synced, and the app keeps the private copy it made for the upload.
  - **Why:** the app tries to open the file for writing to mark it, and Windows refuses because the file is read-only.
  - **Your data:** nothing is lost. A fix is planned for 0.8.19.
- **An edit made while a change was waiting can stay unsent.**
  - **What you see:** you change a file after its upload was queued, but before it was sent. The queued version reaches Beebeeb, and your newer edit stays on the PC and is not sent. The app still says "Up to date".
  - **Your data:** your newer edit is safe on the PC.
  - **How often:** on the maintainer's PC this happened to 4 of 2,984 changes.
  - **Fix:** planned for 0.8.19. Contact support@beebeeb.io if you think you are affected.
- **A lost reply can make a new folder wait for a restart.** If the reply to creating a new folder is lost after the server made it, the changes inside that folder wait until the app is opened again.
- **Some held changes need support.** A change held because an item with the same name is already in Beebeeb stays held. The app says so; contact support to sort it out.
- **macOS:** this release does not change how macOS handles either problem. macOS starts sync differently and did not have the first one.

### What's New

- **[Windows] Settings › Sync asks before sending changes it could not tie to your account.**
  - **The card:** it says what is waiting, for example "Waiting for your decision: 11 new folders and 145 new files." and "Beebeeb couldn't confirm these came from your account on this PC."
  - **Upload to my Beebeeb:** sends exactly the changes the card counted.
  - **Keep on this PC only:** moves those items out of the Beebeeb folder into a dated folder next to it, called "Beebeeb - not uploaded". It only renames, never overwrites, and deletes nothing. When another cloud service syncs that folder too, the button reads **Keep out of Beebeeb** and the question names that service.
- **[Windows, Linux] The app says why a change is waiting.**
  - A change whose folder is not in Beebeeb yet reads "waiting for its folder to reach Beebeeb".
  - A file that is still uploading reads "Not uploaded yet", never "Synced to".

### Bug Fixes / Hardening

- **[Windows] Sync refused to start for a person with one account (desktop #176, #178, task 2267).**
  - **Scenario:**
    1. You signed in to the Windows app with your only account.
    2. It said "This PC still holds local files of another Beebeeb account, so sync didn't start. Sign out, then sign in again."
    3. Signing out was refused because changes were waiting, so you were stuck.

    The cause: a session saved before the app kept its account's email left the files on the PC with no recorded owner, and every start treated them as another account's.
  - **Now:** the app reads your account's file list once and checks every local item the server knows. An item counts as yours when its encrypted name opens with your session's key.
    - The files are adopted only when every checked item is accounted for. Otherwise sync stays off, nothing is uploaded or deleted, and the app says to contact support.
    - A change the app cannot tie to a proven item through a real folder link waits for your decision (see What's New). A matching path alone never sends a file's content.
  - **Evidence, on the maintainer's Windows 11 PC with the real account:**
    - The installed 0.8.17 refused to start sync, which is the bug.
    - The fix checked 40,534 of 40,534 items, connected 6 seconds after launch, and held 156 changes for a decision.
    - Two copies of the data with foreign items mixed in were refused (8 and 6 items the account could not place). Nothing was sent, and the database was restored byte for byte after each.
    - The card showed "11 new folders and 145 new files". Upload released exactly those, and Keep moved an item into the dated folder.
  - **Tests:** 48 automated tests for the ownership check, 26 for holding and releasing changes and for Keep, and 45 for the card. Each new test was seen to fail first.
- **[Windows, Linux] Changes held since a sign-in expired are sent, into the right folders (desktop #180, tasks 1888, 2226).**
  - **Scenario:**
    1. The server stopped accepting the app's sign-in (on the maintainer's PC, on 4 October). The app held the changes it had queued until you signed in again, among them the folders it had just made on the PC.
    2. On Windows and Linux, nothing ever resumed changes held that way. Only macOS did.
    3. Meanwhile, the same folders reached Beebeeb another way. The PC now knew each of them twice: its own held copy and the copy in Beebeeb.
    4. New files you added to those folders were queued under the held copy, which Beebeeb had never seen. Each upload was refused with "not found", retried for about 5 hours, then stopped: "stopped after repeated errors" in Settings, "Sync error" in the tray.
    5. Empty backup folders made for a device with no name were held the same way.

    On the maintainer's PC this added up to 2,984 changes not sent: 2,924 held since 4 October and 60 stopped. Settings said "2,924 changes from 4 Oct are on hold. An update will repair them."
  - **Now:** when the app starts or you sign in, it runs a reconcile pass before it sends anything, at the first moment it has heard back from the server.
    1. A folder that never reached Beebeeb, with exactly one folder of the same name in the same place in Beebeeb, is merged into that folder. Everything inside it moves along.
    2. A folder with no copy in Beebeeb is created.
    3. The empty backup folders are removed, deepest first, and only when empty.
    4. A change that gave up only because its folder was missing gets its retries back.
    5. Anything ambiguous is held, never guessed. Examples: two folders with the same name that differ only in case, a file that meets a file of the same name, or a folder whose two parent records disagree.
    6. The held changes resume only if their names open with your current key. If your key was replaced, they stay held and are never sent under the old one.

    The pass works in batches of 256 folders. A failure undoes only its own batch, and the next start finishes the job.
  - **A change waits for its folder:** a change inside a folder that is not in Beebeeb yet is not sent, and spends no retries, until that folder exists.
  - **Evidence, on the maintainer's Windows 11 PC with the real account, release build of this change:**

    | | Before | After |
    |---|---|---|
    | Home: state | Not in sync | Up to date |
    | Not sent | 2,984 | 0 |
    | Changes that had given up | 60 | 0 |
    | Folders in Beebeeb | 2,837 | 2,842 (the 5 new folders) |
    | Duplicate folders in Beebeeb | 0 | 0 |

    - **The pass:** it ran 10 seconds after launch. It merged 2,828 folders into their copies in Beebeeb, created 5, removed 6 empty backup folders, gave 54 changes their retries back and held 0. The 150 held changes then resumed under the current key. The queue was empty about 2 minutes later.
    - **Downloads still work:** a file inside a merged folder that was only in the cloud downloaded when opened.
    - **Prediction:** a dry run of the same code on a read-only copy of that database predicted every count of the pass exactly, as did the design's analysis of 26 and 28 October.
    - **Speed:** on that copy, the longest batch took 343 ms. Three new database indexes make this possible; without them a batch took 5 to 6 seconds.
  - **Tests:** 87 new automated tests. Each was seen to fail first. They include:
    - a merge leaves nothing pointing at the old folder;
    - a failure halfway undoes only its own batch, and a second run completes;
    - nothing is sent under a replaced key, including changes given their retries back;
    - a folder with no parent record is held, never created at the top level;
    - an upload into a folder that is not in Beebeeb yet waits without using its retries;
    - an older upload of a file still lands before a newer one;
    - the pass runs only after the server has answered.

### Verification

- Release code: `4a4ac7c` plus one test-only change (`38ec576`) that skips one queue test on macOS, where the behaviour it tests does not exist. Off macOS it changes nothing: the Linux library run at `38ec576` reported the same `1861 passed; 0 failed; 8 ignored`.
- Test suite (Linux, at `4a4ac7c`):
  - `cargo test --locked`: the library reported `test result: ok. 1861 passed; 0 failed; 8 ignored`, and three integration suites ran 31, 8 and 3 tests, all with 0 failed.
  - `bun test` 1290 pass / 0 fail, `tsc --noEmit` 0 errors.
- Test suite (native Windows, a clean clone at `4a4ac7c`, rustc 1.98 MSVC):
  - `cargo check --locked --all-targets` exit 0.
  - `cargo test --locked`: the library reported `test result: ok. 1670 passed; 0 failed; 7 ignored`, and three integration suites ran 31, 8 and 3 tests, all with 0 failed. 1,712 tests ran in total.
- Test suite (macOS, Apple Silicon, at `38ec576`):
  - `cargo test --locked`: the library reported `test result: ok. 2170 passed; 0 failed; 10 ignored`, and three integration suites ran 31, 8 and 3 tests, all with 0 failed. 2,212 tests ran in total.
- Continuous integration on the change: 6 of 6 checks passed (Linux tests, Windows and macOS builds, frontend tests, the macOS release script check). The macOS job builds the app but runs no tests; the macOS run above is the one that did.
- Real hardware: the maintainer's Windows 11 PC, real account and sync folder, release builds of these changes run in place before release. The two device runs are in the fixes above.
- The macOS sign-out device checks that 0.8.12 shipped without were waived for this release by the maintainer on 2026-10-29: "Waive for 0.8.18". The blocker stays listed on main for later releases.
- Not verified:
  - the 0.8.18 build itself on the maintainer's PC (it runs after install);
  - Linux on real hardware (the Linux test suite above ran; there was no device run);
  - sending an edit in a file from before 0.8.13 (task 2191).

### Install / Update

Existing desktop installs receive this release through the in-app updater automatically. For a fresh Windows install, download the NSIS `setup.exe` from the GitHub release assets; for macOS, the `.dmg` from the same release.
