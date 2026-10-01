# Beebeeb Desktop 0.8.7 — Finder: a fix for opening files, awaiting the hand test; support bundle: file names and paths redacted, one stated residual; new macOS Settings window behind a dev URL

This is an alpha build. The Mac build is Apple Silicon only (Intel is not built). It follows up
0.8.6, where opening a not-yet-downloaded file from Finder stopped failing with the "helper
application" error but then failed with "daemon response was not valid json" after a long wait and
with no progress shown. This build fixes the cause of that (see Bug Fixes / Hardening), together
with a privacy fix to the support bundle and two smaller fixes. Whether opening a file from Finder
now works on a real Mac is not verified yet: that hand test is what this alpha is for. Windows and
Linux carry the same merged source as everything else on `main`, but nothing in this release was
written for them.

### What's New

- **[macOS] Real download progress in Finder:** when you open a file that is not on this Mac yet,
  Finder now shows a progress bar that advances as the file is downloaded and decrypted, and the
  download can be cancelled from Finder. Before, Finder showed nothing until the whole download had
  finished.
- **[macOS] Long downloads no longer time out:** a large file may keep downloading as long as it
  keeps making progress. The limit is 10 minutes without any progress, not 10 minutes in total.
  Quick requests (listing a folder) still give up after 30 seconds.
- **The support bundle says exactly what it keeps and removes (task 1685):** the Account page copy
  now states what the export contains, including the one residual described below. The button also
  now saves the bundle and reveals it; before, it showed a toast saying the file had been written
  without writing one.
- **[macOS] A new Settings window exists behind a dev URL only (task 1683 slice 4, PR #85):** one
  window with four tabs (General, Account, Sync, About), sized to its content and built from the
  commands the existing pages already use. It mounts only at the dev URL
  `?window=settings-v2&platform=macos` and no window configuration opens it yet, so nothing a user
  sees changes on any platform in this build. Two bugs found in review of this window were fixed
  before it merged: queued settings writes keep their own snapshot after a failed write (a later
  queued change is still saved, the failed change shows as unsaved), and a failed Finder-state
  refresh after a successful reset no longer leaves the stale "Added" state (the row shows
  unavailable with Try again).

### Bug Fixes / Hardening

- **[macOS] Opening a file from Finder no longer fails with "daemon response was not valid json"
  (task 1670, issue 3):** the daemon's success reply was the bare JSON string `"Ok"`, and the
  Finder extension only accepts a JSON object, so every successful download was reported as a
  failure once it finished. Messages are now one JSON object per line in both directions, the
  extension reads and writes in full (a reply over 64 KiB used to be cut short), and a reply that
  cannot be parsed is reported with a fixed message instead of raw bytes. The old extension with
  the new daemon, and the new extension with the old daemon, both keep working. The daemon side
  was proven failing and then passing by tests; the Swift side is exercised only by the macOS CI
  job and by the hand test this alpha exists for.
- **[P0] The support bundle removes file names and folder paths, with one stated residual (task
  1685):** the diagnostics export promised "no plaintext names", but the last error message it
  carried could include the path of the file that failed, so a bundle sent to support could
  disclose file and folder names. Names and paths known to the state database are replaced first,
  every remaining path becomes `[path]`, and every other word that is not a standard error word or
  a number becomes `[name]`. Credential tokens are still removed. The residual, stated in the app:
  a name the state database does not know can survive if it is a common word or digits, and UUIDs
  and the server address are kept. So the bundle can still contain such a name; it is no longer
  guaranteed name-free, only much less likely to carry one.
- **A slow Finder copy can no longer be uploaded twice (task 1684):** if copying a large file into
  Finder took longer than the extension was willing to wait, Finder retried and the daemon queued a
  second upload of the same file. The extension now sends a key derived from the file's name,
  parent, size and modification time with each write, and the daemon returns the first result for a
  repeat instead of queueing again. It does not cover a retry after the daemon restarts, and
  whether Finder's real retry presents the same size and modification time is not yet confirmed on
  a device; if it does not, the behaviour is the old one (one extra upload), not worse.
- **Error toasts are readable, and an error is shown once (task 1683, slice 5):** in the dark
  theme the error toast title had a contrast ratio of 1.10:1 against its background; all four toast
  types now take their colours from the theme and meet 4.5:1 for text and 3:1 for icons in both
  themes. A failed "Install in Finder" used to show the same message twice, as a toast and as a
  banner that outlived it; it is now one inline error.
- **Groundwork for the menu-bar redesign (task 1683, slice 1):** internal surface logic (window
  anchoring, state tracking, a failure ledger), behaviour-neutral. Only a few small policy
  functions are called by the shipping app, and they reproduce today's behaviour on every
  platform, so this contains no visible change.

### Verification

- Test gate on the release source (lead, Linux, fresh tree at 9119239 = desktop main f79fab3 plus
  the release tooling): `bun test` 419 pass / 0 fail across 36 files; `cargo test --locked`
  (src-tauri) per binary: lib `test result: ok. 652 passed` (2 ignored), keychain
  `test result: ok. 20 passed`, windows_session_wiring `test result: ok. 8 passed`,
  windows_signout_cleanup `test result: ok. 3 passed`, all 0 failed, 683 tests executed, count
  guard exit 0;
  `bunx tsc --noEmit` exit 0. The release workflow re-runs `bun test` and `cargo test --locked`
  as its own gate before any installer is built.
- Re-measured after the notes were amended for PR #85: this amendment (the PR #86 commit) changes
  only `RELEASE_NOTES.md` on top of desktop main 4569db1 (the 1683-slice-4 squash, 3700+ lines of
  application and test code), so the tested code is unchanged from 4569db1: `bun test` 542 pass / 0 fail across
  41 files (2077 expects), `bunx tsc --noEmit` exit 0 and `bun run lint` exit 0 (local macOS
  runs, lock-serialized). The `cargo test --locked` (src-tauri) figures come from the green
  "Rust test (Linux)" CI job of 4569db1 (run 36850579832, cargo exit 0 under the count guard),
  the platform this release gate runs on: lib `test result: ok. 782 passed` (4 ignored), keychain
  `test result: ok. 20 passed`, windows_session_wiring `test result: ok. 8 passed`,
  windows_signout_cleanup `test result: ok. 3 passed`, all 0 failed, 813 tests executed. The
  earlier gate's lib figure (652) predates PR #84, which added the engine-status, popover,
  link-health and transfer-progress Rust suites.
- Desktop CI on the release commit, including the macOS job "File Provider Swift (macOS)", which
  compiles the Finder extension and runs the framing tests with a counted result.
- The macOS build is notarized and stapled by `scripts/release-macos-local.sh`, which refuses to
  upload anything unless `spctl` reports `source=Notarized Developer ID` for both the app and the
  dmg.
- **Not verified: opening a not-yet-downloaded file from Finder on a real Mac against a live File
  Provider domain, with the progress bar and the cancel button.** This alpha exists so that can be
  done by hand before anything reaches the stable channel.
- Not verified: that Finder's real retry of a slow copy presents the same size and modification
  time (task 1684), and that a cancel from Finder ends the download on the daemon side on a device.
- Not verified: Windows and Linux real-hardware smoke tests for this release.

### Install / Update

Alpha channel only: this build does not touch the stable manifest, and installs on the stable
channel will not see it. Download the `.dmg` from the `desktop-v0.8.7` GitHub release and open it by
hand; a Developer ID signed, notarized build installs without a Gatekeeper prompt the same way
0.8.5 and 0.8.6 did. On macOS there is no in-app update path onto the alpha channel yet, so this is
a manual install for testing, not a rollout. (The Windows app has a release-channel picker in its
Settings.)
