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
use crate::identity::TEST;
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
    let engine = Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the handle opens"));
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
        let db = StateDb::open(rig.root.parent().unwrap().join("state.db")).unwrap();
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

/// The first run: no state database yet. Nothing waits; that is an answer, not a read that failed (RD-g: the
/// Files window said "the files that wait for a choice could not be read" on every start).
#[test]
fn choice_list_when_nothing_synced_yet_is_empty_not_a_failure() {
    let rig = rig();
    rig.engine
        .use_local_state_for_test(rig.root.parent().unwrap().join("not-made-yet.db"));
    let listed = rig
        .engine
        .conflicts_for(rig.session)
        .expect("an empty list, not a refusal");
    assert!(listed.files.is_empty() && listed.unread == 0);
    assert_eq!(listed.session, rig.session);
    assert_eq!(requests(&rig), 0, "no request");
}

/// A database made and not yet owned (a sync that has not written its first row): nothing waits either.
#[test]
fn choice_list_of_an_empty_state_with_no_owner_is_empty_not_a_failure() {
    let rig = rig();
    let path = rig.root.parent().unwrap().join("fresh.db");
    drop(StateDb::open(&path).unwrap());
    rig.engine.use_local_state_for_test(path);
    let listed = rig
        .engine
        .conflicts_for(rig.session)
        .expect("an empty list, not a refusal");
    assert!(listed.files.is_empty() && listed.unread == 0);
}

/// A start behind the screen lock: a person reads nothing, so nothing is listed, and it is no failure.
#[test]
fn choice_list_behind_a_locked_screen_is_empty_not_a_failure() {
    let rig = rig();
    rig.engine.lock_screen().expect("the screen locks");
    let now = rig.engine.session_turn();
    let listed = rig.engine.conflicts_for(now).expect("an empty list, not a refusal");
    assert!(
        listed.files.is_empty() && listed.unread == 0,
        "nothing is shown behind the lock"
    );
    assert_eq!(requests(&rig), 0, "no request");
    assert_eq!(
        rig.engine.conflicts_for(rig.session).unwrap_err(),
        ConflictsRefused::NotSignedIn,
        "the session from before the lock gets nothing"
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
fn a_later_different_copy_is_kept_beside_the_earlier_one_and_the_same_bytes_once() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("Invented draft.txt");
    let folder = dir.path().join("replaced").join("a-file-id");
    std::fs::write(&file, b"first edit").unwrap();
    let kept = SafetyCopy::make(&file, &folder).unwrap().keep().unwrap();
    assert_eq!(kept, folder.join("Invented draft.txt"));
    // A later Keep there of the same file, worked or failed (the copy is kept either way): a different edit is
    // kept beside the first, which stays as it was.
    std::fs::write(&file, b"second edit").unwrap();
    let second = SafetyCopy::make(&file, &folder).unwrap().keep().unwrap();
    assert_eq!(std::fs::read(&second).unwrap(), b"second edit");
    assert_eq!(
        std::fs::read(&kept).unwrap(),
        b"first edit",
        "the earlier copy is never deleted"
    );
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 2);
    // The same bytes again are kept once.
    let again = SafetyCopy::make(&file, &folder).unwrap().keep().unwrap();
    assert_eq!(again, second);
    assert_eq!(
        std::fs::read_dir(&folder).unwrap().count(),
        2,
        "the same edit is one copy"
    );
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
    copy.keep().unwrap();
    assert_eq!(
        std::fs::read(folder.join("Invented draft.txt")).unwrap(),
        b"this PC's only copy"
    );
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
    drop(next);
    assert_eq!(
        std::fs::read(folder.join("Invented draft.txt")).unwrap(),
        b"this PC's replaced edit"
    );
    // A leftover that is what the file still is, is a spare: it goes (the kept copy and the new incoming one stay).
    let _left_behind = SafetyCopy::make(&file, &folder).unwrap();
    let again = SafetyCopy::make(&file, &folder).unwrap();
    drop(again);
    assert_eq!(
        std::fs::read_dir(&folder).unwrap().count(),
        2,
        "the kept copy and the one incoming copy"
    );
}

#[test]
fn a_kept_copys_folder_names_no_id_and_stays_inside_the_accounts_folder() {
    let account = std::path::Path::new("replaced-account");
    for id in [WAITING, "../../outside", r"..\..\outside", "/abs/path", "C:\\Windows"] {
        let folder = replaced_folder_of(account, id);
        assert_eq!(folder.parent(), Some(account), "{id}");
        let name = folder.file_name().unwrap().to_str().unwrap();
        assert_eq!(name.len(), 16, "{id}");
        assert!(name.chars().all(|c| c.is_ascii_hexdigit()), "{id}");
    }
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

/// A Keep there that fails (here the server has no bytes to send for the file) leaves
/// the sync folder as it was, every time it is tried: no safety copy stays behind to be uploaded as a new file. The
/// copy is kept in the engine's own folder, one per file.
#[test]
fn a_keep_there_that_fails_leaves_the_sync_folder_as_it_was_every_time() {
    let rig = rig();
    let before = names_in(&rig.root);
    for _ in 0..2 {
        // A failed download marks the row; the file waits for a choice again when the conflict is found again.
        put(
            &StateDb::open(rig._dir.path().join("state.db")).unwrap(),
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
    let kept = replaced_folder_of(&replaced_folder(&rig), WAITING);
    assert_eq!(
        std::fs::read(kept.join("Invented draft.txt")).unwrap(),
        b"my edit on this pc",
        "the copy is kept after a failed Keep there"
    );
    assert_eq!(
        std::fs::read_dir(&kept).unwrap().count(),
        1,
        "one copy per file, tried twice"
    );
}

/// A Keep there that fails never deletes an EARLIER kept copy. Two failed tries with two
/// different edits of one file leave both edits on disk (one copy per different edit); the same edit tried twice is
/// one copy.
#[test]
fn two_failed_keep_there_tries_with_different_edits_keep_both_edits() {
    let rig = rig();
    let try_keep_there = |rig: &Rig| {
        put(
            &StateDb::open(rig._dir.path().join("state.db")).unwrap(),
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
    };
    try_keep_there(&rig);
    std::fs::write(rig.root.join("Invented draft.txt"), b"a second, different edit").unwrap();
    try_keep_there(&rig);
    try_keep_there(&rig);
    let kept = replaced_folder_of(&replaced_folder(&rig), WAITING);
    let mut bytes: Vec<Vec<u8>> = std::fs::read_dir(&kept)
        .unwrap()
        .map(|entry| std::fs::read(entry.unwrap().path()).unwrap())
        .collect();
    bytes.sort();
    assert_eq!(
        bytes,
        vec![b"a second, different edit".to_vec(), b"my edit on this pc".to_vec()],
        "both edits are on disk, the repeated one once"
    );
}

const WITH_BYTES: &str = "a0000000-0000-4000-8000-000000000904";

/// Keep there that works ends as Keep there, not as Keep both: the sync folder holds
/// the server's version under the file's name and no second file (which would be uploaded as a new one).
#[test]
fn keep_there_never_leaves_a_second_file_in_the_sync_folder() {
    let rig = rig();
    rig.account
        .add_file_with_content(WITH_BYTES, None, "Invented plan.txt", b"the server's version");
    std::fs::write(rig.root.join("Invented plan.txt"), b"this PC's edit, never sent").unwrap();
    {
        let db = StateDb::open(rig._dir.path().join("state.db")).unwrap();
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
    let kept = replaced_folder_of(&replaced_folder(&rig), WITH_BYTES);
    assert_eq!(names_in(&kept), vec!["Invented plan.txt".to_string()]);
    assert_eq!(
        std::fs::read(kept.join("Invented plan.txt")).unwrap(),
        b"this PC's edit, never sent"
    );
}

/// S6 (R9): Keep there takes the hydrate's start look before its safety copy reads a byte, and hands that look to the
/// download: never the plain `resolve_keep_theirs`, which would look again after the copy (a save between the two
/// would then be replaced). `resolve_conflict_for` needs a signed-in engine and a server, so the order is asserted on
/// the source, on every platform; the behaviour is `engine_bridge/tests/start_look_s6.rs` (Linux).
#[test]
fn s6_start_keep_there_looks_before_its_safety_copy() {
    let source = include_str!("conflicts_api.rs").replace("\r\n", "\n");
    let start = source
        .find("    pub fn resolve_conflict_for(")
        .expect("Keep there's fn");
    let body = &source[start..start + source[start..].find("\n    }\n").unwrap()];
    let at = |needle: &str| {
        body.find(needle)
            .unwrap_or_else(|| panic!("Keep there's fn has {needle}"))
    };
    assert_eq!(body.matches("PlaceSeen::take(").count(), 1, "one look");
    assert!(
        at("PlaceSeen::take(") < at("SafetyCopy::make("),
        "the look before the safety copy"
    );
    assert!(
        body.contains("bridge.resolve_theirs_seen(file_id, &root, seen)"),
        "the look is handed to the download"
    );
    assert!(
        !body.contains("resolve_keep_theirs("),
        "never the call that looks again"
    );
}
