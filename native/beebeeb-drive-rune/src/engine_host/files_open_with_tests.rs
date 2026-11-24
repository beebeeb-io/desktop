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
use beebeeb_engine::test_api::fake_account::{start_fake, FakeAccount, TEST_PHRASE};
use zeroize::Zeroizing;

use super::*;
use crate::engine_host::EngineFilesSource;
use crate::frames::files::source::FilesSource;

// Review of #467: each start compares the session of its press inside the launch lock, a session end empties the
// launch worker's queue, and Open With asks the engine whether the copy is here.
#[path = "files_launch_tests.rs"]
mod files_launch_tests;

const PASSWORD: &str = "an invented password for open with";
const NOTES: &str = "a0000000-0000-4000-8000-000000000601";
const TOOL: &str = "a0000000-0000-4000-8000-000000000602";
const DOWNLOAD: &str = "a0000000-0000-4000-8000-000000000604";
const FOLDER: &str = "a0000000-0000-4000-8000-000000000603";

type Source = EngineFilesSource<MemoryCredentials>;

struct Set {
    engine: Arc<Engine<MemoryCredentials>>,
    source: Source,
    _dir: tempfile::TempDir,
    _fake: beebeeb_engine::test_api::FakeApi,
    account: Arc<FakeAccount>,
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
    put(&db, DOWNLOAD, "Invented download.txt", ItemKind::File);
    let owner = if owner_is_account {
        Identity::new(Some(&account.user_id), Some(&account.email))
    } else {
        Identity::new(Some("someone-else"), Some("someone-else@example.eu"))
    };
    db.set_owner(&owner).unwrap();
    drop(db);
    engine.use_local_state_for_test(path);
    // The sync folder is in this process's own temp home; a path below a folder that is not there is refused.
    let root = engine.sync_root().expect("a sync folder");
    std::fs::create_dir_all(&root).unwrap();
    // Notes is on this PC (as a placeholder or a copy): opening it fetches nothing. Download is not.
    std::fs::write(root.join("Invented notes.txt"), b"here").unwrap();
    let source = EngineFilesSource::new(engine.clone());
    Set {
        engine,
        source,
        _dir: dir,
        _fake: fake,
        account,
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
        choose_app(std::path::Path::new("an invented path.txt"), || true),
        Err(ActionError::NotOnThisSystem)
    );
}

/// Puts the file `id` in the engine's own local state with this status, kept on this PC (`Pinned`) or not.
fn put_kept(set: &Set, id: &str, name: &str, status: FileStatus, kept: bool) {
    use beebeeb_engine::state_db::PinState;
    let entry = FileEntry {
        file_id: id.to_string(),
        path: format!("/{name}"),
        status,
        size_bytes: 10,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: None,
        item_kind: ItemKind::File,
    };
    set.engine
        .local_state_for_changes()
        .expect("the account's local state")
        .with(|db| {
            db.upsert_file_with_contract(&entry, |contract| {
                contract.item_kind = ItemKind::File;
                contract.pin_state = if kept { PinState::Pinned } else { PinState::Unpinned };
            })
        })
        .expect("the account's local state")
        .expect("written");
}

/// Open With ▸ an app follows the default app's rule (dc-096): any file is handed over, online-only included (Cloud
/// Files hydrates it); off Windows the start then says it is not on this system. A folder is never handed over.
#[cfg(not(windows))]
#[test]
fn open_with_an_app_hands_over_any_file() {
    let mut set = set("a5-app", true, true);
    let _running = hold_running(&set.engine);
    let app = crate::frames::files::source::AppChoice {
        id: "invented.exe".into(),
        label: "Invented".into(),
    };
    let notes = row(NOTES, "Invented notes.txt", false);
    assert_eq!(
        set.source.open_in_app(&notes, &app),
        Err(ActionError::NotOnThisSystem),
        "online only"
    );
    put_kept(&set, NOTES, "Invented notes.txt", FileStatus::Local, true);
    assert_eq!(
        set.source.open_in_app(&notes, &app),
        Err(ActionError::NotOnThisSystem),
        "kept and here"
    );
    assert_eq!(
        set.source.open_in_app(&row(FOLDER, "Invented folder", true), &app),
        Err(ActionError::Failed)
    );
}

/// What the stand-in starts saw, in order. Each takes the launch lock, as the system's own do.
static STARTED: std::sync::Mutex<Vec<(&'static str, PathBuf)>> = std::sync::Mutex::new(Vec::new());

fn open_stand_in(path: &std::path::Path, still: &dyn Fn() -> bool) -> Result<(), ActionError> {
    launch::locked(|| still().then(|| STARTED.lock().unwrap().push(("open", path.to_path_buf()))));
    Ok(())
}

fn reveal_stand_in(path: &std::path::Path, still: &dyn Fn() -> bool) -> Result<(), ActionError> {
    launch::locked(|| still().then(|| STARTED.lock().unwrap().push(("reveal", path.to_path_buf()))));
    Ok(())
}

/// Ruling R-OW1: the window's thread never waits on the launch lock. While it is held (Windows' app chooser is open),
/// Files' Open, Show in File Explorer and Open in File Explorer return at once, and so does the window's poll
/// (`changes`); the starts run on the launch worker once the lock is let go, in the order asked.
#[test]
fn files_open_never_waits_on_the_launch_lock() {
    use std::time::{Duration, Instant};
    let mut set = set("a6-worker", true, true);
    let _running = hold_running(&set.engine);
    let root = set.engine.sync_root().expect("a sync folder");
    set.source.starts = files_launch::Starts {
        open: open_stand_in,
        reveal: reveal_stand_in,
        has_default_app: |_| true,
        app: None,
    };
    let (holding, held) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let chooser = std::thread::spawn(move || {
        launch::locked(|| {
            holding.send(()).unwrap();
            let _ = released.recv_timeout(Duration::from_secs(3));
        })
    });
    held.recv().unwrap();
    let notes = row(NOTES, "Invented notes.txt", false);
    let asked_at = Instant::now();
    assert_eq!(set.source.open(std::slice::from_ref(&notes)), Ok(()));
    assert_eq!(set.source.show_in_file_explorer(std::slice::from_ref(&notes)), Ok(()));
    assert_eq!(set.source.open_in_file_explorer(None), Ok(()));
    assert!(set.source.changes().is_empty());
    let took = asked_at.elapsed();
    assert!(
        took < Duration::from_millis(500),
        "the window's thread waited {took:?} for the lock"
    );
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        STARTED.lock().unwrap().is_empty(),
        "nothing starts while the lock is held"
    );
    release.send(()).unwrap();
    chooser.join().unwrap();
    let waited = Instant::now();
    while STARTED.lock().unwrap().len() < 3 && waited.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let started = STARTED.lock().unwrap().clone();
    let kinds: Vec<&str> = started.iter().map(|(kind, _)| *kind).collect();
    assert_eq!(kinds, ["open", "reveal", "open"], "in the order asked");
    assert!(started[0].1.ends_with("Invented notes.txt") && started[0].1.starts_with(&root));
    assert!(started[1].1.ends_with("Invented notes.txt"));
    assert_eq!(started[2].1, root, "Open in File Explorer: the sync folder");
}

/// What the download-first stand-in saw: the path it was handed and the bytes that were at it then.
static OPENED_COPY: std::sync::Mutex<Vec<(PathBuf, Vec<u8>)>> = std::sync::Mutex::new(Vec::new());

fn reading_open(path: &std::path::Path, still: &dyn Fn() -> bool) -> Result<(), ActionError> {
    launch::locked(|| {
        still().then(|| {
            OPENED_COPY
                .lock()
                .unwrap()
                .push((path.to_path_buf(), std::fs::read(path).unwrap_or_default()))
        })
    });
    Ok(())
}

/// DC-RD-e: double-clicking a file that is not on this PC (no file at its path) downloads it first and hands the app
/// the downloaded copy; it never hands over the missing path ("<root>/x does not exist").
#[test]
fn open_file_downloads_a_cloud_only_file_before_it_opens() {
    use std::time::{Duration, Instant};
    let mut set = set("rd-e-open", true, true);
    let _running = hold_running(&set.engine);
    let root = set.engine.sync_root().expect("a sync folder");
    set.account
        .add_file_with_content(DOWNLOAD, None, "Invented download.txt", b"invented bytes of the notes");
    set.source.starts = files_launch::Starts {
        open: reading_open,
        reveal: reveal_stand_in,
        has_default_app: |_| true,
        app: None,
    };
    assert!(!root.join("Invented download.txt").exists());
    let notes = row(DOWNLOAD, "Invented download.txt", false);
    assert_eq!(set.source.open(std::slice::from_ref(&notes)), Ok(()));
    let waited = Instant::now();
    let seen = loop {
        let seen = OPENED_COPY
            .lock()
            .unwrap()
            .iter()
            .find(|(p, _)| p.ends_with(format!("{DOWNLOAD}/Invented download.txt")))
            .cloned();
        if let Some(seen) = seen {
            break seen;
        }
        assert!(waited.elapsed() < Duration::from_secs(10), "the app was never started");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(!seen.0.starts_with(&root), "the copy is outside the Beebeeb folder");
    assert_eq!(seen.1, b"invented bytes of the notes", "the app sees the whole file");
    assert!(set.source.changes().is_empty(), "no failure was said");
}
