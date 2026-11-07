//! LD-0113: the backup's IPC commands that are new with the backup from source (the folder switch and the panel's
//! read stay in `settings_commands`).

/// LD-0113 Amendment 9: the person's answer for a known folder OneDrive's Known Folder Move has moved: back up
/// OneDrive's folder (`onedrive`, where the files are; only the files on this device are read) or the default one
/// (`default`). Kept in the folder's backup setting; the question is settled, and the next pass backs the folder up
/// from there.
#[tauri::command]
pub(crate) async fn set_known_folder_source(key: String, choice: crate::config::KnownFolderSource) -> Result<(), String> {
    if crate::known_folder::known_folder_by_key(&key).is_none() {
        return Err(format!("unknown known-folder key: {key}"));
    }
    let cfg = crate::config::DesktopConfig::update(|cfg| {
        cfg.known_folder_source.insert(key.clone(), choice);
        (true, cfg.clone())
    })?;
    if let Ok(Some(db)) = crate::state_db_for_config(&cfg) {
        if let Err(e) = db.settle_backup_source_question(&key) {
            tracing::warn!(folder = %key, error = %e, "known-folder source: the question was not cleared");
        }
        crate::backup_protect::refresh_own_backups(&db);
    }
    tracing::info!(folder = %key, ?choice, "known-folder source chosen");
    Ok(())
}
