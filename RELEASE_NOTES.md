# Beebeeb Desktop 0.7.0 — restore an old version of a file

You can now browse and restore earlier versions of a file directly from desktop, without ever leaving the app.

### What's New

- **Version history.** Open the version history dialog, find a file, and see every earlier version with its size and timestamp. Restoring an old version queues the restore through the same reliable background sync mechanism the rest of the app already uses — you'll get a confirmation the moment it's queued, and the file updates on the next sync tick, the same way any other change does.
- This uses server infrastructure that already existed and already worked — desktop just never had a way to reach it until now.

### Bug Fixes / Hardening

- The existing conflict-resolution page's restore action now uses the same reliable, queued restore path as the new version history dialog, instead of racing an immediate request first. Its confirmation message was also corrected to accurately describe this (previously implied an instant result; now honestly says the restore is queued for the sync engine).

### Verification

- `cargo test --locked` — 317 passed, 0 failed. `bunx tsc --noEmit` — clean. `bun run lint` — exits 0. `bun test` — 19 passed, 0 failed.
- Because this release changes the behavior of an existing command (used by an existing page, not just the new one), that change was checked specifically for side effects: every place in the app that listens for the events involved was read and confirmed to still behave correctly.
- Not verified: this feature has not been exercised on real hardware, and no live two-device version-history scenario has been run against a production server.

### Install / Update

Existing desktop installs receive this release through the in-app updater automatically. For a fresh Windows install, download the NSIS `setup.exe` from the release assets below.
