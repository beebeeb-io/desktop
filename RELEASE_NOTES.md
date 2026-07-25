# Beebeeb Desktop 0.8.0 — choose where your files live

You can now choose which region stores your new uploads directly from desktop, and see the full list of available regions instead of just a passive mention of your current one.

Desktop already showed your storage region in a few places (Settings, Activity, Insights, onboarding) as a simple inline label. What was missing was any way to see the full picture or change it — that gap is closed with a dedicated Data Residency settings section.

### What's New

- **Data residency settings.** A new "Data residency" section under Settings lists every available storage region with its location (city and country — never the underlying provider, per our brand rule), marks the current default, and lets you pick a different one when more than one is available. Selecting a region updates immediately with a confirmation toast; if the save fails, the selection rolls back and you get a clear error instead of a silently stuck UI.
- Only one region (Falkenstein, Germany) is live in production today, so the section shows that clearly rather than offering a non-functional choice. The full selection flow is built, tested, and ready for the moment a second region goes live.

### Bug Fixes / Hardening

- None this release — this is a pure feature addition. The existing passive region labels used elsewhere in the app (Settings, Activity, Insights, onboarding, Selective Sync) are untouched.

### Verification

- `cargo test --locked` — 320 passed, 0 failed (317 baseline + 3 new tests covering the set-region request/response shape and the server-error/rollback path).
- `bunx tsc --noEmit` — clean. `bun run lint` — exits 0. `bun test` — 22 passed, 0 failed.
- The new client request was checked directly against the real server route handler (`PUT /api/v1/me/region` in the server repo) rather than assumed — request shape, error codes, and response fields all confirmed to match exactly.
- The optimistic-update-and-rollback logic and the "never show the storage provider name" brand rule were both confirmed by reading the actual UI source, not just a screenshot.
- Not verified: this feature has not been exercised on real hardware, and there is no live second region to test an actual region switch against in production today.

### Install / Update

Existing desktop installs receive this release through the in-app updater automatically. For a fresh Windows install, download the NSIS `setup.exe` from the release assets below.
