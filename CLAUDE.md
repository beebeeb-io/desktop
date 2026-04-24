# beebeeb-io/desktop

Beebeeb desktop sync for macOS, Windows, and Linux. Native shells wrapping the Rust sync engine from `core`.

## Architecture

- Sync engine: `beebeeb-sync` crate from the `core` repo (file watcher, conflict resolution, selective sync)
- macOS: Swift shell, menu bar app, Finder extension
- Windows: WinUI 3 / C++ shell, File Explorer overlay, tray notifications
- Linux: Rust + GTK4 tray, FUSE mount

## Design references

- `../../design/hifi/hifi-desktop.jsx` — macOS preferences, selective sync, conflict resolver, first-run, Windows Explorer, Windows tray, Windows settings
- `../../design/hifi/hifi-misc.jsx` — Linux tray, uninstall flow

## Sync engine key behaviors

- Selective sync: folders can be online-only (placeholders until opened)
- Conflict resolution: NEVER silently drop a version. Default: KeepBoth, rename loser as "file (Device, HH:MM).ext"
- Debounced file watching (100ms) — ignores .DS_Store, Thumbs.db, temp files
