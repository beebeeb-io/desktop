# Beebeeb Desktop 0.2.1 — fixes a Windows update bug that's likely been duplicating installs since day one

This release fixes a real bug found while investigating a duplicate-install report: the update manifest had never told Windows which installer type (NSIS vs. MSI) a client is actually running, so every NSIS-installed client's auto-update was silently applying an MSI update on top — and Windows treats those as two separate apps. It also finishes wiring up two notification preferences and changes the local cache limit's default. Confirmed via both 0.2.1-alpha.1 and 0.2.1-beta.1 that the fixed manifest correctly publishes both `windows-x86_64-nsis` and `windows-x86_64-msi` entries before promoting straight to Stable.

### What's New

- **Local cache limit now defaults to Unlimited**, not 100GB. The 100GB choice is still available in Settings → Advanced; only the out-of-the-box behavior for a fresh install changed — existing installs upgrading from before this default are unaffected either way.
- **Sync-complete and local-cache-warning notifications are real now.** Both showed up as toggles in a previous release but did nothing. "Sync complete" fires once when the device genuinely finishes catching up (waits for conflicts and errors to clear, not just the queue draining). "Local cache warning" fires once when local cache usage crosses 90% of your configured limit, and never fires if the limit is Unlimited.

### Bug Fixes / Hardening

- **Fixed the root cause of duplicate Windows installs.** The update channel manifest has only ever published one untyped Windows entry, pointing at the MSI installer. Tauri's updater looks for an installer-type-specific entry first (matching however the app was actually installed) before falling back to the untyped one — so an NSIS-installed client (the documented fresh-install path) always missed the specific entry and fell through to the untyped one, silently installing MSI on top of its NSIS install. NSIS and MSI keep separate Windows registry bookkeeping, so this created a second Programs-and-Features entry on update. The manifest now publishes both installer types distinctly, with the untyped fallback kept on NSIS. This only prevents *future* duplication — if you already have two Beebeeb entries in Windows Settings → Apps, uninstall the older-dated one manually; this release can't merge them retroactively.

### Verification

- Rust test suite: `cargo test --lib` — 285 passed, 0 failed.
- TypeScript: `bunx tsc --noEmit` clean; `bun test` 13/13 passing.
- The manifest fix was verified against the actual pinned dependency source, not assumed: fetched `tauri-plugin-updater` 2.10.1's `updater.rs` and `tauri-utils` 2.11.3's `platform.rs` directly from their tagged GitHub source, confirmed the updater's target-search order (`{os}-{arch}-{installer}` before the untyped fallback) and that bundle-type detection is a genuine build-time binary patch rather than runtime guesswork. The workflow's manifest-generation `jq` expression was run standalone with mock inputs to confirm it produces the correct installer-typed shape.
- Not verified: an actual end-to-end update cycle on a real NSIS-installed Windows machine going through this fix. The two prior duplicate-install entries on Guus's machine (from before this fix existed) still need manual cleanup regardless of this release.

### Install / Update

Existing desktop installs on the Stable channel receive this release through the in-app updater automatically. For a fresh Windows install, download the NSIS `setup.exe` from the release assets below.
