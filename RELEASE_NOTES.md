# Beebeeb Desktop 0.2.0-beta.1 — Trash, a real menu bar, and a working Advanced page

0.2.0-alpha.1 shipped everything built since 0.1.2-beta.2: local Trash support, a fully wired native menu bar, and four new Advanced/Settings features (theme, local cache limit, App Activity, and a notification audit). This beta promotes the same build to the wider beta channel — no code changes since the alpha, just a channel promotion once the alpha survived initial testing.

### What's New

- **Trash support.** Deleting a file now moves it to Trash instead of vanishing — the tray's recent-activity feed shows deletions for the first time, and a new Trash view (with the same restore/permanent-delete affordances as the web app) is reachable from the sidebar. Permanent delete asks for a password re-confirmation.
- **A real native menu bar.** Beebeeb / File / Edit / View / Help replace the previous default/placeholder menu on every platform (not just macOS). Every item does something real — About, Preferences/Settings, Check for updates, Pause/Resume syncing, Sign out, Quit, Open folder, Upload files, New folder, Open web app, view switches, zoom, and help links — each with a sensible keyboard accelerator.
- **Theme.** Settings → Advanced now has a Light / Dark / System control. It applies immediately, no restart, and persists across launches.
- **Local cache limit.** Also in Advanced: cap how much disk space cached/pinned file content may use on this device (5GB / 25GB / 100GB / Unlimited). Exceeding the cap evicts the least-recently-used unpinned files first; if pinned content alone exceeds the cap, you get a clear warning instead of a silent failure.
- **App Activity.** A live CPU/memory/network panel for the Beebeeb process itself, in Advanced — it only samples while you have the panel open, no background cost otherwise.
- **Desktop alerts, decoupled from account notifications.** Notifications settings now separates account-level alerts (new device sign-in, share received, etc.) from device-level alerts. The conflict-alert toggle was already enforced in the sync engine but had no UI control until now.
- **Update downgrade detection.** Switching your release channel to one behind your currently-installed version (e.g. Beta → Stable when Beta is ahead) now tells you honestly and offers a confirmed downgrade, instead of silently claiming "up to date." The downgrade path re-verifies the target version against the channel manifest immediately before installing.

### Bug Fixes / Hardening

- **Menu bar review caught a fake placeholder before it shipped:** the first implementation pass added a "Shared" page to the Windows app purely so the View menu's Shared item had somewhere to go — no such feature exists yet. Fixed to fall back to Files instead, the same pattern already used for Trash on the compact macOS/Linux shell.
- **Storage total was double-counting pinned bytes:** the existing "on this PC" figure (Settings) and the Status page's cache/pinned line were adding `pinned_bytes` on top of a `cache_bytes` value that already included pinned bytes. Fixed as part of the cache-limit work; both figures are now accurate.
- **"Find a setting" search removed.** It only matched the 6 settings-page names, never the individual controls inside a page, so it returned nothing useful for almost any real query.

### Verification

- Rust test suite: `cargo test --lib` — 279 passed, 0 failed.
- TypeScript: `bunx tsc --noEmit` clean; `bun test` 13/13 passing.
- Real UI screenshots (Vite dev server running the actual React settings code, no Tauri runtime): confirmed the search box is gone, the Advanced page's theme and cache-limit controls render and the theme switch re-themes the whole window live, and the Updates page shows the new Check-for-updates button and channel picker.
- Not verified: the native menu bar itself (OS chrome, invisible in a browser), native OS notifications, and an actual channel-downgrade install — all genuinely need real Windows/macOS hardware. This is the same gap 0.2.0-alpha.1 had; promoting to beta does not close it on its own.

### Install / Update

Existing desktop installs on the Beta channel receive this release through the in-app updater automatically. For a fresh Windows install, download the NSIS `setup.exe` from the release assets below.
