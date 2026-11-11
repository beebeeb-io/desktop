# Beebeeb Desktop 0.8.25 - Windows: file and folder names come back after a full re-sync, and updates are checked more strictly

0.8.25 is a fix release for Windows and Linux. It repairs a fault that made the app show "Unavailable file" instead of the names of your files and folders after it read your whole file list again, and it makes the app stricter about which update it accepts. It also prepares Windows installs for the next version of the app, 0.9.0. Your files in your vault were never changed by the naming fault: only what this PC showed was wrong.

**Known limitations:**
- **Windows, after the names come back:** the app renames each affected file and folder on this PC back to its real name, one by one. With many files this can take a while. <!-- lead: state what the native rehearsal saw in Explorer for an install that was hit (leftover "Unavailable file …" folders or none), or remove this item -->
- **Windows, moving to 0.9.0:** an install must be on 0.8.25 before it is offered 0.9.0. This happens by itself through two updates: first to 0.8.25, then to 0.9.0.
- **macOS is not protected by the stricter update rules yet.** 0.8.25 is not released for macOS, so a Mac stays on its current version and keeps reading today's update address, checked with the first update key only. Someone able to change the update list there could still make a Mac show an update that is not the newest one. A Mac on 0.8.21 or later never installs an update by itself: it shows a "Download update" link to the Beebeeb site.
- **Linux:** the Linux app still cannot sign in: it has no support for a system password store, and its sign-in window says "This feature is not available on this device."
- **Still as in 0.8.24:** the limitations listed in 0.8.24's notes still apply.

### What's New

- **[Windows] Ready for the next version of the app.** 0.9.0 will be a new build of Beebeeb for Windows. From 0.8.25 on, a Windows install looks for it at a new update address first. A list of updates at that address must be signed by Beebeeb with the new update key, for this app and for the channel you follow, before the app uses it, and it may only offer 0.9.0 or later. The app also refuses a list that says it was signed before the new key existed, one signed more than 24 hours ahead of this PC's clock, and one older than a list it already accepted. If the list is not there yet, the app uses today's address as before. If the list is there and anything about it is wrong, the app refuses it and installs nothing: it does not fall back to the old address.
- **[Windows] [Linux] Two update keys, each for its own versions.** 0.8.25 trusts two Beebeeb update keys and never mixes them. The key every release so far was signed with is accepted only at today's update address and only for versions below 0.9.0. The new key is accepted only at the new address and only for 0.9.0 and later. An update signed with the right key for the wrong kind of version is refused. 0.8.25 itself is signed with the first key, so every earlier install accepts it.
- **[Windows] Sync is stopped cleanly before 0.9.0 installs.** Before the installer of 0.9.0 or later starts, the app lets uploads that are running finish (it waits up to two minutes), stops sync, and disconnects the Beebeeb folder from Explorer. If an upload is still running after two minutes, the app does not install the update, interrupts nothing, and says so; try again when the upload has finished. An upload that starts in the last moment before sync stops can still be cut; it stays queued and is sent again after the update.

### Bug Fixes / Hardening

- **[Windows] File and folder names show again after a full re-sync (task 2446).**
  - **Before:** since 2 November 2026 the server's full file list carries three empty fields on every ordinary file. The app read an empty field as "this file came in through a file request", could not make sense of it, and marked the file as unavailable before it even tried the file's own key. Every file and folder of a fresh sign-in, or of an install that read the full list again, showed as "Unavailable file" followed by a code.
  - **Now:** an empty field means what it says, and the file's own key is used. An install that was already hit reads the full list once more by itself and gives every file and folder its name back. A file that is rightly unavailable stays unavailable: the repair never unlocks anything by itself. If that read fails, sync goes on as before and the app tries again later: after a minute, then after twice as long each time, at most once an hour, also after a restart. A file this PC cannot store at that moment, or one that still has a change of yours waiting to be sent, is left exactly as it is and read again at the next try; it is never removed from this PC for that reason.
  - **Tests:** the fault was first reproduced on the 0.8.24 code with rows shaped like the server's (2 new tests, 2 failed), then fixed (2 passed). 4 more tests cover the repair: a failed read leaves it pending, the next good read completes it, a healthy install reads nothing extra, and a rightly unavailable file stays so. Each of 4 deliberate breakages of the repair was caught by a test.
- **[Windows] [Linux] An update cannot be swapped for an older build (task 2449).**
  - **Before:** the app checked that an update was signed by Beebeeb, but not that it was the version the update list claimed. Someone able to change that list could have pointed the app at an older, signed installer with its known faults. This needed control of the release host first; no key was at risk and nothing unsigned could run.
  - **Now:** before it offers or installs an update, the app requires that the version is newer than the installed one, and that the update's signature names the very file being offered and that file's name carries the version. An older installer under a newer version number is refused. Choosing an older version on purpose in Settings (a channel downgrade) still works, and installs exactly the version it names.
  - **Tests:** 23 new tests of the update rules, with keys made inside the tests. Each of 29 deliberate breakages (8 of the first rules, 21 of the key, time and pre-install rules) was caught by a test. <!-- lead: confirm both counts from the PR body ("Delta (H1b)") -->
- **[Windows] The app records which channel an update came from.** After an in-app update on Windows, "current channel" in About could be wrong, because the app wrote it down after the installer had already closed it. It is now written before the installer starts, and taken back if the installer could not be started. If the installer starts and is then cancelled or fails, "current channel" can still name the channel of the update that did not finish.
<!-- lead: keep the next item only if the key-owner fix (task 2447, lane H2) is merged into this release; fill its tests from that PR -->
- **[Windows] A safer sign-in when two accounts used one PC (task 2447).** If signing out could not remove an account's saved vault key from Windows, and another account then signed in on the same Windows user, the app could have started with the first account's key: the second account's names would not have been readable, and new uploads would have been unreadable on its other devices. The app now records which account a saved key belongs to and refuses a key that is not this account's. No key ever left the PC. An update from 0.8.24 keeps you signed in and does not ask for your recovery phrase.

### Verification

- Release code: <!-- lead: the merge commit of the 0.8.25 pull request on main -->
- Test suite: <!-- lead: `cargo test --locked` per binary ("test result: ok. <N> passed") on Linux, Windows and macOS CI, and `bun test` ("<N> pass / 0 fail"), from the pull request -->
- Native Windows 11: <!-- lead: the native rung's filters and counts -->
- Real hardware smoke test: <!-- lead: install 0.8.24, update to 0.8.25 from a locally served update list: signed in without the recovery phrase, names present, current channel right -->
- Release workflow check: `node scripts/release-channel-checks.mjs notes 0.8.25 alpha --no-macos` passes on these notes.
- Not verified:
  - the move from 0.8.25 to 0.9.0: no 0.9.0 exists yet, so the new update address was tested against a local server with test keys only, and stopping sync before the installer starts was tested without a running sync engine;
  - Linux on real hardware.

### Install / Update

This release is published to the alpha channel first. Windows installs following that channel receive it through the in-app updater; stable installs receive it only once it is promoted to stable.

macOS: not in this release; it follows in a later version.

For a fresh Windows install, download the NSIS `setup.exe` from the GitHub release assets.
