# Beebeeb Desktop 0.3.0 — release infrastructure and rough-edge cleanup

This release is the "foundations" batch of a broader desktop-perfection push: it hardens the release pipeline itself (tests now gate every release, one build can be promoted across channels instead of rebuilt three times, the auto-updater's signing key moves off its pre-launch passwordless setup) and cleans up three known rough edges from the previous cycle. No new user-facing features — this is deliberately load-bearing plumbing so later releases ship faster and safer.

### What's New

- **Release builds are now gated on tests.** `release.yml` runs the full Rust and frontend test suites before building any installer — a release can no longer ship with a red test suite.
- **Build once, promote to any channel.** A release is now built once with a plain version (e.g. `0.3.0`) and promoted to alpha, beta, or stable by re-running the workflow with `publish_existing=true` — no rebuild per channel. The app's own "current channel" display now comes from which channel manifest actually served the installed build, not from a suffix parsed out of the version string, so it stays correct regardless of how many channels a build has been promoted to.
- **Auto-updater signing key rotation, phase one.** The updater's public key has moved to a fresh, passphrase-protected keypair (fingerprint `C6FADFD59D732197`, replacing the pre-launch passwordless key). This release is signed with the OLD private key, by design — that's what lets every existing install verify and apply it safely. The new key only starts signing releases after this one has had time to reach the installed base; see `docs/RELEASING.md` for the full rollout plan.

### Bug Fixes / Hardening

- **App Activity CPU% is now normalized by core count.** It used to report `sysinfo`'s raw per-core value, so a process using 2 of 8 cores read ~200% instead of ~25% — confusing next to Task Manager's normalized convention. Verified with a new unit test covering the normalization, clamping, and edge cases (zero cores, NaN, negative values).
- **Downgrade confirmation is a real modal, not a browser dialog.** Switching to an older release channel used to pop a native `window.confirm()` — unstyled, off-brand, and inconsistent with every other confirmation in the app. It now uses the same modal component as the release-notes viewer, with proper keyboard dismissal and focus handling.
- **View → Shared no longer dead-ends.** The Shared menu item routed to an empty "no shared roots available" page, since desktop was never wired into the real sharing feature (that's coming in a later release). The menu item is removed until there's something real behind it, rather than leaving a confusing dead end in front of users.

### Verification

- Rust: `cargo test --locked` — 291 passed, 0 failed, run unsandboxed against the fully-merged branch (not just each change in isolation).
- Frontend: `bun test` — 15 passed, 0 failed. `bunx tsc --noEmit` — clean.
- Release workflow: YAML-parsed and the full job dependency graph manually reviewed to confirm the new test gate and channel-promotion logic compose correctly (including that a `publish_existing=true` run correctly skips the test gate via GitHub Actions' implicit skip-propagation, without needing a redundant condition).
- Updater key rotation: `scripts/verify-updater-key-rotation.mjs` — confirms an old-pubkey install accepts an old-key-signed transition release, a new-pubkey install accepts a new-key-signed release, and each install correctly *rejects* a signature made with the wrong key. Uses throwaway keys; never touches the real production private key.
- Not verified: a live promote-only (`publish_existing=true`) workflow run — this release's own alpha→beta→stable promotion is the first real exercise of that path. Not verified on real Windows/macOS hardware (native menu bar changes, tray, and OS-level chrome can't be confirmed by the headless verification technique used above).

### Install / Update

Existing desktop installs receive this release through the in-app updater automatically, signed with the same key your install already trusts. For a fresh Windows install, download the NSIS `setup.exe` from the release assets below.
