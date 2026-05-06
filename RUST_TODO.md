# Desktop Rust — Status

All tasks listed here have been implemented. File kept for historical reference.

## Completed

- **Task 8** (native settings window): `lib.rs` creates a local-asset 680×540 settings
  window. Tray handler shows/focuses it on `open_settings`.

- **Task 9** (settings IPC): `get_desktop_config`, `set_desktop_config`, `account_email`
  all implemented. `sync_status` returns `{ logged_in, engine, sync_root, syncing,
  cloud_only, conflicts }`.

- **Task 12** (conflict window IPC): `open_conflict_window` and `resolve_conflict`
  both implemented and registered in `invoke_handler!`.

## Also done (from plan tasks 1–7, 10–11, 13–16)

- `engine_bridge.rs` — `hydrate_file`, conflict resolution (keep mine / theirs / both)
- `runner.rs` — background sync tick loop
- `ipc_socket.rs` — Unix socket for OS extensions
- `linux_fuse/` — FUSE virtual filesystem
- `windows_cf/` — Windows Cloud Files API placeholders + callbacks
- `BeebeebFileProvider/` — macOS File Provider extension (Swift)
- `conflict.rs` — conflict detection, 24h auto-resolution, text-file detection
- `state_db.rs` — SQLite file state tracking
- `config.rs` — `DesktopConfig` with persistence to `desktop.toml`
- `api_client.rs` — typed API client for vault operations
- `lockfile.rs` — single-instance lock

No pending Rust work tracked here. Add new items to `.claude/tasks/backlog/` instead.
