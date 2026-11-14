//! Two versions at the engine, against the fake account on loopback: which rows are listed and from what (the file
//! on disk, the server's current version), what is left out, what an answer refuses, and the session rule. Every
//! name is invented.

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
use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use crate::test_api::{FakeApi, Request};

const WAITING: &str = "a0000000-0000-4000-8000-000000000901";
const SYNCED: &str = "a0000000-0000-4000-8000-000000000902";
const GONE: &str = "a0000000-0000-4000-8000-000000000903";

type TestEngine = Engine<MemoryCredentials>;

struct Rig {
    engine: Arc<TestEngine>,
    account: Arc<FakeAccount>,
    fake: FakeApi,
    session: SessionGeneration,
    root: PathBuf,
    _dir: tempfile::TempDir,
}

fn put(db: &StateDb, id: &str, name: &str, status: FileStatus) {
    let entry = FileEntry {
        file_id: id.to_string(),
        path: name.to_string(),
        status,
        size_bytes: 10,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: None,
        item_kind: ItemKind::File,
    };
    db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
        .unwrap();
}

/// A session in memory, as a sign-in and an unlock leave it: the fake's own token and key.
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

fn rig() -> Rig {
    let (account, _phrase) = FakeAccount::register("conflicts-api@example.eu", "an invented password", TEST_PHRASE);
    let account = Arc::new(account);
    for (id, name) in [
        (WAITING, "Invented draft.txt"),
        (SYNCED, "Invented fine.txt"),
        (GONE, "Invented gone.txt"),
    ] {
        account.add_item(id, None, name, false, 10);
        account.add_version(id, &format!("{id}-old"), "2026-11-01T10:00:00Z", 11);
        account.add_version(id, &format!("{id}-new"), "2026-11-03T10:00:00Z", 2_048);
    }
    let handler = account_handler(account.clone());
    let fake = FakeApi::start(Arc::new(move |req: &Request| handler(req)));
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "conflicts-api-test-slot".into(),
        state_dir,
        api_base: fake.base.clone(),
    };
    let engine = Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"));
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("sync");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("Invented draft.txt"), b"my edit on this pc").unwrap();
    std::fs::write(root.join("Invented fine.txt"), b"fine").unwrap();
    let state_path = dir.path().join("state.db");
    let db = StateDb::open(&state_path).unwrap();
    put(&db, WAITING, "Invented draft.txt", FileStatus::Conflict);
    put(&db, SYNCED, "Invented fine.txt", FileStatus::Local);
    // In conflict, but its file is not on this PC: a side that cannot be read.
    put(&db, GONE, "Invented gone.txt", FileStatus::Conflict);
    db.set_owner(&Identity::new(Some(&account.user_id), Some(&account.email)))
        .unwrap();
    drop(db);
    engine.use_local_state_for_test(state_path);
    engine.use_sync_root_for_test(root.clone());
    let session = sign_in(&engine, &account);
    Rig {
        engine,
        account,
        fake,
        session,
        root,
        _dir: dir,
    }
}

fn requests(rig: &Rig) -> usize {
    rig.fake
        .requests()
        .iter()
        .map(Request::route)
        .filter(|route| route.contains("/versions") || route.contains("/files/"))
        .count()
}

#[test]
fn only_conflict_rows_are_listed_with_both_sides_read() {
    let rig = rig();
    let listed = rig.engine.conflicts_for(rig.session).expect("listed");
    assert_eq!(listed.session, rig.session);
    assert_eq!(listed.files.len(), 1, "one file waits and has both sides");
    let one = &listed.files[0];
    assert_eq!(one.file_id, WAITING);
    assert_eq!(one.name, "Invented draft.txt");
    assert_eq!(one.here_size, 18, "the file on this PC, by its length on disk");
    assert!(
        one.here_modified.ends_with('Z') && one.here_modified.len() == 20,
        "{}",
        one.here_modified
    );
    assert_eq!(
        one.there_created, "2026-11-03T10:00:00Z",
        "the server's current version"
    );
    assert_eq!(one.there_size, 2_048);
    assert_eq!(
        listed.unread, 1,
        "the file that is not on this PC is counted, not drawn"
    );
    assert!(!format!("{listed:?}").contains("Invented"), "a {{:?}} prints no name");
}

#[test]
fn nothing_is_asked_of_the_server_when_no_file_waits() {
    let rig = rig();
    {
        let db = StateDb::open(&rig.root.parent().unwrap().join("state.db")).unwrap();
        db.set_status(WAITING, FileStatus::Local).unwrap();
        db.set_status(GONE, FileStatus::Local).unwrap();
    }
    let listed = rig.engine.conflicts_for(rig.session).expect("listed");
    assert!(listed.files.is_empty() && listed.unread == 0);
    assert_eq!(requests(&rig), 0, "no request");
}

#[test]
fn a_file_that_does_not_wait_is_never_resolved_and_nothing_is_sent() {
    let rig = rig();
    for id in [SYNCED, "a0000000-0000-4000-8000-000000000999"] {
        for choice in [
            ConflictChoice::KeepHere,
            ConflictChoice::KeepThere,
            ConflictChoice::KeepBoth,
        ] {
            assert_eq!(
                rig.engine.resolve_conflict_for(rig.session, id, choice).unwrap_err(),
                ResolveRefused::NotWaiting,
                "{id} {choice:?}"
            );
        }
    }
    assert_eq!(requests(&rig), 0, "nothing was sent");
    assert_eq!(std::fs::read(rig.root.join("Invented fine.txt")).unwrap(), b"fine");
}

#[test]
fn a_session_that_moved_on_lists_and_resolves_nothing() {
    let rig = rig();
    sign_out(&rig.engine);
    let again = sign_in(&rig.engine, &rig.account);
    assert_ne!(again, rig.session);
    assert_eq!(
        rig.engine.conflicts_for(rig.session).unwrap_err(),
        ConflictsRefused::NotSignedIn
    );
    assert_eq!(
        rig.engine
            .resolve_conflict_for(rig.session, WAITING, ConflictChoice::KeepThere)
            .unwrap_err(),
        ResolveRefused::NotSignedIn
    );
    assert_eq!(requests(&rig), 0, "nothing was sent");
    assert_eq!(
        std::fs::read(rig.root.join("Invented draft.txt")).unwrap(),
        b"my edit on this pc",
        "this PC's file is untouched"
    );
    assert_eq!(
        std::fs::read_dir(&rig.root).unwrap().count(),
        2,
        "no copy was made either"
    );
}

/// This account's folder of replaced versions, as the engine names it.
fn replaced_folder(rig: &Rig) -> PathBuf {
    replaced_folder_in(
        &crate::state_paths::init_for_test(),
        &Identity::new(Some(&rig.account.user_id), Some(&rig.account.email)),
    )
    .expect("an account has a folder")
}

#[test]
fn a_safety_copy_is_one_per_file_and_a_failed_try_leaves_the_earlier_one() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("Invented draft.txt");
    let folder = dir.path().join("replaced").join("a-file-id");
    std::fs::write(&file, b"first edit").unwrap();
    let kept = SafetyCopy::make(&file, &folder).unwrap().keep().unwrap();
    assert_eq!(kept, folder.join("Invented draft.txt"));
    // A later Keep there of the same file that fails, with this PC's file untouched: the new copy goes, the
    // earlier one stays as it was.
    std::fs::write(&file, b"second edit").unwrap();
    assert!(SafetyCopy::make(&file, &folder).unwrap().discard_if_unchanged(&file));
    assert_eq!(std::fs::read(&kept).unwrap(), b"first edit");
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1, "one copy per file");
    // One that works replaces it: still one copy.
    let kept = SafetyCopy::make(&file, &folder).unwrap().keep().unwrap();
    assert_eq!(std::fs::read(&kept).unwrap(), b"second edit");
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1, "one copy per file");
    assert_eq!(
        std::fs::read(&file).unwrap(),
        b"second edit",
        "the file itself is untouched"
    );
}

#[test]
fn a_failed_keep_there_that_already_replaced_the_file_keeps_the_copy() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("Invented draft.txt");
    let folder = dir.path().join("replaced").join("a-file-id");
    std::fs::write(&file, b"this PC's only copy").unwrap();
    let copy = SafetyCopy::make(&file, &folder).unwrap();
    // The server's version landed, then something after it failed: the copy is all that is left of the edit.
    std::fs::write(&file, b"the server's version").unwrap();
    assert!(!copy.discard_if_unchanged(&file));
    assert_eq!(
        std::fs::read(folder.join("Invented draft.txt")).unwrap(),
        b"this PC's only copy"
    );
    // So does a file that cannot be read any more.
    let copy = SafetyCopy::make(&folder.join("Invented draft.txt"), &dir.path().join("other")).unwrap();
    assert!(!copy.discard_if_unchanged(&dir.path().join("gone.txt")));
    assert!(dir.path().join("other").join("Invented draft.txt").is_file());
}

#[test]
fn a_copy_left_by_a_run_that_ended_early_is_never_thrown_away_unread() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("Invented draft.txt");
    let folder = dir.path().join("replaced").join("a-file-id");
    // A run that ended after the server's version landed and before its copy was put in place.
    std::fs::write(&file, b"this PC's replaced edit").unwrap();
    let _left_behind = SafetyCopy::make(&file, &folder).unwrap();
    std::fs::write(&file, b"the server's version").unwrap();
    // The next Keep there of the file keeps that copy, then copies the file as it is now.
    let next = SafetyCopy::make(&file, &folder).unwrap();
    assert_eq!(
        std::fs::read(folder.join("Invented draft.txt")).unwrap(),
        b"this PC's replaced edit"
    );
    assert!(next.discard_if_unchanged(&file));
    assert_eq!(
        std::fs::read(folder.join("Invented draft.txt")).unwrap(),
        b"this PC's replaced edit"
    );
    // A leftover that is what the file still is, is a spare: it goes.
    let _left_behind = SafetyCopy::make(&file, &folder).unwrap();
    let again = SafetyCopy::make(&file, &folder).unwrap();
    assert!(again.discard_if_unchanged(&file));
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1, "the kept copy only");
}

/// What the sync folder holds, by name, sorted.
fn names_in(root: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Review of desktop#423, P2 #8: a Keep there that fails (here the server has no bytes to send for the file) leaves
/// the sync folder as it was, every time it is tried: no safety copy stays behind to be uploaded as a new file.
#[test]
fn a_keep_there_that_fails_leaves_the_sync_folder_as_it_was_every_time() {
    let rig = rig();
    let before = names_in(&rig.root);
    for _ in 0..2 {
        // A failed download marks the row; the file waits for a choice again when the conflict is found again.
        put(
            &StateDb::open(&rig._dir.path().join("state.db")).unwrap(),
            WAITING,
            "Invented draft.txt",
            FileStatus::Conflict,
        );
        assert_eq!(
            rig.engine
                .resolve_conflict_for(rig.session, WAITING, ConflictChoice::KeepThere)
                .unwrap_err(),
            ResolveRefused::Failed
        );
    }
    assert_eq!(names_in(&rig.root), before, "a copy is left in the sync folder");
    assert_eq!(
        std::fs::read(rig.root.join("Invented draft.txt")).unwrap(),
        b"my edit on this pc"
    );
    assert!(
        !replaced_folder(&rig).join(WAITING).exists(),
        "no copy is kept of a file that was not replaced"
    );
}

const WITH_BYTES: &str = "a0000000-0000-4000-8000-000000000904";

/// Review of desktop#423, P2 #8: Keep there that works ends as Keep there, not as Keep both: the sync folder holds
/// the server's version under the file's name and no second file (which would be uploaded as a new one).
#[test]
fn keep_there_never_leaves_a_second_file_in_the_sync_folder() {
    let rig = rig();
    rig.account
        .add_file_with_content(WITH_BYTES, None, "Invented plan.txt", b"the server's version");
    std::fs::write(rig.root.join("Invented plan.txt"), b"this PC's edit, never sent").unwrap();
    {
        let db = StateDb::open(&rig._dir.path().join("state.db")).unwrap();
        put(&db, WITH_BYTES, "Invented plan.txt", FileStatus::Conflict);
    }
    let before = names_in(&rig.root);
    rig.engine
        .resolve_conflict_for(rig.session, WITH_BYTES, ConflictChoice::KeepThere)
        .expect("keep there works");
    assert_eq!(
        std::fs::read(rig.root.join("Invented plan.txt")).unwrap(),
        b"the server's version"
    );
    assert_eq!(
        names_in(&rig.root),
        before,
        "the replaced version was left in the sync folder"
    );
    // This PC's version is where nothing syncs: the account's folder of replaced versions, one copy for the file.
    let kept = replaced_folder(&rig).join(WITH_BYTES);
    assert_eq!(names_in(&kept), vec!["Invented plan.txt".to_string()]);
    assert_eq!(
        std::fs::read(kept.join("Invented plan.txt")).unwrap(),
        b"this PC's edit, never sent"
    );
}
