//! `Engine::backups_overview` / `Engine::set_backup_folder` with the engine's own parts: the rows from a real state
//! database in a folder of the test's own (layout, stops, pass marks), the six folders from invented locations, the
//! switch's database work (purge on off, the stop cleared on on), and a real handle (`Engine::open`, in-memory
//! credentials) that refuses both while no one is signed in, before anything is written. The shared config sandbox
//! is never written. Every name is invented.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::super::{Engine, EngineConfig};
use super::*;
use crate::account_binding::Identity;
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::signin::gate::GateState;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{BackupLayout, BackupPassRecord, BackupStop, OperationKind, PendingOperation};

fn owned_db(dir: &Path) -> StateDb {
    let db = StateDb::open(dir.join("state.db")).unwrap();
    db.set_owner(&Identity::new(Some("u-invented"), None)).unwrap();
    db
}

/// Each known folder at `<base>/<Key>`, not moved.
fn here(base: &Path) -> impl Fn(&str) -> KnownFolderPaths + '_ {
    move |key| KnownFolderPaths {
        default: Some(base.join(key)),
        current: Some(base.join(key)),
    }
}

fn row<'a>(overview: &'a BackupsOverview, key: &str) -> &'a BackupFolderRow {
    overview.folders.iter().find(|f| f.key == key).unwrap()
}

#[test]
fn the_six_known_folders_read_their_switch_their_stop_and_their_last_pass() {
    let base = PathBuf::from("/invented");
    let state = StateRead {
        root: Some(BackupsRootRow {
            root_id: "root-1".into(),
            device_folder: "Laptop".into(),
        }),
        stopped: vec!["music".into()],
        last_pass: BTreeMap::from([("documents".to_string(), 1_700_000_000_000)]),
        stops: vec![BackupStop {
            category: "music".into(),
            reason: "unknown".into(),
            at_ms: 5,
        }],
    };
    let enabled = vec!["documents".to_string(), "desktop".to_string()];
    let counted = std::cell::RefCell::new(Vec::new());
    let overview = overview_from(&enabled, &BTreeMap::new(), &state, here(&base), |path: &Path| {
        counted.borrow_mut().push(path.to_path_buf());
        Some(7)
    });
    assert_eq!(overview.folders.len(), 6);
    assert_eq!(overview.root, state.root);
    let documents = row(&overview, "documents");
    assert_eq!((documents.on, documents.state), (true, FolderState::On));
    assert_eq!(documents.last_pass_at_ms, Some(1_700_000_000_000));
    assert_eq!(documents.items, Some(7));
    assert_eq!(row(&overview, "music").state, FolderState::Stopped);
    assert!(
        row(&overview, "music").stop_words.is_some(),
        "a stopped folder carries core's words for why"
    );
    assert_eq!(
        row(&overview, "documents").stop_words,
        None,
        "a folder that is on has none"
    );
    assert_eq!(row(&overview, "videos").state, FolderState::Off);
    assert_eq!(row(&overview, "videos").last_pass_at_ms, None);
    assert_eq!(
        counted.borrow().len(),
        6,
        "every folder read from its default location is counted"
    );
}

#[test]
fn a_folder_onedrive_moved_waits_for_its_answer_and_is_never_counted_inside_onedrive() {
    let paths = |key: &str| KnownFolderPaths {
        default: Some(PathBuf::from("/invented/home").join(key)),
        current: Some(PathBuf::from("/invented/OneDrive").join(key)),
    };
    let enabled = vec!["documents".to_string(), "pictures".to_string()];
    let choices = BTreeMap::from([("pictures".to_string(), KnownFolderSource::OneDrive)]);
    let overview = overview_from(&enabled, &choices, &StateRead::default(), paths, |_: &Path| Some(3));
    assert_eq!(row(&overview, "documents").state, FolderState::Waiting);
    assert_eq!(row(&overview, "documents").items, None);
    assert_eq!(row(&overview, "pictures").state, FolderState::On);
    assert_eq!(
        row(&overview, "pictures").items,
        None,
        "OneDrive's folder is never listed"
    );
    assert_eq!(row(&overview, "music").state, FolderState::Off);
    assert_eq!(overview.root, None);
}

#[test]
fn a_computer_without_known_folders_lists_six_unavailable_rows() {
    let overview = overview_from(
        &["documents".to_string()],
        &BTreeMap::new(),
        &StateRead::default(),
        |_| KnownFolderPaths::default(),
        |_: &Path| Some(1),
    );
    assert_eq!(overview.folders.len(), 6);
    assert!(
        overview
            .folders
            .iter()
            .all(|f| f.state == FolderState::Unavailable && f.items.is_none())
    );
    assert!(row(&overview, "documents").on);
}

#[test]
fn read_state_reads_the_layout_the_stops_and_the_marks_of_the_owner() {
    let dir = tempfile::tempdir().unwrap();
    let db = owned_db(dir.path());
    assert_eq!(
        read_state(&db).unwrap(),
        StateRead::default(),
        "an empty state reads as nothing"
    );

    db.set_backup_layout(&BackupLayout {
        root_id: "0193b1a6-0000-7000-8000-000000000001".into(),
        device_folder_id: "0193b1a6-0000-7000-8000-000000000002".into(),
        device_folder_name: "Laptop (a3f2)".into(),
    })
    .unwrap();
    db.record_backup_stop(&BackupStop {
        category: "music".into(),
        reason: "unknown".into(),
        at_ms: 5,
    })
    .unwrap();
    db.record_backup_pass(&BackupPassRecord {
        category: "documents".into(),
        last_pass_at_ms: 1_700_000_000_000,
        acknowledged: Vec::new(),
    })
    .unwrap();
    let read = read_state(&db).unwrap();
    assert_eq!(
        read.root,
        Some(BackupsRootRow {
            root_id: "0193b1a6-0000-7000-8000-000000000001".into(),
            device_folder: "Laptop (a3f2)".into(),
        })
    );
    assert_eq!(read.stopped, vec!["music".to_string()]);
    assert_eq!(read.last_pass.get("documents"), Some(&1_700_000_000_000));
    assert_eq!(read.last_pass.len(), 1);
}

fn backup_op(id: &str, key: &str) -> PendingOperation {
    PendingOperation {
        op_id: id.into(),
        kind: OperationKind::UploadVersion,
        file_id: Some(format!("file-{id}")),
        parent_id: None,
        target_path: Some(format!("/{id}.bin")),
        metadata_json: Some(r#"{"operation":"upload_version"}"#.into()),
        payload_path: None,
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 5,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: Some(key.into()),
        created_at: 100,
        updated_at: 100,
    }
}

#[test]
fn switching_off_purges_only_that_folders_pending_uploads() {
    let dir = tempfile::tempdir().unwrap();
    let db = owned_db(dir.path());
    db.enqueue_operation(&backup_op("m-1", "music")).unwrap();
    db.enqueue_operation(&backup_op("p-1", "pictures")).unwrap();
    apply_switch(&REAL, &db, "music", false).unwrap();
    let left: Vec<String> = db
        .list_due_operations(999)
        .unwrap()
        .into_iter()
        .map(|op| op.op_id)
        .collect();
    assert_eq!(left, vec!["p-1".to_string()]);
}

#[test]
fn switching_on_clears_the_stop_back_up_again() {
    let dir = tempfile::tempdir().unwrap();
    let db = owned_db(dir.path());
    db.record_backup_stop(&BackupStop {
        category: "music".into(),
        reason: "unknown".into(),
        at_ms: 5,
    })
    .unwrap();
    db.enqueue_operation(&backup_op("m-1", "music")).unwrap();
    apply_switch(&REAL, &db, "music", true).unwrap();
    assert!(db.backup_stops().unwrap().is_empty(), "the stop is cleared");
    assert_eq!(
        db.list_due_operations(999).unwrap().len(),
        1,
        "switching on purges nothing"
    );
}

#[test]
fn a_handle_with_no_one_signed_in_refuses_the_read_and_the_switch_and_writes_nothing() {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-slot".into(),
        state_dir,
        api_base: "http://127.0.0.1:9".into(),
    };
    let engine = Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"));
    assert_eq!(engine.view().gate, GateState::SignedOut);
    let before = DesktopConfig::load_for(&REAL).ok().map(|c| c.known_folder_backup);
    assert_eq!(
        engine.backups_overview(),
        Err(BackupsError::Refused(LocalStateRefused::SignedOut))
    );
    assert_eq!(
        engine.set_backup_folder("music", true),
        Err(BackupsError::Refused(LocalStateRefused::SignedOut))
    );
    assert_eq!(
        engine.set_backup_folder("not-a-folder", true),
        Err(BackupsError::UnknownFolder)
    );
    let after = DesktopConfig::load_for(&REAL).ok().map(|c| c.known_folder_backup);
    assert_eq!(before, after, "the settings file is not written for a refusal");
}

#[test]
fn a_switch_whose_database_part_fails_puts_the_settings_back() {
    // The settings file is modelled by a cell: written first, as `set_backup_folder` does, then undone.
    let file = std::cell::Cell::new(true);
    let failed = switched_or_undone(
        || {
            let was = file.get();
            file.set(false);
            Ok(was)
        },
        || Err(BackupsError::State),
        |was| file.set(was),
    );
    assert_eq!(failed, Err(BackupsError::State));
    assert!(
        file.get(),
        "a failed purge leaves the folder as it was: on, nothing half done"
    );

    let session_moved = switched_or_undone(
        || {
            let was = file.get();
            file.set(false);
            Ok(was)
        },
        || Err(BackupsError::Refused(LocalStateRefused::SessionChanged)),
        |was| file.set(was),
    );
    assert!(session_moved.is_err());
    assert!(file.get(), "a session that changed in between undoes it too");

    let done = switched_or_undone(
        || {
            let was = file.get();
            file.set(false);
            Ok(was)
        },
        || Ok(()),
        |was| file.set(was),
    );
    assert_eq!(done, Ok(()));
    assert!(!file.get(), "a switch that happened stays");

    let refused_write = switched_or_undone(
        || Err::<bool, _>(BackupsError::Settings),
        || panic!("no database work after a write that failed"),
        |_| panic!("nothing to undo"),
    );
    assert_eq!(refused_write, Err(BackupsError::Settings));
}
