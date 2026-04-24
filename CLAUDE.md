# beebeeb-io/desktop

Beebeeb desktop sync for macOS, Windows, and Linux. Native shells wrapping the Rust sync engine from `core`.

## Architecture

- Sync engine: `beebeeb-sync` crate from the `core` repo (file watcher, conflict resolution, selective sync)
- macOS: Swift shell, menu bar app, Finder extension
- Windows: WinUI 3 / C++ shell, File Explorer overlay, tray notifications
- Linux: Rust + GTK4 tray, FUSE mount

## Platform breakdown

### Windows (`windows/`)

Rust sync daemon (`beebeeb-desktop-windows`) + WinUI 3 / C++ shell.

- `Cargo.toml` — Rust project depending on `beebeeb-sync` and `beebeeb-core` from the core repo
- `src/main.rs` — Entry point: CLI args (--start-minimized, --sync-path), tracing, sync engine, Ctrl+C shutdown
- Shell (future): WinUI 3 Fluent-style settings UI, system tray flyout, File Explorer overlay icons (IShellIconOverlay COM DLL)
- Build: Visual Studio 2022 17.8+, Windows SDK 10.0.22621+, Windows App SDK 1.5+, Rust stable

### macOS (`macos/`)

Rust dylib (C FFI via cbindgen) + Swift/SwiftUI shell.

- Menu bar app (NSStatusItem), Finder extension (FinderSync), preferences window
- Build: Xcode 15+, macOS SDK 14.0+, Rust stable, cbindgen

### Linux (`linux/`)

Pure Rust binary: sync daemon + GTK4 tray + FUSE mount.

- GTK4/libadwaita tray indicator, FUSE filesystem for online-only files
- Runs as systemd user unit or XDG autostart
- Build: Rust stable, libgtk-4-dev, libadwaita-1-dev, libfuse3-dev

## Design references

- `../../design/hifi/hifi-desktop.jsx` — macOS preferences, selective sync, conflict resolver, first-run, Windows Explorer, Windows tray, Windows settings
- `../../design/hifi/hifi-misc.jsx` — Linux tray, uninstall flow

## Sync engine key behaviors

- Selective sync: folders can be online-only (placeholders until opened)
- Conflict resolution: NEVER silently drop a version. Default: KeepBoth, rename loser as "file (Device, HH:MM).ext"
- Debounced file watching (100ms) — ignores .DS_Store, Thumbs.db, temp files


## Keep shared docs in sync

When you add/change/remove endpoints, types, build commands, or dependencies: update the relevant skill file in `/home/guus/code/beebeeb.io/.claude/skills/` (beebeeb-api.md, beebeeb-designs.md, beebeeb-stack.md, beebeeb-dev.md). Other agents depend on these being accurate.
