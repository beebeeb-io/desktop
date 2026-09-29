# Beebeeb Desktop 0.8.5 — macOS is public

Beebeeb Desktop is now available on macOS. Apple Silicon only for this release — Intel Mac
support is a separate follow-up, not silently dropped. The app is Developer ID signed and
notarized by Apple, and installs the same "Beebeeb Drive" Finder location that has been running
on real hardware for weeks: a native File Provider extension, not a synced folder copy. Windows
and Linux carry no source changes in this release — they are rebuilt from the same code as 0.8.4
so every platform ships under one shared version number and one manifest, not because the launch
needed anything from them.

### What's New

- **macOS launch (Apple Silicon / arm64, macOS 14 Sonoma or later required):** download a
  notarized `.dmg` from the GitHub release, or update in place if you already have a
  Developer-signed build. Installs a Finder location (`~/Library/CloudStorage/Beebeeb-Drive`)
  backed by a native File Provider extension — browse, open and save files directly in Finder,
  encrypted on your device before anything leaves it.
- The auto-updater now carries a `darwin-aarch64` entry, so existing macOS installs discover this
  release the same way Windows and Linux installs already do.

### Bug Fixes / Hardening

- **File Provider extension no longer crash-loops on macOS 26 (task 1524):** a hand-written
  `main.swift` that called `NSExtensionMain()` as an ordinary function recursed forever once
  macOS 26's `ExtensionFoundation` started re-invoking the process's real entry point during its
  own bootstrap. The extension binary now has no `main()` of its own — its Mach-O entry point is
  set purely via the linker (`-e _NSExtensionMain`), exactly as Xcode does for every extension
  target. `scripts/macos-release-preflight.sh` now asserts this on every build (source scan +
  built-binary entry-point check) so it can't regress unnoticed.
- **User-disabled File Provider domains are now detected** instead of the app assuming an
  install always succeeds silently.
- **Uploads resume instead of restarting after an interruption** (flow 7): the chunk-upload
  session and acknowledged-chunk watermark are persisted, so a retry continues from where it left
  off instead of re-uploading a large file from byte zero.
- **0-byte files now upload** instead of failing forever with no user-visible error.
- **Orphaned upload sessions are cleaned up** when the file they belonged to is deleted mid-upload.

### Verification

- `bun test` (frontend): 98 pass, 0 fail, 21 files.
- `cargo test --locked` (src-tauri, all binaries): 423 passed, 0 failed (`beebeeb_desktop_lib`
  403, `beebeeb_desktop` main 0, `keychain.rs` integration 20, doc-tests 0).
- `scripts/macos-release-preflight.sh` — source checks and, after building, the built `.app`:
  entitlements/Info.plist lint, File Provider entry-point guard, embedded Developer ID
  provisioning profiles on both the app and the extension.
- Signed: `codesign --verify --deep --strict` on the app, the File Provider `.appex`, and the
  `BeebeebFileProviderCtl` helper, each showing `Authority=Developer ID Application: Devidee B.V.
  (R8352WDJJR)` and the hardened runtime flag.
- Notarized: `xcrun notarytool submit --wait` → `Accepted`; `xcrun stapler staple` + `validate` on
  both the `.app` and the `.dmg`; `spctl -a -vv` (app) and `spctl -a -vv -t install` (dmg) →
  accepted, source=Notarized Developer ID.
- Release workflow: the shared `bun test` + `cargo test --locked` gate (the counts above) runs
  once on `ubuntu-latest` before any platform builds; Windows and Linux are then rebuilt by
  `.github/workflows/release.yml`'s `tauri-action` step on `windows-latest`/`ubuntu-latest` from
  that same gated, unchanged source (no test suite re-runs per platform — the gate is what's
  tested, the platform matrix is what's built).
- Not verified in this release: a real universal (Intel + Apple Silicon) macOS build — Apple
  Silicon only ships today; Intel Mac users should wait for a follow-up release.

### Install / Update

Existing desktop installs (0.3.0 or later, on any platform) receive this release through the
in-app updater automatically. For a fresh macOS install, download the `.dmg` from the GitHub
release and drag Beebeeb into Applications — first launch is Gatekeeper-clean (notarized). For a
fresh Windows install, download the NSIS `setup.exe` from the GitHub release assets.
