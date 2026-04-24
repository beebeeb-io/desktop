# Beebeeb Desktop Sync -- macOS

Native macOS sync client wrapping the Rust `beebeeb-sync` engine from the `core` repo.

## Architecture

```
+--------------------------------------------------+
|  Swift shell  (SwiftUI preferences window)        |
|    |                                              |
|    +-- Menu bar app (NSStatusItem)                |
|    +-- Finder extension (FinderSync)              |
|    +-- First-run wizard                           |
|    +-- Conflict resolver sheet                    |
+--------------------------------------------------+
        |  FFI (C ABI via cbindgen)
+--------------------------------------------------+
|  beebeeb-desktop-macos  (Rust dylib)              |
|    +-- beebeeb-sync   (file watcher, sync engine) |
|    +-- beebeeb-core   (crypto, vault, protocol)   |
+--------------------------------------------------+
```

A Swift menu-bar app launches the Rust sync engine as an in-process dylib
(loaded via C FFI). The engine runs on a Tokio runtime; the Swift layer
provides the UI.

## Menu bar app

A persistent NSStatusItem showing the Beebeeb hex icon. Clicking it opens
a popover with:
- Sync status (up to date, syncing N files, paused)
- Recent activity list
- Quick actions: Pause, Open vault, Preferences

## Finder extension

A FinderSync extension provides:
- Badge icons on files (synced, syncing, online-only, locked)
- Sidebar entry for the vault
- Right-click context menu: "Share via Beebeeb", "Make available offline",
  "View on beebeeb.io"

## Preferences window

A native SwiftUI window with sidebar navigation:
- General: launch at login, Finder sidebar, Spotlight indexing, notifications
- Account, Sync, Bandwidth, Security, Advanced

See `HiDesktopPrefs` in `design/hifi/hifi-desktop.jsx`.

## Selective sync

Tree-view UI to pick which folders live on disk vs. online-only.
See `HiSelectiveSync` in `design/hifi/hifi-desktop.jsx`.

## Conflict resolver

Side-by-side diff view showing both versions with change summaries.
NEVER silently drops a version -- loser is kept as
`file (Device, HH:MM).ext`.
See `HiConflict` in `design/hifi/hifi-desktop.jsx`.

## First-run wizard

Four-step onboarding: unlock, pick vault folder, selective sync,
Finder integration. See `HiFirstRun` in `design/hifi/hifi-desktop.jsx`.

## Build requirements

| Tool | Version | Notes |
|------|---------|-------|
| Xcode | 15+ | Swift 5.9+, SwiftUI |
| macOS SDK | 14.0+ | Sonoma APIs for FinderSync |
| Rust | stable (via rustup) | Edition 2024 |
| cbindgen | 0.26+ | Generates C headers from Rust |

### Steps

```bash
# 1. Build the Rust dylib
cd macos
cargo build --release

# 2. Open the Xcode project
open shell/Beebeeb.xcodeproj
```

## Directory layout

```
macos/
  Cargo.toml              -- Rust dylib for sync engine + macOS bindings
  src/
    lib.rs                -- C FFI entry points for Swift
  shell/                  -- (future) Xcode project
    Beebeeb.xcodeproj
    Beebeeb/              -- Swift menu bar app
    FinderSync/           -- Finder extension target
```
