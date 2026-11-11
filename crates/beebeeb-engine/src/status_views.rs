//! What the status surfaces list from the state database and the transfer board: the recent files, the overview's
//! rows, the transfers a locked vault may show. Pure reads. Must not name a window or a command.

/// How many rows past `limit` a recent list reads, so the manifests [`listable_rows`] leaves out (one
/// per device) do not shorten it.
const HIDDEN_SLACK: usize = 16;

/// The overview's view of file rows. Folders carry no real size and no sync state worth surfacing; they are left out.
pub fn file_overview_inputs(
    rows: Vec<(crate::state_db::FileEntry, bool)>,
) -> Vec<crate::account_dto::FileOverviewInput> {
    rows.into_iter()
        .filter(|(e, _)| !e.is_dir())
        .map(|(e, pinned)| crate::account_dto::FileOverviewInput {
            path: e.path,
            size_bytes: e.size_bytes,
            status: e.status.as_str().to_string(),
            pinned,
            modified_at: e.modified_at,
        })
        .collect()
}

/// LD-0113 (core's `is_hidden_backup_item`): what the overview and the tray may list. A device folder's
/// manifest in Backups never is; each backup pass rewrites it, so it would otherwise lead every recent
/// list.
pub fn listable_rows(
    db: &crate::state_db::StateDb,
    rows: Vec<(crate::state_db::FileEntry, bool)>,
) -> Vec<(crate::state_db::FileEntry, bool)> {
    let backups = crate::backup_protect::BackupsTree::read(db);
    rows.into_iter()
        .filter(|(entry, _)| !backups.is_hidden(&entry.path))
        .collect()
}

/// The newest `limit` local activity events, as the overview's recent list takes them; none about a
/// row [`listable_rows`] leaves out.
pub fn file_overview_activity(
    db: &crate::state_db::StateDb,
    limit: usize,
) -> Result<Vec<crate::account_dto::FileOverviewActivityInput>, String> {
    let backups = crate::backup_protect::BackupsTree::read(db);
    Ok(db
        .list_recent_local_activity(limit.saturating_add(HIDDEN_SLACK))
        .map_err(|e| format!("list local activity: {e}"))?
        .into_iter()
        .filter(|event| event.rel_path.as_deref().is_none_or(|rel| !backups.is_hidden(rel)))
        .take(limit)
        .map(|event| crate::account_dto::FileOverviewActivityInput {
            event_type: event.event_type.as_str().to_string(),
            file_name: event.file_name,
            rel_path: event.rel_path,
            occurred_at: event.occurred_at,
        })
        .collect())
}

/// The tray's recent list (task 2243): the overview's `recent`, without its totals.
pub fn recent_files(
    db: &crate::state_db::StateDb,
    limit: usize,
) -> Result<Vec<crate::account_dto::RecentFile>, String> {
    // The newest `limit` files and the newest `limit` events are all a list of `limit` can hold (and the
    // rows a recent list leaves out).
    let rows = db
        .newest_file_rows(limit.saturating_add(HIDDEN_SLACK))
        .map_err(|e| format!("list files: {e}"))?;
    let rows = listable_rows(db, rows);
    Ok(crate::account_dto::compute_file_overview(
        &file_overview_inputs(rows),
        &file_overview_activity(db, limit)?,
        limit,
    )
    .recent)
}

/// What `get_active_transfers` shows: the board, plus the needs-attention rows (each one names a file
/// from state.db) only while the vault is unlocked, as `desktop_recent_files` hides names when locked.
pub fn shown_transfers(
    rows: Vec<crate::transfer_progress::ActiveTransferDto>,
    unlocked: bool,
    attention: &[crate::transfer_progress::NeedsAttention],
) -> Vec<crate::transfer_progress::ActiveTransferDto> {
    if !unlocked {
        // LD-0105 U1: the board's rows carry names now; a locked vault shows no name (lead 2026-11-06).
        return rows
            .into_iter()
            .map(|row| crate::transfer_progress::ActiveTransferDto {
                name: String::new(),
                folder: String::new(),
                ..row
            })
            .collect();
    }
    crate::transfer_progress::with_attention(rows, attention)
}
