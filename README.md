<p align="center">
  <a href="https://beebeeb.io"><img src="https://beebeeb.io/assets/beebeeb-icon.png" alt="beebeeb" width="72" height="72" /></a>
</p>
<h1 align="center">beebeeb desktop</h1>
<p align="center">Native desktop sync for macOS, Windows, and Linux — files encrypted before they leave your machine.</p>
<p align="center"><strong>We can't recover your data. Not even if we wanted to.</strong> That's the point.</p>
<p align="center">
  <a href="https://github.com/beebeeb-io/desktop/releases/latest"><img src="https://img.shields.io/github/v/release/beebeeb-io/desktop?label=release" alt="Release" /></a> &nbsp;
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-AGPL--3.0-555.svg" alt="License: AGPL-3.0" /></a> &nbsp;
  <img src="https://img.shields.io/badge/Windows%20%C2%B7%20Linux%20%C2%B7%20macOS%20soon-555.svg" alt="Windows · Linux · macOS soon" /> &nbsp;
  <a href="SECURITY.md"><img src="https://img.shields.io/badge/security-policy-555.svg" alt="Security policy" /></a>
</p>
<p align="center"><a href="https://beebeeb.io">Website</a> &nbsp;·&nbsp; <a href="https://beebeeb.io/security">How it works</a> &nbsp;·&nbsp; <a href="SECURITY.md">Report a vulnerability</a></p>
<p align="center"><sub>End-to-end encrypted cloud storage, built in Europe. Operated by Initlabs B.V., Wijchen, Netherlands.</sub></p>

---

## Status — Windows + Linux released, macOS not yet

> **[desktop-v0.8.4](https://github.com/beebeeb-io/desktop/releases/latest) is the latest
> release** — `.msi`/`.exe` (NSIS) for Windows and `.AppImage`/`.deb`/`.rpm` for Linux,
> each with a minisign `.sig` for the auto-updater. v0.8.4 is the first release signed with key ID
> `C6FADFD59D732197`, the updater key every build since 0.3.0 has trusted; releases up to and
> including v0.8.3 were signed with the pre-rotation key `545D7BA77EDEA7E1` (see
> [Verify your download](#verify-your-download) and `docs/RELEASING.md`). **Neither
> installer carries an OS-trusted code-signing certificate yet** — Windows shows an "unknown
> publisher" SmartScreen warning until that's wired up. **macOS has no published release yet** (development-signed builds already run on real Mac
> hardware with the Finder File Provider mount — see `docs/MACOS_BRINGUP_BRIEF.md`): the
> macOS entry in the release workflow's build matrix is commented out (the File Provider
> extension doesn't yet support universal arm64+x86_64 builds, and no Developer ID/notary
> credentials are configured in CI). The site's [/download](https://beebeeb.io/download) page
> links directly to whatever is current once desktop apps are announced there; until then,
> grab a release straight from GitHub.

The [beebeeb](https://beebeeb.io) desktop client gives you a native sync folder on macOS, Windows, and Linux. Drop files into your beebeeb folder and they are encrypted on your machine and synced to the cloud — the server only ever sees ciphertext. The UI and the sync engine are shared with the rest of beebeeb: a [web](https://github.com/beebeeb-io/web) frontend in a native WebView, plus the Rust sync engine and cryptography from [core](https://github.com/beebeeb-io/core).

## Install

Download the installer for your system from the [latest release](https://github.com/beebeeb-io/desktop/releases/latest). Each installer has a matching `.sig` file next to it — you can use it to [check your download](#verify-your-download) before you install. The examples below use version `0.8.4`; replace it with the version you downloaded.

### Windows (64-bit)

Two installers are published. Pick one and stay with it — updates follow the installer type you started with.

- **`Beebeeb_<version>_x64-setup.exe`** (recommended) — per-user installer, no administrator rights needed.
- **`Beebeeb_<version>_x64_en-US.msi`** — for managed machines and software-deployment tools.

The installers are **not signed with a Windows code-signing certificate yet**, so Microsoft Defender SmartScreen shows **"Windows protected your PC"** the first time you run one. To continue:

1. Click **More info**.
2. Check that the file name is the one you downloaded from `github.com/beebeeb-io/desktop`, and click **Run anyway**.

If you would rather check the file first, [verify it](#verify-your-download) before clicking Run anyway.

### Linux (x86_64)

- **Debian / Ubuntu (`.deb`):**
  ```sh
  sudo apt install ./Beebeeb_0.8.4_amd64.deb
  ```
- **Fedora / RHEL / openSUSE (`.rpm`):**
  ```sh
  sudo dnf install ./Beebeeb-0.8.4-1.x86_64.rpm      # openSUSE: sudo zypper install ./Beebeeb-0.8.4-1.x86_64.rpm
  ```
- **Any distribution (`.AppImage`):** no install step — make it executable and run it:
  ```sh
  chmod +x Beebeeb_0.8.4_amd64.AppImage
  ./Beebeeb_0.8.4_amd64.AppImage
  ```
  AppImages need FUSE 2. If it does not start, install it (Ubuntu 22.04: `sudo apt install libfuse2`; Ubuntu 24.04 and later: `sudo apt install libfuse2t64`).

### macOS

No macOS release is published yet (see [Status](#status--windows--linux-released-macos-not-yet)). Building from source is covered under [Build & run](#build--run).

## Verify your download

Every installer is signed with [minisign](https://jedisct1.github.io/minisign/). Checking the signature proves the file is exactly what our release workflow built and has not been changed since.

Which key to use depends on the version you downloaded:

| Release | Key ID | Public key |
|---|---|---|
| **`0.8.4` and later** (current) | `C6FADFD59D732197` | `RWSXIXOd1d/6xmhr0T+FTeFjSnInaMvbhgL5vxbt51DFLwl2qIC8jU6Z` |
| `0.8.3` and earlier | `545D7BA77EDEA7E1` | `RWThp95+p3tdVI3intgGfThMBLsjCtlCF7eV0hTY478SM1BoLKwU4Fl4` |

The current key is the same one the app itself uses to check updates (it is in [`src-tauri/tauri.conf.json`](src-tauri/tauri.conf.json), `plugins.updater.pubkey`). The `.sig` files are in the auto-updater's format (a minisign signature, base64-encoded once more), so decode it first, then verify.

**Linux / macOS** (install minisign with `apt install minisign`, `dnf install minisign`, or `brew install minisign`):

```sh
base64 -d < Beebeeb_0.8.4_amd64.deb.sig > Beebeeb_0.8.4_amd64.deb.minisig
minisign -Vm Beebeeb_0.8.4_amd64.deb -P RWSXIXOd1d/6xmhr0T+FTeFjSnInaMvbhgL5vxbt51DFLwl2qIC8jU6Z
```

**Windows** (PowerShell; install minisign with `scoop install minisign`, or download it from the [minisign releases](https://github.com/jedisct1/minisign/releases)):

```powershell
[IO.File]::WriteAllBytes("$PWD\Beebeeb_0.8.4_x64-setup.exe.minisig", [Convert]::FromBase64String((Get-Content Beebeeb_0.8.4_x64-setup.exe.sig -Raw)))
minisign -Vm Beebeeb_0.8.4_x64-setup.exe -P RWSXIXOd1d/6xmhr0T+FTeFjSnInaMvbhgL5vxbt51DFLwl2qIC8jU6Z
```

A good file prints `Signature and comment signature verified`, followed by a trusted comment that names the file. Anything else — `Signature verification failed`, or a message that the key IDs do not match — means **do not install it**; download it again from the release page, and if it still fails, tell us at security@beebeeb.io.

The commands above use the current key. To check an older download (`0.8.3` or earlier), use the `RWThp95…` key from the table instead. If a release's signature does not match the key listed for its version, stop and ask us.

## Updating

**Installed `0.3.0` or later:** the app updates itself. Every build since `0.3.0` checks updates against key `C6FADFD59D732197`, and `0.8.4` is the first release signed with it, so these installs can verify and install `0.8.4` from the update feed. (Releases `0.3.0` to `0.8.3` were signed with the old key `545D7BA77EDEA7E1`, so none of them could update in-app — if you are on one of those, this is the first update you will see.)

**Installed a version older than `0.3.0`:** those builds only trust the old key, so they reject `0.8.4` and every later release. Update by reinstalling once. The key rotation plan is in [`docs/RELEASING.md`](docs/RELEASING.md).

To update by hand (any version): download the newest installer from the [latest release](https://github.com/beebeeb-io/desktop/releases/latest), [verify it](#verify-your-download), and install it over your current version — same installer type as before (setup.exe or .msi on Windows; your package manager on Linux; for the AppImage, replace the old file). Your settings (`desktop.toml` in your user config folder) and your sync folder are outside the install location and are not touched by the installer.

## Features

- **Automatic sync** — files in your beebeeb folder are encrypted and synced in the background
- **Selective sync** — choose which folders sync locally; the rest stay as online-only placeholders until you open them
- **Conflict resolution** — never silently drops a version. A file changed both here and elsewhere is flagged and a conflict window opens; you choose Keep Mine, Keep Theirs, or Keep Both. If you have not chosen after 24 hours, the app applies Keep Both for you, so both versions stay on disk
- **Online-only files** — FUSE mount on Linux, native placeholder APIs on macOS and Windows
- **Zero-knowledge encryption** — per-file keys derived via HKDF; the server stores only ciphertext
- **Native shell, not Electron** — Tauri uses the OS WebView (~10 MB binaries vs ~150 MB), with a Rust backend that imports `core` directly

## Architecture

The UI is the beebeeb web client running inside a native Tauri (Rust) shell. The same Rust sync engine and cryptography that power the other clients run in the background — no protocol or crypto logic is re-implemented in TypeScript.

```mermaid
graph TD
    WEB["repos/web (React 19 + Vite 6)<br/>UI for files, settings, sharing"]
    TAURI["src-tauri (Rust)<br/>WebView host, IPC, OS integration"]
    SYNC["beebeeb-sync (Rust, from core)<br/>File watcher, conflicts, selective sync"]
    CORE["beebeeb-core (Rust, from core)<br/>AES-256-GCM, Argon2id, HKDF, BIP39"]
    OS["macOS / Windows / Linux<br/>Menu bar, tray, notifications, autostart"]

    WEB --> TAURI
    TAURI -- "Cargo dep" --> SYNC
    SYNC --> CORE
    TAURI --> OS
```

## Platform support

| Platform | Shell | Integration | Status |
|---|---|---|---|
| **macOS** | Tauri + Swift File Provider | Menu bar, Finder File Provider location | Not released — dev-signed builds run on real hardware; release build + Developer ID/notarization not wired into CI |
| **Windows** | Tauri (WinUI WebView) | System tray, Explorer overlay icons | [Released](https://github.com/beebeeb-io/desktop/releases/latest), not code-signed (SmartScreen warns) |
| **Linux** | Tauri (WebKitGTK) | Tray indicator, FUSE mount for online-only files | [Released](https://github.com/beebeeb-io/desktop/releases/latest) (AppImage/.deb/.rpm) |

## Build & run

Requires [Rust](https://rustup.rs/) (stable, edition 2024), [bun](https://bun.sh/), and [Tauri's per-OS prerequisites](https://tauri.app/start/prerequisites/).

```sh
bun install
bun run tauri:dev      # spawns Vite on :5173, opens a native window
bun run tauri:build    # produces .dmg / .msi / .deb / .AppImage
```

To develop against the real web client instead of the placeholder, run `repos/web` separately (`cd ../web && bun dev`) — the Tauri window loads it automatically via `devUrl` in `tauri.conf.json`. `cargo check` from `src-tauri/` validates the Rust side without the frontend.

## Sync engine behavior

The sync engine (`beebeeb-sync`, from the [core](https://github.com/beebeeb-io/core) repo) handles:

- **File watching** — debounced at 100 ms; ignores `.DS_Store`, `Thumbs.db`, and temp files
- **Conflict resolution** — never silently drops data. When a sync tick finds a file changed both on this device and on the server since the last synced version, it marks the file as a conflict, opens the conflict window (with a line diff for text files, sizes for other files) and sends a notification. The file on disk is left as it is until the conflict is resolved:
  - **Keep Mine** uploads your local copy as the new server version.
  - **Keep Theirs** replaces your local copy with the server's version.
  - **Keep Both** renames *your local copy* to `name (conflict - <device name> - <YYYY-MM-DD>).ext` (date in UTC) and downloads the server's version under the original name.
  - **After 24 hours with no choice**, every sync tick checks for conflicts older than that and applies Keep Both automatically (`sweep_auto_resolutions` in `src-tauri/src/runner.rs`). If the download of the server's version fails, your renamed copy stays on disk and the next tick retries.
- **Selective sync** — the vault stays remote by default. Folders marked locally available are hydrated and kept in sync; other items remain online-only placeholders until opened

## Contributing

Bug reports and pull requests are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md) for the build, the test gate, and how pull requests are reviewed.

## Security

Found a vulnerability? Email **security@beebeeb.io** — see [SECURITY.md](SECURITY.md).

## Part of beebeeb

End-to-end encrypted, zero-knowledge cloud storage — made in Europe.
[core](https://github.com/beebeeb-io/core) · [cli](https://github.com/beebeeb-io/cli) · [web](https://github.com/beebeeb-io/web) · [mobile](https://github.com/beebeeb-io/mobile) · [desktop](https://github.com/beebeeb-io/desktop) · [website](https://beebeeb.io)

## License

[AGPL-3.0-or-later](LICENSE) — © Initlabs B.V. (KvK 95157565), Wijchen, Netherlands.
