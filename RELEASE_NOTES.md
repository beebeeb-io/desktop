# Beebeeb Desktop 0.8.7 — Finder: opening a file works, and the support bundle no longer names your files

This is an alpha build. The Mac build is Apple Silicon only (Intel is not built). It follows up
0.8.6, where opening a not-yet-downloaded file from Finder stopped failing with the "helper
application" error but then failed with "daemon response was not valid json" after a long wait and
with no progress shown. That is fixed here at the cause, together with a privacy fix to the support
bundle and two smaller fixes. Windows and Linux carry the same merged source as everything else on
`main`, but nothing in this release was written for them.

### What's New

- **[macOS] Real download progress in Finder:** when you open a file that is not on this Mac yet,
  Finder now shows a progress bar that advances as the file is downloaded and decrypted, and the
  download can be cancelled from Finder. Before, Finder showed nothing until the whole download had
  finished.
- **[macOS] Long downloads no longer time out:** a large file may keep downloading as long as it
  keeps making progress. The limit is 10 minutes without any progress, not 10 minutes in total.
  Quick requests (listing a folder) still give up after 30 seconds.
- **The support bundle says exactly what it keeps and removes (task 1685):** the Account page copy
  now states what the export contains, including the one residual described below. The button also
  now saves the bundle and reveals it; before, it showed a toast saying the file had been written
  without writing one. <!-- lead: confirm merged before cut -->

### Bug Fixes / Hardening

- **[macOS] Opening a file from Finder no longer fails with "daemon response was not valid json"
  (task 1670, issue 3):** the daemon's success reply was the bare JSON string `"Ok"`, and the
  Finder extension only accepts a JSON object, so every successful download was reported as a
  failure once it finished. Messages are now one JSON object per line in both directions, the
  extension reads and writes in full (a reply over 64 KiB used to be cut short), and a reply that
  cannot be parsed is reported with a fixed message instead of raw bytes. The old extension with
  the new daemon, and the new extension with the old daemon, both keep working. The daemon side
  was proven failing and then passing by tests; the Swift side is exercised only by the macOS CI
  job and by the hand test this alpha exists for.
- **[P0] The support bundle no longer contains file names or folder paths (task 1685):** the
  diagnostics export promised "no plaintext names", but the last error message it carried could
  include the path of the file that failed, so a bundle sent to support could disclose file and
  folder names. Names and paths known to the state database are replaced first, every remaining
  path becomes `[path]`, and every other word that is not a standard error word or a number
  becomes `[name]`. Credential tokens are still removed. The residual, stated in the app: a name
  the state database does not know can survive if it is a common word or digits, and UUIDs and the
  server address are kept. <!-- lead: confirm merged before cut -->
- **A slow Finder copy can no longer be uploaded twice (task 1684):** if copying a large file into
  Finder took longer than the extension was willing to wait, Finder retried and the daemon queued a
  second upload of the same file. The extension now sends a key derived from the file's name,
  parent, size and modification time with each write, and the daemon returns the first result for a
  repeat instead of queueing again. It does not cover a retry after the daemon restarts, and
  whether Finder's real retry presents the same size and modification time is not yet confirmed on
  a device; if it does not, the behaviour is the old one (one extra upload), not worse.
  <!-- lead: confirm merged before cut -->
- **Error toasts are readable, and an error is shown once (task 1683, slice 5):** in the dark
  theme the error toast title had a contrast ratio of 1.10:1 against its background; all four toast
  types now take their colours from the theme and meet 4.5:1 for text and 3:1 for icons in both
  themes. A failed "Install in Finder" used to show the same message twice, as a toast and as a
  banner that outlived it; it is now one inline error. <!-- lead: confirm merged before cut -->

### Verification

- <!-- lead: fill in on the release commit --> Test gate on the release commit: `bun test`
  pass / 0 fail, `cargo test --locked` (src-tauri) per-binary `test result: ok. N passed`,
  `bunx tsc --noEmit` exit 0.
- Desktop CI on the release commit, including the macOS job "File Provider Swift (macOS)", which
  compiles the Finder extension and runs the framing tests with a counted result.
- The macOS build is notarized and stapled by `scripts/release-macos-local.sh`, which refuses to
  upload anything unless `spctl` reports `source=Notarized Developer ID` for both the app and the
  dmg.
- **Not verified: opening a not-yet-downloaded file from Finder on a real Mac against a live File
  Provider domain, with the progress bar and the cancel button.** This alpha exists so that can be
  done by hand before anything reaches the stable channel.
- Not verified: that Finder's real retry of a slow copy presents the same size and modification
  time (task 1684), and that a cancel from Finder ends the download on the daemon side on a device.
- Not verified: Windows and Linux real-hardware smoke tests for this release.

### Install / Update

Alpha channel only: this build does not touch the stable manifest, and installs on the stable
channel will not see it. Download the `.dmg` from the `desktop-v0.8.7` GitHub release and open it by
hand; a Developer ID signed, notarized build installs without a Gatekeeper prompt the same way
0.8.5 and 0.8.6 did. There is no in-app update path onto the alpha channel yet, so this is a manual
install for testing, not a rollout.
