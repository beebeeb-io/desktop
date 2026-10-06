# Beebeeb Desktop 0.8.11 — the Mac app catches up

0.8.10 went out for Windows and Linux without the Mac app. This release is that
same code with the macOS build added and the test results written down. Nothing
in the app changed between 0.8.10 and 0.8.11; the only change in the repository
is these notes. If you use the Mac app, you last received 0.8.9, so the changes
below are new to you.

### What's New

- **[macOS] The Mac app is back on the current release.** Apple Silicon only,
  macOS 14 or later. Existing Mac installs receive it through the in-app updater.
- **[Windows] A revoked session no longer traps you (0.8.10, PR #108):** the app
  checks its stored session once at startup. On a definitive HTTP 401 it clears
  the stored credentials and opens the sign-in form with your email filled in.
  Offline laptops and server errors are never signed out. Sign-out is reachable
  again when the profile fails to load, and its errors name the real failing
  stage.
- **Sign-in tells you who is asking (PR #109):** the sign-in screen now says to
  type the code shown in the app on the page it opened, and to approve only a
  sign-in you started yourself. It used to say to check that the codes match,
  which someone who has been phished cannot do. The sign-in request now reports
  the client, release version, OS and hostname so the approval page can show
  them, labelled as reported by the device. The wording change is on the Windows
  first-run screen; the sign-in request itself is shared code.

### Bug Fixes / Hardening

- **Dead sessions (Windows):** a password change on any device revokes every
  session, and the Windows app used to leave you behind a wall of 401s with no
  way to sign out. Covered by the Windows session tests in the cargo run below.
- **Phishable sign-in wording:** the approval link no longer carries the device
  code and the screen no longer asks you to compare codes. Covered by
  `tests/browserLoginCopy.test.ts` and the `browser_login` unit tests.

### Verification

- Test suite, frontend: `bun test` completed with 558 pass / 0 fail.
- Test suite, Rust: `cargo test --locked` from `src-tauri` on macOS (Apple
  Silicon) completed with `test result: ok. 899 passed; 0 failed; 4 ignored`
  for the library, plus `test result: ok. 20 passed` (keychain),
  `test result: ok. 8 passed` (Windows session wiring) and
  `test result: ok. 3 passed` (Windows sign-out cleanup), 0 failed in every
  binary. The Linux release gate counts platform-specific tests differently and
  reported a lower library count (870) for 0.8.10.
- Release workflow check: the release workflow re-runs both suites as its own
  gate before any installer is built.
- Not verified: no person has exercised this release's installers on real
  hardware yet, on any platform. The macOS build is produced after this release
  is cut, by `scripts/release-macos-local.sh`; nothing about its signing or
  notarization is claimed in these notes.

### Install / Update

Existing desktop installs receive this release through the in-app updater
automatically. For a fresh Windows install, download the NSIS `setup.exe` from
the GitHub release assets. For a fresh Mac install, download the `.dmg` (Apple
Silicon, macOS 14 or later).
