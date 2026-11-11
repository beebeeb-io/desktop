//! Local state the engine itself opens or empties: the state database for a configuration, and the macOS
//! hydrate cache. Must not decide when a purge runs (sign-out and lock do, in the shell).

pub fn state_db_for_config(cfg: &crate::config::DesktopConfig) -> Result<Option<crate::state_db::StateDb>, String> {
    let Some(_) = &cfg.sync_root else {
        return Ok(None);
    };
    let db_path = crate::state_paths::beebeeb_state_dir()?.join(crate::state_paths::STATE_DB_FILENAME);
    if !db_path.exists() {
        return Ok(None);
    }
    crate::state_db::StateDb::open(&db_path)
        .map(Some)
        .map_err(|e| format!("open state.db: {e}"))
}

/// Task 1670 round 2: wipe the macOS hydrate-cache staging directory at every
/// account-security boundary (sign-out, lock, daemon startup) so a decrypted
/// plaintext copy staged for a Finder open can never outlive the boundary
/// that's supposed to end its access — independent of, and in addition to,
/// the per-request cleanup `fetchContents` itself does. Distinct from the
/// general local-file/smart-cache purge `purge_local_state_files` runs on
/// sign-out: THAT purge intentionally does NOT run on lock (lock keeps the
/// regular local file cache so re-unlocking stays fast), but the hydrate
/// cache holds nothing but ephemeral per-open staging copies, so it is always
/// safe — and always right — to empty here too. Best-effort and non-fatal:
/// logout/lock must always appear to succeed, and daemon startup must never
/// block on this.
#[cfg(target_os = "macos")]
pub fn purge_macos_hydrate_cache(context: &str) {
    let dir = crate::ipc_socket::macos_hydrate_cache_dir();
    match crate::ipc_socket::macos_purge_hydrate_cache_dir(&dir) {
        Ok(removed) if removed > 0 => {
            tracing::info!(removed, context, "purged macOS hydrate-cache staged plaintext");
        }
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(error = %error, context, "failed to purge macOS hydrate-cache dir (best-effort)");
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn purge_macos_hydrate_cache(_context: &str) {}
