//! "Open in another app…" on the real engine (`EngineFilesSource::open_with`): the path is resolved under the
//! account-bound handle before the chooser is asked, and a refusal asks it never. The chooser is a counting stand-in;
//! nothing here shows a dialog. Engine handles are on the process's state folder, which nothing here writes in; the
//! account's local state is a database of each test's own (`Engine::use_local_state_for_test`). Every name is invented.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;

use beebeeb_engine::account_binding::Identity;
use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::runner::EngineRunner;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use beebeeb_engine::sync_view::Start;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, start_fake};
use zeroize::Zeroizing;

use super::*;
use crate::engine_host::EngineFilesSource;
use crate::frames::files::source::FilesSource;

const PASSWORD: &str = "an invented password for open with";
const NOTES: &str = "a0000000-0000-4000-8000-000000000601";
const TOOL: &str = "a0000000-0000-4000-8000-000000000602";
const FOLDER: &str = "a0000000-0000-4000-8000-000000000603";

type Source = EngineFilesSource<MemoryCredentials>;

struct Set {
    engine: Arc<Engine<MemoryCredentials>>,
    source: Source,
    _dir: tempfile::TempDir,
    _fake: beebeeb_engine::test_api::FakeApi,
}

fn put(db: &StateDb, id: &str, name: &str, kind: ItemKind) {
    let entry = FileEntry {
        file_id: id.to_string(),
        path: format!("/{name}"),
        status: FileStatus::CloudOnly,
        size_bytes: 10,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: None,
        item_kind: kind,
    };
    db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
        .unwrap();
}

/// A signed-in engine whose local state holds a text file, a program and a folder; `owner_is_account` false: the
/// state belongs to someone else.
fn set(tag: &str, signed_in: bool, owner_is_account: bool) -> Set {
    let (account, phrase) = FakeAccount::register(&format!("open-with-{tag}@example.eu"), PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let fake = start_fake(account.clone());
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: format!("open-with-slot-{tag}"),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: fake.base.clone(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    let engine = Arc::new(engine);
    if signed_in {
        engine
            .sign_in_password(&account.email, Zeroizing::new(PASSWORD.into()))
            .expect("the fake accepts the password");
        engine
            .unlock_with_phrase(Zeroizing::new(phrase))
            .expect("the phrase unlocks");
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    put(&db, NOTES, "Invented notes.txt", ItemKind::File);
    put(&db, TOOL, "invented tool.exe", ItemKind::File);
    put(&db, FOLDER, "Invented folder", ItemKind::Folder);
    let owner = if owner_is_account {
        Identity::new(Some(&account.user_id), Some(&account.email))
    } else {
        Identity::new(Some("someone-else"), Some("someone-else@example.eu"))
    };
    db.set_owner(&owner).unwrap();
    drop(db);
    engine.use_local_state_for_test(path);
    // The sync folder is in this process's own temp home; a path below a folder that is not there is refused.
    std::fs::create_dir_all(engine.sync_root().expect("a sync folder")).unwrap();
    let source = EngineFilesSource::new(engine.clone());
    Set {
        engine,
        source,
        _dir: dir,
        _fake: fake,
    }
}

/// The engine's slot holds a task that never runs and the view says sync runs (as the tray's tests do).
fn hold_running(engine: &Engine<MemoryCredentials>) -> tokio::runtime::Runtime {
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let task = runtime.spawn(std::future::pending::<()>());
    let account = engine.accounts().active_account().expect("the account");
    *account.engine.try_lock().expect("the engine slot is free") = Some(EngineRunner::for_test_with_task(task));
    engine.set_view_for_test(|view| view.start = Start::Running);
    runtime
}

fn row(id: &str, name: &str, is_folder: bool) -> DriveItem {
    DriveItem {
        id: id.to_string(),
        parent_id: None,
        name: name.to_string(),
        is_folder,
        size: Some(10),
        child_count: None,
        modified_at: String::new(),
        mime: None,
        has_thumbnail: false,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: None,
        is_protected: false,
    }
}

/// Asks `open_with_by` with a recording chooser: what it answered and the paths the chooser was asked for.
fn ask(set: &Set, item: &DriveItem) -> (Result<(), ActionError>, Vec<PathBuf>) {
    let asked = RefCell::new(Vec::new());
    let answer = set.source.open_with_by(item, |path| {
        asked.borrow_mut().push(path.to_path_buf());
        Ok(())
    });
    (answer, asked.into_inner())
}

#[test]
fn open_with_asks_the_chooser_for_the_path_under_the_account_folder() {
    let set = set("a1", true, true);
    let _running = hold_running(&set.engine);
    let root = set.engine.sync_root().expect("a sync folder");
    let (answer, asked) = ask(&set, &row(NOTES, "Invented notes.txt", false));
    assert_eq!(answer, Ok(()));
    assert_eq!(asked.len(), 1, "the chooser is asked once");
    assert!(asked[0].starts_with(&root), "{:?} is below {root:?}", asked[0]);
    assert!(asked[0].ends_with("Invented notes.txt"));
    // A folder is not handed to a chooser.
    assert_eq!(
        ask(&set, &row(FOLDER, "Invented folder", true)),
        (Err(ActionError::Failed), vec![])
    );
}

#[test]
fn a_kind_the_engine_does_not_hand_over_never_reaches_the_chooser() {
    let set = set("a2", true, true);
    let _running = hold_running(&set.engine);
    assert_eq!(
        ask(&set, &row(TOOL, "invented tool.exe", false)),
        (Err(ActionError::Failed), vec![]),
        "refused, the chooser never asked"
    );
}

#[test]
fn open_with_is_refused_without_sync_and_without_a_session() {
    // No sync running.
    let no_sync = set("a3-sync", true, true);
    assert_eq!(
        ask(&no_sync, &row(NOTES, "Invented notes.txt", false)),
        (Err(ActionError::Unavailable), vec![])
    );
    // Sync runs, no session.
    let no_session = set("a3-session", false, true);
    let _running = hold_running(&no_session.engine);
    assert_eq!(
        ask(&no_session, &row(NOTES, "Invented notes.txt", false)),
        (Err(ActionError::Unavailable), vec![])
    );
    // Sync runs and a session is here, over another account's local state.
    let other = set("a3-other", true, false);
    let _running_too = hold_running(&other.engine);
    assert_eq!(
        ask(&other, &row(NOTES, "Invented notes.txt", false)),
        (Err(ActionError::Unavailable), vec![])
    );
    // The trait method reaches the same refusal, and off Windows says it is not here once past the checks.
    let mut source = set("a3-trait", true, true);
    assert_eq!(
        source.source.open_with(&row(NOTES, "Invented notes.txt", false)),
        Err(ActionError::Unavailable)
    );
}

#[cfg(not(windows))]
#[test]
fn off_windows_the_chooser_says_not_on_this_system() {
    assert_eq!(
        choose_app(std::path::Path::new("an invented path.txt")),
        Err(ActionError::NotOnThisSystem)
    );
}
