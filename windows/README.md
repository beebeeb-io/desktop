# Beebeeb Desktop Sync -- Windows

Native Windows sync client wrapping the Rust `beebeeb-sync` engine from the `core` repo.

## Architecture

```
+--------------------------------------------------+
|  WinUI 3 / C++ shell  (Fluent-style settings UI) |
|    |                                              |
|    +-- File Explorer overlay (IShellIconOverlay)  |
|    +-- System tray flyout (Win11 notification)    |
|    +-- First-run wizard                           |
|    +-- Conflict resolver dialog                   |
+--------------------------------------------------+
        |  FFI (C ABI)
+--------------------------------------------------+
|  beebeeb-desktop-windows  (Rust)                  |
|    +-- beebeeb-sync   (file watcher, sync engine) |
|    +-- beebeeb-core   (crypto, vault, protocol)   |
+--------------------------------------------------+
```

The binary is a Rust console/service application that owns the sync engine.
The WinUI 3 shell launches it, communicates over a local named pipe, and
provides the UI surfaces described below.

## File Explorer integration

A shell namespace extension (SNE) and icon-overlay handler register with
Windows Explorer to display sync-status icons on every file inside the
vault folder:

| Icon | Meaning |
|------|---------|
| Green check | Synced / available locally |
| Blue cloud | Online-only placeholder |
| Spinning arrows | Syncing / encrypting |
| Lock | One-time locked file |

Implementation: COM DLL loaded by Explorer, talks to the sync service over
a named pipe to query per-file status.

See `HiWindowsExplorer` in `design/hifi/hifi-desktop.jsx` for the visual
reference.

## System tray notifications

A Win11-style tray flyout shows:
- Current sync status (up to date, syncing, paused)
- Recent activity (files uploaded, encrypted, synced)
- Progress bars for active transfers
- Quick actions: Pause, Settings

See `HiWindowsTray` in `design/hifi/hifi-desktop.jsx`.

## Settings UI (Fluent)

A full WinUI 3 settings window with a sidebar navigation:

**Beebeeb section:** Account, Sync, Selective sync, Bandwidth
**Security section:** Vault passphrase, Recovery kit, Linked devices
**System section:** Launch, Explorer integration, Advanced

Key settings:
- E2E encryption is always on (locked by design, cannot be toggled off)
- Sync on metered connections (default off)
- Upload/download rate limits (default auto)
- Files on Demand (online-only placeholders in Explorer)
- Sync overlay icons toggle
- "Free up space" to convert untouched files to online-only

See `HiWindowsSettings` in `design/hifi/hifi-desktop.jsx`.

## Sync engine key behaviors

- **Selective sync:** folders can be online-only (placeholders until opened)
- **Conflict resolution:** NEVER silently drop a version.
  Default: KeepBoth, rename loser as `file (Device, HH:MM).ext`
- **Debounced file watching** (100ms) -- ignores Thumbs.db, desktop.ini,
  temp files
- **Zero-knowledge:** every byte on disk is ciphertext; Beebeeb servers
  never see plaintext

## Build requirements

| Tool | Version | Notes |
|------|---------|-------|
| Visual Studio 2022 | 17.8+ | C++ desktop workload, WinUI 3 templates |
| Windows SDK | 10.0.22621+ | For shell extension APIs |
| Windows App SDK | 1.5+ | WinUI 3 runtime |
| Rust | stable (via rustup) | Edition 2024 |
| cargo | latest | Comes with Rust |

### Steps

```powershell
# 1. Install Rust
winget install Rustlang.Rust.MSVC

# 2. Build the sync engine binary
cd windows
cargo build --release

# 3. Open the WinUI solution in Visual Studio
#    (future: windows/shell/BeebeebShell.sln)
start shell\BeebeebShell.sln
```

The Rust binary (`beebeeb-desktop-windows.exe`) is the sync daemon.
The WinUI shell project launches it and provides the GUI.

## Directory layout

```
windows/
  Cargo.toml          -- Rust project for sync daemon + Windows bindings
  src/
    main.rs           -- Entry point (CLI args, tracing, sync engine)
  shell/              -- (future) WinUI 3 C++ project
    BeebeebShell.sln
    overlay/          -- Shell icon overlay COM DLL
    tray/             -- System tray flyout
    settings/         -- Fluent settings window
```
