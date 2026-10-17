//! Spec 2026-10-10 W6-W9 and §5.2 step 7 (task 1887, macOS): after the confirmed engine stop and before the purge,
//! copy every waiting save that can be kept into a new kept folder, flush it, and record it.

use std::path::Path;

use crate::config::ConfigFs;
use crate::state_db::StateDb;

use super::copy::{self, E1_UNSENT_CHANGES, E2_UNSENT_COPY_FAILED, E3_UNSENT_COUNT_FAILED};
use super::kept_names::{KeptRole, kept_relative_path};
use super::kept_record::{save_kept_unsent_durably, saved_kept_unsent_at, take_back_kept_unsent_at};
pub(crate) use super::kept_set::{Acknowledge, KeptSet};
pub(crate) use super::kept_writer::CloneFile;
use super::kept_writer::KeptWriter;
use super::waiting::{WaitingChanges, WriteState};

pub(crate) struct KeepInputs<'a> {
    pub db: &'a StateDb,
    pub acknowledge: Acknowledge<'a>,
    pub root: &'a Path,
    pub event_name: String,
    pub config_path: &'a Path,
    pub clone: &'a dyn CloneFile,
    pub config_fs: &'a dyn ConfigFs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KeepFailure {
    /// An other change the person was not asked about (W6): the ask is shown again.
    Unsent,
    /// E2, with a category for the log (never a path).
    Copy(&'static str),
    /// The queue could not be read (E3).
    Read(String),
}

impl KeepFailure {
    pub(crate) fn message(&self) -> String {
        match self {
            KeepFailure::Unsent => copy::coded(copy::UNSENT_CHANGES_CODE, E1_UNSENT_CHANGES),
            KeepFailure::Copy(_) => copy::coded(copy::UNSENT_COPY_FAILED_CODE, E2_UNSENT_COPY_FAILED),
            KeepFailure::Read(_) => copy::coded(copy::UNSENT_COUNT_FAILED_CODE, E3_UNSENT_COUNT_FAILED),
        }
    }
}

fn category(error: &std::io::Error) -> &'static str {
    match error.kind() {
        std::io::ErrorKind::NotFound => "source_gone",
        std::io::ErrorKind::PermissionDenied => "permission",
        std::io::ErrorKind::StorageFull => "disk_full",
        _ => "io",
    }
}

/// The keep step (spec 2026-10-10 §5.2 step 7): close Finder accepts, then [`keep_after_close`]. A keep that stops
/// reopens them at once (§15 question 3): nothing will be purged.
pub(crate) fn keep_unsent(inputs: KeepInputs<'_>) -> Result<KeptSet, KeepFailure> {
    // Spec 2026-10-10 §15 question 3: a Finder accept can outlive the engine stop (`spawn_blocking`). Closing accepts
    // in one IMMEDIATE transaction lets one already running commit first, so the read below sees it, and refuses every
    // later one.
    inputs
        .db
        .close_finder_accepts()
        .map_err(|_| KeepFailure::Copy("close_accepts"))?;
    let kept = keep_after_close(&inputs);
    // ...and a keep that stops reopens them at once: nothing will be purged.
    if kept.is_err()
        && let Err(error) = inputs.db.reopen_finder_accepts()
    {
        tracing::warn!(%error, "could not reopen Finder accepts; the next engine start reopens them");
    }
    kept
}

/// W6: one read; refuse a late other change before any copy; refuse a missing write nobody was shown (W7); copy the
/// copied set; flush; record (W9, W13) as the last act. A failure before the record removes the half-made folder
/// (W8). Never deletes a queue row or a staged copy.
fn keep_after_close(inputs: &KeepInputs<'_>) -> Result<KeptSet, KeepFailure> {
    let waiting = WaitingChanges::read(inputs.db).map_err(KeepFailure::Read)?;
    let mut kept = KeptSet::default();
    for other in &waiting.others {
        if !inputs.acknowledge.other(&other.op_id) {
            return Err(KeepFailure::Unsent);
        }
        kept.others_acknowledged.insert(other.op_id.clone());
    }
    for write in waiting.files.iter().flat_map(|file| &file.writes) {
        match write.state {
            WriteState::Missing if !inputs.acknowledge.missing(&write.op_id) => {
                return Err(KeepFailure::Copy("missing_not_shown"));
            }
            WriteState::Missing => {
                kept.missing_acknowledged.insert(write.op_id.clone());
            }
            WriteState::Covered => {
                kept.covered.insert(write.op_id.clone());
            }
            WriteState::Copied => {}
        }
    }
    let to_copy: Vec<_> = waiting.files.iter().filter(|file| file.has_copy()).collect();
    if to_copy.is_empty() {
        log_kept(&kept);
        return Ok(kept);
    }
    let mut writer = KeptWriter::create(inputs.root, &inputs.event_name, inputs.clone)
        .map_err(|error| KeepFailure::Copy(category(&error)))?;
    for file in &to_copy {
        // Newest first: the newest copied write gets the plain name, then "(earlier save 1)", "(earlier save 2)"…
        let copied: Vec<_> = file
            .writes
            .iter()
            .rev()
            .filter(|write| write.state == WriteState::Copied)
            .collect();
        for (index, write) in copied.iter().enumerate() {
            let role = if index == 0 {
                KeptRole::Newest
            } else {
                KeptRole::EarlierSave(index as u32)
            };
            let components = kept_relative_path(file.path.as_deref(), &write.op_id, role);
            let source = write.payload_path.as_deref().expect("a copied write has a staged copy");
            if let Err(error) = writer.keep(&components, source) {
                writer.abandon();
                return Err(KeepFailure::Copy(category(&error)));
            }
            kept.copied.insert(write.op_id.clone());
        }
    }
    let event = writer.event().to_path_buf();
    let folder = match writer.finish() {
        Ok(folder) => folder,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&event);
            return Err(KeepFailure::Copy(category(&error)));
        }
    };
    let folder_text = folder.path.to_string_lossy().into_owned();
    let previous = saved_kept_unsent_at(inputs.config_path).ok().flatten();
    if save_kept_unsent_durably(inputs.config_path, &folder_text, inputs.config_fs).is_err() {
        // Best effort: no key may name a folder that is about to be removed (§7.3).
        let _ = take_back_kept_unsent_at(inputs.config_path, &folder_text, previous.as_deref());
        let _ = std::fs::remove_dir_all(&folder.path);
        return Err(KeepFailure::Copy("record"));
    }
    kept.folder = Some(folder.path);
    kept.files = folder.files;
    log_kept(&kept);
    Ok(kept)
}

/// §7.2: counts only, never a path or a name (W10).
fn log_kept(kept: &KeptSet) {
    tracing::info!(
        files = kept.files,
        kept = kept.copied.len(),
        covered = kept.covered.len(),
        missing = kept.missing_acknowledged.len(),
        "kept unsent changes"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RealConfigFs;
    use crate::sign_out::kept_writer::SystemClone;
    use crate::state_db::StateDb;
    use sha2::{Digest, Sha256};
    use std::cell::Cell;
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    struct Fixture {
        dir: tempfile::TempDir,
        /// `dir`, resolved: the writer resolves its root, so a folder it reports is compared with paths built from
        /// here.
        base: PathBuf,
        db: StateDb,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let base = std::fs::canonicalize(dir.path()).unwrap();
            let db = StateDb::open(dir.path().join("state.db")).unwrap();
            Self { dir, base, db }
        }
        fn root(&self) -> PathBuf {
            self.base.join("Documents").join("Changes not sent")
        }
        fn config(&self) -> PathBuf {
            self.dir.path().join("desktop.toml")
        }
        fn file(&self, file_id: &str, path: &str) {
            self.db
                .hold_lock_for_test()
                .execute(
                    "INSERT INTO files (file_id, path) VALUES (?1, ?2)",
                    rusqlite::params![file_id, path],
                )
                .unwrap();
        }
        /// A waiting upload whose staged copy holds `bytes` (`None`: the copy is gone).
        fn write(&self, op_id: &str, file_id: &str, origin: &str, bytes: Option<&[u8]>) -> PathBuf {
            let staged = self.dir.path().join("staging").join(op_id);
            std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
            if let Some(bytes) = bytes {
                std::fs::write(&staged, bytes).unwrap();
            }
            self.db
                .hold_lock_for_test()
                .execute(
                    "INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin)
                     VALUES (?1, 'upload_version', ?2, ?3, ?1, ?4)",
                    rusqlite::params![op_id, file_id, staged.to_str().unwrap(), origin],
                )
                .unwrap();
            staged
        }
        fn other(&self, op_id: &str, kind: &str, file_id: &str) {
            self.db
                .hold_lock_for_test()
                .execute(
                    "INSERT INTO operation_queue (op_id, kind, file_id) VALUES (?1, ?2, ?3)",
                    rusqlite::params![op_id, kind, file_id],
                )
                .unwrap();
        }
        fn keep_with(
            &self,
            acknowledge: Acknowledge<'_>,
            clone: &dyn CloneFile,
            fs: &dyn ConfigFs,
        ) -> Result<KeptSet, KeepFailure> {
            let root = self.root();
            let config = self.config();
            keep_unsent(KeepInputs {
                db: &self.db,
                acknowledge,
                root: &root,
                event_name: "2026-10-10 at 14.05.12".into(),
                config_path: &config,
                clone,
                config_fs: fs,
            })
        }
        fn keep(&self, acknowledge: Acknowledge<'_>) -> Result<KeptSet, KeepFailure> {
            self.keep_with(acknowledge, &SystemClone, &RealConfigFs)
        }
        fn event(&self) -> PathBuf {
            self.root().join("2026-10-10 at 14.05.12")
        }
        fn queued(&self) -> usize {
            self.db.waiting_rows().unwrap().len()
        }
    }

    fn sha(bytes: &[u8]) -> Vec<u8> {
        Sha256::digest(bytes).to_vec()
    }

    fn sha_of(path: &Path) -> Vec<u8> {
        sha(&std::fs::read(path).unwrap())
    }

    fn set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    struct FailingClone {
        calls: Cell<usize>,
        fail_on: usize,
    }

    impl CloneFile for FailingClone {
        fn clone_file(&self, src: &Path, dst: &Path) -> std::io::Result<()> {
            self.calls.set(self.calls.get() + 1);
            if self.calls.get() == self.fail_on {
                return Err(std::io::Error::other("injected"));
            }
            SystemClone.clone_file(src, dst)
        }
    }

    /// U8.
    #[test]
    fn sign_out_keeps_the_kept_set_before_the_purge() {
        let f = Fixture::new();
        f.file("report", "Work/report.docx");
        f.file("notes", "New/notes.txt");
        f.file("a", "Old/a.txt");
        f.write("s1", "report", "minted", Some(b"first save"));
        f.write("s2", "report", "minted", Some(b"second save"));
        f.write("c1", "notes", "minted", Some(b"notes"));
        f.write("a-old", "a", "earlier_build", Some(b"earlier build"));
        f.write("a-new", "a", "minted", Some(b"later save"));
        let kept = f.keep(Acknowledge::Nothing).unwrap();
        let event = f.event();
        assert_eq!(sha_of(&event.join("Work/report.docx")), sha(b"second save"));
        assert_eq!(sha_of(&event.join("New/notes.txt")), sha(b"notes"));
        assert_eq!(sha_of(&event.join("Old/a.txt")), sha(b"later save"));
        assert_eq!(sha_of(&event.join("Old/a (earlier save 1).txt")), sha(b"earlier build"));
        assert!(
            !event.join("Work/report (earlier save 1).docx").exists(),
            "a minted earlier save is covered, never copied"
        );
        assert_eq!(kept.copied, set(&["s2", "c1", "a-old", "a-new"]));
        assert_eq!(kept.covered, set(&["s1"]));
        assert_eq!((kept.folder.as_deref(), kept.files), (Some(event.as_path()), 4));
    }

    /// U8b.
    #[test]
    fn two_minted_saves_of_one_file_keep_one_file() {
        let f = Fixture::new();
        f.file("report", "Work/report.docx");
        f.write("s1", "report", "minted", Some(b"older"));
        f.write("s2", "report", "minted", Some(b"newer"));
        let kept = f.keep(Acknowledge::Nothing).unwrap();
        assert_eq!(std::fs::read_dir(f.event().join("Work")).unwrap().count(), 1);
        assert_eq!(sha_of(&f.event().join("Work/report.docx")), sha(b"newer"));
        assert!(kept.keeps("s1") && kept.covered.contains("s1"));
    }

    /// U13, the keep step's half (keys in memory and the restart are the sign-out's).
    #[test]
    fn a_failed_copy_stops_the_sign_out() {
        let f = Fixture::new();
        f.file("a", "a.txt");
        f.file("b", "b.txt");
        let a = f.write("wa", "a", "minted", Some(b"a"));
        let b = f.write("wb", "b", "minted", Some(b"b"));
        let failing = FailingClone {
            calls: Cell::new(0),
            fail_on: 2,
        };
        let failure = f.keep_with(Acknowledge::Nothing, &failing, &RealConfigFs).unwrap_err();
        assert!(
            failure.message().starts_with("unsent_copy_failed: "),
            "{}",
            failure.message()
        );
        assert!(!f.event().exists(), "the half-made folder is removed");
        assert_eq!(
            crate::sign_out::kept_record::saved_kept_unsent_at(&f.config()).unwrap(),
            None,
            "no record"
        );
        assert_eq!(f.queued(), 2, "the queue is intact");
        assert!(a.exists() && b.exists(), "the staged copies are intact");
    }

    /// U27, the keep step's half (removing both sources is the purge's).
    #[test]
    fn the_keep_step_copies_from_either_staging_root() {
        let f = Fixture::new();
        f.file("x", "x.txt");
        f.file("y", "y.txt");
        let caches = f.dir.path().join("Library/Caches/beebeeb/finder-writes/old-copy");
        let support = f
            .dir
            .path()
            .join("Library/Application Support/beebeeb/finder-writes/new-copy");
        for (path, bytes) in [(&caches, "from caches"), (&support, "from support")] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        f.db.hold_lock_for_test()
            .execute_batch(&format!(
                "INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin) VALUES
                   ('ox', 'upload_version', 'x', '{}', 'ox', 'minted'),
                   ('oy', 'upload_version', 'y', '{}', 'oy', 'minted');",
                caches.display(),
                support.display()
            ))
            .unwrap();
        f.keep(Acknowledge::Nothing).unwrap();
        assert_eq!(sha_of(&f.event().join("x.txt")), sha(b"from caches"));
        assert_eq!(sha_of(&f.event().join("y.txt")), sha(b"from support"));
    }

    /// U29 cases a and c.
    #[test]
    fn a_copy_missing_since_the_ask_stops_the_sign_out() {
        let f = Fixture::new();
        f.file("a", "a.txt");
        f.file("b", "b.txt");
        let a = f.write("wa", "a", "minted", Some(b"a"));
        f.write("wb", "b", "minted", Some(b"b"));
        std::fs::remove_file(&a).unwrap(); // gone after an ask that showed no missing write
        let (others, missing) = (set(&[]), set(&[]));
        let failure = f
            .keep(Acknowledge::Asked {
                others: &others,
                missing: &missing,
            })
            .unwrap_err();
        assert!(failure.message().starts_with("unsent_copy_failed: "));
        assert!(!f.event().exists(), "nothing copied");
        assert_eq!(f.queued(), 2, "nothing purged");
        // Case c: the ask showed it missing.
        let missing = set(&["wa"]);
        let kept = f
            .keep(Acknowledge::Asked {
                others: &others,
                missing: &missing,
            })
            .unwrap();
        assert!(kept.missing_acknowledged.contains("wa"));
        assert_eq!(sha_of(&f.event().join("b.txt")), sha(b"b"), "the other writes are kept");
    }

    /// U36, the keep half.
    #[test]
    fn a_late_other_change_stops_the_sign_out_before_any_copy() {
        let f = Fixture::new();
        f.file("a", "a.txt");
        f.write("wa", "a", "minted", Some(b"a"));
        f.other("r1", "rename_file", "a");
        f.other("r2", "rename_file", "a");
        let (others, missing) = (set(&["r1"]), set(&[]));
        let failure = f
            .keep(Acknowledge::Asked {
                others: &others,
                missing: &missing,
            })
            .unwrap_err();
        assert!(matches!(failure, KeepFailure::Unsent));
        assert!(failure.message().starts_with("unsent_changes: "));
        assert!(!f.root().exists(), "refused before any directory was made");
        // A write that arrived after the gate, with its copy, is kept and counted (U9's case, with a snapshot).
        let f = Fixture::new();
        f.file("a", "a.txt");
        f.file("late", "late.txt");
        f.write("wa", "a", "minted", Some(b"a"));
        f.write("wl", "late", "minted", Some(b"late"));
        let (others, missing) = (set(&[]), set(&[]));
        let kept = f
            .keep(Acknowledge::Asked {
                others: &others,
                missing: &missing,
            })
            .unwrap();
        assert_eq!(kept.files, 2);
    }

    /// U38, the keep half; U15, the copy half.
    #[test]
    fn only_files_with_a_copy_are_copied_and_completed_uploads_never() {
        let f = Fixture::new();
        f.file("x", "x.txt");
        f.file("y", "y.txt");
        f.write("x-old", "x", "earlier_build", None);
        f.write("x-new", "x", "minted", Some(b"x"));
        f.write("y", "y", "minted", None);
        f.write("done", "z", "minted", Some(b"done"));
        f.db.hold_lock_for_test()
            .execute_batch(
                "INSERT INTO upload_resume (op_id, payload_path, payload_size, payload_mtime_ns, upload_session_id,
                   server_file_id, object_version_id, chunk_size_bytes, chunk_count, completed_version)
                 VALUES ('done', 'p', 1, 1, 'u', 'z', 'v', 1, 1, 3);",
            )
            .unwrap();
        let (others, missing) = (set(&[]), set(&["x-old", "y"]));
        let kept = f
            .keep(Acknowledge::Asked {
                others: &others,
                missing: &missing,
            })
            .unwrap();
        assert_eq!(kept.files, 1, "X is copied, Y is not, the completed upload never");
        assert!(!kept.keeps("done"));
    }

    /// A 0-byte save is kept as an empty file, never counted as missing.
    #[test]
    fn an_empty_save_is_kept_as_an_empty_file() {
        let f = Fixture::new();
        f.file("e", "empty.txt");
        f.write("we", "e", "minted", Some(b""));
        let kept = f.keep(Acknowledge::Nothing).unwrap();
        assert!(kept.copied.contains("we"));
        assert_eq!(std::fs::metadata(f.event().join("empty.txt")).unwrap().len(), 0);
    }

    /// A save and a rename of one file, both waiting: the name is the row's path at the keep step, never an op's
    /// `target_path`.
    #[test]
    fn a_waiting_rename_names_the_kept_copy_as_finder_shows_it() {
        let f = Fixture::new();
        f.file("r", "Work/new name.docx");
        f.write("wr", "r", "minted", Some(b"content"));
        f.db.hold_lock_for_test()
            .execute(
                "INSERT INTO operation_queue (op_id, kind, file_id, target_path) VALUES ('rn', 'rename_file', 'r', 'old name.docx')",
                [],
            )
            .unwrap();
        let (others, missing) = (set(&["rn"]), set(&[]));
        f.keep(Acknowledge::Asked {
            others: &others,
            missing: &missing,
        })
        .unwrap();
        assert_eq!(sha_of(&f.event().join("Work/new name.docx")), sha(b"content"));
    }

    /// A bulk drop: 500 new files waiting in one folder count as 500 files, and all 500 are kept under their own names.
    #[test]
    fn five_hundred_waiting_files_are_counted_and_kept_once_each() {
        let f = Fixture::new();
        for i in 0..500 {
            f.file(&format!("f{i}"), &format!("Drop/file {i:03}.bin"));
            f.write(
                &format!("w{i}"),
                &format!("f{i}"),
                "minted",
                Some(format!("bytes {i}").as_bytes()),
            );
        }
        assert_eq!(
            crate::sign_out::waiting::WaitingChanges::read(&f.db)
                .unwrap()
                .counts()
                .files,
            500
        );
        let kept = f.keep(Acknowledge::Nothing).unwrap();
        assert_eq!(kept.files, 500);
        for i in [0, 250, 499] {
            assert_eq!(
                sha_of(&f.event().join(format!("Drop/file {i:03}.bin"))),
                sha(format!("bytes {i}").as_bytes())
            );
        }
    }

    /// U43, the keep step's half: a save that fails after its rename names nothing, and the folder goes.
    #[test]
    fn a_failed_marker_save_removes_the_folder_and_names_nothing() {
        struct FailDirFlush;
        impl ConfigFs for FailDirFlush {
            fn write(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
                RealConfigFs.write(path, bytes)
            }
            fn sync_file(&self, path: &Path) -> std::io::Result<()> {
                RealConfigFs.sync_file(path)
            }
            fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
                RealConfigFs.rename(from, to)
            }
            fn sync_dir(&self, _dir: &Path) -> std::io::Result<()> {
                Err(std::io::Error::other("injected"))
            }
        }
        let f = Fixture::new();
        f.file("a", "a.txt");
        f.write("wa", "a", "minted", Some(b"a"));
        let failure = f
            .keep_with(Acknowledge::Nothing, &SystemClone, &FailDirFlush)
            .unwrap_err();
        assert!(failure.message().starts_with("unsent_copy_failed: "));
        assert!(!f.event().exists());
        let cfg = crate::config::DesktopConfig::load_from(&f.config()).unwrap();
        assert_eq!(
            (cfg.kept_unsent_folder, cfg.kept_unsent_unfinished),
            (None, Vec::<String>::new())
        );
    }

    /// W12: the R10 reset acknowledges everything it cannot ask about.
    #[test]
    fn the_reset_acknowledges_missing_writes_and_other_changes() {
        let f = Fixture::new();
        f.file("a", "a.txt");
        f.write("gone", "a", "minted", None);
        f.other("r1", "rename_file", "a");
        let kept = f.keep(Acknowledge::Everything).unwrap();
        assert!(kept.missing_acknowledged.contains("gone") && kept.others_acknowledged.contains("r1"));
        assert_eq!(kept.folder, None, "no copy, no folder");
    }

    /// U12's last clause (spec §7.1): neither engine-start step touches what a keep step kept. Both run on the same
    /// database right after a real keep, as the next engine start would, over a release-shaped staging layout.
    #[test]
    fn the_engine_start_steps_never_touch_a_kept_file() {
        let f = Fixture::new();
        f.file("report", "Work/report.docx");
        f.write("w1", "report", "minted", Some(b"kept"));
        let kept = f.keep(Acknowledge::Nothing).unwrap();
        let staging = crate::engine_bridge::FinderStagingBases {
            data: Some(f.base.join("Library/Application Support")),
            cache: Some(f.base.join("Library/Caches")),
            temp: f.base.join("tmp"),
        };
        for folder in staging.candidates() {
            std::fs::create_dir_all(&folder).unwrap();
        }
        crate::staged_payload::sweep_unreferenced(&f.db, &staging.candidates()).unwrap();
        let repair = f.db.engine_start_repair().unwrap();
        assert!(repair.released_payloads.is_empty(), "{:?}", repair.released_payloads);
        crate::staged_payload::remove_released(&f.db, &repair.released_payloads);
        assert_eq!(
            std::fs::read(kept.folder.unwrap().join("Work/report.docx")).unwrap(),
            b"kept"
        );
    }

    /// U45: closing accepts is the keep step's first act, before its read; a keep that stops reopens them at once
    /// (spec §15 question 3), and one that completes leaves them closed for the purge, which clears them with the rows.
    #[test]
    fn the_keep_step_closes_accepts_first_and_reopens_them_when_it_stops() {
        let f = Fixture::new();
        f.other("r1", "rename_file", "a");
        assert!(matches!(f.keep(Acknowledge::Nothing), Err(KeepFailure::Unsent)));
        assert!(!f.db.finder_accepts_closed().unwrap(), "a refused keep reopens accepts");
        let (others, missing) = (set(&["r1"]), set(&[]));
        f.keep(Acknowledge::Asked {
            others: &others,
            missing: &missing,
        })
        .unwrap();
        assert!(
            f.db.finder_accepts_closed().unwrap(),
            "a completed keep leaves them closed for the purge"
        );
        // `keep_unsent`'s own body, never the rest of the file, whose tests name both calls too.
        let source = include_str!("keep.rs");
        let start = source.find("pub(crate) fn keep_unsent(").unwrap();
        let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
        let close = body.find("close_finder_accepts()").expect("the close");
        let read = body
            .find("keep_after_close(")
            .expect("the read, inside keep_after_close");
        assert!(close < read, "accepts are closed before the read");
        assert!(
            !body.contains("WaitingChanges::read("),
            "the read is keep_after_close's, after the close"
        );
    }

    /// U45: a close that fails stops the keep step before its read and before any copy (E2, `close_accepts`), so no
    /// keep runs while Finder accepts are open.
    #[test]
    fn a_close_that_fails_stops_the_keep_before_any_copy() {
        let f = Fixture::new();
        f.file("report", "report.docx");
        f.write("s1", "report", "minted", Some(b"first save"));
        f.db.hold_lock_for_test()
            .execute_batch(
                "CREATE TRIGGER no_close BEFORE INSERT ON sync_state WHEN NEW.key = 'finder_accepts_closed'
                 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            )
            .unwrap();
        assert_eq!(
            f.keep(Acknowledge::Nothing).err(),
            Some(KeepFailure::Copy("close_accepts")),
            "a failed close stops the keep (E2)"
        );
        assert!(!f.event().exists(), "nothing was copied");
        assert_eq!(f.queued(), 1, "the waiting write is still queued");
    }
}
