# Beebeeb Desktop 0.8.8 — Windows is signed: installers and app now carry an Authenticode signature from Initlabs B.V., no more "unknown publisher"

Every Windows installer published through 0.8.7 was unsigned, so Windows warned with the
"unknown publisher" / "Windows protected your PC" prompt on every download. This release signs
the Windows artifacts with Azure Artifact Signing: the NSIS `setup.exe`, the MSI, the application
`beebeeb-desktop.exe` inside both installers, and the uninstaller all carry a timestamped
Authenticode signature whose publisher is **Initlabs B.V.** — the verified organization behind
Beebeeb (KvK 95157565, Wijchen). Signing is enforced by a release gate that refuses to publish
any Windows artifact without a valid signature from that publisher (see Verification).

This CI release publishes **Windows and Linux** assets. The macOS build is not part of it: macOS
builds on a local Mac holding the Developer ID identity and is added to the same release and
manifest afterwards (see `docs/RELEASING.md`). macOS users stay on their current version until
that build is published — details under "macOS status". The source in this release also carries
the macOS Finder fix below (task 1694) and two macOS menu-bar slices that are not user-visible
yet.

### What's New

- **[Windows] Signed installers and app:** after downloading, Windows now shows "Initlabs B.V." as
  the verified publisher for `Beebeeb_<version>_x64-setup.exe`, `Beebeeb_<version>_x64_en-US.msi`,
  and the app itself. The signature is RFC3161 timestamped, so it stays valid even though Artifact
  Signing certificates rotate every three days. One honest caveat: with a freshly issued
  certificate, SmartScreen's *reputation* system starts at zero, so on the very first downloads
  Windows may still show a "more info → Run anyway" prompt while reputation accrues with download
  volume (Microsoft treats OV and EV certificates the same here since March 2024). What is gone
  immediately is the "unknown publisher" identity warning — the file is no longer unattributable.

### Bug Fixes / Hardening

- **[macOS] Finder no longer blocks adding files to folders (task 1694):** dragging or copying a
  file into a Beebeeb Drive folder failed because the File Provider item capabilities never
  advertised the "add items" bit. The Rust payload and the Swift mapping now carry it. (Reaches
  macOS users with the macOS 0.8.8 build — see "macOS status".)
- **Nothing unsigned can be published from the release workflow (task 1617):** a gate verifies,
  per artifact, that the Authenticode signature is `Valid`, that the signer subject is
  `CN=Initlabs B.V.`, and that a timestamp counter-signature is present — not only for the
  installers on the release page but for the executables *inside* each installer payload (NSIS
  payload via 7z, MSI payload via a `msiexec /a` administrative extraction). The gate's own
  red-proof runs first on every release build: it must reject an unsigned fixture and a
  wrong-publisher fixture before it is trusted to pass anything. A skipped or broken signing step
  therefore fails the build instead of shipping an unsigned artifact.
- **Release tooling: the WSL `az` wrapper passes arguments faithfully** (workspace tooling, not
  app code): the previous `cmd.exe /c` route mangled arguments containing spaces or quotes, which
  broke Azure CLI JSON payloads during the signing setup.

### macOS status for this release

- No macOS asset ships in `desktop-v0.8.8`; macOS is built locally (Developer ID + notarization)
  and uploaded to this release afterwards, then the update manifest's `darwin-aarch64` entry is
  backfilled (`docs/RELEASING.md`). Until that happens macOS installs stay on their current
  version and are not offered 0.8.8 — the Windows/Linux manifest entries changing does not touch
  them.
- When the macOS 0.8.8 build lands, it carries task 1694 (Finder add-files fix) plus merged
  1683 slices (menu-bar popover backend, a macOS Settings window). The 1683 UI is reachable
  behind a dev URL only and is not user-visible in this build.

### Verification

- The Authenticode gate is proven red-first, with counts (evidence in workspace task 1617):
  guard self-test rejected both fixtures (unsigned bytes: `UnknownError`, 1/1; `cmd.exe`,
  Microsoft-signed `Valid` but wrong publisher, 1/1 — self-test exit 0), and the gate rejected
  the real unsigned 0.8.7 artifacts as fixtures: `Beebeeb_0.8.7_x64-setup.exe` plus its 2 payload
  application PEs (3/3 failed, exit 1) and the 0.8.7 MSI payload app exe (1/1 failed, exit 1).
- The release workflow re-runs `bun test` and `cargo test --locked` as its gate before any
  installer is built, then the Authenticode gate above, then minisign updater signatures are
  produced per installer as before.
- Post-release evidence (CI run URL, per-asset `Get-AuthenticodeSignature` output with SHA-256
  hashes, payload re-verification of the published bytes, manifest check) is recorded in
  workspace task 1617.
- **Not verified:** the SmartScreen first-download prompt behavior noted under "What's New" — that
  needs a clean Windows machine download after publication, watched as reputation accrues.
- **Not verified:** a full Windows real-hardware smoke test of this release (install, sync,
  close-to-tray). The signing change does not alter app behavior, but nothing in this release was
  installed on a device yet.

### Install / Update

Windows and Linux installs on the stable channel are offered 0.8.8 by the auto-updater; fresh
installs come from the `desktop-v0.8.8` GitHub release or the download page. Windows users: the
installer is now signed, so no "unknown publisher" prompt — if you had skipped the warning on an
older build once, you can stop doing that. macOS: stay put until the macOS 0.8.8 build is
published (see "macOS status"); there is no macOS asset in this release to install by hand. (The
Windows app has a release-channel picker in its Settings.)
