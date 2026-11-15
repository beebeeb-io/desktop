//! Task 2530: a way out of a parked upload at the engine handle, for the session of the press: Keep the other keeps
//! this PC's bytes in "Changes not sent" before the write goes, a session that moved on settles nothing, Remove from
//! the list takes only a change with no bytes here. Against the fake account on loopback; nothing is sent. Every name
//! is invented.

use std::path::PathBuf;
use std::sync::Arc;

use super::*;
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::app::EngineConfig;
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use crate::test_api::FakeApi;
use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};

const FILE: &str = "a0000000-0000-4000-8000-000000002530";
const THIS_PC: &[u8] = b"my edit on this pc";

type TestEngine = Engine<MemoryCredentials>;

struct Rig {
    engine: Arc<TestEngine>,
    account: Arc<FakeAccount>,
    _fake: FakeApi,
    session: SessionGeneration,
    staged: PathBuf,
    kept: PathBuf,
    _dir: tempfile::TempDir,
}

fn sign_in(engine: &TestEngine, account: &FakeAccount) -> SessionGeneration {
    {
        let _writes = engine.writes.write().unwrap();
        engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        *engine.acct.session.lock().unwrap() = Some(Session {
            token: account.session_token.clone(),
            master_key: account.master_key,
            email: Some(account.email.clone()),
        });
        engine.remember_session_user_id(Some(account.user_id.clone()));
        engine.set_auth_present(true);
    }
    engine.session_turn()
}

fn sign_out(engine: &TestEngine) {
    engine.forget_session(SessionTransition::SignOut);
    engine.remember_session_user_id(None);
    engine.set_auth_present(false);
}

/// A viewer save of `Invented/notes.txt` on version 1, parked as `class`, its staged copy on disk unless `staged` is
/// false.
fn rig(class: &str, staged: bool) -> Rig {
    let (account, _phrase) = FakeAccount::register("parked-exit@example.eu", "an invented password", TEST_PHRASE);
    let account = Arc::new(account);
    let handler = account_handler(account.clone());
    let fake = FakeApi::start(Arc::new(move |req: &crate::test_api::Request| handler(req)));
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "parked-exit-test-slot".into(),
        state_dir: crate::state_paths::init_for_test(),
        api_base: fake.base.clone(),
    };
    let engine = Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"));
    let dir = tempfile::tempdir().unwrap();
    let staged_path = dir.path().join("staged-copy");
    if staged {
        std::fs::write(&staged_path, THIS_PC).unwrap();
    }
    let state_path = dir.path().join("state.db");
    let db = StateDb::open(&state_path).unwrap();
    db.upsert_file(&FileEntry {
        file_id: FILE.into(),
        path: "Invented/notes.txt".into(),
        status: FileStatus::Error,
        size_bytes: 10,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    db.test_execute(&format!(
        "UPDATE files SET current_version = 1 WHERE file_id = '{FILE}'"
    ))
    .unwrap();
    db.test_execute(&format!(
        "INSERT INTO operation_queue (op_id, kind, file_id, target_path, metadata_json, payload_path, base_version,
                                      attempts, max_attempts, next_retry_at, last_error, last_error_class,
                                      created_at, updated_at, write_id, write_origin)
         VALUES ('parked', 'upload_version', '{FILE}', 'Invented/notes.txt', '{{}}', '{}', 1,
                 25, 25, 0, '{class}', '{class}', 5, 5, 'w-parked', 'minted')",
        staged_path.to_string_lossy()
    ))
    .unwrap();
    db.set_owner(&Identity::new(Some(&account.user_id), Some(&account.email)))
        .unwrap();
    drop(db);
    engine.use_local_state_for_test(state_path);
    let session = sign_in(&engine, &account);
    Rig {
        engine,
        account,
        _fake: fake,
        session,
        staged: staged_path,
        kept: dir.path().join("Changes not sent"),
        _dir: dir,
    }
}

fn still_queued(rig: &Rig) -> bool {
    rig.engine
        .local_state()
        .unwrap()
        .with(|db| db.get_operation("parked").unwrap().is_some())
        .unwrap()
}

fn files_under(root: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}

#[test]
fn keep_the_other_keeps_this_pcs_bytes_in_changes_not_sent_then_the_write_goes() {
    let rig = rig("permission_denied", true);
    let done = rig
        .engine
        .settle_parked_with(
            rig.session,
            "parked",
            ParkedPick::KeepOther,
            Some(1),
            "PC",
            Some(&rig.kept),
        )
        .unwrap();
    let Settled::KeptOther { kept } = done else {
        panic!("kept the other: {done:?}")
    };
    assert_eq!(std::fs::read(&kept).unwrap(), THIS_PC);
    assert!(kept.starts_with(&rig.kept));
    assert!(
        kept.ends_with(PathBuf::from("Invented").join("notes.txt")),
        "under the file's own path"
    );
    assert!(!still_queued(&rig), "then the write goes");
}

#[test]
fn a_session_that_moved_on_settles_nothing() {
    let rig = rig("permission_denied", true);
    sign_out(&rig.engine);
    let again = sign_in(&rig.engine, &rig.account);
    assert_ne!(again, rig.session);
    for pick in [
        ParkedPick::KeepOther,
        ParkedPick::KeepBoth,
        ParkedPick::KeepThisPcs,
        ParkedPick::RemoveFromList,
    ] {
        assert_eq!(
            rig.engine
                .settle_parked_with(rig.session, "parked", pick, Some(1), "PC", Some(&rig.kept))
                .unwrap_err(),
            SettleRefused::NotSignedIn,
            "{pick:?}"
        );
    }
    assert!(still_queued(&rig));
    assert!(files_under(&rig.kept).is_empty(), "no copy was made");
}

#[test]
fn with_no_folder_to_keep_in_the_write_stays() {
    let rig = rig("permission_denied", true);
    assert_eq!(
        rig.engine
            .settle_parked_with(rig.session, "parked", ParkedPick::KeepOther, Some(1), "PC", None)
            .unwrap_err(),
        SettleRefused::CopyFailed
    );
    assert!(still_queued(&rig));
    assert_eq!(std::fs::read(&rig.staged).unwrap(), THIS_PC);
}

#[test]
fn a_choice_against_a_version_the_file_moved_past_changes_nothing() {
    let rig = rig("permission_denied", true);
    assert_eq!(
        rig.engine
            .settle_parked_with(
                rig.session,
                "parked",
                ParkedPick::KeepOther,
                Some(0),
                "PC",
                Some(&rig.kept)
            )
            .unwrap(),
        Settled::Moved
    );
    assert!(still_queued(&rig));
    assert!(files_under(&rig.kept).is_empty(), "a moved write keeps nothing either");
}

#[test]
fn remove_from_the_list_takes_only_a_change_with_no_bytes_here() {
    let with_bytes = rig("payload_missing", true);
    assert_eq!(
        with_bytes
            .engine
            .settle_parked_with(
                with_bytes.session,
                "parked",
                ParkedPick::RemoveFromList,
                None,
                "PC",
                None
            )
            .unwrap_err(),
        SettleRefused::BytesHere
    );
    assert!(still_queued(&with_bytes));

    let without = rig("payload_missing", false);
    let done =
        without
            .engine
            .settle_parked_with(without.session, "parked", ParkedPick::RemoveFromList, None, "PC", None);
    if cfg!(target_os = "macos") {
        assert_eq!(done.unwrap_err(), SettleRefused::BytesHere);
    } else {
        assert_eq!(done.unwrap(), Settled::Removed);
        assert!(!still_queued(&without));
    }
}

#[test]
fn a_kept_copy_is_whole_and_never_written_over() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("staged");
    std::fs::write(&source, THIS_PC).unwrap();
    let root = dir.path().join("Changes not sent");
    let first = keep_in_changes_not_sent(&root, "2026-11-15 at 14.10.00", "op", "A/b.txt", &source).unwrap();
    let second = keep_in_changes_not_sent(&root, "2026-11-15 at 14.10.00", "op", "A/b.txt", &source).unwrap();
    assert_eq!(first, root.join("2026-11-15 at 14.10.00").join("A").join("b.txt"));
    assert_eq!(second, root.join("2026-11-15 at 14.10.00 (2)").join("A").join("b.txt"));
    assert_eq!(std::fs::read(&first).unwrap(), THIS_PC);
    assert_eq!(std::fs::read(&second).unwrap(), THIS_PC);
    // A staged copy that is not a file is kept nowhere, and nothing is made for it.
    assert!(keep_in_changes_not_sent(&root, "dir", "op", "A/c.txt", dir.path()).is_err());
    assert!(!root.join("dir").exists());
}
