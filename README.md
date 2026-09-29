> **Correction — 2026-09-29, task 1611:** The historical architecture/build
> descriptions below are retained for provenance, not current instructions.
> Shipping entrypoints are desktop-owned `src/main.tsx` and `src-tauri/src/lib.rs`;
> `runner.rs`, `engine_bridge.rs`, and `state_db.rs` own scheduling and sync.
> `repos/web` is not this frontend; core provides shared primitives.
> Run `bun run tauri:dev` / `bun run tauri:build` from the desktop root.
> macOS uses the active `macos/FileProviderExtension` (not FinderSync), Windows
> uses `src-tauri/src/windows_cf` (CFAPI), and Linux FUSE is an unmounted prototype.
> The legacy native-shell build recipes and Linux online-only claims below are
> historical. Spec 030 is historical and never authorizes raw keys in config.
> Credentials belong in the OS store; Linux currently fails closed.
> See [the current capability contract](docs/CAPABILITIES.md) for shell reachability and limitations.

<p align="center">
  <a href="https://beebeeb.io"><img src="https://beebeeb.io/assets/beebeeb-icon.png" alt="beebeeb" width="72" height="72" /></a>
</p>
<h1 align="center">beebeeb desktop</h1>
<p align="center">Native desktop sync for macOS, Windows, and Linux — files encrypted before they leave your machine.</p>
<p align="center"><strong>We can't recover your data. Not even if we wanted to.</strong> That's the point.</p>
<p align="center">
  <a href="https://github.com/beebeeb-io/desktop/releases/latest"><img src="https://img.shields.io/github/v/release/beebeeb-io/desktop?label=release" alt="Release" /></a> &nbsp;
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-AGPL--3.0-555.svg" alt="License: AGPL-3.0" /></a> &nbsp;
  <img src="https://img.shields.io/badge/Windows%20%C2%B7%20Linux%20%C2%B7%20macOS%20(Apple%20Silicon)-555.svg" alt="Windows · Linux · macOS (Apple Silicon)" /> &nbsp;
  <a href="SECURITY.md"><img src="https://img.shields.io/badge/security-policy-555.svg" alt="Security policy" /></a>
</p>
<p align="center"><a href="https://beebeeb.io">Website</a> &nbsp;·&nbsp; <a href="https://beebeeb.io/security">How it works</a> &nbsp;·&nbsp; <a href="SECURITY.md">Report a vulnerability</a></p>
<p align="center"><sub>End-to-end encrypted cloud storage, built in Europe. Operated by Initlabs B.V., Wijchen, Netherlands.</sub></p>

---

## Status — Windows, Linux and macOS (Apple Silicon) released

> **[desktop-v0.8.5](https://github.com/beebeeb-io/desktop/releases/latest) is the latest
> release** — `.msi`/`.exe` (NSIS) for Windows, `.AppImage`/`.deb`/`.rpm` for Linux, and a
> notarized `.dmg` for macOS (Apple Silicon only — Intel is not built yet), each with a
> minisign `.sig` for the auto-updater. Assets are signed with key ID `C6FADFD59D732197` (see
> `docs/RELEASING.md` for the key-rotation history). **Windows and Linux installers do not carry
> an OS-trusted code-signing certificate yet** — Windows shows an "unknown publisher" SmartScreen
> warning until that's wired up. **macOS IS Developer ID signed and notarized by Apple** —
> Gatekeeper accepts it with no warning. macOS builds locally on a Mac holding the Developer ID
> keychain identity rather than in CI (see the matrix comment in
> `.github/workflows/release.yml`) — CI has no Developer ID credentials wired in, and the File
> Provider extension doesn't yet support a universal arm64+x86_64 build, so only Apple Silicon
> ships today. The site's [/download](https://beebeeb.io/download) page always links to
> whatever is current.

The [beebeeb](https://beebeeb.io) desktop client gives you a native sync folder on macOS, Windows, and Linux. Drop files into your beebeeb folder and they are encrypted on your machine and synced to the cloud — the server only ever sees ciphertext. The UI and the sync engine are shared with the rest of beebeeb: a [web](https://github.com/beebeeb-io/web) frontend in a native WebView, plus the Rust sync engine and cryptography from [core](https://github.com/beebeeb-io/core).

## Features

- **Automatic sync** — files in your beebeeb folder are encrypted and synced in the background
- **Selective sync** — choose which folders sync locally; the rest stay as online-only placeholders until you open them
- **Conflict resolution** — never silently drops a version. Default: keep both, rename the older version as `file (Device, HH:MM).ext`
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
| **macOS** | Tauri + Swift File Provider | Menu bar, Finder File Provider location | [Released](https://github.com/beebeeb-io/desktop/releases/latest) — Apple Silicon only, Developer ID signed + notarized (built locally, not in CI — see Status above) |
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
- **Conflict resolution** — never silently drops data. Default strategy is `KeepBoth`: the older version is renamed `file (Device, HH:MM).ext`
- **Selective sync** — the vault stays remote by default. Folders marked locally available are hydrated and kept in sync; other items remain online-only placeholders until opened

## Security

Found a vulnerability? Email **security@beebeeb.io** — see [SECURITY.md](SECURITY.md).

## Part of beebeeb

End-to-end encrypted, zero-knowledge cloud storage — made in Europe.
[core](https://github.com/beebeeb-io/core) · [cli](https://github.com/beebeeb-io/cli) · [web](https://github.com/beebeeb-io/web) · [mobile](https://github.com/beebeeb-io/mobile) · [desktop](https://github.com/beebeeb-io/desktop) · [website](https://beebeeb.io)

## License

[AGPL-3.0-or-later](LICENSE) — © Initlabs B.V. (KvK 95157565), Wijchen, Netherlands.
