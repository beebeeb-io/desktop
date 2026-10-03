# Beebeeb Desktop 0.8.9 — the Finder gets real: your files, thumbnails, Trash, badges, and a new Settings window

This release rebuilds the macOS Finder integration on the machinery Apple's File Provider
framework actually drives: Finder now shows your real file tree, keeps it up to date through a
change feed and sync anchors, renders thumbnails for images and videos, shows sync badges, and
deleted files land in Finder Trash. The redesigned four-tab Settings window (introduced behind a
dev URL in 0.8.8) becomes a real window that the app's menus open. Windows and Linux carry no
behavior changes in this release — their assets are rebuilt from the same source.

### What's New

- **[macOS] Finder shows your files (task 1697):** the Beebeeb volume in Finder was empty even
  though the account had thousands of files, because the extension implemented only a stub
  enumerator — no change feed, no sync anchor, no working-set signal, so macOS had no way to
  learn what changed. The daemon now keeps a change log and anchor, the extension walks it with
  paged enumeration, materialized items are reported back, and item payloads carry real creation
  and modification dates, folder item counts, and separate content/metadata versions. Files
  added, renamed, moved, trashed, or restored by other devices now appear in Finder promptly
  instead of only after a full rescan.
- **[macOS] Deleted files go to Finder Trash (task 1698):** deleting a Beebeeb file in Finder now
  moves it to the server's trash instead of failing or silently dropping it, and the File
  Provider's trash container shows trashed items — including subtrees — in Finder Trash. Trashed
  items are restored from the Beebeeb app or web app; Finder's drag-back and "Put Back" restore
  routes are not wired yet. Emptying Finder Trash on Beebeeb items is not yet wired to permanent
  server deletion either (the server requires a password-confirmed confirmation that a background
  extension cannot provide); items stay restorable in the app or web app until they are purged
  there. Permanently deleting via the app's Trash view is unchanged.
- **[macOS] Thumbnails in Finder (task 1699):** image and video files render thumbnails in
  Finder's icon view, fetched from the server and decrypted by the daemon on demand. Thumbnail
  availability does not bump an item's content version, so the system's thumbnail cache stays
  valid.
- **[macOS] Sync badges in Finder (task 1699):** items show a badge for their sync state —
  uploading, downloading, error, conflict, or trashing — so a file's state is visible without
  opening the app.
- **[macOS] The new Settings window is now the real one (task 1683 slice 6):** Preferences…,
  Settings…, and Check for updates… in the app menus open the redesigned four-tab window
  (General, Account, Sync, About) instead of the legacy 680×540 flyout. It opens top-right under
  the menu bar on the menu-bar icon's display, never centred, and never remembers a position; a
  menu-triggered update check lands the window on its About tab and answers inline. Sign out got
  a destructive red confirm button, and confirm sheets follow a deliberate Enter contract (Enter
  confirms only once the confirm button is focused, so an accidental Enter on open cancels).
- **[macOS] Old File Provider leftovers are cleaned up (task 1698):** installs that ran
  0.8.6-or-earlier registered a second, dead File Provider domain ("Drive") that kept a zombie
  volume in Finder. The app now removes stale domains at startup, and the integration is named
  "Beebeeb" everywhere.
- **[macOS] Transport glitches no longer permanently fail items (task 1698):** a daemon
  connection hiccup during a Finder operation was reported as a definitive failure and never
  retried; it is now classified as transient and retried. Only genuinely invalid requests stay
  definitive.

### Windows and Linux

No behavior changes. Assets are rebuilt from the same source as 0.8.8 and carry the same
Authenticode signatures (task 1617) and minisign updater signatures. Windows test-infra fixes
landed in this release (a flaky CI test made deterministic; local macOS test isolation), but
neither affects the shipped app.

### macOS status for this release

- No macOS asset ships in `desktop-v0.8.9` at publication time; macOS is built locally
  (Developer ID + notarization) and uploaded to this release afterwards, then the update
  manifest's `darwin-aarch64` entry is backfilled (`docs/RELEASING.md`).
- This release ships on the **alpha** channel. Stable-channel installs stay on 0.8.8 until the
  macOS build has been hand-tested (Finder file tree, thumbnails, Trash, the new Settings
  window) and the release is promoted.

### Verification

- The release test gate re-ran the full suite on CI Linux before any installer is built (run
  https://github.com/beebeeb-io/desktop/actions/runs/37072585845 on desktop commit 04061c0 —
  the code this release ships; the notes commit changes only `RELEASE_NOTES.md`, so the tested
  code is unchanged): `bun test` 553 pass / 0 fail across 41 files (2109 expects), with
  `bunx tsc --noEmit` and `bunx eslint .` clean in the same job; `cargo test --locked` —
  library `test result: ok. 852 passed` (4 ignored), keychain integration
  `test result: ok. 20 passed`, Windows session wiring `test result: ok. 8 passed`, Windows
  signout cleanup `test result: ok. 3 passed`, main harness and doctests empty — 883 cargo
  tests passed, 0 failed in total (the macOS-only test subset runs on macOS, where the library
  suite is 881 passed / 0 failed locally).
- The Swift File Provider harness ran green on macOS CI: `ipc-framing: 87 passed, 0 failed
  (expected 87)`.
- **Not verified (device rungs, ride the hand test):** Finder thumbnail and badge rendering in
  icon view, the Trash round-trip against the real domain, the Settings window's on-screen
  placement and natural content height, and the zombie-domain cleanup on an install that ran
  0.8.6 or earlier.

### Install / Update

Installs on the alpha channel are offered 0.8.9 by the auto-updater (Windows/Linux assets at
publication, macOS once the local build is published). Fresh installs come from the
`desktop-v0.8.9` GitHub release or the download page. Stable-channel installs are not offered
this release yet — it promotes after the macOS hand test. (The Windows app has a release-channel
picker in its Settings.)