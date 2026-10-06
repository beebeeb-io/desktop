# Beebeeb Desktop 0.8.11 — the Mac app catches up

0.8.10 went out for Windows and Linux without the Mac app. This release adds the
macOS build and puts the test results on record. The app itself behaves the same
as 0.8.10: the only code change is in unit-test code (below), and the rest of the
repository change is these notes. If you use the Mac app, you last received
0.8.9, so the changes below are new to you.

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

- **A failing Windows test is fixed (PR #111, test only):** the Windows CI check
  had been red since the 0.8.10 sign-out fix merged, so 0.8.10's Windows build
  shipped while its own Windows test run read `782 passed; 1 failed`. The failing
  test was `signout_teardown_tests::clear_session_when_already_signed_out_skips_the_engine_and_succeeds`.
  On Windows, sign-out runs the local-state purge even when you are already signed
  out, and the test never set up the state directory the real app sets at
  startup. The app was not affected; the test was incomplete. After the fix the
  same Windows check reads `783 passed; 0 failed`.

- **Dead sessions (Windows):** a password change on any device revokes every
  session, and the Windows app used to leave you behind a wall of 401s with no
  way to sign out. Covered by the Windows session tests in the cargo run below.
- **Phishable sign-in wording:** the approval link no longer carries the device
  code and the screen no longer asks you to compare codes. Covered by
  `tests/browserLoginCopy.test.ts` and the `browser_login` unit tests.

### Verification

- Test suite, frontend: `bun test` on this release's tree completed with
  558 pass / 0 fail.
- Test suite, Rust (macOS, Apple Silicon): `cargo test --locked` from `src-tauri`
  on this release's tree completed with
  `test result: ok. 899 passed; 0 failed; 4 ignored` for the library, plus
  `test result: ok. 20 passed` (keychain), `test result: ok. 8 passed` (Windows
  session wiring) and `test result: ok. 3 passed` (Windows sign-out cleanup),
  0 failed in every binary.
- Windows CI on the code of this release (PR #111, same tree as `main` at the
  fix): `Rust check (Windows)` ran `783 passed; 0 failed; 2 ignored`. On 0.8.10
  the same check ran `782 passed; 1 failed`.
- Linux CI on the same code: `870 passed; 0 failed; 4 ignored`, after one rerun.
  The first run failed `link_health::tests::a_refused_connection_is_offline`
  (a local network test: it saw "server did not answer" instead of "refused")
  and passed on the rerun with no code change. That test is not touched by this
  release and may be flaky; it has not been investigated.
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
