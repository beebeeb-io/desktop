<p align="center">
  <img src="https://beebeeb.io/icon.png" alt="Beebeeb" width="60" />
</p>
<h3 align="center">Beebeeb Desktop</h3>
<p align="center">Desktop sync client for macOS, Windows, and Linux — files encrypted before they leave your machine.</p>

<p align="center">
  <a href="https://github.com/beebeeb-io/desktop/blob/main/LICENSE"><img src="https://img.shields.io/github/license/beebeeb-io/desktop" alt="License"></a>
  <a href="https://github.com/beebeeb-io/desktop/actions"><img src="https://img.shields.io/github/actions/workflow/status/beebeeb-io/desktop/ci.yml" alt="CI"></a>
  <a href="https://github.com/beebeeb-io/desktop/graphs/contributors"><img src="https://img.shields.io/github/contributors/beebeeb-io/desktop" alt="Contributors"></a>
  <a href="https://github.com/beebeeb-io/desktop/stargazers"><img src="https://img.shields.io/github/stars/beebeeb-io/desktop" alt="Stars"></a>
  <a href="https://github.com/beebeeb-io/desktop/issues"><img src="https://img.shields.io/github/issues/beebeeb-io/desktop" alt="Issues"></a>
</p>

---

> **Early development** -- the sync engine is working and the native platform shells are in progress. Contributions and feedback are welcome.

## What is Beebeeb?

Beebeeb is end-to-end encrypted cloud storage where your files are encrypted before they leave your device. The server never sees your plaintext data, file names, or encryption keys. Beebeeb is open source and built by [Initlabs B.V.](https://beebeeb.io), Wijchen, Netherlands.

## This repo

The Beebeeb desktop client gives you a native sync experience on macOS, Windows, and Linux. Files in your Beebeeb folder are automatically encrypted and synced to the cloud. Each platform gets a native shell that feels at home on your OS, all powered by a shared Rust sync engine from the [core](https://github.com/beebeeb-io/core) repo.

## Platform support

| Platform | Shell | Integration | Status |
|---|---|---|---|
| **macOS** | Swift / SwiftUI | Menu bar app, Finder extension (FinderSync) | In progress |
| **Windows** | WinUI 3 / C++ | System tray, File Explorer overlay icons | In progress |
| **Linux** | Rust + GTK4 / libadwaita | Tray indicator, FUSE mount for online-only files | In progress |

## Tech stack

| Layer | Technology |
|---|---|
| Sync engine | Rust -- [`beebeeb-sync`](https://github.com/beebeeb-io/core/tree/main/beebeeb-sync) crate from the core repo |
| Crypto | [`beebeeb-core`](https://github.com/beebeeb-io/core/tree/main/beebeeb-core) (AES-256-GCM, Argon2id, HKDF) |
| macOS shell | Swift, SwiftUI, Xcode 15+, macOS SDK 14.0+ |
| macOS FFI | C FFI via cbindgen (Rust dylib) |
| Windows shell | WinUI 3, C++, Visual Studio 2022 17.8+ |
| Windows SDK | Windows SDK 10.0.22621+, Windows App SDK 1.5+ |
| Linux shell | GTK4, libadwaita, FUSE 3 |
| File watching | `notify` crate (100 ms debounce) |

## Features

- **Selective sync** -- choose which folders sync locally; the rest stay as online-only placeholders until you open them
- **Conflict resolution** -- never silently drops a version. Default strategy: keep both, rename the older version as `file (Device, HH:MM).ext`
- **Online-only files** -- on Linux, served via a FUSE filesystem; on macOS and Windows, via native placeholder APIs
- **Zero-knowledge encryption** -- files are encrypted with per-file keys (HKDF-derived) before upload; the server stores only ciphertext
- **Native experience** -- each platform gets a shell built with its native UI toolkit, not a cross-platform wrapper

## Architecture

```
+-------------------+     +-------------------+     +-------------------+
|   macOS Shell     |     |  Windows Shell    |     |   Linux Shell     |
|   (Swift/SwiftUI) |     |  (WinUI 3/C++)   |     |   (GTK4/Rust)     |
+--------+----------+     +--------+----------+     +--------+----------+
         |        C FFI            |   Cargo dep             |   Cargo dep
         v                        v                          v
+------------------------------------------------------------------------+
|                         beebeeb-sync (Rust)                            |
|   File watcher  ·  Conflict resolution  ·  Selective sync  ·  Upload  |
+------------------------------------------------------------------------+
         |
         v
+------------------------------------------------------------------------+
|                         beebeeb-core (Rust)                            |
|   AES-256-GCM  ·  Argon2id  ·  HKDF  ·  BIP39  ·  Key management    |
+------------------------------------------------------------------------+
```

The sync engine and cryptographic layer are written once in Rust and shared across all platforms. Platform-specific shells call into the engine via Cargo dependencies (Windows, Linux) or C FFI (macOS).

## Getting started

### Prerequisites

The Rust sync engine requires:

- [Rust](https://rustup.rs/) (stable, edition 2024)

Platform-specific shells have additional requirements -- see below.

### Windows

```sh
# Prerequisites: Visual Studio 2022 17.8+, Windows SDK 10.0.22621+, Windows App SDK 1.5+

cd windows
cargo build
# Binary at target/debug/beebeeb-desktop-windows

# Run with a sync path
./target/debug/beebeeb-desktop-windows --sync-path ~/Beebeeb
```

### macOS

```sh
# Prerequisites: Xcode 15+, macOS SDK 14.0+, cbindgen

cd macos
# Build Rust dylib, then open the Xcode project
cargo build --release
open BeebeebDesktop.xcodeproj
```

### Linux

```sh
# Prerequisites: libgtk-4-dev, libadwaita-1-dev, libfuse3-dev

cd linux
cargo build

# Run as a foreground process, or install as a systemd user unit
./target/debug/beebeeb-desktop-linux --sync-path ~/Beebeeb
```

## Security

All encryption happens on your device using the [beebeeb-core](https://github.com/beebeeb-io/core) crate. The server stores only ciphertext and never has access to your keys or plaintext data.

**Found a vulnerability?** Please report it responsibly. Email [security@beebeeb.io](mailto:security@beebeeb.io). We aim to acknowledge reports within 48 hours.

## Contributing

We welcome contributions! Whether it is a bug report, a feature request, or a pull request -- we appreciate your help making Beebeeb better.

1. Fork the repository
2. Create a feature branch (`git checkout -b feat/your-feature`)
3. Make your changes and add tests where applicable
4. Run the linter (`cargo clippy -- -D warnings` for Rust code)
5. Commit your changes -- pre-commit hooks will run secret scanning automatically
6. Open a pull request against `main`

Since the desktop client spans three platforms, contributions to any single platform are valuable. You do not need to build or test all three.

## Built with Beebeeb

This client is part of the Beebeeb ecosystem:

| Repo | Description |
|---|---|
| [core](https://github.com/beebeeb-io/core) | Cryptographic core, shared types, and sync engine |
| [cli](https://github.com/beebeeb-io/cli) | `bb` -- end-to-end encrypted cloud storage from the terminal |
| **[desktop](https://github.com/beebeeb-io/desktop)** | Desktop sync client for macOS, Windows, and Linux (you are here) |

## License

This project is licensed under the [GNU Affero General Public License v3.0 or later](./LICENSE).

Copyright (c) Initlabs B.V.

## Links

- [Website](https://beebeeb.io)
- [GitHub organization](https://github.com/beebeeb-io)
