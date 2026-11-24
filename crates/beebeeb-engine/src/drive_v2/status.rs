//! Batch B02b: what the window draws beyond `DriveSource`'s first contract, computed from what the engine and the
//! state DB already know (spec 3.1; `design/desktop-v2/PUNCHLIST.md`, the "GAP (data)" items). Pure functions over
//! their inputs, so each mapping has a unit test (`status_tests.rs`); the commands that gather the inputs are in
//! `status_cmds.rs`.
//!
//! The rule: a value the engine cannot know is `None` and serialises as absent, never a guess. No key, token, URL,
//! raw error text or absolute path is in any of these types.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::dto::DriveItem;
use crate::state_db::{FileEntry, HeldEdits, UnsentChange};
use crate::transfer_progress::{ActiveTransferDto, Direction};

/// The engine snapshot fields the tray flyout, the banners, Backups and the getting-ready strip draw.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveStatus {
    /// Signed out, or the session expired: sync and backups have stopped (banner DW-0101).
    pub signed_out: bool,
    /// Whether this PC's connection is metered. `None`: this build does not read the connection's cost.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metered: Option<bool>,
    /// Changes the queue stopped retrying (the flyout's "Sync stopped").
    pub stopped_files: u32,
    /// Items that can't sync: stopped changes plus files that need a decision (banner DW-0103).
    pub sync_problems: u32,
    /// Items that couldn't back up: stopped backup changes plus backups that stopped (banner DW-0104).
    pub backup_problems: u32,
    /// Upload bytes per second now; absent while nothing uploads or before a rate is measured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_bps: Option<u64>,
    /// Seconds the uploads in flight take at that rate; absent without a rate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_seconds: Option<u64>,
    /// Files in Beebeeb as this PC's list has them; absent before the first list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_count: Option<u64>,
    /// Free bytes on the disk that holds the sync folder (DW-1517); absent when unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_space_bytes: Option<u64>,
    pub backup: BackupView,
    /// The engine's first check of the file list while it runs (the getting-ready strip, DW-1107).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listing: Option<ListingProgress>,
    /// Files that changed in two places and wait for a decision (the conflict dialog, DW-0429).
    pub conflicts: Vec<ConflictRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ListingProgress {
    pub done: u64,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConflictRef {
    pub id: String,
    pub name: String,
}

/// This PC's backup, for Backups' "Last backup" and "Status" columns.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupView {
    pub state: BackupState,
    /// When the newest pass of a folder that is on finished (ISO 8601); absent before the first pass.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_backup_at: Option<String>,
    /// This PC's folder in Backups (its name as the server layout has it); absent before the layout is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_folder: Option<String>,
    /// Folders that are on.
    pub folders_on: u32,
    /// Folders that are on but have not finished a first pass.
    pub folders_getting_ready: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackupState {
    /// No folder of this PC is backed up.
    #[default]
    Off,
    /// On, and no folder has finished its first pass yet.
    GettingReady,
    /// On and backing up.
    On,
    /// At least one backup stopped and waits for "Back up again".
    Stopped,
}

/// `backup_view`: `enabled` are the folders that are on, `marks` each one's last pass (ms, server clock), `stops`
/// how many backups stopped.
pub fn backup_view(enabled: &[&str], marks: &HashMap<String, Option<i64>>, stops: usize) -> BackupView {
    let passes: Vec<i64> = enabled
        .iter()
        .filter_map(|k| marks.get(*k).copied().flatten())
        .collect();
    let folders_on = enabled.len() as u32;
    let folders_getting_ready = folders_on - passes.len() as u32;
    let state = if stops > 0 {
        BackupState::Stopped
    } else if folders_on == 0 {
        BackupState::Off
    } else if passes.is_empty() {
        BackupState::GettingReady
    } else {
        BackupState::On
    };
    let last_backup_at = passes.iter().max().map(|ms| super::model::iso(ms / 1000));
    BackupView {
        state,
        last_backup_at,
        device_folder: None,
        folders_on,
        folders_getting_ready,
    }
}

/// Upload rate and time left from the transfers in flight. Only uploads count (the flyout's figure is the upload);
/// a rate of 0 is "not measured yet", so both stay absent.
pub fn upload_rate(board: &[ActiveTransferDto]) -> (Option<u64>, Option<u64>) {
    let ups: Vec<&ActiveTransferDto> = board
        .iter()
        .filter(|t| t.direction == Direction::Up && !t.failed)
        .collect();
    let bps: u64 = ups.iter().map(|t| t.speed_bps).sum();
    if bps == 0 {
        return (None, None);
    }
    let left: u64 = ups.iter().map(|t| t.total.saturating_sub(t.done)).sum();
    (Some(bps), Some(left.div_ceil(bps)))
}

/// The free bytes of the disk whose mount point is the longest prefix of `path`.
pub fn free_space_for(path: &Path, disks: &[(PathBuf, u64)]) -> Option<u64> {
    disks
        .iter()
        .filter(|(mount, _)| path.starts_with(mount))
        .max_by_key(|(mount, _)| mount.as_os_str().len())
        .map(|(_, free)| *free)
}

/// One row of the Uploads panel (DW-0427) and of the live per-row progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferRow {
    /// The file id for a moving row, the queue's change id for a waiting or stopped one.
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    pub name: String,
    /// The folder it goes to, `""` at the root.
    pub folder: String,
    pub direction: TransferDirection,
    pub state: TransferState,
    pub bytes: u64,
    /// Total bytes; absent when the engine has not learned it yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    /// Why it stopped, in the board's plain words; only on a stopped row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    /// Try again applies (the queue's own rule).
    pub retryable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferDirection {
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferState {
    Moving,
    Waiting,
    Stopped,
}

fn leaf(path: &str) -> (String, String) {
    let trimmed = path.trim_matches(|c| c == '/' || c == '\\');
    match trimmed.rfind(['/', '\\']) {
        Some(i) => (trimmed[i + 1..].to_string(), trimmed[..i].to_string()),
        None => (trimmed.to_string(), String::new()),
    }
}

/// Why a change did not sync, in plain words (DW-0428: "never a raw error code"). `None` when it has not stopped.
pub fn reason_for(change: &UnsentChange) -> Option<&'static str> {
    if let Some(block) = change.blocked {
        return Some(match block {
            "superseded" => "A newer version is already in Beebeeb.",
            _ => concat!("The copy to upload is gone from ", crate::device_word!(), "."),
        });
    }
    if change.parked {
        return Some(parked_words(change.park, &change.exits));
    }
    Some(match change.group.as_str() {
        "gave_up" => "Stopped after 5 tries.",
        "not_marked_synced" => concat!("Uploaded, but ", crate::device_word!(), " couldn’t mark it as synced."),
        "storage_full" | "quota" => "Not enough space in your Beebeeb.",
        "signed_out" | "auth" => "Waiting for you to sign in.",
        "waiting_for_folder" => "Waiting for its folder to upload.",
        "waiting" if change.retry_at.is_some() => "Couldn’t reach Beebeeb. Trying again soon.",
        _ => return None,
    })
}

/// Task 2054: the words for an upload the server refused because the chosen storage country cannot take new files. A
/// country, never a city; the file stays queued, so the words say it is kept and will be tried again. An empty
/// country names none.
pub fn storage_country_words(country: &str) -> String {
    let country = country.trim();
    if country.is_empty() {
        return "Your storage country can’t take new files right now. This file stays in the queue and is tried again later."
            .to_string();
    }
    format!(
        "{country} can’t take new files right now. Your choice is set to only {country}, so this file stays in the queue and is tried again later."
    )
}

/// Task 2530: a way out a problem row may offer (the engine's list, `state_db::parked_exit`).
pub use crate::state_db::ParkExit as Exit;

/// Task 2530: why a parked change is not sent, by why it parked, in words that say what the row's ways out do. A
/// change with no way out never promises one.
fn parked_words(park: Option<&str>, exits: &[Exit]) -> &'static str {
    match park {
        Some("stale_base") if !exits.is_empty() => {
            "Not sent: this file changed in Beebeeb before this change reached it. Choose which version to keep. Nothing was lost."
        }
        Some("stale_base") => {
            concat!("Not sent: this file changed in Beebeeb before this change reached it. Your copy stays on ", crate::device_word!(), ". Nothing was lost.")
        }
        Some("permission_denied") => {
            concat!("Not sent: this account can no longer change this file in Beebeeb. Your change stays on ", crate::device_word!(), ". Nothing was lost.")
        }
        Some("predecessor_lost") => {
            concat!("Not sent: Beebeeb lost track of the change made before this one. Your change stays on ", crate::device_word!(), ". Nothing was lost.")
        }
        Some("predecessor_parked") => {
            concat!("Waiting: an earlier change to this file on ", crate::device_word!(), " has to be sent first. Nothing was lost.")
        }
        Some("rekey_failed") => {
            "Not sent: Beebeeb couldn’t prepare this change after the file was created. Nothing was lost."
        }
        Some("payload_missing") => {
            concat!(crate::device_word_cap!(), " no longer has the bytes of this change, so it can’t be sent. The file on Beebeeb is as it was.")
        }
        Some("base_unknown") => {
            concat!("Not sent: Beebeeb doesn’t know which version this change started from. Your copy stays on ", crate::device_word!(), ". Nothing was lost.")
        }
        _ => "Beebeeb couldn’t apply this change. Nothing was lost.",
    }
}

/// A change counts as a problem when it stopped, is blocked or parked, or was uploaded but not marked as synced.
pub fn is_problem(change: &UnsentChange) -> bool {
    change.blocked.is_some() || change.parked || matches!(change.group.as_str(), "gave_up" | "not_marked_synced")
}

/// The Uploads panel: what moves now (the transfer board's uploads), then the queue's changes not sent yet. A
/// change whose file is moving is shown once, as moving.
pub fn transfer_rows(board: &[ActiveTransferDto], queue: &[UnsentChange]) -> Vec<TransferRow> {
    let mut rows: Vec<TransferRow> = board
        .iter()
        .filter(|t| t.direction != Direction::Stream)
        .map(|t| TransferRow {
            id: t.file_id.clone(),
            file_id: Some(t.file_id.clone()),
            name: t.name.clone(),
            folder: t.folder.clone(),
            direction: if t.direction == Direction::Up {
                TransferDirection::Up
            } else {
                TransferDirection::Down
            },
            state: if t.failed {
                TransferState::Stopped
            } else {
                TransferState::Moving
            },
            bytes: t.done,
            total: (t.total > 0).then_some(t.total),
            reason: t
                .failed
                .then_some("It didn’t finish. Nothing was lost; Beebeeb will try again."),
            retryable: false,
        })
        .collect();
    // The queue lists no downloads (`unsent_changes`); a waiting row is a file's upload, a stopped row any change.
    for change in queue
        .iter()
        .filter(|c| !c.backup && (is_problem(c) || matches!(c.kind, "new_file" | "edit")))
    {
        if change
            .file_id
            .as_ref()
            .is_some_and(|id| rows.iter().any(|r| r.file_id.as_deref() == Some(id.as_str())))
        {
            continue;
        }
        let (name, folder) = leaf(&change.path);
        let problem = is_problem(change);
        rows.push(TransferRow {
            id: change.id.clone(),
            file_id: change.file_id.clone(),
            name,
            folder,
            direction: TransferDirection::Up,
            state: if problem {
                TransferState::Stopped
            } else {
                TransferState::Waiting
            },
            bytes: 0,
            total: None,
            // Task 2530: the Uploads panel offers no way out of a parked change, so its words promise none.
            reason: match (problem, change.parked) {
                (true, true) => Some(parked_words(change.park, &[])),
                (true, false) => reason_for(change),
                (false, _) => None,
            },
            retryable: change.retryable,
        });
    }
    rows
}

/// One Sync history problem row (DW-0428): what did not sync and why, with Try again where it applies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncProblem {
    /// The queue's change id: what `drive_sync_retry` takes.
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    pub name: String,
    pub folder: String,
    pub reason: &'static str,
    /// Try again applies: true exactly where [`Self::exits`] holds [`Exit::TryAgain`].
    pub retryable: bool,
    pub backup: bool,
    /// When it was queued (ISO 8601); absent when not recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    /// Task 2530: the ways out the row offers, and only these (a window never draws one the engine did not list).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exits: Vec<Exit>,
    /// Task 2530: the file's version a Keep choice is made against (the read's); absent without one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<i64>,
}

/// Task 2530: a queue row's ways out: a parked change's from the engine's read; any other change's Try again where the
/// queue's own rule allows it.
fn exits_of(change: &UnsentChange) -> Vec<Exit> {
    if change.parked {
        change.exits.clone()
    } else if change.retryable {
        vec![Exit::TryAgain]
    } else {
        Vec::new()
    }
}

pub fn sync_problems(queue: &[UnsentChange]) -> Vec<SyncProblem> {
    queue
        .iter()
        .filter(|c| is_problem(c))
        .map(|c| {
            let (name, folder) = leaf(&c.path);
            let exits = exits_of(c);
            SyncProblem {
                id: c.id.clone(),
                file_id: c.file_id.clone(),
                name,
                folder: if c.backup { String::new() } else { folder },
                reason: reason_for(c).unwrap_or("It didn’t sync. Nothing was lost."),
                retryable: exits.contains(&Exit::TryAgain),
                backup: c.backup,
                at: (c.queued_at > 0).then(|| super::model::iso(c.queued_at)),
                version: c.version.filter(|_| !exits.is_empty()),
                exits,
            }
        })
        .collect()
}

/// Task 2524: why a held edit is not sent. A reason this build does not know says only that it is held back.
fn held_reason(reason: &str) -> &'static str {
    match reason {
        "base_unknown" => {
            concat!(
                "Not sent: Beebeeb doesn’t know which version this change started from. Your copy stays on ",
                crate::device_word!(),
                ". Nothing was lost."
            )
        }
        _ => concat!("Not sent: ", crate::device_word!(), " is holding this change back."),
    }
}

/// The one row for every `content_unknown` file.
const UNCHECKED_WORDS: &str = concat!("Not sent until they are checked: ", crate::device_word!(), " has no record of what these files held when they last synced, so it can’t tell whether any was changed. Nothing to do now. Nothing was lost.");

/// Task 2524: what the engine holds and does not send, as problems so it never looks synced: one per held edit whose
/// base is unknown (id `held:<file id>`), and ONE summary row for the `content_unknown` files (id
/// `held:content_unknown`: the count and the first names). None can be tried again (`retryable` is false).
pub fn held_edit_problems(held: &HeldEdits) -> Vec<SyncProblem> {
    let at = |t: i64| (t > 0).then(|| super::model::iso(t));
    let mut out: Vec<SyncProblem> = held
        .each
        .iter()
        .map(|(file_id, path, reason, detected_at)| {
            let (name, folder) = leaf(path);
            SyncProblem {
                id: format!("held:{file_id}"),
                file_id: Some(file_id.clone()),
                name,
                folder,
                reason: held_reason(reason),
                retryable: false,
                backup: false,
                at: at(*detected_at),
                // Task 2530 (step 0): no engine path sends a held edit as a new file yet, so the row has words only.
                exits: Vec::new(),
                version: None,
            }
        })
        .collect();
    if held.unchecked > 0 {
        let names: Vec<String> = held.unchecked_first.iter().map(|f| leaf(&f.1).0).collect();
        let more = held.unchecked - i64::try_from(names.len()).unwrap_or(0);
        let files = if held.unchecked == 1 {
            "1 file".to_string()
        } else {
            format!("{} files", held.unchecked)
        };
        let and_more = if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        };
        out.push(SyncProblem {
            id: "held:content_unknown".into(),
            file_id: held.unchecked_first.first().map(|f| f.0.clone()),
            name: format!("{files} not checked yet ({}{and_more})", names.join(", ")),
            folder: String::new(),
            reason: UNCHECKED_WORDS,
            retryable: false,
            backup: false,
            at: at(held.unchecked_first.first().map_or(0, |f| f.2)),
            exits: Vec::new(),
            version: None,
        });
    }
    out
}

/// One Available-offline row (DW-0715): a kept item that no kept folder holds, with a folder's file count and bytes.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflineEntry {
    pub item: DriveItem,
    /// Files inside a folder (all levels); absent for a file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<u64>,
    /// The file's size, or the sum of every file inside the folder.
    pub bytes: i64,
}

/// `rows` are the live rows with whether each is kept (its own or inherited pin). Only the top-most kept items are
/// listed; a kept folder's total counts every file under its path.
pub fn offline_entries(rows: &[(FileEntry, bool, DriveItem)]) -> Vec<OfflineEntry> {
    let kept: HashMap<&str, bool> = rows.iter().map(|(e, k, _)| (e.file_id.as_str(), *k)).collect();
    let mut out = Vec::new();
    for (entry, is_kept, item) in rows {
        if !is_kept
            || entry
                .parent_id
                .as_deref()
                .is_some_and(|p| kept.get(p).copied().unwrap_or(false))
        {
            continue;
        }
        if entry.is_dir() {
            let prefix = format!("{}/", entry.path.trim_matches('/'));
            let files: Vec<&FileEntry> = rows
                .iter()
                .map(|(e, _, _)| e)
                .filter(|e| !e.is_dir() && e.path.trim_start_matches('/').starts_with(&prefix))
                .collect();
            out.push(OfflineEntry {
                item: item.clone(),
                items: Some(files.len() as u64),
                bytes: files.iter().map(|e| e.size_bytes.max(0)).sum(),
            });
        } else {
            out.push(OfflineEntry {
                item: item.clone(),
                items: None,
                bytes: entry.size_bytes.max(0),
            });
        }
    }
    out.sort_by(|a, b| {
        b.item
            .is_folder
            .cmp(&a.item.is_folder)
            .then_with(|| a.item.name.to_lowercase().cmp(&b.item.name.to_lowercase()))
    });
    out
}

#[cfg(test)]
mod held_tests;

#[cfg(test)]
mod exits_tests;

#[cfg(test)]
mod region_strict_words_tests {
    use super::storage_country_words;

    #[test]
    fn region_strict_words_name_the_country_and_say_the_file_is_kept() {
        let words = storage_country_words("Finland");
        assert!(words.starts_with("Finland can’t take new files"), "{words}");
        assert!(words.contains("stays in the queue"), "{words}");
    }

    #[test]
    fn region_strict_words_without_a_country_name_none() {
        assert!(!storage_country_words("").contains("  "));
        assert!(storage_country_words(" ").starts_with("Your storage country"));
    }
}
