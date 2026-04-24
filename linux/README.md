# Beebeeb Desktop Sync -- Linux

Native Linux sync client wrapping the Rust `beebeeb-sync` engine from the `core` repo.

## Architecture

```
+--------------------------------------------------+
|  GTK4 tray indicator  (libappindicator3)          |
|    |                                              |
|    +-- Status icon + dropdown menu                |
|    +-- GTK4 preferences window                    |
|    +-- Notification via libnotify                  |
+--------------------------------------------------+
|  FUSE mount  (fuser crate)                        |
|    +-- ~/Beebeeb appears as a filesystem          |
|    +-- Online-only files fetched on open           |
+--------------------------------------------------+
        |
+--------------------------------------------------+
|  beebeeb-desktop-linux  (Rust binary)             |
|    +-- beebeeb-sync   (file watcher, sync engine) |
|    +-- beebeeb-core   (crypto, vault, protocol)   |
+--------------------------------------------------+
```

Everything is Rust. The binary runs as a user-session daemon (systemd user
unit or autostart desktop entry). GTK4 is used for the tray indicator and
preferences window. A FUSE filesystem provides on-demand file access for
online-only placeholders.

## System tray

Uses libappindicator3 (via the `ksni` or `tray-icon` crate) for broad
desktop environment support (GNOME with extension, KDE, XFCE, etc.).

The tray icon shows:
- Sync status (up to date, syncing, paused)
- Recent activity
- Quick actions: Pause, Open vault, Preferences, Quit

See `HiLinuxTray` in `design/hifi/hifi-misc.jsx`.

## FUSE mount

The vault folder is mounted via FUSE (`fuser` crate):
- All files appear in the mount point (e.g. `~/Beebeeb`)
- Locally-synced files are served from the on-disk cache
- Online-only files trigger a transparent download on `open(2)`
- Write operations go through the sync engine (encrypt then upload)

This avoids the need for a file-manager extension -- status information
is provided via extended attributes (`getfattr`) and the tray indicator.

## Preferences window

A GTK4/libadwaita window with:
- General: autostart, notifications
- Sync: selective sync, bandwidth limits
- Security: vault passphrase, recovery kit, linked devices
- Advanced: FUSE mount point, log level

## Conflict resolution

Desktop notifications via libnotify. Clicking the notification opens a
GTK4 dialog with side-by-side version comparison. NEVER silently drops
a version.

## Build requirements

| Tool | Version | Notes |
|------|---------|-------|
| Rust | stable (via rustup) | Edition 2024 |
| GTK4 dev libs | 4.12+ | `libgtk-4-dev` (Debian/Ubuntu) or `gtk4-devel` (Fedora) |
| libadwaita dev | 1.4+ | `libadwaita-1-dev` / `libadwaita-devel` |
| libfuse3 dev | 3.10+ | `libfuse3-dev` / `fuse3-devel` |
| pkg-config | any | For finding system libraries |

### Steps

```bash
# 1. Install system dependencies (Debian/Ubuntu)
sudo apt install libgtk-4-dev libadwaita-1-dev libfuse3-dev pkg-config

# 2. Build
cd linux
cargo build --release

# 3. Install systemd user unit (optional)
cp beebeeb-sync.service ~/.config/systemd/user/
systemctl --user enable --now beebeeb-sync
```

## Directory layout

```
linux/
  Cargo.toml              -- Rust binary: sync daemon + GTK4 tray + FUSE
  src/
    main.rs               -- Entry point (CLI args, tracing, sync engine)
    tray.rs               -- GTK4/appindicator tray icon
    fuse.rs               -- FUSE filesystem implementation
    ui/
      preferences.rs      -- GTK4 preferences window
      conflict.rs         -- Conflict resolver dialog
  beebeeb-sync.service    -- (future) systemd user unit
  beebeeb-sync.desktop    -- (future) XDG autostart entry
```
