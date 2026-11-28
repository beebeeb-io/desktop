//! Tests of the drive model (`model.rs`), the drive error codes (`mod.rs`) and the DTO secret guard. Ported from
//! `src-tauri/src/drive_v2/tests.rs` (`drive_v2_errors_carry_a_code_and_no_detail_from_outside`,
//! `drive_v2_no_dto_has_a_secret_shaped_field`, `drive_v2_shared_lists_with_me_and_by_me`,
//! `drive_v2_sync_summary_phase`) and `src-tauri/src/drive_v2/absent_row.rs` (`absent_row_leaves`), and from
//! `src-tauri/src/selective_sync_wave2_tests.rs` (the two per-path merge tests, against `model::live_rows`).
//! `known_folder_is_trashable` is pinned already by `file_ops::tests::trash_check_accepts_synced_file`.

use std::collections::HashSet;

use super::DriveError;
use super::dto::*;
use super::model::{self, Sort};
use crate::state_db::{self, FileEntry, FileStatus, ItemKind, Namespace, StateDb};

struct Fixture {
    _dir: tempfile::TempDir,
    db: StateDb,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    Fixture { _dir: dir, db }
}

#[allow(clippy::too_many_arguments)]
fn row_in(
    db: &StateDb,
    id: &str,
    path: &str,
    parent: Option<&str>,
    folder: bool,
    size: i64,
    modified: i64,
    status: FileStatus,
    namespace: Namespace,
    owner: Option<&str>,
    bits: i64,
) {
    let entry = FileEntry {
        file_id: id.to_string(),
        path: path.to_string(),
        status,
        size_bytes: size,
        modified_at: modified,
        content_hash: None,
        remote_updated_at: modified,
        parent_id: None,
        item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
    };
    db.upsert_file_with_contract(&entry, |c| {
        c.parent_id = parent.map(str::to_string);
        c.item_kind = if folder { ItemKind::Folder } else { ItemKind::File };
        c.namespace = namespace;
        c.owner_email = owner.map(str::to_string);
        c.permission_bits = bits;
        c.current_version = 3;
    })
    .unwrap();
}

#[allow(clippy::too_many_arguments)]
fn row(
    db: &StateDb,
    id: &str,
    path: &str,
    parent: Option<&str>,
    folder: bool,
    size: i64,
    modified: i64,
    status: FileStatus,
) {
    row_in(
        db,
        id,
        path,
        parent,
        folder,
        size,
        modified,
        status,
        Namespace::MyFiles,
        None,
        state_db::PERMISSION_OWNER,
    );
}

/// A Docs folder with 2 children, two root files, a photo, a row being trashed, and a shared root.
fn drive(f: &Fixture) {
    let db = &f.db;
    row(db, "d0c5", "Docs", None, true, 0, 100, FileStatus::Local);
    row(
        db,
        "a1",
        "Docs/Report.pdf",
        Some("d0c5"),
        false,
        2_000,
        300,
        FileStatus::CloudOnly,
    );
    row(
        db,
        "a2",
        "Docs/notes.txt",
        Some("d0c5"),
        false,
        10,
        200,
        FileStatus::Local,
    );
    row(db, "b1", "zebra.txt", None, false, 50, 400, FileStatus::Local);
    row(db, "b2", "Beach.jpg", None, false, 9_000, 500, FileStatus::Downloading);
    row(db, "gone", "old.txt", None, false, 1, 600, FileStatus::Trashing);
    row_in(
        db,
        "s1",
        "Shared/Plans",
        None,
        true,
        0,
        50,
        FileStatus::CloudOnly,
        Namespace::SharedWithMe,
        Some("ana.lima@example.eu"),
        state_db::PERMISSION_READ,
    );
}

/// A row the server no longer has leaves the list when the snapshot is applied, even when its
/// stored name is the engine's blocked label, and a terminal-failed operation leaves with it.
#[test]
fn absent_row_leaves() {
    let f = fixture();
    row(
        &f.db,
        "keep",
        "keep.txt",
        None,
        false,
        0,
        1_700_000_000,
        FileStatus::CloudOnly,
    );
    row(
        &f.db,
        "stale",
        "Unavailable file 0123456789abcdef",
        None,
        false,
        0,
        0,
        FileStatus::Local,
    );
    f.db.enqueue_operation(&state_db::PendingOperation {
        op_id: "op-stale".to_string(),
        kind: state_db::OperationKind::UploadFile,
        file_id: Some("stale".to_string()),
        parent_id: None,
        target_path: None,
        metadata_json: None,
        payload_path: None,
        base_version: None,
        base_object_version_id: None,
        attempts: 5,
        max_attempts: 5,
        next_retry_at: 0,
        last_error: Some("failed".to_string()),
        backup_source_key: None,
        created_at: 0,
        updated_at: 0,
    })
    .unwrap();
    let seen: HashSet<String> = ["keep".to_string()].into_iter().collect();
    let pruned = model::drop_absent_rows(&f.db, &seen, 1_800_000_000).unwrap();
    assert!(
        pruned.iter().any(|r| r.file_id == "stale"),
        "the absent row must be pruned: {pruned:?}"
    );
    let listing = model::list(&f.db, None, Sort::default(), false).unwrap();
    assert!(
        !listing.items.iter().any(|item| item.id == "stale"),
        "the absent row must not be listed"
    );
    assert!(
        f.db.list_operations_for_file("stale").unwrap().is_empty(),
        "the dropped row's operation must be gone"
    );
    assert!(
        !f.db
            .waiting_rows()
            .unwrap()
            .iter()
            .any(|op| op.file_id.as_deref() == Some("stale")),
        "the dropped row's operation must not be in the waiting queue"
    );
}

/// Not in the Tauri file: a row whose operation is still in flight (attempts left) stays, and a row
/// the snapshot did name stays; an empty snapshot drops nothing.
#[test]
fn a_row_with_an_operation_in_flight_stays_and_an_empty_snapshot_drops_nothing() {
    let f = fixture();
    row(
        &f.db,
        "keep",
        "keep.txt",
        None,
        false,
        0,
        1_700_000_000,
        FileStatus::CloudOnly,
    );
    row(&f.db, "busy", "busy.txt", None, false, 0, 0, FileStatus::Local);
    f.db.enqueue_operation(&state_db::PendingOperation {
        op_id: "op-busy".to_string(),
        kind: state_db::OperationKind::UploadFile,
        file_id: Some("busy".to_string()),
        parent_id: None,
        target_path: None,
        metadata_json: None,
        payload_path: None,
        base_version: None,
        base_object_version_id: None,
        attempts: 1,
        max_attempts: 5,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 0,
        updated_at: 0,
    })
    .unwrap();
    let seen: HashSet<String> = ["keep".to_string()].into_iter().collect();
    let pruned = model::drop_absent_rows(&f.db, &seen, 1_800_000_000).unwrap();
    assert!(!pruned.iter().any(|r| r.file_id == "busy"), "in flight: {pruned:?}");
    assert!(f.db.get_file("busy").unwrap().is_some());
    let none = model::drop_absent_rows(&f.db, &HashSet::new(), 1_800_000_000).unwrap();
    assert!(none.is_empty(), "an empty snapshot never drops: {none:?}");
    assert!(f.db.get_file("keep").unwrap().is_some());
}

/// A row at `path`: `server` is a row the server confirmed (a remote time, not local), else one this PC made.
fn at(db: &StateDb, id: &str, path: &str, parent: Option<&str>, folder: bool, server: bool) {
    let entry = FileEntry {
        file_id: id.to_string(),
        path: path.to_string(),
        status: if server {
            FileStatus::CloudOnly
        } else {
            FileStatus::Local
        },
        size_bytes: 0,
        modified_at: 1_700_000_000,
        content_hash: None,
        remote_updated_at: if server { 1_700_000_000 } else { 0 },
        parent_id: None,
        item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
    };
    db.upsert_file_with_contract(&entry, |c| {
        c.parent_id = parent.map(str::to_string);
        c.item_kind = if folder { ItemKind::Folder } else { ItemKind::File };
        c.namespace = Namespace::MyFiles;
        c.permission_bits = state_db::PERMISSION_OWNER;
        c.current_version = if server { 3 } else { 0 };
    })
    .unwrap();
}

fn live_ids(db: &StateDb) -> Vec<String> {
    let mut ids: Vec<String> = model::live_rows(db).unwrap().into_iter().map(|e| e.file_id).collect();
    ids.sort();
    ids
}

/// Ported from `selective_sync_shows_one_folder_per_path_preferring_the_server_row` (desktop 0.8.15: a folder listed
/// twice, one row the server never confirmed and the server's row at the same path): one row per path, the
/// server's; a never-sent child with no twin stays; the same file twice is one row.
#[test]
fn selective_sync_shows_one_folder_per_path_preferring_the_server_row() {
    let f = fixture();
    at(&f.db, "ghost-archive", "Archive", None, true, false);
    at(&f.db, "srv-archive", "Archive", None, true, true);
    at(
        &f.db,
        "ghost-other",
        "Archive/other",
        Some("ghost-archive"),
        true,
        false,
    );
    at(&f.db, "srv-other", "Archive/other", Some("srv-archive"), true, true);
    at(&f.db, "ghost-only", "Archive/new", Some("ghost-archive"), true, false);
    at(&f.db, "f1", "Archive/other/a.txt", Some("srv-other"), false, true);
    at(&f.db, "f2", "Archive/b.txt", Some("srv-archive"), false, true);
    at(&f.db, "f2-again", "Archive/b.txt", Some("srv-archive"), false, false);
    assert_eq!(
        live_ids(&f.db),
        ["f1", "f2", "ghost-only", "srv-archive", "srv-other"],
        "the server's rows win their path; the twin never sent is folded away; the child with no twin stays"
    );
    assert_eq!(
        f.db.file_overview_rows().unwrap().len(),
        8,
        "the rows themselves are untouched (display only)"
    );
}

/// Ported from `per_path_merge_keeps_two_server_items_and_never_mixes_kinds`, the parts the engine keeps: two
/// never-confirmed rows at one path are one entry (the smallest id); a file and a folder at one path are never
/// merged; a row with no path is left alone; a path is compared without its slashes and its case; a row being trashed
/// is not live.
#[test]
fn per_path_merge_folds_two_unsent_rows_and_never_mixes_kinds() {
    let f = fixture();
    at(&f.db, "ghost-2", "Docs", None, true, false);
    at(&f.db, "ghost-1", "Docs", None, true, false);
    at(&f.db, "a-file-named-docs", "/Docs/", Some("ghost-2"), false, true);
    at(&f.db, "no-path-1", "", None, true, false);
    at(&f.db, "no-path-2", "", None, true, true);
    at(&f.db, "cased", "photos", None, true, false);
    at(&f.db, "cased-srv", "Photos/", None, true, true);
    row(
        &f.db,
        "going",
        "going.txt",
        None,
        false,
        1,
        1_700_000_000,
        FileStatus::Trashing,
    );
    assert_eq!(
        live_ids(&f.db),
        ["a-file-named-docs", "cased-srv", "ghost-1", "no-path-1", "no-path-2"]
    );
}

/// Ported from `per_path_merge_keeps_two_server_items_and_never_mixes_kinds` (match 0.8.x): two rows the
/// server confirmed at one path are two server items, each by its own id, and never hidden; an unsent row at that
/// path still folds into them.
#[test]
fn two_server_confirmed_rows_at_one_path_both_stay() {
    let f = fixture();
    at(&f.db, "srv-a", "Photos", None, true, true);
    at(&f.db, "srv-b", "Photos", None, true, true);
    at(&f.db, "unsent", "Photos", None, true, false);
    assert_eq!(live_ids(&f.db), ["srv-a", "srv-b"]);
}

/// Spec 2.1: no DTO carries a field named like a secret.
pub(super) fn assert_no_secret_fields(value: &serde_json::Value, at: &str) {
    const FORBIDDEN: [&str; 5] = ["key", "token", "url", "nonce", "ciphertext"];
    match value {
        serde_json::Value::Object(map) => {
            for (name, child) in map {
                let lower = name.to_ascii_lowercase();
                // `undoToken` is the one allowed name: an opaque handle into this process's undo table.
                let allowed = lower == "undotoken";
                assert!(
                    allowed || !(FORBIDDEN.iter().any(|f| lower.contains(f)) || lower.starts_with("wrapped")),
                    "{at}.{name} is a secret-shaped field"
                );
                assert_no_secret_fields(child, &format!("{at}.{name}"));
            }
        }
        serde_json::Value::Array(items) => items.iter().for_each(|i| assert_no_secret_fields(i, at)),
        _ => {}
    }
}

#[test]
fn drive_v2_no_dto_has_a_secret_shaped_field() {
    let f = fixture();
    drive(&f);
    let item = model::details(&f.db, "a1").unwrap().item;
    let shared = DriveItem {
        shared: SharedState::People {
            people: vec![Person {
                name: "Ana".into(),
                initials: "A".into(),
            }],
        },
        ..item.clone()
    };
    let sync = model::sync_summary(
        SyncPhase::UpToDate,
        &model::SyncCounts {
            on_device: 1,
            online_only: 1,
            moving: 0,
            conflicts: 0,
            on_hold: 0,
        },
        1,
    );
    let storage = StorageSummary {
        used_bytes: 1,
        quota_bytes: 2,
        files_bytes: 1,
        photos_bytes: 0,
        backups_bytes: 0,
    };
    let dtos: Vec<(&str, serde_json::Value)> = vec![
        ("DriveItem", serde_json::to_value(&shared).unwrap()),
        (
            "Listing",
            serde_json::to_value(model::list(&f.db, None, Sort::default(), false).unwrap()).unwrap(),
        ),
        (
            "ItemDetails",
            serde_json::to_value(model::details(&f.db, "a1").unwrap()).unwrap(),
        ),
        (
            "VersionEntry",
            serde_json::to_value(VersionEntry {
                id: "v".into(),
                created_at: "t".into(),
                size: 1,
                current: true,
            })
            .unwrap(),
        ),
        (
            "TrashEntry",
            serde_json::to_value(TrashEntry {
                item: item.clone(),
                trashed_at: "t".into(),
                original_path: "Files".into(),
            })
            .unwrap(),
        ),
        ("StorageSummary", serde_json::to_value(&storage).unwrap()),
        ("SyncSummary", serde_json::to_value(&sync).unwrap()),
        (
            "ShareEntry",
            serde_json::to_value(ShareEntry {
                item: item.clone(),
                direction: ShareDirection::ByMe,
                people: vec![SharePerson {
                    name: "a".into(),
                    email: Some("a@b.eu".into()),
                    role: ShareRole::Editor,
                }],
                has_link: true,
            })
            .unwrap(),
        ),
        (
            "PhotoPage",
            serde_json::to_value(PhotoPage {
                entries: vec![PhotoEntry {
                    item: item.clone(),
                    taken_at: "t".into(),
                }],
                next: Some("1".into()),
            })
            .unwrap(),
        ),
        (
            "UndoTicket",
            serde_json::to_value(UndoTicket { undo_token: "u".into() }).unwrap(),
        ),
        (
            "UploadStarted",
            serde_json::to_value(UploadStarted { started: 1 }).unwrap(),
        ),
        (
            "TextDocument",
            serde_json::to_value(TextDocument {
                text: "x".into(),
                version_id: "v1-ab".into(),
            })
            .unwrap(),
        ),
        (
            "DriveEvent::Changed",
            serde_json::to_value(DriveEvent::Changed {
                folder_id: Some("d".into()),
            })
            .unwrap(),
        ),
        (
            "DriveEvent::Item",
            serde_json::to_value(DriveEvent::Item { item }).unwrap(),
        ),
        (
            "DriveEvent::Removed",
            serde_json::to_value(DriveEvent::Removed { id: "x".into() }).unwrap(),
        ),
        (
            "DriveEvent::Sync",
            serde_json::to_value(DriveEvent::Sync { sync }).unwrap(),
        ),
        (
            "DriveEvent::Storage",
            serde_json::to_value(DriveEvent::Storage { storage }).unwrap(),
        ),
    ];
    assert_eq!(dtos.len(), 17);
    for (name, value) in &dtos {
        assert_no_secret_fields(value, name);
    }
    // The wire names match the TypeScript contract.
    let listing = &dtos[1].1;
    assert!(listing.get("asOf").is_some() && listing["items"][0].get("isFolder").is_some());
    assert_eq!(dtos[12].1["kind"], "changed");
    assert_eq!(dtos[12].1["folderId"], "d");
    assert_eq!(dtos[6].1["phase"], "up-to-date");
}

#[test]
fn drive_v2_errors_carry_a_code_and_no_detail_from_outside() {
    assert_eq!(String::from(DriveError::NameTaken), "name-taken: drive");
    assert_eq!(DriveError::from_text("vault is locked"), DriveError::Locked);
    assert_eq!(
        DriveError::from_text("HTTP 404 for /api/v1/files/x"),
        DriveError::NotFound
    );
    assert_eq!(
        DriveError::from_text("error sending request for url"),
        DriveError::Offline
    );
    assert_eq!(DriveError::from_text("413 Payload Too Large"), DriveError::Quota);
    assert_eq!(
        String::from(DriveError::from_text("C:\\Users\\alice\\secret.txt broke")),
        "failed: request"
    );
}

#[test]
fn drive_v2_shared_lists_with_me_and_by_me() {
    let f = fixture();
    drive(&f);
    let with_me = model::shared_with_me(&f.db).unwrap();
    assert_eq!(with_me.len(), 1);
    assert_eq!(with_me[0].item.name, "Plans");
    assert_eq!(with_me[0].item.read_only, Some(true));
    assert_eq!(with_me[0].people[0].role, ShareRole::Viewer);

    let sent = serde_json::json!({ "invites": [
        { "file_id": "d0c5", "recipient_email": "bram@example.eu", "status": "approved", "permission": "write", "file_name_encrypted": "Y2lwaGVy" },
        { "file_id": "d0c5", "recipient_email": "noor@example.eu", "status": "pending", "permission": "read" },
        { "file_id": "a1", "recipient_email": "x@example.eu", "status": "revoked", "permission": "read" },
        { "file_id": "not-in-mirror", "recipient_email": "y@example.eu", "status": "approved", "permission": "read" }
    ]});
    let by_me = model::shared_by_me(&f.db, &sent).unwrap();
    assert_eq!(by_me.len(), 1, "revoked and unknown items left out");
    assert_eq!(
        by_me[0].item.name, "Docs",
        "the name comes from the mirror, never the invite's ciphertext"
    );
    assert_eq!(by_me[0].people.len(), 2);
    assert_eq!(by_me[0].people[0].role, ShareRole::Editor);
}

#[test]
fn drive_v2_sync_summary_phase() {
    let counts = |moving, conflicts| model::SyncCounts {
        on_device: 4,
        online_only: 4,
        moving,
        conflicts,
        on_hold: 1,
    };
    assert_eq!(
        model::sync_phase(true, "idle", false, &counts(0, 0)),
        SyncPhase::UpToDate
    );
    assert_eq!(
        model::sync_phase(true, "idle", false, &counts(2, 0)),
        SyncPhase::Syncing
    );
    assert_eq!(
        model::sync_phase(true, "running", false, &counts(0, 1)),
        SyncPhase::NeedsDecision
    );
    assert_eq!(model::sync_phase(true, "idle", true, &counts(0, 0)), SyncPhase::Paused);
    assert_eq!(
        model::sync_phase(true, "offline", false, &counts(0, 0)),
        SyncPhase::Offline
    );
    assert_eq!(model::sync_phase(true, "", false, &counts(0, 0)), SyncPhase::Checking);
    assert_eq!(
        model::sync_phase(false, "idle", false, &counts(0, 0)),
        SyncPhase::Stopped
    );
    let s = model::sync_summary(SyncPhase::UpToDate, &counts(2, 0), 3);
    assert_eq!(
        (s.on_device, s.available_offline, s.online_only, s.pending),
        (4, 3, 4, 3)
    );
}

/// system_files: desktop.ini / Thumbs.db rows already on the server are hidden from the listing, not deleted.
#[test]
fn system_files_are_hidden_from_listings_but_their_rows_stay() {
    let f = fixture();
    at(&f.db, "d1", "desktop.ini", None, false, true);
    at(&f.db, "d2", "Thumbs.db", None, false, true);
    at(&f.db, "keep", "report.pdf", None, false, true);
    let listed: Vec<String> = model::list(&f.db, None, Sort::default(), false)
        .unwrap()
        .items
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(listed, ["keep"]);
    let (paged, total) = model::children_of(&f.db, None, Sort::default(), 0, 50).unwrap();
    assert_eq!((paged.len(), total), (1, 1));
    assert!(
        f.db.get_file("d1").unwrap().is_some(),
        "the row is hidden, never deleted"
    );
}

/// Every HEIC/HEIF and camera RAW extension, in either case, maps to a mime the macOS ImageIO thumbnail decoder
/// (native/beebeeb-drive-rune/src/engine_host/thumbs_mac.rs) accepts, and counts as a photo (Photos, the meter).
#[test]
fn raw_and_heic_names_get_the_mime_the_thumbnail_decoder_matches_and_count_as_photos() {
    let table = [
        ("heic", "image/heic"),
        ("heif", "image/heif"),
        ("dng", "image/x-adobe-dng"),
        ("cr2", "image/x-canon-cr2"),
        ("cr3", "image/x-canon-cr3"),
        ("nef", "image/x-nikon-nef"),
        ("arw", "image/x-sony-arw"),
        ("raf", "image/x-fuji-raf"),
        ("orf", "image/x-olympus-orf"),
        ("rw2", "image/x-panasonic-rw2"),
    ];
    for (ext, mime) in table {
        for name in [
            format!("IMG_0001.{ext}"),
            format!("IMG_0001.{}", ext.to_ascii_uppercase()),
        ] {
            assert_eq!(model::mime_for_name(&name), Some(mime), "{name}");
            assert!(model::is_photo_or_video(&name), "{name} is a photo");
            assert!(model::has_thumbnail(&name), "{name} has a thumbnail");
            assert!(
                matches!(
                    model::storage_category(&format!("Camera/{name}"), &name, &[]),
                    model::StorageCategory::Photos
                ),
                "{name} counts under Photos"
            );
        }
    }
    for name in ["notes.xyz", "raw.raw2", "photo.CRW3", "IMG_0001", ".cr2"] {
        assert_eq!(model::mime_for_name(name), None, "{name} stays unknown");
        assert!(!model::is_photo_or_video(name), "{name} is no photo");
    }
    assert_eq!(
        model::mime_for_name("report.PDF"),
        Some("application/pdf"),
        "other extensions unchanged"
    );
    assert_eq!(
        model::mime_for_name("clip.MOV"),
        Some("video/quicktime"),
        "other extensions unchanged"
    );
}
