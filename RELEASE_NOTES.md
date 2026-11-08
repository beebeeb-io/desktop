# Beebeeb Desktop 0.8.24 - Windows: a list for "Not sent" with Try again, and file names from the server are checked before they are stored on this PC

0.8.24 adds a list behind the "Not sent" count on Windows: Activity › Not sent shows each change that has not been sent, why, and a Try again button where one can help. It also makes the app check every file and folder name the server sends before it stores or uses it on this PC, and stop a sync write, rename or delete from reaching a different file than the one it names. This release is for Windows and Linux; macOS follows in a later version.

**Known limitations:**
- **Windows, items this PC cannot show:** a file or folder whose name Windows cannot hold (for example `aux`, `10:30.txt` or a name ending in a dot) is not stored and not shown on this PC, and nothing in the app says so yet. Your copy in your vault is not changed. A name changed on another device to one Windows cannot hold keeps its old name and version on this PC; an edit made here then waits as a conflict instead of writing over the other device's change. A list of such items is planned (task 2417).
- **Windows, a sync folder that is itself a link:** if the sync folder is a junction or symbolic link, the app now skips placeholders and remote deletions inside it, and the app shows no message. Whether Windows accepts such a folder as a sync folder has not been checked (task 2417).
- **Windows, a narrow window on download:** a link made on the destination between the app's check and the moment it opens the file is not covered (task 2417).
- **Windows, "Not downloaded":** on Windows a pinned file downloads through a path that is not in the queue, so "Not downloaded N" does not count a pinned file whose download failed, and the In progress card still drops it after 15 minutes (task 2419).
- **Windows, a blocked "Not sent" row:** a change that gave up and is now blocked (a newer version is already in Beebeeb, or the copy to send is gone) says why but has no button; the tray stays "Not in sync" for it (task 2419).
- **Linux:** the Linux app still cannot sign in: it has no support for a system password store, and its sign-in window says "This feature is not available on this device." For Linux this release changes only the core library it is built on.
- **Still as in 0.8.23:** the limitations listed in 0.8.23's notes still apply.

### What's New

- **[Windows] Activity › Not sent lists every change the "Not sent" count counts (task 2409).**
  - Home said "Not sent 2" and the tray said "2 changes waiting to sync", but no screen showed which two. Now one row per change shows the name, a kind (New file, Edit, Folder, Rename, Move, Delete, Restore; a backup change starts with "Backup ·"), the folder, the reason, and when it was queued. The list is on the Timeline tab and loads 100 rows at a time ("Show more").
  - Home's "Not sent N" figure and the tray's waiting, held and given-up lines are links to it. A main window opened from the tray lands on that section.
  - The count and the list are read in one step from the same rules, so they cannot disagree. Downloads (opening or pinning a file) are no longer counted as "Not sent".
  - **Try again** appears on a change that gave up, or that is waiting after an error ("Couldn't send; trying again at HH:MM"). It resets that change's attempts and wakes the sync; the row then reads "Queued". It does nothing for a held change or an unknown one.
- **[Windows] Try again checks first, and "Not downloaded N" appears (task 2414).**
  - Before a change is sent again, the app runs the same checks the sync pass runs. A change whose copy to send is gone, or that a newer version of the file has replaced, is not sent again: its row says "Can't send: the copy to send is gone." or "Can't send: a newer version of this file is already in Beebeeb. Nothing was overwritten.", and Try again is not offered. A change that is still waiting after an error keeps "trying again at HH:MM". A backup change, and an upload that is already recorded as complete, skip these checks.
  - Downloads that failed or are paused (a file you chose to keep on this PC) are counted as "Not downloaded N", shown on Home next to "Not sent" only when N is above 0, linked from the tray, and listed in Activity › Not downloaded. See the Windows limitation above for what it does not count.

### Bug Fixes / Hardening

- **[Windows] A sync write, rename or delete never touches a different file that Windows resolves the name to (task 2407).** Windows can reach one file by more than one name (the short 8.3 name, such as `PROGRA~1`). Before the app writes, renames, deletes, frees up space for or moves a file by name, it now reads the existing entry's own long name from the file system and, if it is not the name the app means, takes its conflict path instead of acting on that entry. This covers 17 places: placeholders, downloads, Keep Mine and Keep Both in a conflict, conflict copies (which now never replace an earlier copy and are numbered "… 2", "… 3"), the conflict window, freeing up space, selective-sync exclusion, a request's edited copy, "Keep on this PC only", remote deletions, Office `~$` owner files and kept or scaffold folders. A download is created with `create_new` and no longer truncates an existing file. On macOS and Linux the check answers without reading the disk. Verified: the new tests fail before the change (native Windows: 18 of 22 failed at the first red, 4 of 29 failed at the delta red) and pass after it; the full native Windows run at `f17c854e` passed (lib 2382 passed, 0 failed, 7 ignored). The app now uses beebeeb-core v0.2.7 (`124389a8`), which adds the check this uses.
- **[Windows, Linux] A file name the server sends is checked before it is stored on this PC (task 2413).** A name from the server is stored only if it is one name (not a path) and every part of its folder path is a name this computer holds as one entry, using beebeeb-core's rules for the host (Windows 255 UTF-16 units, so long CJK and emoji names stay; Linux 255 bytes). A name that fails is refused: nothing is written for it, a copy already stored keeps its path, status, size, times and version, and it is not treated as deleted elsewhere. Items below a refused folder are held, not placed at the top level. A shared item whose name decrypts but is not one name is refused too. One function maps a stored path onto the sync folder (both `/` and `\`, plain Windows names only, exactly one plain component per part), and the placeholder pass, downloads, the Backups tree and the state reconcile use it. On Windows, a download, a remote deletion, Office `~$` owner-file cleanup, the placeholder pass and the Backups read-only state now refuse a path that goes through a junction or symbolic link, as the Unix writer already did; Cloud Files placeholders pass. A name is refused, never rewritten, so a stored path keeps matching the name on disk. Verified: red on `main` (11 of 11 new tests failed), red at each review round (Linux 4 of 16, then 1 of 19; native Windows 7 of 21 and 2 of 24), and green after: native Windows at `4c87ab86`, lib 2406 passed, 0 failed, 7 ignored. Four mutants of the length check each failed a test.

### Verification

- Release code: `release/0.8.24`, cut at `31a1f94944134dcd939d73e61fea570d63d6e165` on `main` (the merge of desktop PR #238), plus the merge of the pull request that adds these notes. Since 0.8.23 (cut at `2633185f`) it adds four pull requests: #236 (task 2409), #237 (task 2407), #239 (task 2414) and #238 (task 2413). The notes change no app code, so the counts below are those of Desktop CI run 42508138225 at `31a1f949`.
- Continuous integration at `31a1f949` (Desktop CI run 42508138225): 8 of 8 jobs passed on the final attempt, 1 of them on the second attempt.
  - Linux, `cargo test --locked`: the library reported `test result: ok. 2544 passed; 0 failed; 7 ignored`, and three integration suites ran 33, 10 and 3 tests, all with 0 failed. 2,590 of 2,590 tests passed.
  - Windows (GitHub's Windows runner), `cargo test`, second attempt: the library reported `test result: ok. 2411 passed; 0 failed; 7 ignored`, and the same three suites ran 33, 10 and 3, all with 0 failed. 2,457 of 2,457 tests passed. The first attempt failed 1 test, `round7_signout_retries_completed_identity_without_reupload`, a Cloud Files test that failed with "The handle is invalid" on GitHub's runner. The re-run passed it with no code change.
  - macOS (not in this release), `cargo test`: the library reported `test result: ok. 2889 passed; 0 failed; 9 ignored`, and the three suites 33, 10 and 3. 2,935 of 2,935 tests passed. At `ae064388` (the merge of #239) the macOS job had failed 1 test, `a_sign_out_leaves_no_account_row_and_no_owner`; it passed at `31a1f949`, and macOS is not in this release.
  - Frontend: `bun test` 2208 pass / 0 fail across 124 files; the same job's `tsc --noEmit` and `eslint` steps passed.
- Native Windows 11, on the maintainer's PC (MSVC), the full library rung (`cargo test --locked`) on each pull request's own head. The release code, `31a1f949`, contains each of these commits (checked with `git merge-base --is-ancestor`), but the merged tip `31a1f949` itself did not get a native run:
  - #236 (task 2409), at `b3b7794c`: 2,399 tests executed, 0 failed, 7 ignored.
  - #237 (task 2407), at `f17c854e` (after the delta fix; the earlier run at `a12f6e39` passed 2361): the library reported 2382 passed, 0 failed, 7 ignored; the three suites 33, 10 and 3 passed.
  - #238 (task 2413), at `4c87ab86` (its last head; the earlier runs at `4fccc1e9` and `ceaf7a35` passed 2398 and 2405): the library reported 2406 passed, 0 failed, 7 ignored; the three suites 33, 10 and 3 passed.
  - #239 (task 2414), at `cafc0f28`: the library reported 2387 passed, 0 failed, 7 ignored; the three suites 33, 10 and 3 passed.
  - These pull requests were built one after another on `main`, so no single native run contains all four; the CI counts above are the only run of the combined code.
- Core: 0.8.23 pinned `f9257d7362376e77b9a04fc0a014d0d2b8a84987` (v0.2.5); 0.8.24 pins `124389a82755970b5ec1722c323a0c08fa909e43` (v0.2.7, `Cargo.lock`).
- Release workflow check: `node scripts/release-channel-checks.mjs notes 0.8.24 alpha --no-macos`, the form the release workflow uses while `[macOS]` items bind, passes on these notes; without `--no-macos` it fails, as it should.
- Real hardware smoke test: the native runs above ran test builds of each pull request's code on the maintainer's PC, not the signed installer. The alpha is installed on the maintainer's Windows 11 PC before it is promoted to stable.
- Not verified:
  - the combined release code natively on Windows (see above);
  - the signed installer on real hardware before the alpha publish;
  - a sync folder that is a junction or link (see the limitation above);
  - Linux beyond the CI build and tests;
  - macOS: not in this release.

### Install / Update

This release is published to the alpha channel first. Windows installs following that channel receive it through the in-app updater; stable installs receive it only once it is promoted to stable.

macOS: not in this release; it follows in a later version.

For a fresh Windows install, download the NSIS `setup.exe` from the GitHub release assets.
