//! What this account uploads, for the native Files window's Uploads panel: the queue's uploads of files (what
//! stopped, what moves, what waits), Try again on one, and the cancel of an upload of a NEW file.
//!
//! - Read from the signed-in account's local state through the account-bound handle, for the session of the ask: a
//!   session that moved on is [`LocalStateRefused::SessionChanged`], nothing is handed over.
//! - The transfer board is one per process: a board row whose file is not among this account's unsent rows is never
//!   listed.
//! - What stopped is worded by the engine ([`crate::drive_v2::status::reason_for`]), never by a host.
//! - Cancel is only for a NEW file never in Beebeeb: the local copy goes to the Recycle Bin first (the caller's
//!   `recycle`), then the queued change and the file's row are dropped in one transaction. Nothing is sent to
//!   Beebeeb; what a half-sent upload left there is swept later by the server.
//!
//! BLOCKS on the local state and (for cancel) the file system: a worker's. Must not: log or `Debug` a name or a
//! folder.

use std::collections::HashMap;
use std::path::Path;

use super::{CredentialStores, Engine, LocalStateRefused};
use crate::account::SessionGeneration;
use crate::drive_v2::status::{is_problem, reason_for};
use crate::state_db::{CancelNew, RetryOutcome, UnsentChange};
use crate::transfer_progress::Direction;

#[cfg(test)]
#[path = "uploads_api_tests.rs"]
mod tests;

/// How many queued changes one read looks at.
const PAGE: u32 = 200;

/// Where one upload stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UploadStands {
    /// Moving: encrypted bytes sent of the total.
    Moving { sent: u64, total: u64 },
    /// Queued, not started (or waiting out a backoff the engine tries again).
    Waiting,
    /// Stopped: the engine's own words, and whether Try again applies.
    Stopped { words: String, retryable: bool },
}

#[derive(Clone, PartialEq, Eq)]
pub struct UploadRow {
    pub file_id: String,
    pub change_id: String,
    pub name: String,
    pub folder: String,
    pub size: u64,
    /// A NEW file never in Beebeeb: Cancel applies.
    pub new_file: bool,
    pub stands: UploadStands,
}

/// Never a name or a folder.
impl std::fmt::Debug for UploadRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UploadRow")
            .field("stands", &self.stands_word())
            .finish()
    }
}

impl UploadRow {
    fn stands_word(&self) -> &'static str {
        match self.stands {
            UploadStands::Moving { .. } => "moving",
            UploadStands::Waiting => "waiting",
            UploadStands::Stopped { .. } => "stopped",
        }
    }
}

/// The rows, and the session they were read for.
#[derive(Clone, PartialEq, Eq)]
pub struct UploadsRead {
    pub session: SessionGeneration,
    /// Stopped first, then moving, then waiting; oldest first inside each.
    pub rows: Vec<UploadRow>,
    /// The sync pause is on (`Engine::pause`).
    pub paused: bool,
}

/// Counts only.
impl std::fmt::Debug for UploadsRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UploadsRead")
            .field("session", &self.session)
            .field("rows", &self.rows.len())
            .field("paused", &self.paused)
            .finish()
    }
}

/// What a cancel did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelOutcome {
    /// The copy is in the Recycle Bin, the change and the row are gone.
    Cancelled,
    /// Not a new file's upload (an edit, a backup, ...): left as it is.
    NotNew,
    /// Being sent right now, or more is queued for the file: left as it is.
    Busy,
    /// No such change any more (sent meanwhile).
    Unknown,
    /// The copy could not be moved to the Recycle Bin: nothing was changed.
    RecycleFailed,
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn leaf(path: &str) -> (String, String) {
    let trimmed = path.trim_matches(|c| c == '/' || c == '\\');
    match trimmed.rfind(['/', '\\']) {
        Some(i) => (trimmed[i + 1..].to_string(), trimmed[..i].to_string()),
        None => (trimmed.to_string(), String::new()),
    }
}

impl<C: CredentialStores> Engine<C> {
    /// See the module's doc. One page of the queue, one query for the sizes.
    pub fn uploads_for(&self, session: SessionGeneration) -> Result<UploadsRead, LocalStateRefused> {
        let state = self.local_state()?;
        if state.session() != session {
            return Err(LocalStateRefused::SessionChanged);
        }
        let now = unix_now();
        let (queue, sizes) = state
            .with(
                |db| -> Result<(Vec<UnsentChange>, HashMap<String, i64>), rusqlite::Error> {
                    let page = db.unsent_changes(0, PAGE, now)?;
                    let wanted: Vec<UnsentChange> = page
                        .items
                        .into_iter()
                        .filter(|c| !c.backup && c.file_id.is_some() && matches!(c.kind, "new_file" | "edit"))
                        .collect();
                    let ids: Vec<String> = wanted.iter().filter_map(|c| c.file_id.clone()).collect();
                    Ok((wanted, db.sizes_of(&ids)?))
                },
            )?
            .map_err(|_| LocalStateRefused::Unreadable)?;
        let board = self
            .host
            .transfer_board()
            .map(|b| b.snapshot_dtos())
            .unwrap_or_default();
        let moving: HashMap<&str, (u64, u64)> = board
            .iter()
            .filter(|t| t.direction == Direction::Up && !t.failed)
            .map(|t| (t.file_id.as_str(), (t.done, t.total)))
            .collect();
        let mut stopped = Vec::new();
        let mut moves = Vec::new();
        let mut waits = Vec::new();
        for change in &queue {
            let Some(file_id) = change.file_id.clone() else {
                continue;
            };
            let (name, folder) = leaf(&change.path);
            let size = sizes.get(&file_id).copied().unwrap_or(0).max(0) as u64;
            let mut row = UploadRow {
                file_id: file_id.clone(),
                change_id: change.id.clone(),
                name,
                folder,
                size,
                new_file: change.kind == "new_file",
                stands: UploadStands::Waiting,
            };
            if is_problem(change) {
                row.stands = UploadStands::Stopped {
                    words: reason_for(change)
                        .unwrap_or("It didn\u{2019}t sync. Nothing was lost.")
                        .to_string(),
                    retryable: change.retryable,
                };
                stopped.push(row);
            } else if let Some(&(sent, total)) = moving.get(file_id.as_str()) {
                row.stands = UploadStands::Moving {
                    sent,
                    total: if total == 0 { size } else { total },
                };
                moves.push(row);
            } else {
                waits.push(row);
            }
        }
        let mut rows = stopped;
        rows.extend(moves);
        rows.extend(waits);
        Ok(UploadsRead {
            session,
            rows,
            paused: self.view().paused,
        })
    }

    /// Try again on ONE queued change, by the queue's own rule. A session that moved on changes nothing.
    pub fn retry_upload_for(
        &self,
        session: SessionGeneration,
        change_id: &str,
    ) -> Result<RetryOutcome, LocalStateRefused> {
        let state = self.local_state_for_changes()?;
        if state.session() != session {
            return Err(LocalStateRefused::SessionChanged);
        }
        let now = unix_now();
        state
            .with(|db| db.retry_unsent_change(change_id, now))?
            .map_err(|_| LocalStateRefused::Unreadable)
    }

    /// Cancel the upload of a NEW file (see the module's doc). `recycle` moves the local copy to the Recycle Bin; a
    /// copy that is already gone counts as moved (the closure says `Ok`). It is asked only after the change was found cancellable, and the
    /// change is dropped only after it succeeded.
    pub fn cancel_upload_for(
        &self,
        session: SessionGeneration,
        change_id: &str,
        recycle: &dyn Fn(&Path) -> std::io::Result<()>,
    ) -> Result<CancelOutcome, LocalStateRefused> {
        let state = self.local_state_for_changes()?;
        if state.session() != session {
            return Err(LocalStateRefused::SessionChanged);
        }
        let unreadable = |_: rusqlite::Error| LocalStateRefused::Unreadable;
        let target = state.with(|db| db.new_upload_target(change_id))?.map_err(unreadable)?;
        let file_id = match target {
            CancelNew::Ready { file_id } => file_id,
            CancelNew::Unknown => return Ok(CancelOutcome::Unknown),
            CancelNew::NotNew => return Ok(CancelOutcome::NotNew),
            CancelNew::Busy => return Ok(CancelOutcome::Busy),
        };
        let sync_root = self
            .configured_sync_root()
            .map_err(|_| LocalStateRefused::Unreadable)?
            .unwrap_or_else(|| crate::config::default_sync_root_suggestion(self.identity));
        let path = state
            .with(|db| {
                crate::file_ops::resolve(db, &file_id)
                    .ok()
                    .and_then(|entry| crate::file_ops::item_path(&sync_root, &entry).ok())
            })?
            .ok_or(LocalStateRefused::Unreadable)?;
        if recycle(&path).is_err() {
            return Ok(CancelOutcome::RecycleFailed);
        }
        Ok(
            match state.with(|db| db.drop_new_upload(change_id))?.map_err(unreadable)? {
                CancelNew::Ready { .. } => CancelOutcome::Cancelled,
                // The engine took it meanwhile (the removal of the copy was seen): the copy is in the bin all the same.
                CancelNew::Unknown => CancelOutcome::Cancelled,
                CancelNew::NotNew => CancelOutcome::NotNew,
                CancelNew::Busy => CancelOutcome::Busy,
            },
        )
    }
}

/// Move one file to the Recycle Bin (recoverable). Windows only: another system says `Unsupported`.
#[cfg(target_os = "windows")]
pub fn move_to_recycle_bin(path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::Shell::{
        FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW, SHFileOperationW,
    };
    if !path.exists() {
        return Ok(());
    }
    // The list is double-NUL terminated.
    let mut from: Vec<u16> = path.as_os_str().encode_wide().collect();
    from.extend([0, 0]);
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: windows::core::PCWSTR(from.as_ptr()),
        fFlags: (FOF_ALLOWUNDO.0 | FOF_NOCONFIRMATION.0 | FOF_NOERRORUI.0 | FOF_SILENT.0) as u16,
        ..Default::default()
    };
    // SAFETY: `op` and `from` live for the call; the flags forbid any UI.
    let code = unsafe { SHFileOperationW(&mut op) };
    if code != 0 || op.fAnyOperationsAborted.as_bool() {
        return Err(std::io::Error::other("the copy could not be moved to the Recycle Bin"));
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn move_to_recycle_bin(_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}
