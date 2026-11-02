//! SQLite-backed per-file sync state.
//!
//! Tracks the local mirror's view of every file in the vault: cloud-only
//! placeholder, downloading, fully local, uploading, conflicting, or in
//! some error state. The OS extensions (File Provider on macOS, Cloud
//! Files on Windows, FUSE on Linux) read this table to render the right
//! Finder/Explorer overlay icon; the daemon writes it as it makes
//! progress.
//!
//! Spec: `docs/superpowers/plans/2026-05-07-desktop-sync-client.md`
//! Phase 1 Task 1.
//!
//! ## Schema
//!
//! Single `files` table keyed by `file_id` (the server's UUID for the
//! file). `path` is the relative path inside the sync root. `status`
//! drives the overlay icon; the rest are bookkeeping.
//!
//! WAL journal mode so reads from the OS extension don't block the
//! daemon's writes.

use crate::diagnostic_redaction::{
    DiagnosticErrorCode, KnownNames, PAUSE_REASON_LABELS, QUEUE_KIND_LABELS, allowed_label, classify_error_code,
    redact_for_export, redact_secrets_only,
};
use rusqlite::{Connection, OptionalExtension, Result, params};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

#[cfg(test)]
#[path = "reconcile_measure_tests.rs"]
mod reconcile_measure_tests;

#[cfg(test)]
thread_local! {
    /// How many of this thread's next landings fail, and whether as "database is locked".
    /// A thread-local, so parallel tests never share it: under `#[tokio::test]` the landing
    /// runs on the test's own thread.
    static FAIL_LANDINGS: std::cell::Cell<(u32, bool)> = const { std::cell::Cell::new((0, false)) };
}

/// Task 2267 (fix round 2): one `files` row as the ownership proof judges it.
#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofRow {
    pub file_id: String,
    pub parent_id: Option<String>,
    /// Its path under the sync root ('/'-separated). 128 of the 2,979 nested rows made on the maintainer's PC have no
    /// parent link, so a walk up falls back to the row at the path's folder.
    pub path: String,
    /// In the person's own files (`namespace = 'my_files'`), not a folder shared with them.
    pub my_files: bool,
    /// Judged against the server's tree: not made on this PC. A row made here has `current_version` and
    /// `remote_updated_at` both at 0 and a queued create names it (lead review of round 2, P2-1).
    pub server_named: bool,
}

/// Task 2267 (fix round 2): one queued operation, by what it acts on.
#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofOp {
    pub op_id: String,
    pub file_id: Option<String>,
    pub parent_id: Option<String>,
    /// `operation_queue.kind`, as stored.
    pub kind: String,
}

/// Task 2267 (lead review round 3, P2-2): one change held as `found_while_paused`, as the card counts it.
#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldChange {
    pub op_id: String,
    /// `operation_queue.kind`, as stored.
    pub kind: String,
    /// It creates its item (`CREATE_OP_SQL`).
    pub creates: bool,
    /// Its own row or its folder is in a folder shared with this account (`namespace != 'my_files'`).
    pub shared: bool,
}

/// T61, P9: the next `n` landings on this thread fail with a database error.
#[cfg(test)]
pub(crate) fn fail_next_landings_for_test(n: u32) {
    FAIL_LANDINGS.with(|left| left.set((n, false)));
}

/// m-11's pause filter: the next `n` landings on this thread fail as SQLite's "database is
/// locked", which `classify_operation_error` reads as `Locked`, a pause.
#[cfg(test)]
pub(crate) fn fail_next_landings_as_locked_for_test(n: u32) {
    FAIL_LANDINGS.with(|left| left.set((n, true)));
}

/// Owned handle to the local SQLite state database.
///
/// One instance per running daemon process. The inner `Connection` is
/// wrapped in a `Mutex` so `StateDb: Send + Sync` and an `Arc<StateDb>`
/// can be shared with `tokio::spawn`-ed futures (rusqlite's
/// `Connection` is `Send` but `!Sync` on its own). Lock contention is
/// fine for our access pattern: the daemon does one tick every 5
/// seconds and OS-extension callbacks are infrequent.
pub struct StateDb(Mutex<Connection>, std::sync::atomic::AtomicU64);

#[cfg(test)]
mod tests {
    mod open_and_cache;

    /// Task 2240: file rows in a fresh state.db, enough to fill several times SQLite's default page cache (about 300
    /// bytes each). The scan's lookups find them as `dir/<i>.txt`.
    const TASK_2240_ROWS: usize = 20_000;

    fn task_2240_add_rows(path: &Path, from: usize, rows: usize, padding: usize) {
        let mut conn = Connection::open(path).unwrap();
        let tx = conn.transaction().unwrap();
        let content_type = "x".repeat(padding);
        for i in from..from + rows {
            tx.execute(
                "INSERT INTO files (file_id, path, content_type) VALUES (?1, ?2, ?3)",
                params![format!("f{i}"), format!("dir/{i}.txt"), content_type],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }

    fn task_2240_db() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        StateDb::open(&path).unwrap();
        task_2240_add_rows(&path, 0, TASK_2240_ROWS, 200);
        (dir, path)
    }

    /// One pass of the scan's lookups when every file is settled: the row stamp of every file, as
    /// `run_one_scan_with` reads it. Returns the pages the pass read from the file.
    fn task_2240_pass(db: &StateDb) -> i64 {
        let before = db.page_reads();
        for i in 0..TASK_2240_ROWS {
            assert!(db.file_row_stamp_by_path(&format!("dir/{i}.txt")).unwrap().is_some());
        }
        db.page_reads() - before
    }

    fn task_2240_cache_size(db: &StateDb) -> i64 {
        db.0.lock()
            .unwrap()
            .query_row("PRAGMA cache_size", [], |row| row.get(0))
            .unwrap()
    }

    /// Task 2240: runs `measure` in a fresh copy of this test binary that runs only the test `name`.
    ///
    /// The bundled SQLite is built with `SQLITE_ENABLE_MEMORY_MANAGEMENT` (libsqlite3-sys), so all the connections of
    /// a process share one page-cache LRU: a connection that reads more than its own cache takes the oldest pages of
    /// the others. A test that counts page reads in this binary would count the reads of every test running beside
    /// it. The parent passes only when the child ran exactly one test, passed, and printed what it measured.
    fn task_2240_in_own_process(name: &str, measure: impl FnOnce() -> String) {
        const CHILD: &str = "BB_TASK_2240_PAGE_CACHE_CHILD";
        if std::env::var_os(CHILD).is_some() {
            println!("task 2240 child {name}: {}", measure());
            return;
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("state_db::tests::{name}"),
                "--test-threads=1",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success(),
            "the child run of {name} failed:\n{stdout}\n{stderr}"
        );
        assert!(
            stdout.contains("test result: ok. 1 passed;"),
            "the child ran no test:\n{stdout}"
        );
        let proof = format!("task 2240 child {name}: ");
        let line = stdout.lines().find(|line| line.contains(&proof));
        println!(
            "{}",
            line.unwrap_or_else(|| panic!("the child never measured:\n{stdout}"))
        );
    }

    mod signout_and_activity;

    fn finalization(op: &str) -> UploadFinalization {
        UploadFinalization {
            op_id: op.into(),
            local_file_id: format!("local-{op}"),
            server_file_id: format!("server-{op}"),
            target_path: format!("dir/{op}.txt"),
            payload_path: format!("proof-{op}"),
            stamped: false,
            review: None,
        }
    }

    use super::*;

    /// Task 2190: a `local` file row for the held-edit tests.
    fn held_row(file_id: &str, path: &str) -> FileEntry {
        FileEntry {
            file_id: file_id.to_string(),
            path: path.to_string(),
            status: FileStatus::Local,
            size_bytes: 1,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        }
    }

    use tempfile::tempdir;

    // ── transfer activity + backlog (task 1683 slice 2) ──────────────────────

    fn queued_op(id: &str, kind: OperationKind, file_id: &str) -> PendingOperation {
        PendingOperation {
            op_id: id.into(),
            kind,
            file_id: Some(file_id.into()),
            parent_id: None,
            target_path: None,
            metadata_json: None,
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn transfer(direction: &'static str, name: &str, at: i64) -> TransferActivityInput {
        TransferActivityInput {
            direction,
            file_id: Some(format!("id-{name}")),
            file_name: name.into(),
            rel_path: Some(format!("Work/{name}")),
            bytes: 10,
            occurred_at: at,
        }
    }

    mod files_and_purge;

    fn seed_contract_row(
        db: &StateDb,
        file_id: &str,
        path: &str,
        parent_id: Option<&str>,
        status: FileStatus,
        cache_bytes: i64,
    ) {
        db.upsert_file(&FileEntry {
            file_id: file_id.into(),
            path: path.into(),
            status,
            size_bytes: cache_bytes,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: parent_id.map(str::to_string),
            item_kind: ItemKind::File,
        })
        .unwrap();
        let mut contract = db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.parent_id = parent_id.map(str::to_string);
        contract.cache_bytes = cache_bytes;
        db.set_file_contract_state(&contract).unwrap();
    }

    // ── prune_absent direct unit coverage (issues 4/5/6) ──────────────────────

    /// Insert a settled own-tree row with an explicit `remote_updated_at`.
    fn seed_own_row(db: &StateDb, file_id: &str, status: FileStatus, remote_updated_at: i64) {
        db.upsert_file(&FileEntry {
            file_id: file_id.into(),
            path: format!("{file_id}.txt"),
            status,
            size_bytes: 1,
            modified_at: 0,
            content_hash: None,
            remote_updated_at,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
    }

    const FAR_FUTURE: i64 = 4_000_000_000; // > any realistic remote_updated_at

    mod prune_and_finder;

    fn queued(op_id: &str, kind: OperationKind, file_id: &str, payload: Option<&str>) -> PendingOperation {
        PendingOperation {
            op_id: op_id.into(),
            kind,
            file_id: Some(file_id.into()),
            parent_id: None,
            target_path: None,
            metadata_json: None,
            payload_path: payload.map(str::to_string),
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 25,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    mod landing_and_snapshots;

    // ── orphan-subtree + delete_file_subtree coverage (task 0806) ──────────────

    /// Seed an own-tree row at an explicit `path` with a given kind, so the
    /// path-prefix orphan logic can be exercised (a folder + its descendants).
    fn seed_own_at(db: &StateDb, file_id: &str, path: &str, is_dir: bool) {
        db.upsert_file(&FileEntry {
            file_id: file_id.into(),
            path: path.into(),
            status: FileStatus::CloudOnly,
            size_bytes: 1,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: if is_dir { ItemKind::Folder } else { ItemKind::File },
        })
        .unwrap();
        // upsert_file does NOT persist item_kind (it's owned by the contract row),
        // so a folder must have its kind stamped via the contract state.
        if is_dir {
            db.set_file_contract_state(&FileContractState {
                file_id: file_id.into(),
                namespace: Namespace::MyFiles,
                parent_id: None,
                shared_root_id: None,
                share_id: None,
                owner_email: None,
                permission_bits: 0,
                item_kind: ItemKind::Folder,
                content_type: None,
                current_version: 0,
                current_object_version_id: None,
                local_base_version: 0,
                local_hash: None,
                cache_path: None,
                cache_bytes: 0,
                pin_state: PinState::Inherit,
                inherited_pin_state: PinState::Inherit,
                last_sync_at: 0,
            })
            .unwrap();
        }
    }

    /// A row the server holds at `version`, under `object`.
    fn seed_restorable_row(db: &StateDb, file_id: &str, version: i64, object: &str) {
        db.upsert_file(&FileEntry {
            file_id: file_id.into(),
            path: "notes.txt".into(),
            status: FileStatus::Local,
            size_bytes: 1,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        let mut contract = db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.current_version = version;
        contract.current_object_version_id = Some(object.into());
        db.set_file_contract_state(&contract).unwrap();
    }

    fn version_and_object(db: &StateDb, file_id: &str) -> (i64, Option<String>) {
        let contract = db.get_file_contract_state(file_id).unwrap().unwrap();
        (contract.current_version, contract.current_object_version_id)
    }

    // ── bandwidth_samples (task 0810 — P3) ───────────────────────────────────

    mod change_log_and_account;

    // ── Mbps <-> kbps conversion (task 0810 — E) ─────────────────────────────
    //
    // The desktop UI exposes bandwidth caps as Mbps; config.rs stores them as
    // kbps.  These tests encode the conversion contract so a future refactor
    // can't silently break it.

    // ── task 0828 regression: orphaned children of an absent folder ───────────
    //
    // Scenario: a folder was already trashed on the server when the desktop's
    // snapshot ran, so the snapshot excluded the folder row.  The folder's
    // CHILDREN were ingested by a prior snapshot and stored with
    // `parent_id = <folder_id>` via `set_file_contract_state`.  When the
    // desktop later receives a `file_trash` sync op for the folder,
    // `get_file(folder_id)` returns `None` — the old `None => {}` no-op left
    // the children as permanent ghost rows.
    //
    // `delete_orphaned_children_of_absent_folder` must sweep and remove ALL
    // descendant rows even when the folder row itself is absent.

    // ------------------------------------------------------------------
    // Task 1697: per-domain change log + monotonic anchor (state_db).
    // RED-first: these were written against unmodified code and were seen
    // failing (no fp_changes table, no API) before the implementation.
    // ------------------------------------------------------------------

    use super::FpChangeKind;

    fn change_entry() -> FileEntry {
        FileEntry {
            file_id: "ch-1".into(),
            path: "/docs/report.txt".into(),
            status: FileStatus::Local,
            size_bytes: 10,
            modified_at: 100,
            content_hash: None,
            remote_updated_at: 100,
            parent_id: None,
            item_kind: ItemKind::File,
        }
    }

    fn seed_child(db: &StateDb, file_id: &str, path: &str, kind: ItemKind) {
        seed_child_under(db, file_id, path, kind, None);
    }

    fn seed_child_under(db: &StateDb, file_id: &str, path: &str, kind: ItemKind, parent_id: Option<&str>) {
        db.upsert_file(&FileEntry {
            file_id: file_id.into(),
            path: path.into(),
            status: FileStatus::Local,
            size_bytes: 5,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: kind.clone(),
        })
        .unwrap();
        // parent_id is a contract-owned column (upsert_file never writes it);
        // the contract write is what puts the row in its real parent folder.
        let parent = parent_id.map(str::to_string);
        db.set_file_contract_state(&FileContractState {
            file_id: file_id.into(),
            namespace: Namespace::MyFiles,
            parent_id: parent,
            shared_root_id: None,
            share_id: None,
            owner_email: None,
            permission_bits: 0,
            item_kind: kind.clone(),
            content_type: None,
            current_version: 1,
            current_object_version_id: None,
            local_base_version: 0,
            local_hash: None,
            cache_path: None,
            cache_bytes: 0,
            pin_state: PinState::Inherit,
            inherited_pin_state: PinState::Unpinned,
            last_sync_at: 0,
        })
        .unwrap();
    }

    // ------------------------------------------------------------------
    // Task 1697 review fix (T2): `record_file_change` stamped
    // `recorded_at = 0`, so the macOS signal path's 7-day sweep
    // (`sweep_file_changes(now - 7d)`) deleted the ENTIRE fresh log right
    // after asking File Provider to enumerate it — Finder received an empty
    // feed and missed every update. RED-first: these tests were run against
    // the `recorded_at = 0` insert and were seen failing (0 fresh rows
    // survived the sweep; recorded_at read back 0).
    // ------------------------------------------------------------------

    fn recorded_at_of(db: &StateDb, file_id: &str) -> i64 {
        db.0.lock()
            .expect("state_db mutex poisoned")
            .query_row(
                "SELECT recorded_at FROM fp_changes WHERE file_id = ?1",
                params![file_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
    }

    // ── R10 (spec 2026-10-06 §5.6): local data is bound to the account that created it ──

    // ── Rule 5: the item's status follows the queue (spec §9.1–§9.2) ──────────

    mod startup_and_accepts;

    /// A row the server holds at version 1.
    fn seed_v1_row(db: &StateDb, file_id: &str) {
        seed_own_row(db, file_id, FileStatus::Local, 10);
        let mut contract = db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.current_version = 1;
        db.set_file_contract_state(&contract).unwrap();
    }

    /// One File Provider save of `file_id` on `base`, through the accept transaction.
    fn accept_save(db: &StateDb, op_id: &str, file_id: &str, base: Option<&str>) -> AcceptOutcome {
        let payload_path = format!("/staged/{op_id}");
        db.track_staged_payload(&payload_path, None, false).unwrap();
        db.accept_finder_write(
            &FinderAccept {
                retire_parked: None,
                op_id,
                file_id,
                kind: FinderAcceptKind::Modify { incoming_base: base },
                parent_id: None,
                target_path: Some("a.txt"),
                metadata_json: "{}",
                payload_path: &payload_path,
                size_bytes: 3,
                modified_at: 100,
                backup_source_key: None,
                now: 100,
            },
            &|_, _| Vec::new(),
        )
        .unwrap()
    }

    fn queued_token(outcome: AcceptOutcome) -> String {
        match outcome {
            AcceptOutcome::Queued { token, .. } => token,
            other => panic!("the save is queued: {other:?}"),
        }
    }

    fn status_of(db: &StateDb, file_id: &str) -> FileStatus {
        db.get_file(file_id).unwrap().unwrap().status
    }

    fn changes_of(db: &StateDb, file_id: &str) -> usize {
        db.list_file_changes(None)
            .unwrap()
            .map(|(changes, _)| changes.into_iter().filter(|change| change.file_id == file_id).count())
            .unwrap_or(0)
    }

    fn accept_late_save(db: &StateDb, payload_path: &str) -> AcceptOutcome {
        db.accept_finder_write(
            &FinderAccept {
                retire_parked: None,
                op_id: "op-late",
                file_id: "late",
                kind: FinderAcceptKind::Modify {
                    incoming_base: Some("1"),
                },
                parent_id: None,
                target_path: Some("late.txt"),
                metadata_json: "{}",
                payload_path,
                size_bytes: 4,
                modified_at: 20,
                backup_source_key: None,
                now: 30,
            },
            &|_, _| Vec::new(),
        )
        .unwrap()
    }

    fn seeded_for_a_late_accept() -> (tempfile::TempDir, StateDb) {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "late", FileStatus::Local, 10);
        let mut contract = db.get_file_contract_state("late").unwrap().unwrap();
        contract.current_version = 1;
        db.set_file_contract_state(&contract).unwrap();
        db.track_staged_payload("/staged/late", None, false).unwrap();
        (dir, db)
    }

}

/// Tasks 1888 + 2226 (spec §4 D1-D4, D8, D14; I1-I4, §5): the reconcile pass against a real database.
#[cfg(test)]
mod reconcile_tests {
    mod pass;
}

mod activity;
mod cache;
mod custody;
mod files;
mod landing;
mod ops;
mod removals;
mod schema;
mod signout;
mod sync;
mod test_hooks;

mod types;
use types::*;
pub use types::{UploadFinalization, FileStatus, PERMISSION_READ, PERMISSION_WRITE, PERMISSION_SHARE, PERMISSION_OWNER, Namespace, ItemKind, PinState, OperationKind, FpChangeKind, FileChange, PrunedRow, FileEntry, FileContractState, RevokedSharedCache, LocalStatePurge, LocalActivityKind, KeptReason, KeptCopyInput, KeptCopy, LocalActivityEventInput, LocalActivityEvent, TransferActivityInput, TransferActivity, TransferBacklog, PendingOperation, UploadResume, WriteOrigin, FinderWrite, ItemPresentation, WaitingRow, PurgeGuardRead, FinderEnqueue, FinderFolderRow, FinderAccept, RetireParked, FinderAcceptKind, AcceptOutcome, ParkReason, LandingInput, LandingOutcome, SnapshotVersion, ClaimedOp, TookOver, ClaimOutcome, EngineStartRepair, OperationPauseReason, QueueDiagnostics, QueueHold, HeldGroup, TreeRow, BandwidthSample};

mod sql;
use sql::*;
pub use sql::{GuardedRow, LOCAL_ACTIVITY_MAX_ROWS, ALIAS_MAX_AGE_SECS, TRANSFER_ACTIVITY_MAX_ROWS, decode_os_state};
pub(crate) use sql::{page_cache_limit_kib, tree_holds_a_file, is_file_provider_trash};

mod conn;
use conn::*;

mod error;
use error::*;
pub use error::StateDbError;

mod flags;
use flags::*;

mod ids;
use ids::*;
pub(crate) use ids::{LocalFileId, ServerFileId, VersionId};

mod reconcile;
use reconcile::*;
pub use reconcile::{RECONCILE_UNITS_PER_TX};
