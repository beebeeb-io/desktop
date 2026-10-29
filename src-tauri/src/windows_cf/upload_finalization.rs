//! Local completion after the server has accepted an upload. Never calls the API.
//!
//! Tasks 2270 / 2280 (desktop 0.8.20, spec `docs/superpowers/specs/2026-10-29-2270-2280-desktop-0820-read-only-
//! finalization-design.md` §4): every journal row ends in one of three states. Done: nothing is left to stamp (the
//! row is marked stamped, its staged copy removed, the row forgotten). Review: it cannot be finalized safely; it is
//! counted in the tray, logged once, and tried again once per engine start. Pending: a transient error (the file is
//! open elsewhere); the next tick tries again. 0.8.19 had only "done" and an error on every tick.
use crate::state_db::{StateDb, UploadFinalization};
use crate::upload_finalization_plan::{ReviewReason, Step, first_step, judge};
use std::{path::Path, sync::Mutex};

// Watcher and periodic passes may race. Keep stamp -> durable acknowledgement ->
// proof unlink ordered, including retries after process restart.
static FINALIZING: Mutex<()> = Mutex::new(());

/// Tasks 2270 / 2280 (spec R1): the sync roots whose rows in review this process has already tried again. A row in
/// review is tried again once per engine start, by the first `retry` for its root.
static REVIEW_RETRIED: Mutex<Vec<std::path::PathBuf>> = Mutex::new(Vec::new());

/// Tests only: as if the engine started again for `root`.
#[cfg(test)]
pub(crate) fn forget_review_retry(root: &Path) {
    REVIEW_RETRIED.lock().unwrap_or_else(|e| e.into_inner()).retain(|retried| retried != root);
}

/// Tests only: as if this start had already tried `root`'s rows in review again.
#[cfg(test)]
pub(crate) fn mark_review_retried(root: &Path) {
    REVIEW_RETRIED.lock().unwrap_or_else(|e| e.into_inner()).push(root.to_path_buf());
}

/// Whether this is the first `retry` for `root` in this process (spec R1), and from now on it is not.
fn first_retry_of(root: &Path) -> bool {
    let mut retried = REVIEW_RETRIED.lock().unwrap_or_else(|e| e.into_inner());
    if retried.iter().any(|seen| seen == root) {
        return false;
    }
    retried.push(root.to_path_buf());
    true
}

/// How one attempt at a row ended (spec I4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Done,
    Review(ReviewReason),
    /// A transient error (F8), or bytes arrived on a cloud-only file since its state was read: the next tick.
    Pending,
}

pub fn retry(db: &StateDb, root: &Path) {
    let _guard = FINALIZING.lock().unwrap_or_else(|e| e.into_inner());
    let rows = match db.upload_finalizations() {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(%error, "cannot read upload finalization journal");
            return;
        }
    };
    let with_review = first_retry_of(root);
    for row in rows {
        // Spec R1: a row in review is left alone until the next engine start.
        if row.review.is_some() && !with_review {
            continue;
        }
        match finish(db, root, &row) {
            Ok(Outcome::Done) => {}
            Ok(Outcome::Pending) => {
                // Spec F8 ("no review"): a row in review that meets a transient error leaves review, so the next tick
                // tries it again instead of the next start (self-review of #195).
                if row.review.is_some() {
                    if let Err(error) = db.clear_upload_finalization_review(&row.op_id) {
                        tracing::warn!(
                            reason = crate::engine_bridge::error_category(&error),
                            "cannot clear an upload finalization review"
                        );
                    }
                }
                tracing::debug!(file_id = %row.server_file_id, "upload identity finalization pending");
            }
            // Spec R2: one warn when a row enters review (or its reason changes), never one per tick.
            Ok(Outcome::Review(reason)) if row.review.as_deref() != Some(reason.as_str()) => {
                if let Err(error) = db.mark_upload_finalization_review(&row.op_id, reason.as_str(), now_secs()) {
                    tracing::warn!(
                        reason = crate::engine_bridge::error_category(&error),
                        "cannot record an upload finalization review"
                    );
                    continue;
                }
                tracing::warn!(
                    file_id = %row.server_file_id,
                    reason = reason.as_str(),
                    "upload identity finalization held for review"
                );
            }
            Ok(Outcome::Review(_)) => {}
            Err(error) => tracing::warn!(
                file_id = %row.server_file_id,
                reason = crate::engine_bridge::error_category(error.as_ref()),
                "upload identity finalization deferred"
            ),
        }
    }
}

fn finish(db: &StateDb, root: &Path, row: &UploadFinalization) -> anyhow::Result<Outcome> {
    if !row.stamped {
        let outcome = stamp(db, root, row)?;
        if outcome != Outcome::Done {
            return Ok(outcome);
        }
        // A crash after unlink must not require a proof that was already removed.
        db.mark_upload_finalization_stamped(&row.op_id)?;
    }
    crate::staged_payload::remove(db, Path::new(&row.payload_path))?;
    db.forget_upload_finalization(&row.op_id)?;
    tracing::info!(file_id = %row.server_file_id, "upload identity finalization completed");
    Ok(Outcome::Done)
}

/// Spec F0-F9 for a row not stamped yet. `Done` once nothing is left to stamp. The platform's view of the file is read
/// first, through a handle that fetches nothing; a file is opened for its data only when it is plain or wholly on
/// disk. `Err` only for the journal's own reads, which leave the row pending.
fn stamp(db: &StateDb, root: &Path, row: &UploadFinalization) -> anyhow::Result<Outcome> {
    use super::placeholders::{self, Local, Restamped};
    let server_path = db.get_file(&row.server_file_id)?.map(|entry| entry.path);
    let on_disk = crate::engine_bridge::local_file_path_under_sync_root(root, &row.target_path)?;
    let view = placeholders::platform_view(&on_disk);
    let applied = match first_step(row, server_path.as_deref(), &view) {
        Step::Done => return Ok(Outcome::Done),
        Step::Review(reason) => return Ok(Outcome::Review(reason)),
        Step::ReStampCloudOnly => placeholders::restamp_cloud_only(&on_disk, &row.local_file_id, &row.server_file_id)
            .map(|restamped| match restamped {
                Restamped::Done => Outcome::Done,
                Restamped::NoLongerCloudOnly => Outcome::Pending,
                Restamped::OtherFile => Outcome::Review(ReviewReason::OtherFile),
            }),
        Step::JudgeBytes => {
            // Anti-goal 3: the agreed content counts only while no upload of the file is queued.
            let agreed = if db.sync_folder_upload_queued(&row.server_file_id)? {
                None
            } else {
                db.get_file_contract_state(&row.server_file_id)?.and_then(|contract| contract.local_hash)
            };
            // The staged copy, as large as the upload, is read only once the file itself is open for its hash (0.8.19's
            // order; self-review of #195): a file open in another program stays pending without reading it.
            let payload = Path::new(&row.payload_path);
            placeholders::finalize_wholly_local(&on_disk, &row.local_file_id, &row.server_file_id, &mut |bytes| {
                Ok(judge(bytes, payload_hash(payload)?.as_deref(), agreed.as_deref()))
            })
            .map(|local| match local {
                Local::Finalized(_) => Outcome::Done,
                Local::NotWhollyLocal => Outcome::Review(ReviewReason::PartlyOnDisk),
                Local::OtherFile => Outcome::Review(ReviewReason::OtherFile),
            })
        }
    };
    Ok(match applied {
        Ok(outcome) => outcome,
        Err(error) => after_error(&error, &row.server_file_id),
    })
}

/// Spec F8 / F3 / F9: how an error of the platform steps ends the attempt. A file (or its folder) gone since its view
/// was read is pending: the next tick reads the view again, and F2 ends the row (self-review of #195).
fn after_error(error: &anyhow::Error, file_id: &str) -> Outcome {
    use super::placeholders;
    if placeholders::is_transient(error) || placeholders::is_gone(error) {
        Outcome::Pending
    } else if placeholders::is_access_denied(error) {
        Outcome::Review(ReviewReason::Unreadable)
    } else {
        tracing::debug!(file_id, %error, "upload identity finalization platform error");
        Outcome::Review(ReviewReason::PlatformError)
    }
}

#[cfg(test)]
thread_local! {
    /// Staged payloads [`payload_hash`] opened on this thread (tests count them).
    static PAYLOADS_READ: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Tests only: how many staged payloads this thread has opened to hash.
#[cfg(test)]
pub(crate) fn payloads_read() -> usize {
    PAYLOADS_READ.with(std::cell::Cell::get)
}

/// The staged payload's SHA-256, or `None` when it is gone (a crash after its unlink).
fn payload_hash(path: &Path) -> anyhow::Result<Option<String>> {
    use sha2::{Digest, Sha256};
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    #[cfg(test)]
    PAYLOADS_READ.with(|read| read.set(read.get() + 1));
    // Stream it; never load a large upload into memory.
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(Some(format!("{:x}", hasher.finalize())))
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec F8 (self-review of #195): a file that is gone, or whose folder is gone, since its view was read (os 2,
    /// os 3 from the open) is pending: the next tick reads the view again and ends the row (F2). Never review.
    #[test]
    fn a_file_gone_since_its_view_was_read_is_pending() {
        for code in [2, 3] {
            let error = anyhow::Error::from(std::io::Error::from_raw_os_error(code));
            assert_eq!(after_error(&error, "file-id"), Outcome::Pending, "os error {code}");
        }
        let denied = anyhow::Error::from(std::io::Error::from_raw_os_error(5));
        assert_eq!(after_error(&denied, "file-id"), Outcome::Review(ReviewReason::Unreadable));
        let unexpected = anyhow::Error::from(std::io::Error::from_raw_os_error(1117));
        assert_eq!(after_error(&unexpected, "file-id"), Outcome::Review(ReviewReason::PlatformError));
    }
}
