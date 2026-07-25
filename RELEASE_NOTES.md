# Beebeeb Desktop 0.6.0 — Linux thumbnails, file search, and a real notification system

Three independent improvements: Linux gets real file previews for the first time, every desktop platform gets file search, and the app's error/status messages move off a pile of one-off banners onto a proper notification system.

### What's New

- **Linux thumbnails.** Files synced to Linux now get real preview thumbnails in your file manager — GNOME Files (Nautilus), Dolphin, and most others pick them up automatically, the same way Dropbox or Nextcloud thumbnails work. No extra setup. (Windows already had this; this closes the gap.)
- **File search.** Type to find a file by name from anywhere in the app via a quick-open dialog. This reuses the same encrypted search index the web app already uses — searching works the same way and finds the same files whether you're on desktop or in the browser.
- **Toast notifications.** Status messages ("Settings saved," connection issues, and similar) now appear as a consistent toast instead of each screen inventing its own banner. Confirmation dialogs are also now fully keyboard-navigable — Tab no longer escapes out of an open dialog into the page behind it.

### Bug Fixes / Hardening

- Fixed a real accessibility bug in notification settings where a checkbox's label wasn't properly associated with its control.
- Fixed a real missing-dependency bug in the sync status view, caught by newly-enabled linting rather than by a user hitting a stale-data edge case.
- Added a working lint gate to the codebase (accessibility + React rules) so issues like the two above get caught before they ship, not after.

### Verification

- `cargo test --locked` — 315 passed, 0 failed. `bunx tsc --noEmit` — clean. `bun run lint` — exits 0 against a real, networked dependency install (not just a sandboxed claim). `bun test` — 17 passed, 0 failed.
- The Linux thumbnail format's technical details (size limits, filename hashing, required metadata) were checked against the actual published freedesktop.org specification, not just generated from memory. The file-search wire protocol was checked line-by-line against the web app's real, already-shipped implementation to confirm both clients actually speak the same protocol.
- Not verified: real thumbnail pickup in an actual GNOME/KDE file manager (no Linux desktop environment available in this build environment — the cache format itself is verified correct, just not watched render in a real file manager window). Not verified on real hardware generally.

### Install / Update

Existing desktop installs receive this release through the in-app updater automatically. For a fresh Windows install, download the NSIS `setup.exe` from the release assets below.
