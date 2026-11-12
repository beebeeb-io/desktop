//! `found_while_paused` unit tests, split out of `found_while_paused.rs`. The keep/plan/recover
//! decisions for changes found while sync was paused: the move out of the Beebeeb folder and its
//! dated kept folder, the journal and crash recovery, what stays held, the card's review counts,
//! and the OneDrive/Dropbox detection.
//!
//! `super` is still `crate::found_while_paused`, so every `use super::*;` resolves as before. Tests
//! only: no production item lives in this file.
use super::*;
use crate::account_binding::Identity;
use crate::state_db::{FileEntry, FileStatus, ItemKind, Namespace, OperationKind, PendingOperation};
use std::sync::Arc;

const TODAY: &str = "2026-10-28";

/// A PC: `<temp>\Beebeeb` is the sync root, its database in `<temp>\state`, a staging folder in `<temp>\staging`.
struct Pc {
    db: Arc<StateDb>,
    root: PathBuf,
    staging: PathBuf,
    temp: tempfile::TempDir,
}

impl Pc {
    fn new() -> Self {
        Self::new_in(&std::env::temp_dir())
    }

    /// A PC whose temp folder is made under `parent` (task 2407: a volume that makes 8.3 short names).
    fn new_in(parent: &Path) -> Self {
        let temp = tempfile::tempdir_in(parent).unwrap();
        let root = temp.path().join("Beebeeb");
        let staging = temp.path().join("staging");
        for dir in [&root, &staging, &temp.path().join("state")] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let db = Arc::new(StateDb::open(temp.path().join("state").join("state.db")).unwrap());
        Self {
            db,
            root,
            staging,
            temp,
        }
    }

    /// Lead ruling on self-review F9 (2026-10-29): the kept folder is "Beebeeb - not uploaded", true whether or not
    /// OneDrive or Dropbox syncs the folder it is made in. Spelled out here so the name is tested, not assumed.
    fn kept_dir(&self) -> PathBuf {
        self.temp.path().join("Beebeeb - not uploaded").join(TODAY)
    }

    /// A row: `server` says the server named it (a version), `None` parent at the top level.
    fn row(&self, id: &str, path: &str, parent: Option<&str>, folder: bool, server: bool, namespace: Namespace) {
        let kind = if folder { ItemKind::Folder } else { ItemKind::File };
        self.db
            .upsert_file_with_contract(
                &FileEntry {
                    file_id: id.to_string(),
                    path: path.to_string(),
                    status: FileStatus::Local,
                    size_bytes: 3,
                    modified_at: 1_700_000_000,
                    content_hash: None,
                    remote_updated_at: 0,
                    parent_id: parent.map(str::to_string),
                    item_kind: kind.clone(),
                },
                |contract| {
                    contract.parent_id = parent.map(str::to_string);
                    contract.namespace = namespace;
                    contract.item_kind = kind;
                    contract.current_version = if server { 3 } else { 0 };
                },
            )
            .unwrap();
    }

    /// A new folder made on this PC, on disk and in the database, with its create queued.
    fn new_folder(&self, id: &str, path: &str, parent: Option<&str>) {
        std::fs::create_dir_all(self.on_disk(path)).unwrap();
        self.row(id, path, parent, true, false, Namespace::MyFiles);
        self.queue(&format!("op-{id}"), OperationKind::CreateFolder, Some(id), parent, None);
    }

    /// A new file made on this PC, on disk and in the database, with its upload queued from a staged copy.
    fn new_file(&self, id: &str, path: &str, parent: Option<&str>, bytes: &[u8]) {
        std::fs::write(self.on_disk(path), bytes).unwrap();
        self.row(id, path, parent, false, false, Namespace::MyFiles);
        let staged = self.staging.join(format!("{id}.staged"));
        std::fs::write(&staged, bytes).unwrap();
        self.db
            .track_staged_payload(
                &staged.to_string_lossy(),
                Some(&self.on_disk(path).to_string_lossy()),
                false,
            )
            .unwrap();
        self.queue_create_file(&format!("op-{id}"), Some(id), parent, Some(&staged));
    }

    /// A queued change to an item that already exists (no create).
    fn queue(
        &self,
        op_id: &str,
        kind: OperationKind,
        file_id: Option<&str>,
        parent: Option<&str>,
        payload: Option<&Path>,
    ) {
        self.enqueue(op_id, kind, file_id, parent, payload, None);
    }

    /// A new file's upload, which says it creates the file, as the scan queues it (`engine_bridge`'s
    /// `"operation": "create_file"`): only then does its row count as made on this PC (lead review round 2, P2-1).
    fn queue_create_file(&self, op_id: &str, file_id: Option<&str>, parent: Option<&str>, payload: Option<&Path>) {
        self.enqueue(
            op_id,
            OperationKind::UploadVersion,
            file_id,
            parent,
            payload,
            Some(r#"{"operation":"create_file"}"#),
        );
    }

    fn enqueue(
        &self,
        op_id: &str,
        kind: OperationKind,
        file_id: Option<&str>,
        parent: Option<&str>,
        payload: Option<&Path>,
        metadata: Option<&str>,
    ) {
        self.db
            .enqueue_operation(&PendingOperation {
                op_id: op_id.to_string(),
                kind,
                file_id: file_id.map(str::to_string),
                parent_id: parent.map(str::to_string),
                target_path: Some("x".to_string()),
                metadata_json: metadata.map(str::to_string),
                payload_path: payload.map(|path| path.to_string_lossy().into_owned()),
                base_version: None,
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 5,
                next_retry_at: 0,
                last_error: None,
                backup_source_key: None,
                created_at: 1,
                updated_at: 1,
            })
            .unwrap();
    }

    /// Hold every queued change for review, as an adoption that proved none of them does.
    fn hold_all(&self) {
        let owner = Identity::new(Some("user-owner"), Some("owner@example.eu"));
        assert!(
            self.db
                .adopt_owner_holding(None, &owner, &HashSet::new(), 5)
                .unwrap()
                .is_some_and(|held| held > 0),
            "precondition: changes held"
        );
    }

    fn on_disk(&self, path: &str) -> PathBuf {
        self.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR))
    }

    fn held(&self) -> Vec<String> {
        let mut held: Vec<String> = self
            .db
            .found_while_paused_ops()
            .unwrap()
            .into_iter()
            .map(|op| op.op_id)
            .collect();
        held.sort();
        held
    }

    fn row_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .db
            .list_files()
            .unwrap()
            .into_iter()
            .map(|row| row.file_id)
            .collect();
        ids.sort();
        ids
    }

    fn ops(&self) -> usize {
        self.db.own_data_proof_ops().unwrap().len()
    }

    /// Keep what is held, all of it shown on the card.
    fn keep(&self) -> KeepResult {
        keep(&self.db, &self.root, TODAY, &rename_no_replace, &self.held()).expect("keep runs")
    }
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

/// The lead's case: a new folder (with a file and a subfolder) at the top level and a new file at the top level,
/// both held. Keep moves both next to the Beebeeb folder, in the dated folder, and forgets their rows and changes:
/// nothing is uploaded and nothing is deleted. The server's rows are untouched.
#[test]
fn keep_moves_new_items_out_of_the_beebeeb_folder_and_forgets_them() {
    let pc = Pc::new();
    pc.row("server-file", "Report.pdf", None, false, true, Namespace::MyFiles);
    pc.new_folder("new-folder", "Trip", None);
    pc.new_folder("new-sub", "Trip/Day 1", Some("new-folder"));
    pc.new_file("new-photo", "Trip/Day 1/photo.jpg", Some("new-sub"), b"jpg");
    pc.new_file("new-note", "Trip/note.txt", Some("new-folder"), b"txt");
    pc.new_file("new-top", "todo.txt", None, b"todo");
    pc.hold_all();
    let staged: Vec<PathBuf> = ["new-photo", "new-note", "new-top"]
        .iter()
        .map(|id| pc.staging.join(format!("{id}.staged")))
        .collect();

    let result = pc.keep();

    let kept = pc.kept_dir();
    assert_eq!(
        result,
        KeepResult {
            moved_files: 3,
            moved_items: 2,
            kept_path: Some(kept.to_string_lossy().into_owned()),
            not_moved: 0,
            stays_held: 0,
            synced_by: None,
        }
    );
    assert_eq!(
        std::fs::read(kept.join("Trip").join("Day 1").join("photo.jpg")).unwrap(),
        b"jpg"
    );
    assert_eq!(std::fs::read(kept.join("Trip").join("note.txt")).unwrap(), b"txt");
    assert_eq!(std::fs::read(kept.join("todo.txt")).unwrap(), b"todo");
    assert!(
        !pc.on_disk("Trip").exists() && !pc.on_disk("todo.txt").exists(),
        "out of the Beebeeb folder"
    );
    assert!(!kept.starts_with(&pc.root), "next to the Beebeeb folder, never in it");
    assert_eq!(pc.row_ids(), strings(&["server-file"]), "only the server's row is left");
    assert_eq!(pc.ops(), 0, "nothing queued, nothing to upload");
    assert!(
        staged.iter().all(|path| !path.exists()),
        "the staged copies of kept files are removed"
    );
    assert_eq!(pc.db.keep_journal().unwrap(), None);
    assert_eq!(
        pc.db.last_kept_dir().unwrap(),
        Some(kept.to_string_lossy().into_owned())
    );
}

/// A name already in the dated folder is never overwritten: the item gets " (2)" (before the extension), and the
/// one already there keeps its bytes.
#[test]
fn keep_never_overwrites_a_name_already_in_the_kept_folder() {
    let pc = Pc::new();
    let kept = pc.kept_dir();
    std::fs::create_dir_all(kept.join("Trip")).unwrap();
    std::fs::write(kept.join("Trip").join("old.txt"), b"kept earlier").unwrap();
    std::fs::write(kept.join("todo.txt"), b"kept earlier").unwrap();
    pc.new_folder("new-folder", "Trip", None);
    pc.new_file("new-top", "todo.txt", None, b"new");
    pc.hold_all();

    let result = pc.keep();

    assert_eq!((result.moved_items, result.not_moved), (2, 0), "{result:?}");
    assert!(kept.join("Trip (2)").is_dir());
    assert_eq!(std::fs::read(kept.join("todo (2).txt")).unwrap(), b"new");
    assert_eq!(std::fs::read(kept.join("todo.txt")).unwrap(), b"kept earlier");
    assert_eq!(
        std::fs::read(kept.join("Trip").join("old.txt")).unwrap(),
        b"kept earlier"
    );
}

#[test]
fn a_unique_target_counts_up_before_the_extension() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(unique_target(dir.path(), "a.txt"), dir.path().join("a.txt"));
    std::fs::write(dir.path().join("a.txt"), b"").unwrap();
    std::fs::write(dir.path().join("a (2).txt"), b"").unwrap();
    assert_eq!(unique_target(dir.path(), "a.txt"), dir.path().join("a (3).txt"));
    std::fs::create_dir(dir.path().join("Folder")).unwrap();
    assert_eq!(unique_target(dir.path(), "Folder"), dir.path().join("Folder (2)"));
    std::fs::write(dir.path().join(".env"), b"").unwrap();
    assert_eq!(unique_target(dir.path(), ".env"), dir.path().join(".env (2)"));
    std::fs::write(dir.path().join("archive.tar.gz"), b"").unwrap();
    assert_eq!(
        unique_target(dir.path(), "archive.tar.gz"),
        dir.path().join("archive.tar (2).gz")
    );
}

/// Lead ruling (a) 2: an item on another volume (or whose rename fails for any reason) is never copied. It stays
/// where it is, its changes stay held, its rows stay, and the person is told.
#[test]
fn an_item_that_cannot_be_renamed_stays_where_it_is_and_stays_held() {
    let pc = Pc::new();
    pc.new_folder("new-folder", "Trip", None);
    pc.new_file("new-note", "Trip/note.txt", Some("new-folder"), b"txt");
    pc.new_file("new-top", "todo.txt", None, b"todo");
    pc.hold_all();
    let held = pc.held();
    const ERROR_NOT_SAME_DEVICE: i32 = 17;
    let other_volume = |from: &Path, to: &Path| {
        if from.ends_with("Trip") {
            Err(std::io::Error::from_raw_os_error(ERROR_NOT_SAME_DEVICE))
        } else {
            rename_no_replace(from, to)
        }
    };

    let result = keep(&pc.db, &pc.root, TODAY, &other_volume, &pc.held()).unwrap();

    assert_eq!(
        (result.moved_items, result.moved_files, result.not_moved),
        (1, 1, 1),
        "{result:?}"
    );
    assert_eq!(
        std::fs::read(pc.on_disk("Trip/note.txt")).unwrap(),
        b"txt",
        "left where it was"
    );
    assert!(!pc.kept_dir().join("Trip").exists(), "nothing copied");
    assert_eq!(pc.held(), strings(&["op-new-folder", "op-new-note"]), "still held");
    assert!(held.contains(&"op-new-top".to_string()));
    assert_eq!(pc.row_ids(), strings(&["new-folder", "new-note"]));
    assert_eq!(pc.db.keep_journal().unwrap(), None);
}

/// Lead ruling (a) 4: a crash after the move and before the transaction. The next start finds the item moved and
/// finishes: its rows and changes are forgotten, nothing is queued again, nothing is lost, and the scan of the
/// Beebeeb folder finds nothing to upload.
#[test]
fn a_crash_between_the_move_and_the_transaction_is_finished_at_the_next_start() {
    let pc = Pc::new();
    pc.new_folder("new-folder", "Trip", None);
    pc.new_file("new-note", "Trip/note.txt", Some("new-folder"), b"txt");
    pc.hold_all();
    let crash_after_moving = |from: &Path, to: &Path| -> std::io::Result<()> {
        rename_no_replace(from, to).unwrap();
        panic!("the process ends here");
    };
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        keep(&pc.db, &pc.root, TODAY, &crash_after_moving, &pc.held())
    }));
    assert!(crashed.is_err(), "precondition: the keep was interrupted");
    assert!(
        pc.db.keep_journal().unwrap().is_some(),
        "precondition: the journal says a move was in progress"
    );
    assert_eq!(pc.ops(), 2, "precondition: the transaction did not run");

    assert_eq!(recover(&pc.db, &pc.root).unwrap(), Recovery::Finished);

    assert_eq!(
        std::fs::read(pc.kept_dir().join("Trip").join("note.txt")).unwrap(),
        b"txt",
        "nothing lost"
    );
    assert_eq!(pc.row_ids(), Vec::<String>::new());
    assert_eq!(pc.ops(), 0, "nothing queued again");
    assert_eq!(pc.db.keep_journal().unwrap(), None);
    assert_eq!(recover(&pc.db, &pc.root).unwrap(), Recovery::Nothing, "once");
    scan(&pc);
    assert_eq!(pc.ops(), 0, "the scan finds nothing to upload");
}

/// A crash before the move: the next start finds the item where it was and leaves it held.
#[test]
fn a_crash_before_the_move_leaves_the_item_held() {
    let pc = Pc::new();
    pc.new_folder("new-folder", "Trip", None);
    pc.hold_all();
    let crash_before_moving = |_: &Path, _: &Path| -> std::io::Result<()> { panic!("the process ends here") };
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        keep(&pc.db, &pc.root, TODAY, &crash_before_moving, &pc.held())
    }));
    assert!(
        crashed.is_err() && pc.db.keep_journal().unwrap().is_some(),
        "precondition"
    );

    assert_eq!(recover(&pc.db, &pc.root).unwrap(), Recovery::Cleared);

    assert!(pc.on_disk("Trip").is_dir());
    assert_eq!(pc.held(), strings(&["op-new-folder"]));
    assert_eq!(pc.db.keep_journal().unwrap(), None);
}

/// The kept folder is next to the Beebeeb folder, so the scan of the Beebeeb folder never picks it up: after a
/// keep, a scan queues nothing and writes no row.
#[test]
fn the_scan_never_picks_up_the_kept_folder() {
    let pc = Pc::new();
    pc.new_folder("new-folder", "Trip", None);
    pc.new_file("new-note", "Trip/note.txt", Some("new-folder"), b"txt");
    pc.new_file("new-top", "todo.txt", None, b"todo");
    pc.hold_all();
    assert_eq!(pc.keep().moved_items, 2);

    scan(&pc);
    scan(&pc);

    assert_eq!(pc.ops(), 0);
    assert_eq!(pc.row_ids(), Vec::<String>::new());
    assert!(pc.kept_dir().join("todo.txt").is_file());
}

/// A held change to an item the server already has (an edit of a file in a folder shared with this account) cannot
/// be kept off Beebeeb by a move: it stays held, and the file stays. A new file made in that shared folder is the
/// person's own, made here: it moves.
#[test]
fn a_change_to_an_item_the_server_has_stays_held_and_a_new_file_next_to_it_moves() {
    let pc = Pc::new();
    std::fs::create_dir_all(pc.on_disk("Shared")).unwrap();
    std::fs::write(pc.on_disk("Shared/plan.txt"), b"theirs").unwrap();
    pc.row("shared-folder", "Shared", None, true, true, Namespace::SharedWithMe);
    pc.row(
        "shared-file",
        "Shared/plan.txt",
        Some("shared-folder"),
        false,
        true,
        Namespace::SharedWithMe,
    );
    pc.queue("op-edit", OperationKind::UploadVersion, Some("shared-file"), None, None);
    pc.new_file("new-in-shared", "Shared/mine.txt", Some("shared-folder"), b"mine");
    pc.hold_all();

    let result = pc.keep();

    assert_eq!(
        (result.moved_items, result.moved_files, result.stays_held),
        (1, 1, 1),
        "{result:?}"
    );
    assert_eq!(std::fs::read(pc.kept_dir().join("mine.txt")).unwrap(), b"mine");
    assert_eq!(std::fs::read(pc.on_disk("Shared/plan.txt")).unwrap(), b"theirs");
    assert_eq!(pc.held(), strings(&["op-edit"]));
    assert_eq!(pc.row_ids(), strings(&["shared-file", "shared-folder"]));
}

/// A folder made here that holds a row the server named is not only this PC's: its changes stay held, nothing in
/// it moves.
#[test]
fn a_new_folder_holding_a_server_row_stays_held() {
    let pc = Pc::new();
    pc.new_folder("new-folder", "Trip", None);
    std::fs::write(pc.on_disk("Trip/synced.txt"), b"s").unwrap();
    pc.row(
        "server-file",
        "Trip/synced.txt",
        Some("new-folder"),
        false,
        true,
        Namespace::MyFiles,
    );
    pc.hold_all();

    let result = pc.keep();

    assert_eq!((result.moved_items, result.stays_held), (0, 1), "{result:?}");
    assert!(pc.on_disk("Trip/synced.txt").is_file());
    assert_eq!(pc.held(), strings(&["op-new-folder"]));
}

/// An item part of which the server may already hold is not "not uploaded": it stays held. Here a file in the
/// new folder landed (its finalization replaced its queued change), and another file has a resumable upload.
#[test]
fn an_item_whose_upload_had_begun_stays_held() {
    let pc = Pc::new();
    pc.new_folder("new-folder", "Trip", None);
    pc.new_file("landed", "Trip/landed.txt", Some("new-folder"), b"l");
    pc.new_file("resuming", "todo.txt", None, b"todo");
    pc.hold_all();
    pc.db
        .put_upload_finalization(&crate::state_db::UploadFinalization {
            op_id: "op-landed".to_string(),
            local_file_id: "landed".to_string(),
            server_file_id: "server-id".to_string(),
            target_path: "Trip/landed.txt".to_string(),
            payload_path: "staged".to_string(),
            stamped: false,
            review: None,
        })
        .unwrap();
    pc.db
        .put_upload_resume(&crate::state_db::UploadResume {
            op_id: "op-resuming".to_string(),
            payload_path: "staged".to_string(),
            payload_size: 4,
            payload_mtime_ns: 1,
            upload_session_id: "session".to_string(),
            server_file_id: "server-id-2".to_string(),
            object_version_id: "version".to_string(),
            chunk_size_bytes: 4,
            chunk_count: 1,
            acked_chunks: 0,
            metadata_applied: false,
            is_create: true,
            self_minted: false,
            completed_version: None,
            completed_object_version_id: None,
            completed_mime_type: None,
        })
        .unwrap();

    let result = pc.keep();

    assert_eq!((result.moved_items, result.stays_held), (0, 2), "{result:?}");
    assert!(pc.on_disk("Trip/landed.txt").is_file() && pc.on_disk("todo.txt").is_file());
    assert_eq!(pc.held(), strings(&["op-new-folder", "op-resuming"]));
}

/// A held change whose item has no row at all (it is gone) has nothing to move: it stays held, and nothing else
/// changes.
#[test]
fn a_change_whose_item_has_no_row_stays_held() {
    let pc = Pc::new();
    pc.queue("op-orphan", OperationKind::TrashFile, Some("gone"), None, None);
    pc.new_file("new-top", "todo.txt", None, b"todo");
    pc.hold_all();
    let result = pc.keep();
    assert_eq!((result.moved_items, result.stays_held), (1, 1), "{result:?}");
    assert_eq!(pc.held(), strings(&["op-orphan"]));
}

/// A staged copy is removed only when it is not in the Beebeeb folder or the kept folder: a person's file is never
/// deleted, whatever a queued change names as its copy.
#[test]
fn a_payload_inside_the_beebeeb_folder_is_never_deleted() {
    let pc = Pc::new();
    std::fs::write(pc.on_disk("todo.txt"), b"todo").unwrap();
    std::fs::write(pc.on_disk("keep-me.txt"), b"mine").unwrap();
    pc.row("new-top", "todo.txt", None, false, false, Namespace::MyFiles);
    pc.queue_create_file("op-new-top", Some("new-top"), None, Some(&pc.on_disk("keep-me.txt")));
    pc.hold_all();
    assert_eq!(pc.keep().moved_items, 1);
    assert_eq!(std::fs::read(pc.on_disk("keep-me.txt")).unwrap(), b"mine");
}

/// Nothing held: nothing moves, no folder is made.
#[test]
fn nothing_held_moves_nothing() {
    let pc = Pc::new();
    pc.new_file("new-top", "todo.txt", None, b"todo");
    assert_eq!(pc.keep(), KeepResult::default());
    assert!(!pc.temp.path().join("Beebeeb - not uploaded").exists());
    assert!(pc.on_disk("todo.txt").is_file());
}

/// Self-review F3: a change queued after the hold to an item the person has not decided about yet is held too: the
/// item saved again, and a new file in a held new folder. A change to anything else is due, and a held change that is
/// queued again stays held.
#[test]
fn a_change_queued_after_the_hold_to_a_held_item_is_held_too() {
    let pc = Pc::new();
    pc.new_folder("new-folder", "Trip", None);
    pc.new_file("new-top", "todo.txt", None, b"todo");
    pc.queue("op-no-row", OperationKind::CreateFolder, None, None, None);
    pc.hold_all();
    assert_eq!(
        pc.held(),
        strings(&["op-new-folder", "op-new-top", "op-no-row"]),
        "precondition"
    );

    pc.queue(
        "op-saved-again",
        OperationKind::UploadVersion,
        Some("new-top"),
        None,
        None,
    );
    pc.queue_create_file("op-new-inside", Some("new-inside"), Some("new-folder"), None);
    pc.queue_create_file("op-new-top", Some("new-top"), None, None);
    pc.queue("op-no-row", OperationKind::CreateFolder, None, None, None);
    pc.queue(
        "op-elsewhere",
        OperationKind::UploadVersion,
        Some("own-file"),
        Some("own-folder"),
        None,
    );

    assert_eq!(
        pc.held(),
        strings(&[
            "op-new-folder",
            "op-new-inside",
            "op-new-top",
            "op-no-row",
            "op-saved-again"
        ]),
        "held: what acts on a held item, and a held change queued again"
    );
    let due: Vec<String> = pc
        .db
        .list_due_operations(i64::MAX)
        .unwrap()
        .into_iter()
        .map(|op| op.op_id)
        .collect();
    assert_eq!(due, strings(&["op-elsewhere"]), "a change to anything else is due");
}

/// Lead review round 3, P2-2: the card counts the held changes by kind (a change in a folder shared with this
/// account under that kind only) and names exactly the changes it counted.
#[test]
fn the_review_counts_the_held_changes_by_kind() {
    let pc = Pc::new();
    pc.row("server-file", "Report.pdf", None, false, true, Namespace::MyFiles);
    pc.row("server-old", "Old.pdf", None, false, true, Namespace::MyFiles);
    pc.row("shared-folder", "Shared", None, true, true, Namespace::SharedWithMe);
    pc.row(
        "shared-file",
        "Shared/Plan.txt",
        Some("shared-folder"),
        false,
        true,
        Namespace::SharedWithMe,
    );
    pc.new_folder("new-folder", "Trip", None);
    pc.new_file("new-photo", "Trip/photo.jpg", Some("new-folder"), b"jpg");
    pc.queue("op-edit", OperationKind::UploadVersion, Some("server-file"), None, None);
    pc.queue("op-rename", OperationKind::RenameFile, Some("server-file"), None, None);
    pc.queue("op-trash", OperationKind::TrashFile, Some("server-old"), None, None);
    pc.queue_create_file("op-new-in-shared", Some("new-in-shared"), Some("shared-folder"), None);
    pc.queue(
        "op-shared-edit",
        OperationKind::UploadVersion,
        Some("shared-file"),
        Some("shared-folder"),
        None,
    );
    pc.hold_all();

    let review = review(&pc.db.found_while_paused_changes().unwrap());
    assert_eq!(
        (
            review.new_folders,
            review.new_files,
            review.edits,
            review.deletions,
            review.in_shared_folders
        ),
        (1, 1, 2, 1, 2),
        "{review:?}"
    );
    let mut shown = review.op_ids.clone();
    shown.sort();
    assert_eq!(shown, pc.held(), "the card names exactly the held changes");
    assert_eq!(
        review.synced_by, None,
        "the backend says which service syncs the kept folder, not the review"
    );
}

/// Self-review of round 3 (F8): a change anywhere inside a folder shared with this account counts as such, however
/// deep: here a new file in a new folder made here inside the shared folder (the new rows are My Files rows).
#[test]
fn the_review_counts_a_change_deep_in_a_shared_folder_as_shared() {
    let pc = Pc::new();
    pc.row("shared-folder", "Shared", None, true, true, Namespace::SharedWithMe);
    pc.new_folder("new-folder", "Shared/Trip", Some("shared-folder"));
    pc.new_file("new-photo", "Shared/Trip/photo.jpg", Some("new-folder"), b"jpg");
    pc.hold_all();

    let review = review(&pc.db.found_while_paused_changes().unwrap());

    assert_eq!(
        (review.new_folders, review.new_files, review.in_shared_folders),
        (0, 0, 2),
        "{review:?}"
    );
}

/// Lead review round 3, P3-1: "Upload to my Beebeeb" releases the changes the card showed: never one held after
/// the card read them, and a change released once is not counted again.
#[test]
fn upload_releases_only_the_changes_the_card_showed() {
    let pc = Pc::new();
    pc.new_folder("new-folder", "Trip", None);
    pc.new_file("new-top", "todo.txt", None, b"todo");
    pc.hold_all();
    let shown = pc.held();
    assert_eq!(shown, strings(&["op-new-folder", "op-new-top"]), "precondition");
    pc.queue_create_file("op-new-inside", Some("new-inside"), Some("new-folder"), None);
    assert!(
        pc.held().contains(&"op-new-inside".to_string()),
        "precondition: held after the card read them"
    );

    assert_eq!(pc.db.release_found_while_paused(&shown, 5).unwrap(), 2);
    assert_eq!(
        pc.held(),
        strings(&["op-new-inside"]),
        "the change the card did not show stays held"
    );
    assert_eq!(pc.db.release_found_while_paused(&shown, 5).unwrap(), 0, "released once");
}

/// Lead review round 3, P3-3: a staged copy whose live file is gone holds the only copy of those bytes. Keep copies
/// it into the kept folder (never over a name there) before it removes it; a copy whose live file moved with the
/// item goes.
#[test]
fn an_orphaned_staged_copy_is_copied_into_the_kept_folder_never_destroyed() {
    let pc = Pc::new();
    pc.new_folder("new-folder", "Trip", None);
    pc.new_file("new-photo", "Trip/photo.jpg", Some("new-folder"), b"only copy");
    pc.new_file("new-note", "Trip/note.txt", Some("new-folder"), b"note");
    std::fs::remove_file(pc.on_disk("Trip/photo.jpg")).unwrap();
    std::fs::create_dir_all(pc.kept_dir()).unwrap();
    std::fs::write(pc.kept_dir().join("photo.jpg"), b"already there").unwrap();
    pc.hold_all();

    assert_eq!(pc.keep().moved_items, 1);
    assert_eq!(
        std::fs::read(pc.kept_dir().join("photo (2).jpg")).unwrap(),
        b"only copy",
        "the only copy is kept"
    );
    assert_eq!(
        std::fs::read(pc.kept_dir().join("photo.jpg")).unwrap(),
        b"already there",
        "never over a name"
    );
    assert_eq!(
        std::fs::read(pc.kept_dir().join("Trip").join("note.txt")).unwrap(),
        b"note"
    );
    assert!(
        !pc.staging.join("new-photo.staged").exists(),
        "then the staged copy goes"
    );
    assert!(
        !pc.staging.join("new-note.staged").exists(),
        "a copy whose file moved with the item goes"
    );
    assert!(
        pc.db.staged_payloads_for_signout().unwrap().is_empty(),
        "the journal forgets both"
    );
}

/// Self-review of round 3 (F7): Keep moves only the items of the changes the card showed; a change held after the
/// card read them stays held, and its item stays where it is.
#[test]
fn keep_moves_only_the_items_of_the_changes_the_card_showed() {
    let pc = Pc::new();
    pc.new_folder("shown", "Shown", None);
    pc.new_folder("later", "Later", None);
    pc.hold_all();

    let result = keep(&pc.db, &pc.root, TODAY, &rename_no_replace, &strings(&["op-shown"])).unwrap();

    assert_eq!(result.moved_items, 1);
    assert!(pc.kept_dir().join("Shown").exists());
    assert!(pc.on_disk("Later").exists(), "not shown: not moved");
    assert_eq!(pc.held(), strings(&["op-later"]));
}

/// Self-review of round 3 (F3): the only copy of a file's bytes is in the kept folder before the keep forgets the
/// item, so a crash right after the forget cannot leave that copy named by nothing.
#[test]
fn an_only_copy_is_kept_before_the_item_is_forgotten() {
    let pc = Pc::new();
    pc.new_folder("new-folder", "Trip", None);
    pc.new_file("new-photo", "Trip/photo.jpg", Some("new-folder"), b"only copy");
    std::fs::remove_file(pc.on_disk("Trip/photo.jpg")).unwrap();
    pc.hold_all();
    let seen = std::rc::Rc::new(std::cell::RefCell::new(None));
    let (kept, db, seen_in_hook) = (pc.kept_dir().join("photo.jpg"), pc.db.clone(), seen.clone());
    before_forget_for_test(move || {
        *seen_in_hook.borrow_mut() = Some((std::fs::read(&kept).ok(), db.own_data_proof_ops().unwrap().len()));
    });

    assert_eq!(pc.keep().moved_items, 1);

    assert_eq!(
        seen.borrow().clone(),
        Some((Some(b"only copy".to_vec()), 2)),
        "copied while the item's changes were still there"
    );
}

/// Self-review of round 3 (F3): a keep that stopped after copying an only copy and before forgetting the item is
/// finished at the next start, and the copy already in the kept folder is not made twice.
#[test]
fn a_keep_stopped_after_the_copy_is_finished_at_the_next_start_with_one_copy() {
    let pc = Pc::new();
    pc.new_folder("new-folder", "Trip", None);
    pc.new_file("new-photo", "Trip/photo.jpg", Some("new-folder"), b"only copy");
    std::fs::remove_file(pc.on_disk("Trip/photo.jpg")).unwrap();
    pc.hold_all();
    before_forget_for_test(|| panic!("the process ends here"));
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pc.keep()));
    assert!(crashed.is_err(), "precondition: the keep was interrupted");

    assert_eq!(recover(&pc.db, &pc.root).unwrap(), Recovery::Finished);

    assert_eq!(std::fs::read(pc.kept_dir().join("photo.jpg")).unwrap(), b"only copy");
    assert!(!pc.kept_dir().join("photo (2).jpg").exists(), "one copy, not two");
    assert!(!pc.staging.join("new-photo.staged").exists());
    assert!(pc.db.staged_payloads_for_signout().unwrap().is_empty());
}

/// Lead review round 3, P3-3: Keep removes only the staged copies the journal names; any other file a queued change
/// names as its copy is left where it is.
#[test]
fn a_staged_copy_the_journal_does_not_name_is_never_deleted() {
    let pc = Pc::new();
    pc.new_folder("new-folder", "Trip", None);
    std::fs::write(pc.on_disk("Trip/photo.jpg"), b"jpg").unwrap();
    pc.row(
        "new-photo",
        "Trip/photo.jpg",
        Some("new-folder"),
        false,
        false,
        Namespace::MyFiles,
    );
    let unjournalled = pc.staging.join("not-journalled.bin");
    std::fs::write(&unjournalled, b"someone's file").unwrap();
    pc.queue_create_file(
        "op-new-photo",
        Some("new-photo"),
        Some("new-folder"),
        Some(&unjournalled),
    );
    pc.hold_all();

    assert_eq!(pc.keep().moved_items, 1);
    assert_eq!(std::fs::read(&unjournalled).unwrap(), b"someone's file");
}

/// Lead review round 3, P3-4: the kept folder is made next to the Beebeeb folder. When OneDrive or Dropbox syncs
/// that place, the card says which instead of claiming the items stay on this PC.
#[test]
fn a_folder_in_a_onedrive_or_dropbox_folder_says_which_syncs_it() {
    let roots = vec![
        (PathBuf::from("/home/user/OneDrive - Initlabs"), "OneDrive"),
        (PathBuf::from("/home/user/Dropbox"), "Dropbox"),
    ];
    assert_eq!(
        synced_by(Path::new("/home/user/OneDrive - Initlabs"), &roots),
        Some("OneDrive"),
        "the root itself"
    );
    assert_eq!(
        synced_by(Path::new("/home/user/onedrive - initlabs/Docs"), &roots),
        Some("OneDrive"),
        "any case"
    );
    assert_eq!(synced_by(Path::new("/home/user/Dropbox/Work"), &roots), Some("Dropbox"));
    assert_eq!(
        synced_by(Path::new("/home/user/Dropbox Old"), &roots),
        None,
        "a name that only starts the same"
    );
    assert_eq!(synced_by(Path::new("/home/user"), &roots), None, "above it");
}

#[test]
fn dropbox_names_its_personal_and_business_folders_in_info_json() {
    let info = r#"{"personal": {"path": "C:\\Users\\a\\Dropbox", "host": 1}, "business": {"path": "C:\\Users\\a\\Dropbox (Initlabs)"}, "other": 3}"#;
    let mut roots = dropbox_roots(info);
    roots.sort();
    assert_eq!(
        roots,
        vec![
            PathBuf::from(r"C:\Users\a\Dropbox"),
            PathBuf::from(r"C:\Users\a\Dropbox (Initlabs)")
        ]
    );
    assert!(dropbox_roots("not json").is_empty());
    assert!(
        dropbox_roots(r#"{"personal": {"path": ""}}"#).is_empty(),
        "an empty path is no folder"
    );
}

/// The real scan of the Beebeeb folder, as the watcher runs it.
fn scan(pc: &Pc) {
    let api = Arc::new(crate::api_client::ApiClient::new(
        "https://api.invalid".into(),
        "token".into(),
        [17u8; 32],
    ));
    let bridge = crate::engine_bridge::EngineBridge::new(&crate::identity::REAL, pc.db.clone(), api);
    crate::watcher::run_one_scan(&bridge, &pc.root);
}

// ---- Windows: a file made in the Beebeeb folder is an unsynced Cloud Files placeholder ----

/// A sync root registered for the test, unregistered at the end.
#[cfg(target_os = "windows")]
struct Registered(PathBuf);

#[cfg(target_os = "windows")]
impl Drop for Registered {
    fn drop(&mut self) {
        let _ = crate::windows_cf::unregister_any_sync_root(&self.0);
    }
}

/// A connection to it with no callbacks, disconnected at the end. Nothing answers a fetch, so anything that asks for
/// a cloud-only file's bytes blocks until Cloud Files gives up (about a minute).
/// The connection's key, and its callback table, which Cloud Files reads for as long as the root is connected.
#[cfg(target_os = "windows")]
struct Connected(
    windows::Win32::Storage::CloudFilters::CF_CONNECTION_KEY,
    #[allow(dead_code)] Box<[windows::Win32::Storage::CloudFilters::CF_CALLBACK_REGISTRATION; 1]>,
);

#[cfg(target_os = "windows")]
impl Drop for Connected {
    fn drop(&mut self) {
        // SAFETY: the key of the connection `connect` made, disconnected once.
        let _ = unsafe { windows::Win32::Storage::CloudFilters::CfDisconnectSyncRoot(self.0) };
    }
}

/// Register and connect `root` as a Cloud Files sync root for one test (a tuple drops in order: disconnected
/// first, then unregistered).
#[cfg(target_os = "windows")]
fn sync_root(root: &Path) -> (Connected, Registered) {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::CloudFilters::{
        CF_CALLBACK_REGISTRATION, CF_CALLBACK_TYPE_NONE, CF_CONNECT_FLAG_NONE, CfConnectSyncRoot,
    };
    crate::windows_cf::register_test_sync_root(root).unwrap();
    let registered = Registered(root.to_path_buf());
    let table = Box::new([CF_CALLBACK_REGISTRATION {
        Type: CF_CALLBACK_TYPE_NONE,
        Callback: None,
    }]);
    let wide: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: the path and the table outlive the call.
    let key = unsafe {
        CfConnectSyncRoot(
            windows::core::PCWSTR(wide.as_ptr()),
            table.as_ptr(),
            None,
            CF_CONNECT_FLAG_NONE,
        )
    }
    .unwrap();
    (Connected(key, table), registered)
}

/// The watcher turns a new file into an unsynced placeholder when it queues its upload. Kept, it must be a plain
/// file the person can open, outside any sync root, with its bytes.
#[cfg(target_os = "windows")]
#[test]
fn a_kept_unsynced_placeholder_is_a_plain_file_with_its_bytes() {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    let pc = Pc::new();
    let _root = sync_root(&pc.root);
    pc.new_folder("new-folder", "Trip", None);
    pc.new_file("new-note", "Trip/note.txt", Some("new-folder"), b"made here");
    pc.new_file("new-top", "todo.txt", None, b"todo");
    for (path, id) in [("Trip/note.txt", "new-note"), ("todo.txt", "new-top")] {
        crate::windows_cf::placeholders::convert_to_unsynced_placeholder(&pc.on_disk(path), id).unwrap();
        let attributes = std::fs::symlink_metadata(pc.on_disk(path)).unwrap().file_attributes();
        assert_ne!(
            attributes & FILE_ATTRIBUTE_REPARSE_POINT,
            0,
            "precondition: a placeholder"
        );
    }
    pc.hold_all();

    let result = pc.keep();

    assert_eq!((result.moved_items, result.not_moved), (2, 0), "{result:?}");
    for (path, bytes) in [
        (pc.kept_dir().join("Trip").join("note.txt"), &b"made here"[..]),
        (pc.kept_dir().join("todo.txt"), b"todo"),
    ] {
        let attributes = std::fs::symlink_metadata(&path).unwrap().file_attributes();
        assert_eq!(attributes & FILE_ATTRIBUTE_REPARSE_POINT, 0, "a plain file");
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}

/// An item holding a file whose bytes are not all on this PC (a cloud-only placeholder) is not moved: it would
/// leave the Beebeeb folder without them. Nothing asks for its bytes to find that out: made plain, it would be
/// downloaded first (here, where nothing answers, that blocks for about a minute), and it stays cloud-only.
#[cfg(target_os = "windows")]
#[test]
fn an_item_whose_bytes_are_not_all_here_is_not_moved() {
    let pc = Pc::new();
    let _root = sync_root(&pc.root);
    pc.new_folder("new-folder", "Trip", None);
    assert!(
        crate::windows_cf::placeholders::create_placeholder(
            &pc.on_disk("Trip"),
            "cloud",
            "cloud.bin",
            1 << 20,
            1_700_000_000,
            false
        )
        .unwrap()
    );
    pc.row(
        "cloud",
        "Trip/cloud.bin",
        Some("new-folder"),
        false,
        false,
        Namespace::MyFiles,
    );
    // Made on this PC: a queued create names it (lead review round 2, P2-1); else it is the server's and the
    // item stays held for that reason instead.
    pc.queue_create_file("op-cloud", Some("cloud"), Some("new-folder"), None);
    pc.hold_all();

    let started = std::time::Instant::now();
    let result = pc.keep();
    let took = started.elapsed();

    assert_eq!((result.moved_items, result.not_moved), (0, 1), "{result:?}");
    assert!(pc.on_disk("Trip").is_dir());
    assert_eq!(pc.held(), strings(&["op-cloud", "op-new-folder"]));
    assert!(
        took < std::time::Duration::from_secs(20),
        "nothing asked for the bytes: {took:?}"
    );
    let attributes = std::os::windows::fs::MetadataExt::file_attributes(
        &std::fs::symlink_metadata(pc.on_disk("Trip/cloud.bin")).unwrap(),
    );
    assert_ne!(attributes & 0x0040_0000, 0, "still cloud-only (RECALL_ON_DATA_ACCESS)");
}

/// Task 2407 (core 2404): an item whose name reaches a different entry (on Windows an 8.3 short name; stubbed here,
/// the stubbed file standing in for that entry) is never made plain or moved: it stays where it is, held.
#[test]
fn keep_never_moves_the_entry_an_alias_reaches() {
    let pc = Pc::new();
    pc.new_file("new-top", "REPORT~1.PDF", None, b"a different file");
    pc.hold_all();
    let _alias = crate::name_alias::stub::alias(&pc.on_disk("REPORT~1.PDF"));

    let result = pc.keep();

    assert_eq!((result.moved_items, result.not_moved), (0, 1));
    assert_eq!(std::fs::read(pc.on_disk("REPORT~1.PDF")).unwrap(), b"a different file");
    assert!(!pc.kept_dir().exists(), "nothing was moved out");
    assert_eq!(pc.held(), ["op-new-top"], "it stays held");
}

/// Task 2407, Windows: the same against a real 8.3 short name. The item's row names `REPORT~1.PDF`, the short name of
/// the person's `Report for 2026.pdf`; keep never moves that file out. Skipped (with the reason printed) on a volume
/// that makes no 8.3 names; with `CI` set, it fails instead.
#[cfg(target_os = "windows")]
#[test]
fn keep_never_moves_the_file_a_short_name_reaches() {
    use std::os::windows::ffi::OsStrExt as _;
    use windows::Win32::Storage::FileSystem::GetShortPathNameW;
    // The temp folder, then the local-app-data temp folder (often the system drive, which makes 8.3 names by
    // default; CI's temp volume makes none).
    let mut roots = vec![std::env::temp_dir()];
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join("Temp"));
    }
    let short_of = |path: &Path| -> String {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let mut buffer = vec![0u16; 1024];
        // SAFETY: `wide` is NUL-terminated; `buffer` is writable for its whole length.
        let written = unsafe { GetShortPathNameW(windows::core::PCWSTR(wide.as_ptr()), Some(&mut buffer)) } as usize;
        assert!(written != 0 && written < buffer.len(), "GetShortPathNameW failed");
        let short_path = String::from_utf16(&buffer[..written]).unwrap();
        short_path.rsplit('\\').next().unwrap().to_owned()
    };
    let Some((pc, short)) = roots.iter().find_map(|root| {
        let pc = Pc::new_in(root);
        std::fs::write(pc.on_disk("Report for 2026.pdf"), b"precious").unwrap();
        let short = short_of(&pc.on_disk("Report for 2026.pdf"));
        (short != "Report for 2026.pdf").then_some((pc, short))
    }) else {
        assert!(
            std::env::var_os("CI").is_none(),
            "no volume among {roots:?} made an 8.3 short name; this test proves nothing here"
        );
        eprintln!("skipped: no volume among {roots:?} makes 8.3 short names");
        return;
    };
    let precious = pc.on_disk("Report for 2026.pdf");
    assert_eq!(
        std::fs::read(pc.on_disk(&short)).unwrap(),
        b"precious",
        "the fixture must alias"
    );
    // A new file's row and queued upload at the short name; nothing of its own is on disk there.
    pc.row("new-top", &short, None, false, false, Namespace::MyFiles);
    let staged = pc.staging.join("new-top.staged");
    std::fs::write(&staged, b"new").unwrap();
    pc.db
        .track_staged_payload(
            &staged.to_string_lossy(),
            Some(&pc.on_disk(&short).to_string_lossy()),
            false,
        )
        .unwrap();
    pc.queue_create_file("op-new-top", Some("new-top"), None, Some(&staged));
    pc.hold_all();

    let result = pc.keep();

    assert_eq!((result.moved_items, result.not_moved), (0, 1));
    assert_eq!(
        std::fs::read(&precious).unwrap(),
        b"precious",
        "the person's file is where it was"
    );
}
