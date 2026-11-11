# Desktop v2: Native Rust UI with Rune — Implementation Plan

Spec: `docs/superpowers/specs/2026-11-11-desktop-v2-native-rune-design.md`
Filed: 2026-11-11 by the Rune spike lane (task 2445).

## Prerequisites

- [ ] Guus approves go-with-conditions verdict
- [ ] Rust 1.99.1 installed on Legion and testrig (`rustup install 1.99.1`)
- [ ] Disk space freed to >= 40 GB on Legion

## Phase 0: Foundation (slices 1-5, sequential then parallel)

### Slice 1: Extract `beebeeb-engine` crate
**Files**: New `crates/beebeeb-engine/Cargo.toml`, `crates/beebeeb-engine/src/lib.rs`; move `state_db.rs`, `state_db/`, `config.rs`, `config/`, `runner.rs`, `runner/`, `engine_bridge.rs`, `engine_bridge/` into it.
**Check**: `cargo check -p beebeeb-engine` AND `cargo check -p beebeeb-desktop` (Tauri) both pass. `cargo test -p beebeeb-engine` runs existing tests.
**Done means**: The engine crate builds standalone; Tauri's `lib.rs` imports from it.

### Slice 2: Caption bar + window chrome
**Files**: `native/src/chrome.rs`
**Check**: `cargo run -- --snap chrome.png --size 1280x60 --scale 2 --theme dark`; PNG shows Minimize/Maximize/Close buttons at trailing edge; `--tree` has those three elements.
**Done means**: Caption bar matches board DW-0401 top band, verified by looking at both PNGs.

### Slice 3: Sidebar
**Files**: `native/src/sidebar.rs`
**Check**: Headless PNG; sidebar width = 240 pt; sections listed; meter fill proportional.
**Done means**: Sidebar matches board DW-0401 sidebar.

### Slice 4: File list (fixture)
**Files**: `native/src/file_list.rs`
**Check**: 5,000 rows: `--tree` shows < 100 mounted elements; arrow keys move selection (headless script test); Enter on folder navigates.
**Done means**: Virtual scrolling works; keyboard nav works.

### Slice 5: File list (real data)
**Files**: `native/src/source/engine.rs` implementing `DriveSource` over `beebeeb-engine::StateDb`
**Check**: `cargo test -p beebeeb-drive-rune` with a test DB fixture; listed items match `drive_v2::model::list` output.
**Done means**: Real file names appear from the engine's local state.

## Phase 1: File actions (slices 6-8, parallelizable)

### Slice 6: Toolbar
**Files**: `native/src/toolbar.rs`
**Check**: Headless PNG; search field takes text; sort changes order.
**Done means**: Toolbar matches board.

### Slice 7: Context menu
**Files**: additions to `native/src/file_list.rs`
**Check**: Headless script right-click -> `--tree` shows `popupMenu` with items.
**Done means**: Right-click opens native Windows context menu.

### Slice 8: File dialogs + actions
**Files**: `native/src/actions.rs`
**Check**: Scripted `file-dialog` in headless; `cargo test` for create/rename/delete.
**Done means**: Upload, new folder, rename, delete, move all work.

## Phase 2: Sign-in (slices 9-11, sequential)

### Slice 9: Sign-in window (email + password)
**Files**: `native/src/auth/sign_in.rs`
**Check**: Headless PNG of DW-A02; password field: `--tree` shows role=text, NO password value.
**Done means**: Sign-in screen renders; password secure.

### Slice 10: Sign-in states + vault unlock
**Files**: `native/src/auth/`
**Check**: Headless PNGs of DW-A18-A22; state machine unit test.
**Done means**: All error states render.

### Slice 11: Session persistence + keep_running
**Files**: `native/src/app.rs`
**Check**: Close and relaunch: still signed in (keychain read).
**Done means**: Session survives app close.

## Phase 3: Tray + Settings (slices 12-15, parallelizable)

### Slice 12: Tray icon
**Files**: `native/src/tray.rs`
**Check**: Windows native test: icon in system tray.
**Done means**: Tray icon visible.

### Slice 13: Tray flyout
**Files**: `native/src/tray/flyout.rs`
**Check**: Headless PNGs of DW-1201-1205.
**Done means**: Flyout states match board.

### Slice 14: Settings window
**Files**: `native/src/settings.rs`
**Check**: Headless PNGs of DW-0701-0704.
**Done means**: Settings window opens; toggles function.

### Slice 15: Notifications
**Files**: `native/src/notifications.rs`
**Check**: `cargo test`: post + click callback.
**Done means**: Toast notifications work.

## Remaining phases (slices 16-60, estimated)

- Phase 4: Remaining sign-in paths (another-device, approval, passkey) — 8 slices
- Phase 5: Sidebar sections (Shared, Recent, Starred, Trash) — 8 slices
- Phase 6: Preview + editor (photo, video, PDF, markdown, office WebView2) — 10 slices
- Phase 7: Advanced features (versioning, selective sync, bandwidth limits, conflicts) — 8 slices
- Phase 8: Onboarding + first-run — 3 slices
- Phase 9: Installer, updater, autostart, deep links — 4 slices
- Phase 10: Polish, accessibility audit, performance tuning — 4 slices

## Parallel execution

After slice 1 (engine extraction), up to 3 lanes:
- **Lane A**: Chrome components (slices 2-4)
- **Lane B**: Sign-in (slices 9-11, after slice 1)
- **Lane C**: File actions (slices 6-8, after slice 4)

After slice 15, up to 4 lanes on the remaining phases.

## Cut-over decision

The Tauri shell remains the shipping binary until the Rune shell passes:
1. All sign-in paths work on a real account (native Windows test)
2. Files window shows 40,000+ real files with names (the real account)
3. Tray shows correct sync status
4. Settings work
5. Single-instance + autostart configured
6. Installer builds

At that point, merge the Rune shell as the primary target. The Tauri shell stays as a fallback branch for ~2 weeks, then archived.
