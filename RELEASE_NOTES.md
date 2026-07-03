# Beebeeb Desktop 0.1.2-beta.2 — the tray fix that actually reaches you

0.1.2-beta.1 shipped a taller tray flyout, but existing installs never saw it: the app restores each window's last saved size on launch, so the old small size kept winning over the new default. This beta removes the tray flyout from that persistence entirely, corrects its on-screen position on scaled displays, and adds a manual update check to Settings. It is a beta-channel release; the stable 0.1.2 cut follows once these fixes are confirmed on real hardware.

### What's New

- **Check for updates on demand.** Settings → Updates has a "Check for updates" button that immediately checks the release channel you have selected — no more waiting for the next scheduled check after switching between Stable and Beta. It reports one of three honest states: checking, up to date (with your current version and channel), or update available, which hands off to the existing one-click update banner.
- **Authored release notes.** Starting with this release, every release ships with written notes like these instead of a bare auto-generated changelog, and the in-app updater shows them. The release pipeline now refuses to publish without notes for the exact version being cut.

### Bug Fixes / Hardening

- **Tray flyout stuck at its old, too-small size on existing installs:** the window-state plugin persisted the flyout's size per user and restored it at launch, overriding the corrected 560px height shipped in beta.1. The tray flyout (and the onboarding window) are now excluded from window-state persistence, so their size is always app-controlled; the main app window still remembers your size and position. Verified against the plugin's source that excluded windows skip the restore path — stale saved state on existing installs becomes inert.
- **Tray flyout mispositioned on scaled displays:** the positioning math subtracted the flyout's logical size (400×560) from the tray click's physical coordinates, so at 125/150/200% Windows display scaling the flyout could sit over the taskbar or away from the icon. Position is now computed from the scale factor of the monitor containing the click (with sane fallbacks), and clamped so the flyout never opens off the left screen edge. Covered by unit tests at 1.0/1.5/2.0 scale.

### Verification

- Rust test suite: `cargo test --lib` — 262 passed, 0 failed (includes new tests for the window-state exclusion list, flyout geometry at three scale factors, the edge clamp, and the manual update check result states).
- TypeScript: `bunx tsc --noEmit` clean; `bun test` 8/8 passing (includes the new update-check view-model tests).
- `monitor_from_point` confirmed present in the pinned tauri 2.11.3 — the Windows-only positioning path is not compile-checked on Linux, so this was verified against the crate source directly.
- Release workflow changes validated with yaml-lint; the fail-closed notes check runs before the build matrix.
- Not verified yet: the tray flyout rendering at its full 560px height and correct position on real Windows hardware with a stale saved window state — this beta exists to be exactly that test.

### Install / Update

Existing desktop installs receive this release through the in-app updater automatically (checked at launch and every 4 hours — or immediately via the new Settings → Updates button). For a fresh Windows install, download the NSIS `setup.exe` from the release assets below.
