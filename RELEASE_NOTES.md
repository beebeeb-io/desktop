# Beebeeb Desktop 0.8.6 — Finder: truthful state, and opening a file that isn't downloaded yet

This is an alpha build, macOS only. It exists so the Finder fix below can be tested by hand
before it goes anywhere near the stable channel — it is not a general feature release, and
Windows/Linux carry no new work of their own in it.

### What's New

Nothing user-facing beyond the fixes below. This release exists to get the Finder fix in front of
a real Mac.

### Bug Fixes / Hardening

- **[macOS] Finder location pane no longer shows contradictory install state (task 1670):** the
  Settings → Finder location card could show the green "Installed" pill, the amber "Install in
  Finder" button, and "Open in Finder" all at once. The underlying state was already correct —
  the button row just rendered unconditionally. A single `finderLocationButtonPlan(installed)`
  helper now decides the one truthful button to show.
- **[macOS] Opening a not-yet-downloaded file from Finder no longer fails with "Couldn't
  communicate with a helper application" (task 1670):** the File Provider extension and the
  daemon run in separate sandboxes, each with its own private temp directory, so the extension's
  hydration destination was never inside a root the daemon was allowed to write to — every
  hydration was silently rejected. Fixed end to end: the daemon decrypts into the shared App
  Group staging directory both sandboxes are actually entitled to; the extension then copies that
  plaintext into the system-managed `NSFileProviderManager.temporaryDirectoryURL()` and deletes
  our staging copy immediately, handing the system copy's URL to Finder — from that point its
  lifetime belongs to the OS, not to a timer. The daemon's own staging directory is additionally
  swept on a 10-minute TTL as a crash backstop, and purged outright on sign-out, lock, and daemon
  startup. A failure now surfaces as a specific `NSFileProviderError`, not the generic
  helper-application dialog that was masking the real cause.
- **[Windows] Lock/sign-out now revokes in-flight CFAPI callbacks and hydration (task 1639,
  P0):** a lock or sign-out could leave the Windows sync engine mid-hydration with credentials
  and callbacks that outlived the session, including upload-identity finalization, stale
  auth-attempt cancellation, and registry/vault teardown races.
- **[Windows] Recovery-word entry no longer blanks the onboarding WebView (task 1644, P1):**
  Backspace moving focus to the previous field, and a deferred Windows state update landing
  mid-entry, could both wipe recovery-phrase input the user had already typed. Full-phrase paste
  into any field is now honored, and setup render failures show a retry state instead of a blank
  screen.
- **[Windows] The configured sync root is now shown consistently across Files, Settings, and the
  tray (task 1646):** these surfaces could each resolve a different path; they now share the same
  backend-resolved root, and "Open folder" works from all of them. macOS Finder behavior is
  unchanged.
- **Native "Check for updates" now reports its result (task 1665):** the menu action previously
  gave no feedback either way. It now routes through the Settings controller and surfaces success
  or failure with the existing retry toast.
- **Data residency copy no longer depends on provider metadata (task 1664) and now says "Stored
  in the EU" per the brand rule (task 1663):** location copy is derived from the effective
  continent/region instead of a value that could be absent, and macOS/desktop copy was aligned
  with the "Europe" / "the EU" wording used everywhere else.
- **IPC socket is owner-only from the first moment it can be connected to (task 1637):** the
  socket is now bound inside a private sibling directory and published atomically at 0600,
  instead of being creatable-then-chmod'd, closing a narrow window where another local process
  could have raced a connection before permissions were tightened.
- **Deterministic native-parity test fixtures (task 1612):** a reusable, socket-free fixture
  corpus for exercising sync/engine behavior without a live server, used by several of the fixes
  above. No user-facing change.

### Verification

- Gate run by the lead on `99856b2` plus PR #75 (task 1670, merge `01e3c4f`): `bun test` 359
  pass, 0 fail; `cargo test --locked` (src-tauri, all binaries) 466 passed (`beebeeb_desktop_lib`)
  + 0 (`beebeeb_desktop` main) + 20 (`keychain.rs`) + 8 (`windows_session_wiring.rs`) + 3
  (`windows_signout_cleanup.rs`), 0 failed. Independently re-run for this release in a fresh
  worktree at the same commit: identical counts.
- `scripts/build-fileprovider-extension.sh` — the File Provider extension and its
  `BeebeebFileProviderCtl` helper build and codesign cleanly (`codesign --verify` satisfies its
  Designated Requirement on both).
- Desktop CI (`.github/workflows/ci.yml`): 4/4 green on the release PR.
- **Not verified: opening a file from Finder on a real Mac against a live File Provider domain.**
  This alpha exists so that can be tested before anything reaches the stable channel. The task
  1670 notes explain why it couldn't be reached in the development sandbox: this same Mac already
  runs Guus's real Beebeeb.app 0.8.5, the IPC socket path and File Provider domain ID are
  hardcoded singletons under the shared App Group container, and macOS sandbox home resolution
  ignores `$HOME`, so a second, isolated daemon instance risked hijacking the real one's IPC
  channel rather than testing in isolation.
- Not verified: Windows/Linux real-hardware smoke tests for this release specifically (0.8.5's
  hardware-smoke gap is unchanged; these platforms carry the same merged source as everything
  else on `main` since 0.8.5 but are not the reason for this release).

### Install / Update

Alpha channel only — this build does not touch the stable manifest, and existing installs on the
stable channel will not see it. Download the `.dmg` from the `desktop-v0.8.6` GitHub release and
open it by hand; a Developer ID signed, notarized build installs Gatekeeper-clean the same way
0.8.5 did. There is no in-app update path onto the alpha channel yet — this is a manual install
for testing, not a rollout.
