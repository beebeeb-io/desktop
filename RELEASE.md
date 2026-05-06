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
| `entitlements`       | `entitlements.plist`   | File-Provider extension + folder picker   |
| `providerShortName`  | `null`                 | Filled in once Apple provider id is final |

`entitlements.plist` requests `com.apple.security.files.user-selected.read-write`
(folder picker on first launch) and `com.apple.developer.fileprovider.extension`
(File Provider host — Apple notarisation rejects extension-hosting bundles
without it).

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
3. Download `.cer`, double-click to install; the matching private key
   should already be in your keychain from the CSR step.

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

### CI signing (GitHub Actions)

Set repository secrets:

- `APPLE_CERTIFICATE` — base64 of the `.p12` exported from Keychain Access
- `APPLE_CERTIFICATE_PASSWORD` — the export password
- `APPLE_SIGNING_IDENTITY` — the human-readable identity string
- `APPLE_ID` — Apple ID email
- `APPLE_TEAM_ID` — 10-character team id
- `APPLE_PASSWORD` — app-specific password

`tauri-action` reads these automatically; no extra wiring beyond
declaring them in the workflow's `env`.

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
2. The CA delivers a `.pfx` (PKCS#12) that bundles cert + private key.
   Treat as a secret.
3. For local builds, set:
   ```sh
   export TAURI_WINDOWS_CERTIFICATE="$(base64 < cert.pfx)"
   export TAURI_WINDOWS_CERTIFICATE_PASSWORD="<password>"
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
next update signed with the new private key.

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
| Apple app-specific password     | CI secret only              |
| Authenticode `.pfx`             | CI secret only              |
| `~/.tauri/beebeeb-desktop.key`  | Build machine, mode 0600    |
