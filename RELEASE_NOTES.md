# Beebeeb Desktop 0.8.12 — Finder saves on macOS

0.8.12 lets you create and save files in Finder on macOS, and adds a sign-out
flow for saves that are still waiting to upload. Read the Verification section
before you rely on that sign-out flow: of the twelve planned device checks for
it, one was run before this release.

### What's New

- **[macOS] Create and save files in Finder.** Finder writes enter a durable
  staging queue and the desktop engine sends them to the server (PR #118).
- **[macOS] Signing out with changes still waiting.**
  - **Sending first.** A sign-out first tries to send waiting changes for up to
    60 seconds, with a visible count and a "Stop waiting" button.
  - **When it asks.** If everything is sent, it signs out without asking. If
    changes are still waiting, it asks first.
  - **What is kept.** On "Sign out", it copies each waiting **file save** to a
    dated "Changes not sent" folder on this Mac, then clears the local account
    state. Settings → Sync can show that folder or dismiss the notice.
  - **What is dropped.** Other waiting changes (renames, moves, deletions, new
    folders, restores) are not copied. Signing out drops them, and those items
    stay as they are on the server. A save whose staged copy is missing cannot
    be kept either. The question before signing out says both.
  - **Testing.** Automated tests cover the flow. On a device, only the case
    where everything was sent was checked (PR #130).
- **[macOS] Finder setup.** The app adds its Finder location itself after launch
  and sign-in, and offers a retry when that fails. The manual "Install in Finder"
  button is gone on macOS. If a session expires and you sign in again to the same
  account, its waiting changes and Finder location are kept. Signing in to a
  different account goes through the account-switch step (PR #117).
- **Storage location by country.** Settings shows the countries the server
  returns, with their availability. Germany is the only live location today, and
  other countries are listed as coming soon. Europe remains the default. "Only
  store in" asks the server to keep new uploads in one country (PRs #128 and
  #129). If the server refuses an upload under that setting, the app shows its
  standard upload error, not a specific message.

### Bug Fixes / Hardening

- **Concurrent Settings changes preserve each other.** Configuration updates now
  serialize the read, change and write, and Settings sends only the fields that
  changed. An older Settings snapshot can no longer overwrite a newer tray pause
  or another preference update (PR #124; automated coverage in
  `config_write_pin_tests` and `settingsWriteOnlyChangedFields.test.tsx`).
- **[macOS] Open in Finder reports the actual result.** The app opens the
  security-scoped File Provider URL through the native bridge. It reports when
  opening fails, instead of treating a successfully spawned process as success
  (PR #122).
- **[macOS] Quick search shows only what Finder has.** Files shared with you are
  not in the Finder location, so a Mac's quick search no longer lists them
  (PR #122).
- **[macOS] Finder removal requests preservation of unsynced files.** Removal uses
  macOS's preserve-dirty-data mode, and shows a kept folder when that folder has
  contents or cannot be inspected. Saves that macOS already handed to the app but
  that are still queued are covered by the sign-out flow above (PRs #119 and #130).

### Verification

- **Product code:** desktop `680ea7ca` (PR #130 merged). The release commit adds
  only release documentation on top of it.
- **Frontend:** `bun test` 1163 pass / 0 fail across 69 files (in CI on
  `680ea7ca`, and again on a Mac on 2026-10-24).
- **Rust**, `cargo test --locked` on `680ea7ca`:
  - **Linux (CI):** library `test result: ok. 1587 passed; 0 failed; 6 ignored`.
    Keychain 31 passed, Windows session wiring 8 passed, Windows sign-out
    cleanup 3 passed.
  - **Windows (CI):** library `test result: ok. 1388 passed; 0 failed; 5 ignored`.
    The same three integration suites: 31, 8 and 3 passed.
  - **macOS (Apple Silicon, 2026-10-24):** library
    `test result: ok. 1904 passed; 0 failed; 8 ignored`. The same three suites:
    31, 8 and 3 passed.
- **Other CI on `680ea7ca`:** the File Provider Swift build and the macOS
  release-script test passed
  ([run 39310409598](https://github.com/beebeeb-io/desktop/actions/runs/39310409598)).
- **Device check of the sign-out flow** (macOS, signed debug build, local test
  server). One of the twelve planned cases was run: signing out with the server
  reachable while one Finder save was waiting.
  - The app counted 1 waiting file and sent it in about 1 second.
  - It then signed out without asking and without copying anything aside.
  - After signing back in, the file's SHA-256 matched the original.
- **Device cases not run before this release** (eleven, tracked for a later
  release):
  - server unreachable
  - Cancel
  - the menu shortcut
  - a failed copy
  - several saves of one file
  - the 60-second limit
  - account switch
  - two kept folders
  - closing Settings while sending
  - closing Settings while asking
  - held changes
- **Not verified before publication:** installing 0.8.12 fresh, and updating to it
  from 0.8.11, on physical hardware.

### Install / Update

Existing desktop installs receive this release through the in-app updater. On a
Mac, that starts once the signed and notarized Mac build has been added to this
release, which happens after the Windows and Linux builds. For a fresh Windows
install, download the NSIS `setup.exe`; the MSI is also available. For a fresh
Mac install, download the `.dmg` (Apple Silicon, macOS 14 or later).
