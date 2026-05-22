# Releasing the Beebeeb desktop app

This document covers the cross-platform release pipeline: macOS `.dmg`,
Windows `.msi`, Linux `.AppImage` + `.deb`. Tracks Phase 6 (Tasks 14-16)
of `docs/superpowers/plans/2026-05-07-desktop-sync-client.md`.

The TL;DR is at the bottom under [Quick reference](#quick-reference).
Read top-to-bottom on first release; consult by section after that.

## Prerequisites (one-time setup per machine)

```sh
# Tauri CLI is in package.json devDependencies — bun install pulls it.
cd repos/desktop && bun install

# macOS: both arches for the universal-apple-darwin target.
rustup target add aarch64-apple-darwin x86_64-apple-darwin

# Windows: requires the MSVC target. WiX toolset is also needed at
# build time for .msi packaging — Tauri downloads it on first build.
rustup target add x86_64-pc-windows-msvc

# Linux: build host must be Linux. Cross-compilation from macOS is not
# supported by tauri-bundler for AppImage / .deb (linuxdeploy is
# x86_64-Linux-only and can't be invoked from a macOS host without a
# Linux container). Use one of:
#   - a Linux build machine
#   - GitHub Actions ubuntu-latest runner
#   - Docker (image: `tauri/tauri-cli` or build-from-Dockerfile)
```

## Versioning

The version string lives in three places that **must** stay in sync:

- `repos/desktop/package.json` `"version"`
- `repos/desktop/src-tauri/tauri.conf.json` `"version"`
- `repos/desktop/src-tauri/Cargo.toml` `[package] version`

Bump them as a unit. The auto-updater compares the running app's
`CARGO_PKG_VERSION` against the version reported by
`https://releases.beebeeb.io/desktop/{{target}}/{{arch}}/{{current_version}}`,
so a mismatched manifest will silently fail to roll users forward.

---

## Task 14 — macOS `.dmg`

### Current macOS identifiers

These values must match App Store Connect, Certificates/Identifiers/Profiles,
the Tauri config, entitlements, and the File Provider bundle:

| Item | Value |
|------|-------|
| App bundle id | `io.beebeeb.app` |
| File Provider extension id | `io.beebeeb.app.FileProvider` |
| File Provider domain id | `io.beebeeb.app.domain` |
| Shared app group | `R8352WDJJR.io.beebeeb.app.fileprovider` |

If Finder install times out or File Provider logs show entitlement rejection,
verify the downloaded provisioning profiles first. Both the containing app
profile and extension profile must include the exact shared app group. Expo/EAS
does not build or provision this desktop app.

A Tauri warning that the identifier ends in `.app` is known for the current
App Store Connect record. Do not change bundle ids without an explicit product
migration decision.

### Build

```sh
cd repos/desktop
bun run tauri build --target universal-apple-darwin
```

Outputs:

- `src-tauri/target/universal-apple-darwin/release/bundle/macos/Beebeeb.app`
- `src-tauri/target/universal-apple-darwin/release/bundle/dmg/Beebeeb_<version>_universal.dmg`

The build is "universal" (Intel + Apple Silicon in one fat binary).
Arm-only and Intel-only builds are also valid:

```sh
bun run tauri build --target aarch64-apple-darwin     # Apple Silicon only
bun run tauri build --target x86_64-apple-darwin      # Intel only
```

### Bundle config

`src-tauri/tauri.conf.json`'s `bundle.macOS` block currently sets:

| Field                | Value                  | Why                                       |
|----------------------|------------------------|-------------------------------------------|
| `frameworks`         | `[]`                   | We embed nothing — only system frameworks |
| `exceptionDomain`    | `""`                   | No NSAppTransportSecurity exception needed |
| `signingIdentity`    | `null`                 | Set via env var at sign time, not in repo |
| `entitlements`       | `entitlements.plist`   | App sandbox, app group, network, folder picker |
| `providerShortName`  | `null`                 | Filled in once Apple provider id is final |

`entitlements.plist` requests the containing-app sandbox, the Beebeeb File
Provider app group, network access for API sync, and
`com.apple.security.files.user-selected.read-write` for the first-launch folder
picker.

### Code signing (Guus runs this — Apple Developer ID required)

The Tauri build picks up signing identity from one of:

- `APPLE_SIGNING_IDENTITY` env var (preferred for CI)
- `tauri.conf.json` → `bundle.macOS.signingIdentity`

Set the env var before building:

```sh
export APPLE_SIGNING_IDENTITY="Developer ID Application: Initlabs B.V. (TEAMID)"
bun run tauri build --target universal-apple-darwin
```

Confirm the cert is in your login keychain:

```sh
security find-identity -v -p codesigning | grep "Developer ID"
```

If you see no Developer ID Application identity:
1. Sign in to <https://developer.apple.com/account/resources/certificates>.
2. Create a *Developer ID Application* certificate (not "Apple Distribution").
3. Download `.cer`, double-click to install; the matching signing material
   should already be in your keychain from the CSR step.

### Production macOS signing contract

Release builds must use the Developer ID identity owned by Initlabs B.V.:

```text
Developer ID Application: Initlabs B.V. (<APPLE_TEAM_ID>)
```

The repo intentionally keeps `bundle.macOS.signingIdentity` as `null`; the
identity is supplied by `APPLE_SIGNING_IDENTITY` locally or as a GitHub Actions
secret. Never commit a certificate, `.p12`, app-specific password, or updater
signing secret.

Required app entitlements live in `src-tauri/entitlements.plist`:

| Entitlement | Required for |
|-------------|--------------|
| `com.apple.security.app-sandbox` | Required containing-app shape for macOS File Provider. |
| `com.apple.security.application-groups` | Shared File Provider document group with the extension. |
| `com.apple.security.files.user-selected.read-write` | User-approved sync/cache folder access. |
| `com.apple.security.network.client` | API sync and update checks. |
| `com.apple.security.network.server` | Local daemon/socket integration where macOS applies network policy. |

Provisioning profile requirement:

- containing app profile for `io.beebeeb.app` includes
  `R8352WDJJR.io.beebeeb.app.fileprovider`;
- extension profile for `io.beebeeb.app.FileProvider` includes the same app
  group;
- both profiles use the same Team ID and signing certificate family as the
  final build.

The File Provider extension target, once embedded in an Xcode project, must be
signed by the same Team ID as the containing app and use:

- bundle id `io.beebeeb.app.FileProvider`;
- extension point `com.apple.fileprovider-nonui`;
- principal class `$(PRODUCT_MODULE_NAME).FileProviderExtension`;
- `NSExtensionFileProviderDocumentGroup` matching the app-group entitlement;
- matching keychain-access-group values only if the extension ever shares a
  Keychain item with the Tauri app.

Before public release, inspect the built app and extension:

```sh
APP="src-tauri/target/universal-apple-darwin/release/bundle/macos/Beebeeb.app"
scripts/sign-local-macos-app.sh "$APP" # local verification; set APPLE_SIGNING_IDENTITY for release signing
codesign --verify --deep --strict --verbose=2 "$APP"
codesign -dvvv --entitlements :- "$APP"
find "$APP/Contents/PlugIns" -maxdepth 2 -name "*.appex" -print -exec \
  codesign --verify --strict --verbose=2 {} \; -exec \
  codesign -dvvv --entitlements :- {} \;
```

If the File Provider extension is missing from `Contents/PlugIns`, the Finder
drive is not part of the shipped artifact even if the Tauri shell launches.

### Notarisation (Guus runs this — also requires Apple Developer ID)

Notarisation is what stops Gatekeeper from showing
"Beebeeb cannot be opened because the developer cannot be verified."
Apple requires it on every macOS build distributed outside the Mac App
Store.

```sh
# 1. Make sure the .dmg is signed first (the build above does this).

# 2. Submit and wait. Replace the placeholders.
xcrun notarytool submit \
  "src-tauri/target/universal-apple-darwin/release/bundle/dmg/Beebeeb_<version>_universal.dmg" \
  --apple-id "guus@beebeeb.io" \
  --team-id "<TEAMID>" \
  --password "<app-specific-password>" \
  --wait

# Expected: "status: Accepted" within ~5 minutes.
# On rejection, fetch the log: notarytool log <submission-id> ...

# 3. Staple the notarisation ticket so Gatekeeper can verify offline.
xcrun stapler staple \
  "src-tauri/target/universal-apple-darwin/release/bundle/dmg/Beebeeb_<version>_universal.dmg"

# 4. Verify.
spctl --assess --type install \
  "src-tauri/target/universal-apple-darwin/release/bundle/dmg/Beebeeb_<version>_universal.dmg"
# Expected: "<path>: accepted"
```

App-specific passwords are minted at <https://appleid.apple.com> →
Sign-In and Security → App-Specific Passwords. Keep the value out of
shell history (use `read -s NOTARY_PWD` and `--password "$NOTARY_PWD"`).

### Local macOS release preflight

Run this before handing an artifact to a tester:

```sh
cd repos/desktop
scripts/macos-release-preflight.sh

# After a local build, include the artifact path:
scripts/macos-release-preflight.sh \
  src-tauri/target/universal-apple-darwin/release/bundle/dmg/Beebeeb_<version>_universal.dmg
```

Without an artifact argument the script validates JSON/YAML/plist syntax,
checks the updater signing config, checks whether a Developer ID identity is
available locally, and runs the staged secret scan. With a `.dmg` or `.app`
argument it also runs the local trust checks that do not require Apple
credentials (`hdiutil`, `codesign`, and `spctl` as appropriate).

### CI signing (GitHub Actions)

Set repository secrets:

- `APPLE_CERTIFICATE` — base64 of the `.p12` exported from Keychain Access
- `APPLE_CERTIFICATE_PASSWORD` — the export password
- `APPLE_SIGNING_IDENTITY` — the human-readable identity string
- `APPLE_ID` — Apple ID email
- `APPLE_TEAM_ID` — 10-character team id
- `APPLE_ID_PASSWORD` — app-specific password used by Tauri's macOS notarizer
- `TAURI_SIGNING_PRIVATE_KEY` — updater signing secret contents
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` — updater signing key password

The workflow also exports `APPLE_PASSWORD` from `APPLE_ID_PASSWORD` for older
Tauri-action compatibility. Keep both spellings out of local shell history.

`tauri-action` reads these automatically; no extra wiring beyond
declaring them in the workflow's `env`.

### Login item and daemon startup expectations

The app currently uses `tauri-plugin-autostart` with the macOS launcher wired in
`src-tauri/src/lib.rs`. The release contract is:

- The user opts into "Start at login" from the account page or tray menu.
- macOS registers Beebeeb as a user login item. It should start quietly in the
  menu bar/control center, not force the settings window open.
- On startup after reboot, the daemon restores session/config state, rebinds
  the local IPC socket, and leaves the vault locked until the user unlocks.
- File Provider enumeration may show the Beebeeb domain while locked, but
  hydration/upload operations must fail closed with an unlock-required error.

Manual verification on a clean macOS user:

```sh
# After installing the signed/notarized DMG:
open -a Beebeeb
# Enable "Start at login" in Beebeeb.
osascript -e 'tell application "System Events" to get the name of every login item'
# Expected: Beebeeb appears, or System Settings > General > Login Items shows it.

sudo shutdown -r now
# After reboot and login:
pgrep -fl Beebeeb
log show --predicate 'process CONTAINS "Beebeeb"' --last 10m --style compact
```

Do not mark login-item verification complete until the app starts after an
actual user-session login, not just after manually reopening it.

### File Provider domain persistence

The File Provider domain identifier is fixed in
`BeebeebFileProvider/DomainRegistration.swift`:

```text
io.beebeeb.app.domain
```

Do not change this identifier after public release without a migration plan.
macOS treats a new domain id as a different Finder location.

Production verification matrix:

1. Install signed/notarized DMG.
2. Sign in and install the Beebeeb Finder location.
3. Confirm Finder shows `Beebeeb` with `My files`, `Shared with me`, `Offline`,
   and `Conflicts`.
4. Reboot and confirm the domain remains visible.
5. Install a newer signed build over the old one and confirm the same domain id
   remains mounted and does not duplicate.
6. Lock the vault and confirm Finder content operations require unlock.
7. Unlock and confirm enumeration/hydration resume without re-adding the domain.

Useful local checks:

```sh
pluginkit -m -v -p com.apple.fileprovider-nonui | grep -A3 -i Beebeeb
brctl log --wait --shorten 2>/dev/null | grep -i Beebeeb
```

The exact Finder state must still be verified manually on a signed build; plist
syntax checks only prove the extension metadata is parseable.

### Safe uninstall and remount cleanup

Uninstall must not delete remote data. It may remove local registration,
login-item state, sockets, logs, and local cache after pending uploads are
settled or explicitly discarded by the user.

Required cleanup sequence for the product UI or a future uninstall helper:

1. Pause sync and show any pending upload count.
2. If the user chooses "remove local data", delete only local cache/state after
   pending uploads are resolved or abandoned with confirmation.
3. Disable the login item.
4. Remove the File Provider domain with `NSFileProviderManager.remove`.
5. Stop the daemon/control center.
6. Remove the local IPC socket and stale lock file.
7. Remove local state/cache/log directories owned by Beebeeb.
8. Leave Keychain/session material only if the user chooses "keep me signed in";
   sign-out must delete it.

Manual cleanup checklist while the helper is not yet shipped:

```sh
# Run from the signed app/debug helper when available:
# BeebeebFileProviderDomain.remove { ... }

# Confirm the app is not running.
pkill -x Beebeeb || true

# Remove stale IPC/socket artifacts if present. Do not use sudo unless needed.
rm -f "${TMPDIR%/}/beebeeb-fileprovider.sock"
rm -f "$HOME/Library/Application Support/io.beebeeb.app/.beebeeb-sync.lock"

# Inspect local app containers before deletion.
ls -la "$HOME/Library/Application Support/io.beebeeb.app" 2>/dev/null || true
ls -la "$HOME/Library/Logs/io.beebeeb.app" 2>/dev/null || true
```

Do not script `rm -rf` of the application-support directory in release docs
until the daemon exposes pending-upload and cache ownership checks.

---

## Task 15 — Windows `.msi`

### Build

```sh
cd repos/desktop
bun run tauri build --target x86_64-pc-windows-msvc
```

Output: `src-tauri/target/x86_64-pc-windows-msvc/release/bundle/msi/Beebeeb_<version>_x64_en-US.msi`.

WiX toolset is downloaded by Tauri on first build.

### Cross-compiling from macOS

Native cross-compilation from macOS to Windows is fragile (linker
issues, manifest generation, code signing). Recommended path: a
Windows GitHub Actions runner. If you do want to try locally, install
the `cargo-xwin` toolchain — it's not part of this repo's standard
build.

### Code signing (Guus runs this — third-party CA required)

Windows SmartScreen flags unsigned `.exe`s with "Windows protected your
PC." Avoid that with an Authenticode signature backed by a code-signing
certificate from a trusted CA.

1. Buy a code-signing certificate. The two ones Tauri's docs cover:
   - **DigiCert** — fastest issuance, ~$500/yr. EV variant
     (~$700/yr) gets you out of SmartScreen warnings on day one.
   - **Sectigo** — cheaper (~$200/yr), but standard certs build
     SmartScreen reputation slowly (weeks of installs).
2. The CA delivers a `.pfx` (PKCS#12) that bundles cert + signing material.
   Treat as a secret.
3. For local builds, set:
   ```sh
   export TAURI_WINDOWS_CERTIFICATE="$(base64 < cert.pfx)"
   read -r -s TAURI_WINDOWS_CERTIFICATE_PASSWORD
   export TAURI_WINDOWS_CERTIFICATE_PASSWORD
   ```
4. For GitHub Actions, set `WINDOWS_CERTIFICATE` and
   `WINDOWS_CERTIFICATE_PASSWORD` as repo secrets. `tauri-action`
   surfaces these to the bundler automatically.

EV certs require a hardware token (USB) by default. Some CAs offer
cloud-HSM EV certs (DigiCert KeyLocker, Azure Key Vault); those work
with `signtool /tr` but need extra Tauri config — out of scope for
the first release.

### Auto-update signing

Separate from code signing: Tauri's auto-updater verifies update
bundles with **minisign**. The keypair is the one in
`~/.tauri/beebeeb-desktop.key` (see `repos/desktop/CLAUDE.md`'s
"Auto-update signing" section). The public key in `tauri.conf.json`
is what shipped clients trust; rotating it requires shipping an app
update first that contains the new public key, then publishing the
next update signed with the new updater signing secret.

The current pre-launch key was generated without a password. Before the first
public release, rotate it:

```sh
# Generate a protected replacement key outside the repo.
mkdir -p ~/.tauri
bunx tauri signer generate -w ~/.tauri/beebeeb-desktop-release.key
chmod 600 ~/.tauri/beebeeb-desktop-release.key

# Copy the printed public key into src-tauri/tauri.conf.json:
# plugins.updater.pubkey = "<new public key>"

# Store these as GitHub Actions secrets:
# TAURI_SIGNING_PRIVATE_KEY = contents of ~/.tauri/beebeeb-desktop-release.key
# TAURI_SIGNING_PRIVATE_KEY_PASSWORD = password entered during generation
```

Rotation after users already have Beebeeb installed is a two-release process:

1. Release `N` is signed with the old updater signing secret and contains the new public
   key in `tauri.conf.json`.
2. Release `N+1` and later are signed with the new updater signing secret.

Never publish an update signed by a signing secret that the installed app does not
already trust, or the updater will reject it and users will need a manual DMG
install.

---

## Task 16 — Linux `.AppImage` + `.deb`

### Build (must run on Linux — see Prerequisites)

```sh
cd repos/desktop
bun run tauri build --target x86_64-unknown-linux-gnu
```

Outputs:
- `src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/appimage/beebeeb-desktop_<version>_amd64.AppImage`
- `src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/deb/beebeeb-desktop_<version>_amd64.deb`

Tauri's bundler invokes `linuxdeploy` for AppImage and `dpkg-deb` for
.deb. Both must be on `$PATH`. On Ubuntu they come with the
`linuxdeploy` and `dpkg` packages; on Fedora the .deb step needs
`dpkg-dev` from the rpmsphere repo.

### Why this can't run on macOS

`linuxdeploy` is an x86_64-Linux ELF binary. It's not packaged for
macOS, doesn't run under Rosetta, and can't be cross-invoked. Tauri's
upstream advice is unambiguous: build Linux artifacts on Linux.

Practical paths from a macOS dev box:

1. **GitHub Actions** (easiest for releases). Add a job that runs on
   `ubuntu-latest`, installs `libfuse2` + `libwebkit2gtk-4.1-dev`,
   then `bun run tauri build`.
2. **Docker**:
   ```sh
   docker run --rm -v "$PWD":/workspace -w /workspace/repos/desktop \
     ghcr.io/tauri-apps/cli:latest \
     bun run tauri build --target x86_64-unknown-linux-gnu
   ```
   Image rebuilds glibc/webkit2gtk artifacts every run if `target/` is
   on the host filesystem — slow on macOS without `:cached`.
3. **Hetzner Cloud one-shot**: provision a CX22, run the build, pull
   the artifacts down, delete the box. Fastest if doing infrequent
   manual releases.

### `.deb` runtime requirement

`libfuse2` is required by the AppImage runtime, not by the binary
itself. The `.deb` should declare it as a `Depends`. Tauri's deb
template adds it automatically; if it ever drops off, edit the
generated `DEBIAN/control` post-build or add to `tauri.conf.json`'s
`bundle.deb.depends`.

### .rpm and .pacman

Tauri 2 supports both via `bundle.linux.rpm` and `bundle.linux.pacman`
config blocks. We don't ship those for v1 — Beebeeb's Linux user base
is small enough that AppImage + .deb covers >95%, and rpm/pacman add
maintenance cost without proportionate reach.

---

## Cross-platform CI matrix

The launch model is GitHub Actions — `tauri-action` on a release
trigger. Skeleton workflow:

```yaml
# .github/workflows/release.yml (in repos/desktop)
name: Desktop release
on:
  push:
    tags: ['desktop-v*']
jobs:
  build:
    strategy:
      matrix:
        include:
          - { os: macos-latest,    target: universal-apple-darwin }
          - { os: ubuntu-latest,   target: x86_64-unknown-linux-gnu }
          - { os: windows-latest,  target: x86_64-pc-windows-msvc }
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: oven-sh/setup-bun@v2
      - uses: dtolnay/rust-toolchain@stable
        with: { targets: ${{ matrix.target }} }
      - run: bun install
      - uses: tauri-apps/tauri-action@v0
        env:
          APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}
          APPLE_CERTIFICATE_PASSWORD: ${{ secrets.APPLE_CERTIFICATE_PASSWORD }}
          APPLE_SIGNING_IDENTITY: ${{ secrets.APPLE_SIGNING_IDENTITY }}
          APPLE_ID: ${{ secrets.APPLE_ID }}
          APPLE_TEAM_ID: ${{ secrets.APPLE_TEAM_ID }}
          APPLE_PASSWORD: ${{ secrets.APPLE_PASSWORD }}
          WINDOWS_CERTIFICATE: ${{ secrets.WINDOWS_CERTIFICATE }}
          WINDOWS_CERTIFICATE_PASSWORD: ${{ secrets.WINDOWS_CERTIFICATE_PASSWORD }}
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
        with:
          tagName: ${{ github.ref_name }}
          releaseName: 'Beebeeb Desktop ${{ github.ref_name }}'
          args: --target ${{ matrix.target }}
```

The release publishes signed artifacts to GitHub Releases; the
auto-updater feed at `https://releases.beebeeb.io/desktop/...` mirrors
those binaries (rsync from the GH release tarball, or a Cloudflare
Workers redirect — pick one when wiring up the production updater).

---

## Quick reference

| Platform | Build command                                                | Signing requires                | Notes                          |
|----------|--------------------------------------------------------------|---------------------------------|--------------------------------|
| macOS    | `bun run tauri build --target universal-apple-darwin`        | Apple Developer ID + notarytool | Must run on macOS              |
| Windows  | `bun run tauri build --target x86_64-pc-windows-msvc`        | Authenticode cert (.pfx)        | Best on Windows runner         |
| Linux    | `bun run tauri build --target x86_64-unknown-linux-gnu`      | None (signing optional)         | **Must run on Linux** — see §  |

| Secret                          | Where it goes               |
|---------------------------------|-----------------------------|
| Apple Developer ID `.p12`       | macOS Keychain or CI secret |
| Apple app-specific password     | CI secret only (`APPLE_ID_PASSWORD`) |
| Authenticode `.pfx`             | CI secret only              |
| Tauri updater signing secret    | CI secret only (`TAURI_SIGNING_PRIVATE_KEY`) |
| Updater signing secret password | CI secret only (`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`) |
