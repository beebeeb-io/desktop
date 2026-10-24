# Beebeeb Desktop 0.8.12 — Finder saves and storage location

0.8.12 brings Finder create-and-save uploads on macOS, a sign-out flow for changes
still waiting to upload, and storage location selection by country. This is
release preparation: task 1887's required device checks remain open, and
`RELEASE_BLOCKERS.md` still prevents desktop publication.

### What's New

- **[macOS] Create and save files in Finder.** Finder writes enter a durable
  staging queue and are sent by the desktop engine. The queue and File Provider
  changes merged in PR #118 are included in this release candidate.
- **[macOS] Sign out with a visible waiting-change count.** The new flow tries to
  send waiting changes for up to 60 seconds, offers "Stop waiting", and asks before
  continuing. When signing out with changes still waiting, it copies them to a
  dated "Changes not sent" folder before clearing local account state. Settings
  → Sync can show or dismiss the kept-folder notice. This implementation is
  covered by automated tests; its required device verification is still pending
  under task 1887 (PR #130).
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
  (PRs #119 and #130). Its device checks remain a release gate.

### Verification

- Source baseline: desktop main `680ea7ca`, including merged PR #130. This change
  updates release notes only; it changes no application code or dependencies.
- Automated CI on that baseline passed all 6 checks: frontend, Rust on Linux,
  Windows and macOS, the macOS release-script shim, and File Provider Swift.
  [Run 39310409598, attempt 2](https://github.com/beebeeb-io/desktop/actions/runs/39310409598)
  includes a successful rerun of the Swift job after its initial hosted-runner
  capacity cancellation. The release candidate's own CI remains pending.
- No product build or test suite was repeated for this notes-only change. The
  canonical release workflow still runs `bun test` and `cargo test --locked`
  before building the required Windows and Linux artifacts.
- Not verified: task 1887's 12 signed-device cases (D1–D11 and D10b), installation
  and updating of 0.8.12 on physical hardware, or signing/notarization of 0.8.12
  release artifacts. The existing signed DEBUG QA app is a functional test
  artifact, not a public release build. No 0.8.12 release is claimed here.

### Install / Update

After the release gates pass and the release is published, existing desktop
installs receive it through the in-app updater. For a fresh Windows install,
use the NSIS `setup.exe`; MSI remains available separately. The Mac build is
Apple Silicon only and requires macOS 14 or later. macOS artifacts follow the
existing local Developer ID signing, notarization and updater-signature process.
