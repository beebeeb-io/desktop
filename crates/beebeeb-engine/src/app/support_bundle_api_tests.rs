//! `Engine::export_support_bundle_into` end to end: a signed-in handle, a state database of the test's own with a
//! queued upload that failed, a sync folder, and the file the bundle writes read back. Ported from the old shell
//! crate's `diagnostics_export_tests.rs` (its mail-draft, partial-success and off-main-thread tests were Tauri-only).
//! Every name is invented.

use std::path::{Path, PathBuf};

use super::super::{Engine, EngineConfig};
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, FileStatus, ItemKind, OperationKind, PendingOperation, StateDb};

type TestEngine = Engine<MemoryCredentials>;

const OWNER: &str = "bundle-owner@example.eu";

fn signed_in_engine(slot: &str) -> TestEngine {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: slot.into(),
        state_dir,
        api_base: "http://127.0.0.1:9".into(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the handle opens");
    {
        let _writes = engine.writes.write().unwrap();
        engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        *engine.acct.session.lock().unwrap() = Some(Session {
            token: "an-invented-token".into(),
            master_key: [7; 32],
            email: Some(OWNER.into()),
        });
        engine.set_auth_present(true);
    }
    engine
}

/// A state database with one file at `name_path` and one queued upload of it that failed with `error`; the engine
/// reads it, and `root` is its configured sync folder.
fn seeded(engine: &TestEngine, dir: &Path, name_path: &str, error: &str, root: &Path) {
    let db_path = dir.join("state.db");
    let db = StateDb::open(&db_path).unwrap();
    db.upsert_file(&FileEntry {
        file_id: "f1".into(),
        path: name_path.into(),
        status: FileStatus::Local,
        size_bytes: 1,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    db.enqueue_operation(&PendingOperation {
        op_id: "op-1".into(),
        kind: OperationKind::UploadVersion,
        file_id: Some("f1".into()),
        parent_id: None,
        target_path: Some(name_path.into()),
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
    })
    .unwrap();
    db.record_operation_attempt("op-1", 1, 10, Some(error)).unwrap();
    db.set_owner(&Identity::new(None, Some(OWNER))).unwrap();
    engine.use_local_state_for_test(db_path);
    engine.use_sync_root_for_test(root.to_path_buf());
}

/// The bundle the engine writes for that state, as a person would open it.
fn exported(engine: &TestEngine, folder: &Path) -> (PathBuf, serde_json::Value) {
    let saved = engine
        .export_support_bundle_into(engine.session_turn(), folder)
        .expect("the bundle is saved");
    let text = std::fs::read_to_string(&saved.path).unwrap();
    (saved.path, serde_json::from_str(&text).unwrap())
}

/// Ported from `bundle_has_no_path_or_known_name_and_reports_the_redaction_count` (task 1685): the whole file the user
/// reads before sending it, not one struct.
#[test]
fn bundle_has_no_path_or_known_name_and_reports_the_redaction_count() {
    let engine = signed_in_engine("an-invented-bundle-slot-1");
    let dir = tempfile::tempdir().unwrap();
    let root = Path::new("/Users/alice/Library/CloudStorage/Beebeeb-Drive");
    seeded(
        &engine,
        dir.path(),
        "Tax 2025/aangifte.pdf",
        "/Users/alice/Library/CloudStorage/Beebeeb-Drive/Tax 2025/aangifte.pdf failed Bearer abc.def.ghi",
        root,
    );
    let (path, bundle) = exported(&engine, dir.path());
    let text = std::fs::read_to_string(path).unwrap();
    for leaked in [
        "alice",
        "Users",
        "CloudStorage",
        "Beebeeb-Drive",
        "Tax 2025",
        "Tax",
        "aangifte",
        "abc.def.ghi",
    ] {
        assert!(!text.contains(leaked), "{leaked:?} leaked into the bundle:\n{text}");
    }
    assert_eq!(bundle["format"], 1);
    assert_eq!(bundle["queue"]["last_error_code"], "other");
    assert!(
        bundle["queue"]["last_error_redactions"].as_u64().unwrap() >= 2,
        "a path and a token were removed, the count must say so:\n{text}"
    );
    assert!(bundle["queue"]["last_error"].as_str().unwrap().contains("failed"));
}

/// Ported from `sync_root_components_are_scrubbed_even_when_the_error_names_only_the_root`: the error text names the
/// sync folder's own names with no path around them.
#[test]
fn sync_root_components_are_scrubbed_even_when_the_error_names_only_the_root() {
    let engine = signed_in_engine("an-invented-bundle-slot-2");
    let dir = tempfile::tempdir().unwrap();
    let root = Path::new("/Users/alice/Library/CloudStorage/Beebeeb-Drive");
    seeded(
        &engine,
        dir.path(),
        "a.txt",
        "sync root Beebeeb-Drive alice is not writable",
        root,
    );
    let (path, _) = exported(&engine, dir.path());
    let text = std::fs::read_to_string(path).unwrap();
    assert!(!text.contains("alice"), "{text}");
    assert!(!text.contains("Beebeeb-Drive"), "{text}");
}

/// Ported from `sync_root_components_reach_the_name_scan`: `folder` is an ordinary error word, so only the name scan
/// (fed the sync root) can remove it. This proves the extra-path plumbing of the export.
#[test]
fn sync_root_components_reach_the_name_scan() {
    let engine = signed_in_engine("an-invented-bundle-slot-3");
    let dir = tempfile::tempdir().unwrap();
    seeded(
        &engine,
        dir.path(),
        "a.txt",
        "sync root folder is not allowed",
        Path::new("/srv/folder"),
    );
    let (_, bundle) = exported(&engine, dir.path());
    let text = bundle["queue"]["last_error"].as_str().unwrap().to_string();
    assert_eq!(text, "sync root [name] is not allowed", "{text}");
}
