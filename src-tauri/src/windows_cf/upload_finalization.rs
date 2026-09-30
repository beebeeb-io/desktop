//! Local completion after the server has accepted an upload. Never calls the API.
use crate::state_db::{StateDb, UploadFinalization};
use std::{path::Path, sync::Mutex};

// Watcher and periodic passes may race. Keep stamp -> durable acknowledgement ->
// proof unlink ordered, including retries after process restart.
static FINALIZING: Mutex<()> = Mutex::new(());

pub fn retry(db: &StateDb, root: &Path) {
    let _guard = FINALIZING.lock().unwrap_or_else(|e| e.into_inner());
    let rows = match db.upload_finalizations() {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(%error, "cannot read upload finalization journal");
            return;
        }
    };
    for row in rows {
        if let Err(error) = finish(db, root, &row) {
            tracing::warn!(file_id = %row.server_file_id, %error, "upload identity finalization deferred");
        }
    }
}

fn finish(db: &StateDb, root: &Path, row: &UploadFinalization) -> anyhow::Result<()> {
    if !row.stamped {
        let entry = db
            .get_file(&row.server_file_id)?
            .ok_or_else(|| anyhow::anyhow!("Completed upload row is unavailable"))?;
        anyhow::ensure!(entry.path == row.target_path, "Completed upload target changed");
        let on_disk = crate::engine_bridge::local_file_path_under_sync_root(root, &row.target_path)?;
        super::placeholders::complete_upload_placeholder(
            &on_disk,
            &row.server_file_id,
            Some((&row.local_file_id, Path::new(&row.payload_path))),
        )?;
        // A crash after unlink must not require a proof that was already removed.
        db.mark_upload_finalization_stamped(&row.op_id)?;
    }
    crate::staged_payload::remove(db, Path::new(&row.payload_path))?;
    db.forget_upload_finalization(&row.op_id)?;
    tracing::info!(file_id = %row.server_file_id, "upload identity finalization completed");
    Ok(())
}
