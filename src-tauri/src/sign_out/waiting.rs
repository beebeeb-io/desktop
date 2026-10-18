//! Spec 2026-10-10 §2 and W2 (task 1887): what waits in the queue, as one sign-out counts it. A waiting write is an
//! upload with no recorded completion; per file, each write is copied, covered or missing (§7.2). An other waiting
//! change is a folder create, rename, move, trash or restore. Every number a person sees comes from here.
#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::state_db::{OperationKind, StateDb, WaitingRow, WriteOrigin};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteState {
    /// The keep step copies it (§2 "Copied").
    Copied,
    /// A newer copied write of the same file holds its bytes (§2 "Covered").
    Covered,
    /// Its staged copy is gone and nothing covers it (§2 "Missing").
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitingWrite {
    pub op_id: String,
    pub payload_path: Option<PathBuf>,
    pub origin: WriteOrigin,
    pub queue_order: i64,
    pub live: bool,
    pub state: WriteState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitingFile {
    pub file_id: String,
    /// `files.path` as the keep step names the copy (§7.2), or `None` (a bug: `Unnamed …`).
    pub path: Option<String>,
    /// Oldest first.
    pub writes: Vec<WaitingWrite>,
}

impl WaitingFile {
    pub fn has_copy(&self) -> bool {
        self.writes.iter().any(|write| write.state == WriteState::Copied)
    }

    pub fn is_missing(&self) -> bool {
        self.writes.iter().any(|write| write.state == WriteState::Missing)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtherChange {
    pub op_id: String,
    pub live: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WaitingCounts {
    pub files: usize,
    pub copied_files: usize,
    pub others: usize,
    pub missing_files: usize,
    /// Live ops, writes and other changes.
    pub live: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WaitingChanges {
    pub files: Vec<WaitingFile>,
    pub others: Vec<OtherChange>,
}

/// What a queued op is to a sign-out (§2).
pub(crate) enum OpRole {
    /// A waiting write, unless its upload has a recorded completion.
    Upload,
    /// An other waiting change.
    OtherChange,
    /// A download the engine queued, never a change the person made: not counted.
    Download,
}

/// Every kind is named, with no wildcard arm, so a kind added to `OperationKind` later does not compile until it is
/// placed here, instead of being counted as nothing. A `kind` string the database holds but this build does not know
/// never arrives as its own case: `OperationKind::from_str` reads it as `UploadFile`, so it is counted as a waiting write.
/// The count ([`WaitingChanges::classify`]) and the purge guard (`StateDb::unkept_waiting_changes`) both place ops
/// here, so what the keep step keeps and what the guard asks for are the same ops.
pub(crate) fn op_role(kind: &OperationKind) -> OpRole {
    match kind {
        OperationKind::UploadFile | OperationKind::UploadVersion => OpRole::Upload,
        OperationKind::CreateFolder
        | OperationKind::RenameFile
        | OperationKind::MoveFile
        | OperationKind::TrashFile
        | OperationKind::RestoreFile
        | OperationKind::RestoreVersion => OpRole::OtherChange,
        OperationKind::HydrateFile | OperationKind::PinTree => OpRole::Download,
    }
}

/// §2 "Missing": `metadata()` is `NotFound`, the test 1873 uses for `payload_missing`. Any other answer counts as
/// present, and a copy that then fails stops the sign-out (W8) instead of dropping the write.
pub fn staged_copy_exists(path: &Path) -> bool {
    !matches!(std::fs::metadata(path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
}

impl WaitingChanges {
    /// W2: one `StateDb` call, then the file system is asked which staged copies exist.
    pub fn read(db: &StateDb) -> Result<Self, String> {
        let rows = db.waiting_rows().map_err(|error| format!("read the queue: {error}"))?;
        Ok(Self::classify(rows, staged_copy_exists))
    }

    pub fn classify(rows: Vec<WaitingRow>, exists: impl Fn(&Path) -> bool) -> Self {
        let mut by_file: BTreeMap<String, WaitingFile> = BTreeMap::new();
        let mut others = Vec::new();
        for row in rows {
            let live = row.attempts < row.max_attempts && !row.paused;
            match op_role(&row.kind) {
                OpRole::Download => {}
                OpRole::OtherChange => others.push(OtherChange { op_id: row.op_id, live }),
                // A recorded completion: the server has these bytes, so nothing waits (§2 "Waiting write").
                OpRole::Upload if row.completed => {}
                OpRole::Upload => {
                    let key = row.file_id.clone().unwrap_or_else(|| row.op_id.clone());
                    let present = row.payload_path.as_deref().is_some_and(|path| exists(Path::new(path)));
                    let file = by_file.entry(key.clone()).or_insert_with(|| WaitingFile {
                        file_id: key,
                        path: row.file_path.clone(),
                        writes: Vec::new(),
                    });
                    file.writes.push(WaitingWrite {
                        op_id: row.op_id,
                        payload_path: row.payload_path.map(PathBuf::from),
                        // An op with no recorded origin is treated as an earlier build's: its copy is kept (§7.2).
                        origin: row.origin.unwrap_or(WriteOrigin::EarlierBuild),
                        queue_order: row.queue_order,
                        live,
                        state: if present {
                            WriteState::Copied
                        } else {
                            WriteState::Missing
                        },
                    });
                }
            }
        }
        let mut files: Vec<WaitingFile> = by_file.into_values().collect();
        for file in &mut files {
            settle_states(&mut file.writes);
        }
        Self { files, others }
    }

    pub fn counts(&self) -> WaitingCounts {
        WaitingCounts {
            files: self.files.len(),
            copied_files: self.files.iter().filter(|file| file.has_copy()).count(),
            others: self.others.len(),
            missing_files: self.files.iter().filter(|file| file.is_missing()).count(),
            live: self
                .files
                .iter()
                .flat_map(|file| &file.writes)
                .filter(|write| write.live)
                .count()
                + self.others.iter().filter(|other| other.live).count(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.others.is_empty()
    }

    /// The progress line's unit (§2 "Waiting change"): waiting files plus other waiting changes.
    pub fn changes(&self) -> usize {
        self.files.len() + self.others.len()
    }

    pub fn any_live(&self) -> bool {
        self.counts().live > 0
    }

    pub fn file_ids(&self) -> BTreeSet<String> {
        self.files.iter().map(|file| file.file_id.clone()).collect()
    }

    pub fn other_op_ids(&self) -> BTreeSet<String> {
        self.others.iter().map(|other| other.op_id.clone()).collect()
    }

    pub fn missing_op_ids(&self) -> BTreeSet<String> {
        self.files
            .iter()
            .flat_map(|file| &file.writes)
            .filter(|write| write.state == WriteState::Missing)
            .map(|write| write.op_id.clone())
            .collect()
    }
}

/// §2 and §7.2. On entry `state` says only whether the staged copy exists (`Copied`) or not (`Missing`). The newest
/// write with a copy is copied; so is every earlier-build write with a copy; a minted write older than the newest
/// copied one is covered; every other write is missing.
fn settle_states(writes: &mut [WaitingWrite]) {
    writes.sort_by_key(|write| write.queue_order);
    let newest_copied = writes
        .iter()
        .rev()
        .find(|write| write.state == WriteState::Copied)
        .map(|write| write.queue_order);
    for write in writes.iter_mut() {
        let present = write.state == WriteState::Copied;
        write.state = match newest_copied {
            Some(newest) if write.queue_order == newest => WriteState::Copied,
            _ if present && write.origin == WriteOrigin::EarlierBuild => WriteState::Copied,
            Some(newest) if write.origin == WriteOrigin::Minted && write.queue_order < newest => WriteState::Covered,
            _ => WriteState::Missing,
        };
    }
}

/// §6.1: why a sign-out would not send first, in the spec's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotSending {
    EngineStopped,
    Session,
    Offline,
    Paused,
    Held,
}

/// §6.1 conditions 1-4; condition 5 (something is live) is read from the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendConditions {
    pub engine_running: bool,
    pub vault_unlocked: bool,
    pub session_expired: bool,
    pub link_failed: bool,
    pub sync_paused: bool,
}

/// `None`: nothing waits, or it can send. `Some(reason)`: the first condition that fails. Tests only: production
/// reads a snapshot it already holds, through [`readiness`].
#[cfg(test)]
pub fn send_readiness(conditions: SendConditions, waiting: &WaitingChanges) -> Option<NotSending> {
    readiness(conditions, waiting.is_empty(), waiting.any_live())
}

/// `send_readiness` on what a snapshot already holds (W2: no second read).
pub fn readiness(conditions: SendConditions, nothing_waits: bool, any_live: bool) -> Option<NotSending> {
    if nothing_waits {
        return None;
    }
    if !conditions.engine_running || !conditions.vault_unlocked {
        Some(NotSending::EngineStopped)
    } else if conditions.session_expired {
        Some(NotSending::Session)
    } else if conditions.link_failed {
        Some(NotSending::Offline)
    } else if conditions.sync_paused {
        Some(NotSending::Paused)
    } else if !any_live {
        Some(NotSending::Held)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_db::StateDb;
    use std::path::Path;

    fn db_with(sql: &str) -> (tempfile::TempDir, StateDb) {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let sql = sql.replace("<dir>", dir.path().to_str().unwrap());
        db.hold_lock_for_test().execute_batch(&sql).unwrap();
        (dir, db)
    }

    fn stage(dir: &Path, names: &[&str]) {
        for name in names {
            std::fs::write(dir.join(name), name.as_bytes()).unwrap();
        }
    }

    const COMPLETED: &str = "INSERT INTO upload_resume (op_id, payload_path, payload_size, payload_mtime_ns,
        upload_session_id, server_file_id, object_version_id, chunk_size_bytes, chunk_count, completed_version)
        VALUES ('d1', '<dir>/d1', 1, 1, 'u', 'f3', 'v', 1, 1, 7);";

    /// U1.
    #[test]
    fn waiting_changes_counts_files_and_other_changes_once() {
        let (dir, db) = db_with(&format!(
            "INSERT INTO files (file_id, path) VALUES ('f1', 'Work/report.docx'), ('f2', 'New/notes.txt'), ('f3', 'Done.txt');
             INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin) VALUES
               ('s1', 'upload_version', 'f1', '<dir>/s1', 'w1', 'minted'),
               ('s2', 'upload_version', 'f1', '<dir>/s2', 'w2', 'minted'),
               ('c1', 'upload_version', 'f2', '<dir>/c1', 'w3', 'minted'),
               ('r1', 'rename_file', 'f1', NULL, NULL, NULL),
               ('t1', 'trash_file', 'f9', NULL, NULL, NULL),
               ('h1', 'hydrate_file', 'f1', NULL, NULL, NULL),
               ('p1', 'pin_tree', 'f1', NULL, NULL, NULL),
               ('d1', 'upload_version', 'f3', '<dir>/d1', 'w4', 'minted');
             {COMPLETED}"
        ));
        stage(dir.path(), &["s1", "s2", "c1", "d1"]);
        let counts = WaitingChanges::read(&db).unwrap().counts();
        assert_eq!(counts.files, 2, "one file per file, the completed upload not counted");
        assert_eq!(
            counts.others, 2,
            "the rename and the trash; never the hydrate or the pin"
        );
    }

    /// U2 (spec §2 "Live").
    #[test]
    fn parked_and_paused_writes_are_waiting_but_not_live() {
        let (dir, db) = db_with(
            "INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin,
                                          attempts, max_attempts, paused_reason) VALUES
               ('parked', 'upload_version', 'f1', '<dir>/a', 'w1', 'minted', 25, 25, NULL),
               ('auth', 'upload_version', 'f2', '<dir>/b', 'w2', 'minted', 0, 25, 'auth'),
               ('held', 'upload_version', 'f3', '<dir>/c', 'w3', 'minted', 0, 25, 'key_replaced'),
               ('folder', 'create_folder', 'f4', NULL, NULL, NULL, 0, 5, 'quota');",
        );
        stage(dir.path(), &["a", "b", "c"]);
        let waiting = WaitingChanges::read(&db).unwrap();
        let counts = waiting.counts();
        assert_eq!((counts.files, counts.others, counts.live), (3, 1, 0));
        let ready = SendConditions {
            engine_running: true,
            vault_unlocked: true,
            session_expired: false,
            link_failed: false,
            sync_paused: false,
        };
        assert_eq!(
            send_readiness(ready, &waiting),
            Some(NotSending::Held),
            "can_send is false, and the reason is held"
        );
    }

    /// U3.
    #[test]
    fn a_missing_staged_copy_counts_as_missing_never_as_nothing() {
        let (_dir, db) = db_with(
            "INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin)
             VALUES ('s1', 'upload_version', 'f1', '<dir>/gone', 'w1', 'minted'),
                    ('s2', 'upload_version', NULL, NULL, 'w2', 'minted'),
                    ('s3', 'upload_version', NULL, NULL, 'w3', 'minted');",
        );
        let counts = WaitingChanges::read(&db).unwrap().counts();
        // An upload with no file id is its own waiting file, never merged with another; with no staged copy it is missing.
        assert_eq!((counts.files, counts.missing_files, counts.copied_files), (3, 3, 0));
    }

    /// U4: one `StateDb` call, one mutex hold, one transaction.
    #[test]
    fn waiting_changes_is_one_read() {
        let state_db = include_str!("../state_db.rs");
        let from = state_db.find("pub fn waiting_rows(").expect("waiting_rows");
        let body = &state_db[from..from + state_db[from..].find("\n    }\n").unwrap()];
        assert_eq!(body.matches(".lock()").count(), 1, "{body}");
        assert_eq!(body.matches("transaction(").count(), 1, "{body}");
        let this = include_str!("waiting.rs");
        let from = this.find("pub fn read(db: &StateDb)").expect("read");
        let read = &this[from..from + this[from..].find("\n    }\n").unwrap()];
        assert_eq!(read.matches("db.").count(), 1, "one StateDb call: {read}");
        assert!(read.contains("db.waiting_rows()"), "{read}");
    }

    /// U5: one row per condition, and rows where two fail, in the spec's order (§6.1).
    #[test]
    fn can_send_follows_each_condition() {
        let (dir, db) = db_with(
            "INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin)
             VALUES ('s1', 'upload_version', 'f1', '<dir>/a', 'w1', 'minted');",
        );
        stage(dir.path(), &["a"]);
        let waiting = WaitingChanges::read(&db).unwrap();
        let good = SendConditions {
            engine_running: true,
            vault_unlocked: true,
            session_expired: false,
            link_failed: false,
            sync_paused: false,
        };
        let cases = [
            (
                SendConditions {
                    engine_running: false,
                    ..good
                },
                Some(NotSending::EngineStopped),
            ),
            (
                SendConditions {
                    vault_unlocked: false,
                    ..good
                },
                Some(NotSending::EngineStopped),
            ),
            (
                SendConditions {
                    session_expired: true,
                    ..good
                },
                Some(NotSending::Session),
            ),
            (
                SendConditions {
                    link_failed: true,
                    ..good
                },
                Some(NotSending::Offline),
            ),
            (
                SendConditions {
                    sync_paused: true,
                    ..good
                },
                Some(NotSending::Paused),
            ),
            (good, None),
            // Two conditions fail: the earlier one is named.
            (
                SendConditions {
                    engine_running: false,
                    session_expired: true,
                    ..good
                },
                Some(NotSending::EngineStopped),
            ),
            (
                SendConditions {
                    session_expired: true,
                    link_failed: true,
                    ..good
                },
                Some(NotSending::Session),
            ),
            (
                SendConditions {
                    link_failed: true,
                    sync_paused: true,
                    ..good
                },
                Some(NotSending::Offline),
            ),
        ];
        for (conditions, expected) in cases {
            assert_eq!(send_readiness(conditions, &waiting), expected, "{conditions:?}");
        }
        // Nothing live: the one write waits for sign-in. Paused is named before held.
        let (held_dir, held_db) = db_with(
            "INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin, paused_reason)
             VALUES ('s1', 'upload_version', 'f1', '<dir>/a', 'w1', 'minted', 'auth');",
        );
        stage(held_dir.path(), &["a"]);
        let held = WaitingChanges::read(&held_db).unwrap();
        assert_eq!(send_readiness(good, &held), Some(NotSending::Held), "nothing live");
        assert_eq!(
            send_readiness(
                SendConditions {
                    sync_paused: true,
                    ..good
                },
                &held
            ),
            Some(NotSending::Paused),
            "paused before held"
        );
        assert_eq!(
            send_readiness(good, &WaitingChanges::default()),
            None,
            "nothing waits: nothing to send, no reason"
        );
    }

    /// U15, the count half (the keep step's tests hold the copy half).
    #[test]
    fn completed_and_released_payloads_are_never_sources() {
        let (dir, db) = db_with(&format!(
            "INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin)
             VALUES ('d1', 'upload_version', 'f3', '<dir>/d1', 'w4', 'minted');
             {COMPLETED}
             INSERT INTO staged_payloads (path, completed) VALUES ('<dir>/released', 1), ('<dir>/orphan', 0);"
        ));
        stage(dir.path(), &["d1", "released", "orphan"]);
        let waiting = WaitingChanges::read(&db).unwrap();
        assert!(waiting.is_empty(), "{waiting:?}");
    }

    /// U38, the count half (the keep step's tests hold the keep half).
    #[test]
    fn copied_files_counts_files_with_a_copy() {
        let (dir, db) = db_with(
            "INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin) VALUES
               ('x-old', 'upload_version', 'x', '<dir>/x-old', 'w1', 'earlier_build'),
               ('x-new', 'upload_version', 'x', '<dir>/x-new', 'w2', 'minted'),
               ('y', 'upload_version', 'y', '<dir>/y', 'w3', 'minted');",
        );
        stage(dir.path(), &["x-new"]);
        let counts = WaitingChanges::read(&db).unwrap().counts();
        assert_eq!((counts.files, counts.missing_files, counts.copied_files), (2, 2, 1));
    }

    /// §2 and §7.2: each write is exactly one of copied, covered, missing.
    #[test]
    fn classification_marks_copied_covered_and_missing() {
        use WriteState::*;
        /// A case's name, each write's origin and whether its staged copy exists (oldest first), the states expected.
        type Case = (&'static str, &'static [(&'static str, bool)], &'static [WriteState]);
        let cases: [Case; 6] = [
            (
                "two minted saves",
                &[("minted", true), ("minted", true)],
                &[Covered, Copied],
            ),
            (
                "earlier build, then minted",
                &[("earlier_build", true), ("minted", true)],
                &[Copied, Copied],
            ),
            (
                "newest copy gone",
                &[("minted", true), ("minted", false)],
                &[Copied, Missing],
            ),
            (
                "earlier build gone",
                &[("earlier_build", false), ("minted", true)],
                &[Missing, Copied],
            ),
            (
                "older minted gone",
                &[("minted", false), ("minted", true)],
                &[Covered, Copied],
            ),
            // `WriteOrigin::from_db` reads an origin it does not know as `None`, as it reads a NULL.
            (
                "an unrecognised origin, then minted",
                &[("bogus", true), ("minted", true)],
                &[Copied, Copied],
            ),
        ];
        for (name, writes, expected) in cases {
            let mut sql = String::from(
                "INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin) VALUES ",
            );
            let rows: Vec<String> = writes
                .iter()
                .enumerate()
                .map(|(i, (origin, _))| format!("('op{i}', 'upload_version', 'f1', '<dir>/op{i}', 'w{i}', '{origin}')"))
                .collect();
            sql.push_str(&rows.join(", "));
            sql.push(';');
            let (dir, db) = db_with(&sql);
            for (i, (_, present)) in writes.iter().enumerate() {
                if *present {
                    stage(dir.path(), &[&format!("op{i}")]);
                }
            }
            let waiting = WaitingChanges::read(&db).unwrap();
            let states: Vec<WriteState> = waiting.files[0].writes.iter().map(|w| w.state).collect();
            assert_eq!(states, expected, "{name}");
        }
    }
}
