use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tauri::Manager;

pub(crate) const LEGACY_STATE_DIR_NAME: &str = ".beebeeb";
pub(crate) const STATE_DB_FILENAME: &str = "state.db";
pub(crate) const LOCK_FILENAME: &str = ".beebeeb-sync.lock";
pub(crate) const DEVICE_IDENTITY_FILENAME: &str = ".device.json";

const LEGACY_STATE_ARCHIVE_DIR_NAME: &str = "legacy-sync-root-state";

static BEEBEEB_STATE_DIR: OnceLock<PathBuf> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StateDirMigration {
    FreshInstall,
    Migrated,
    SkippedExistingState,
}

pub(crate) fn init_from_app(app: &tauri::AppHandle) -> Result<(), String> {
    let app_local_data_dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| format!("resolve app-local-data dir: {e}"))?;
    let state_dir = beebeeb_state_dir_from_app_local_data(&app_local_data_dir);

    match BEEBEEB_STATE_DIR.set(state_dir.clone()) {
        Ok(()) => {
            tracing::info!(state_dir = %state_dir.display(), "Beebeeb state dir resolved from Tauri app-local-data dir");
        }
        Err(_) => {
            if BEEBEEB_STATE_DIR.get() != Some(&state_dir) {
                tracing::warn!(
                    state_dir = %state_dir.display(),
                    existing = BEEBEEB_STATE_DIR
                        .get()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "<unset>".to_string()),
                    "Beebeeb state dir was already initialized to a different path"
                );
            }
        }
    }

    Ok(())
}

pub(crate) fn beebeeb_state_dir() -> Result<PathBuf, String> {
    BEEBEEB_STATE_DIR
        .get()
        .cloned()
        .ok_or_else(|| "Beebeeb state dir has not been initialized".to_string())
}

pub(crate) fn beebeeb_state_dir_from_app_local_data(app_local_data_dir: &Path) -> PathBuf {
    app_local_data_dir.to_path_buf()
}

pub(crate) fn state_db_path_from_state_dir(state_dir: &Path) -> PathBuf {
    state_dir.join(STATE_DB_FILENAME)
}

pub(crate) fn prepare_beebeeb_state_dir_at(sync_root: &Path, state_dir: &Path) -> Result<StateDirMigration, String> {
    let legacy_state_dir = sync_root.join(LEGACY_STATE_DIR_NAME);
    let legacy_lock = sync_root.join(LOCK_FILENAME);
    let legacy_device_identity = sync_root.join(DEVICE_IDENTITY_FILENAME);
    let new_state_db = state_db_path_from_state_dir(state_dir);

    if new_state_db.exists() {
        fs::create_dir_all(state_dir).map_err(|e| format!("create state dir {}: {e}", state_dir.display()))?;
        let archived_dir = archive_legacy_state_dir_if_present(&legacy_state_dir, state_dir)?;
        let moved_lock = relocate_legacy_file_if_present(&legacy_lock, &state_dir.join(LOCK_FILENAME))?;
        let moved_device =
            relocate_legacy_file_if_present(&legacy_device_identity, &state_dir.join(DEVICE_IDENTITY_FILENAME))?;
        tracing::info!(
            sync_root = %sync_root.display(),
            state_dir = %state_dir.display(),
            archived_legacy_state_dir = archived_dir,
            moved_legacy_lock = moved_lock,
            moved_legacy_device_identity = moved_device,
            "Beebeeb state migration skipped because app-local state.db already exists"
        );
        return Ok(StateDirMigration::SkippedExistingState);
    }

    let mut migrated_any = false;
    if legacy_state_dir.exists() {
        migrate_legacy_state_dir(&legacy_state_dir, state_dir)?;
        migrated_any = true;
    } else {
        fs::create_dir_all(state_dir).map_err(|e| format!("create state dir {}: {e}", state_dir.display()))?;
    }

    let moved_lock = relocate_legacy_file_if_present(&legacy_lock, &state_dir.join(LOCK_FILENAME))?;
    let moved_device =
        relocate_legacy_file_if_present(&legacy_device_identity, &state_dir.join(DEVICE_IDENTITY_FILENAME))?;
    migrated_any = migrated_any || moved_lock || moved_device;

    if migrated_any {
        tracing::info!(
            sync_root = %sync_root.display(),
            state_dir = %state_dir.display(),
            moved_legacy_lock = moved_lock,
            moved_legacy_device_identity = moved_device,
            "Beebeeb metadata migrated out of the sync root into app-local data"
        );
        Ok(StateDirMigration::Migrated)
    } else {
        tracing::info!(
            sync_root = %sync_root.display(),
            state_dir = %state_dir.display(),
            "No legacy Beebeeb metadata found in sync root; initialized app-local state dir"
        );
        Ok(StateDirMigration::FreshInstall)
    }
}

pub(crate) fn prepare_beebeeb_state_dir_for_sync_root(sync_root: &Path) -> Result<StateDirMigration, String> {
    let state_dir = beebeeb_state_dir()?;
    prepare_beebeeb_state_dir_at(sync_root, &state_dir)
}

fn migrate_legacy_state_dir(legacy_state_dir: &Path, state_dir: &Path) -> Result<(), String> {
    if !legacy_state_dir.is_dir() {
        return Err(format!(
            "legacy state path {} exists but is not a directory",
            legacy_state_dir.display()
        ));
    }

    if !state_dir.exists() {
        if let Some(parent) = state_dir.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create state parent {}: {e}", parent.display()))?;
        }
        match fs::rename(legacy_state_dir, state_dir) {
            Ok(()) => return Ok(()),
            Err(rename_error) => {
                tracing::warn!(
                    error = %rename_error,
                    legacy_state_dir = %legacy_state_dir.display(),
                    state_dir = %state_dir.display(),
                    "atomic Beebeeb state migration rename failed; falling back to copy-verify-delete"
                );
            }
        }
    } else {
        fs::create_dir_all(state_dir).map_err(|e| format!("create state dir {}: {e}", state_dir.display()))?;
    }

    copy_dir_contents_no_clobber(legacy_state_dir, state_dir)?;
    verify_dir_contents(legacy_state_dir, state_dir)?;
    fs::remove_dir_all(legacy_state_dir)
        .map_err(|e| format!("delete migrated legacy state dir {}: {e}", legacy_state_dir.display()))
}

fn archive_legacy_state_dir_if_present(legacy_state_dir: &Path, state_dir: &Path) -> Result<bool, String> {
    if !legacy_state_dir.exists() {
        return Ok(false);
    }
    if !legacy_state_dir.is_dir() {
        return Err(format!(
            "legacy state path {} exists but is not a directory",
            legacy_state_dir.display()
        ));
    }

    fs::create_dir_all(state_dir).map_err(|e| format!("create state dir {}: {e}", state_dir.display()))?;
    let archive_dest = unused_path(&state_dir.join(LEGACY_STATE_ARCHIVE_DIR_NAME))?;
    move_path_verified(legacy_state_dir, &archive_dest)?;
    Ok(true)
}

fn relocate_legacy_file_if_present(src: &Path, preferred_dest: &Path) -> Result<bool, String> {
    if !src.exists() {
        return Ok(false);
    }
    if !src.is_file() {
        return Err(format!(
            "legacy metadata path {} exists but is not a file",
            src.display()
        ));
    }

    let dest = if preferred_dest.exists() {
        let file_name = preferred_dest
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("metadata destination has no file name: {}", preferred_dest.display()))?;
        unused_path(&preferred_dest.with_file_name(format!("legacy-{file_name}")))?
    } else {
        preferred_dest.to_path_buf()
    };
    move_path_verified(src, &dest)?;
    Ok(true)
}

fn move_path_verified(src: &Path, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create destination parent {}: {e}", parent.display()))?;
    }

    match fs::rename(src, dest) {
        Ok(()) => return Ok(()),
        Err(rename_error) => {
            tracing::warn!(
                error = %rename_error,
                src = %src.display(),
                dest = %dest.display(),
                "atomic Beebeeb metadata rename failed; falling back to copy-verify-delete"
            );
        }
    }

    if src.is_dir() {
        copy_dir_all(src, dest)?;
        verify_dir_contents(src, dest)?;
        fs::remove_dir_all(src).map_err(|e| format!("delete copied directory {}: {e}", src.display()))?;
    } else {
        copy_file(src, dest)?;
        verify_file_contents(src, dest)?;
        fs::remove_file(src).map_err(|e| format!("delete copied file {}: {e}", src.display()))?;
    }

    Ok(())
}

fn copy_dir_contents_no_clobber(src_dir: &Path, dest_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dest_dir).map_err(|e| format!("create destination dir {}: {e}", dest_dir.display()))?;
    for entry in fs::read_dir(src_dir).map_err(|e| format!("read dir {}: {e}", src_dir.display()))? {
        let entry = entry.map_err(|e| format!("read dir entry {}: {e}", src_dir.display()))?;
        let src = entry.path();
        let dest = unused_path(&dest_dir.join(entry.file_name()))?;
        if src.is_dir() {
            copy_dir_all(&src, &dest)?;
        } else {
            copy_file(&src, &dest)?;
        }
    }
    Ok(())
}

fn copy_dir_all(src_dir: &Path, dest_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dest_dir).map_err(|e| format!("create destination dir {}: {e}", dest_dir.display()))?;
    for entry in fs::read_dir(src_dir).map_err(|e| format!("read dir {}: {e}", src_dir.display()))? {
        let entry = entry.map_err(|e| format!("read dir entry {}: {e}", src_dir.display()))?;
        let src = entry.path();
        let dest = dest_dir.join(entry.file_name());
        if src.is_dir() {
            copy_dir_all(&src, &dest)?;
        } else {
            copy_file(&src, &dest)?;
        }
    }
    Ok(())
}

fn copy_file(src: &Path, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create destination parent {}: {e}", parent.display()))?;
    }
    fs::copy(src, dest)
        .map(|_| ())
        .map_err(|e| format!("copy {} to {}: {e}", src.display(), dest.display()))
}

fn verify_dir_contents(src_dir: &Path, dest_dir: &Path) -> Result<(), String> {
    for entry in fs::read_dir(src_dir).map_err(|e| format!("read dir {}: {e}", src_dir.display()))? {
        let entry = entry.map_err(|e| format!("read dir entry {}: {e}", src_dir.display()))?;
        let src = entry.path();
        let dest = dest_dir.join(entry.file_name());
        if src.is_dir() {
            verify_dir_contents(&src, &dest)?;
        } else {
            verify_file_contents(&src, &dest)?;
        }
    }
    Ok(())
}

fn verify_file_contents(src: &Path, dest: &Path) -> Result<(), String> {
    let src_bytes = fs::read(src).map_err(|e| format!("verify read source {}: {e}", src.display()))?;
    let dest_bytes = fs::read(dest).map_err(|e| format!("verify read destination {}: {e}", dest.display()))?;
    if src_bytes == dest_bytes {
        Ok(())
    } else {
        Err(format!(
            "verify copied file {} -> {}: contents differ",
            src.display(),
            dest.display()
        ))
    }
}

fn unused_path(preferred: &Path) -> Result<PathBuf, String> {
    if !preferred.exists() {
        return Ok(preferred.to_path_buf());
    }
    let parent = preferred
        .parent()
        .ok_or_else(|| format!("path has no parent: {}", preferred.display()))?;
    let name = preferred
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("path has no file name: {}", preferred.display()))?;
    for idx in 1..1000 {
        let candidate = parent.join(format!("{name}-{idx}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "could not find unused archive path for {}",
        preferred.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn fresh_sync_root_uses_app_local_state_dir() {
        let temp = tempfile::tempdir().unwrap();
        let sync_root = temp.path().join("sync-root");
        let app_local_data = temp.path().join("app-local-data");
        let state_dir = beebeeb_state_dir_from_app_local_data(&app_local_data);
        fs::create_dir_all(&sync_root).unwrap();

        let outcome = prepare_beebeeb_state_dir_at(&sync_root, &state_dir).unwrap();

        assert_eq!(outcome, StateDirMigration::FreshInstall);
        assert!(state_dir.exists(), "new app-local state dir should be created");
        let state_db_path = state_db_path_from_state_dir(&state_dir);
        assert_eq!(state_db_path, state_dir.join("state.db"));
        let _db = crate::state_db::StateDb::open(&state_db_path).unwrap();
        assert!(state_db_path.exists(), "state.db should be created in app-local data");
        assert!(!sync_root.join(LEGACY_STATE_DIR_NAME).exists());
        assert!(!sync_root.join(LOCK_FILENAME).exists());
        assert!(!sync_root.join(DEVICE_IDENTITY_FILENAME).exists());
    }

    #[test]
    fn populated_legacy_state_moves_to_app_local_and_cleans_sync_root() {
        let temp = tempfile::tempdir().unwrap();
        let sync_root = temp.path().join("sync-root");
        let legacy_dir = sync_root.join(LEGACY_STATE_DIR_NAME);
        let state_dir = temp.path().join("app-local-data");
        fs::create_dir_all(&legacy_dir).unwrap();
        fs::write(legacy_dir.join(STATE_DB_FILENAME), b"legacy-state").unwrap();
        fs::write(legacy_dir.join("extra.json"), br#"{"legacy":true}"#).unwrap();
        fs::write(sync_root.join(LOCK_FILENAME), b"legacy-lock").unwrap();
        fs::write(sync_root.join(DEVICE_IDENTITY_FILENAME), b"legacy-device").unwrap();

        let outcome = prepare_beebeeb_state_dir_at(&sync_root, &state_dir).unwrap();

        assert_eq!(outcome, StateDirMigration::Migrated);
        assert_eq!(fs::read(state_dir.join(STATE_DB_FILENAME)).unwrap(), b"legacy-state");
        assert_eq!(fs::read(state_dir.join("extra.json")).unwrap(), br#"{"legacy":true}"#);
        assert_eq!(fs::read(state_dir.join(LOCK_FILENAME)).unwrap(), b"legacy-lock");
        assert_eq!(
            fs::read(state_dir.join(DEVICE_IDENTITY_FILENAME)).unwrap(),
            b"legacy-device"
        );
        assert!(!sync_root.join(LEGACY_STATE_DIR_NAME).exists());
        assert!(!sync_root.join(LOCK_FILENAME).exists());
        assert!(!sync_root.join(DEVICE_IDENTITY_FILENAME).exists());
    }

    #[test]
    fn migration_is_idempotent_when_new_state_already_exists() {
        let temp = tempfile::tempdir().unwrap();
        let sync_root = temp.path().join("sync-root");
        let legacy_dir = sync_root.join(LEGACY_STATE_DIR_NAME);
        let state_dir = temp.path().join("app-local-data");
        fs::create_dir_all(&legacy_dir).unwrap();
        fs::create_dir_all(&state_dir).unwrap();
        fs::write(legacy_dir.join(STATE_DB_FILENAME), b"legacy-state").unwrap();
        fs::write(state_dir.join(STATE_DB_FILENAME), b"new-state").unwrap();
        fs::write(sync_root.join(LOCK_FILENAME), b"legacy-lock").unwrap();
        fs::write(sync_root.join(DEVICE_IDENTITY_FILENAME), b"legacy-device").unwrap();

        let outcome = prepare_beebeeb_state_dir_at(&sync_root, &state_dir).unwrap();

        assert_eq!(outcome, StateDirMigration::SkippedExistingState);
        assert_eq!(fs::read(state_dir.join(STATE_DB_FILENAME)).unwrap(), b"new-state");
        assert!(!sync_root.join(LEGACY_STATE_DIR_NAME).exists());
        assert!(!sync_root.join(LOCK_FILENAME).exists());
        assert!(!sync_root.join(DEVICE_IDENTITY_FILENAME).exists());

        let outcome = prepare_beebeeb_state_dir_at(&sync_root, &state_dir).unwrap();

        assert_eq!(outcome, StateDirMigration::SkippedExistingState);
        assert_eq!(fs::read(state_dir.join(STATE_DB_FILENAME)).unwrap(), b"new-state");
    }
}
