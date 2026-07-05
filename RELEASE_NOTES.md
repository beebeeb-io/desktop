# Beebeeb Desktop 0.2.2-beta.1 — a working Release notes button on the Updates page

0.2.2-alpha.1 fixed a Settings page bug: whenever a downgrade was available, the Updates panel dumped the entire release-notes Markdown body inline as wrapped monospace text — literal `#`/`##`/`-` characters visible, pushing everything below it (including the release-channel picker) far down the page. It's now a compact summary line plus a "Release notes" button that opens a modal with the same text, readable and scrollable. This beta promotes the same build to the wider beta channel.

### Bug Fixes / Hardening

- **Release notes no longer dump raw Markdown inline in Settings → Updates.** The `downgrade_available` card in the Updates panel used to render the full release body directly in the card's flow. A new "Release notes" button opens an in-app modal instead, rendering the body as plain pre-wrapped text (no new Markdown-rendering dependency was added). The existing external button that opens the GitHub release page is unchanged in behavior, relabeled "View on GitHub" so it isn't confused with the new in-app button.

### Verification

- TypeScript: `bunx tsc --noEmit` clean.
- Playwright screenshots of the Updates panel in its idle state, the downgrade-available card (confirming no raw Markdown dump), and the release-notes modal open (confirming scrollable plain-text rendering) — captured against the real running app.
- Not verified: the real `downgrade_available` state reached through actual Tauri updater IPC in this environment (no local mock harness exists for that path) — verified instead via a temporary dev-only state seed that was fully removed before the commit shipped in this release. Not verified on real Windows/macOS hardware.

### Install / Update

Existing desktop installs receive this release through the in-app updater automatically. For a fresh Windows install, download the NSIS `setup.exe` from the release assets below.
