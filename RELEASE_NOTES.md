# Beebeeb Desktop 0.8.12 — Finder saves and storage location

0.8.12 brings Finder create-and-save uploads on macOS, a sign-out flow for changes
still waiting to upload, and storage location selection by country. Read the
Verification section before relying on the sign-out flow: of the twelve planned
device checks for it, one was run before this release.

### What's New

- **[macOS] Create and save files in Finder.** Finder writes enter a durable
  staging queue and are sent by the desktop engine. The queue and File Provider
  changes merged in PR #118 are included in this release.
- **[macOS] Sign out with a visible waiting-change count.** The new flow tries to
  send waiting changes for up to 60 seconds, offers "Stop waiting", and asks before
  continuing. When signing out with changes still waiting, it copies them to a
  dated "Changes not sent" folder before clearing local account state. Settings
  → Sync can show or dismiss the kept-folder notice. Automated tests cover the
  flow; on a device, only the reachable-server case was checked (PR #130).
- **[macOS] Finder setup and re-sign-in.** The app checks the Finder location
  after launch and sign-in and offers a retry when setup fails. Signing back
  into the same account retains its local queue and Finder location; switching
  accounts uses the account-change flow (PR #117).
- **Storage location by country.** Europe remains the default. Settings renders
  the countries and availability returned by the server, and "Only store in"
  requests a strict country preference (PRs #128 and #129). A strict-country
  upload refusal currently uses the standard upload-error handling rather than
  a dedicated desktop message.

### Bug Fixes / Hardening

- **Concurrent Settings changes preserve each other.** Configuration updates now
  serialize the read, change and write; Settings sends only fields that changed.
  This prevents an older Settings snapshot from overwriting a newer tray pause
  or another preference update (PR #124; automated coverage in
  `config_write_pin_tests` and `settingsWriteOnlyChangedFields.test.tsx`).
- **[macOS] Open in Finder reports the actual result.** The app opens the
  security-scoped File Provider URL through the native bridge and reports an
  opening failure instead of treating a successfully spawned process as success
  (PR #122; automated `test_1885_*` coverage).
- **[macOS] Finder removal requests preservation of unsynced files.** Removal
  uses macOS's preserve-dirty-data mode and surfaces a kept folder when it has
  contents or cannot be inspected. The additional protection for acknowledged
  but still-queued Finder saves is the task 1887 sign-out flow described above
  (PRs #119 and #130).

### Verification

- Product code: desktop `680ea7ca` (PR #130 merged). The release commit adds
  only these notes and removes the task 1887 item from `RELEASE_BLOCKERS.md`.
- Frontend: `bun test` 1163 pass / 0 fail across 69 files (CI on `680ea7ca`,
  and again on a Mac on 2026-10-24).
- Rust, `cargo test --locked` on `680ea7ca`:
  - Linux (CI): library `test result: ok. 1587 passed; 0 failed; 6 ignored`;
    keychain 31 passed, Windows session wiring 8 passed, Windows sign-out
    cleanup 3 passed.
  - Windows (CI): library `test result: ok. 1388 passed; 0 failed; 5 ignored`;
    the same three integration suites, 31 + 8 + 3 passed.
  - macOS (Apple Silicon, 2026-10-24): library
    `test result: ok. 1904 passed; 0 failed; 8 ignored`; 31 + 8 + 3 passed.
- CI on `680ea7ca` also passed the File Provider Swift build and the macOS
  release-script shim test:
  [run 39310409598](https://github.com/beebeeb-io/desktop/actions/runs/39310409598).
- Device check of the sign-out flow (macOS, signed debug build, local test
  server). One of the twelve planned cases was run. Signing out with the
  server reachable and one Finder save waiting: the app counted 1 waiting
  file, sent it in about 1 second, then signed out without asking and without
  copying anything aside. After signing back in, the file's SHA-256 matched the
  original. The other eleven cases were not run before this release: server
  unreachable, Cancel, the menu shortcut, a failed copy, several saves of one
  file, the 60-second limit, account switch, two kept folders, closing Settings
  while sending or while asking, and held changes. They are tracked for a later
  release.
- Not verified before publication: installing 0.8.12 fresh, and updating to it
  from 0.8.11, on physical hardware.

### Install / Update

Existing desktop installs receive this release through the in-app updater. For a fresh Windows install,
use the NSIS `setup.exe`; MSI remains available separately. The Mac build is
Apple Silicon only and requires macOS 14 or later. macOS artifacts follow the
existing local Developer ID signing, notarization and updater-signature process.
