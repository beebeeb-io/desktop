//! Where the rows stand in the Backups tree on the real engine (`EngineFilesSource::folder_shown` / `protection` /
//! `backup_entry`), against the engine's fake accounts on loopback and in the Files window's element tree: what is
//! answered for an item, a folder and a row dropped on, nothing before the engine answered, and the session rule: an
//! answer belongs to the session and the account it was asked under. The local state is a database of each test's
//! own (`Engine::use_local_state_for_test`), recorded as the first account's. Every name and id is invented.

use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use beebeeb_engine::account_binding::Identity;
use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::runner::EngineRunner;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_db::{BackupLayout, FileEntry, FileStatus, ItemKind, StateDb};
use beebeeb_engine::sync_view::Start;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{FakeApi, Handler, Request};
use zeroize::Zeroizing;

use super::super::files_window_support::{drawn, settle, until_drawn, window};
use super::*;
use crate::engine_host::EngineFilesSource;
use crate::frames::files::source::{ActionError, FilesSource};

const PASSWORD: &str = "an invented password for places";
/// A second valid 12-word phrase (a BIP-39 test vector), for the second account.
const OTHER_PHRASE: &str = "legal winner thank year wave sausage worth useful legal winner thank yellow";
const WAIT: Duration = Duration::from_secs(30);

const ROOT: &str = "d0000000-0000-4000-8000-000000000001";
const THIS_PC: &str = "d0000000-0000-4000-8000-000000000002";
const OTHER_PC: &str = "d0000000-0000-4000-8000-000000000003";
const DOCS: &str = "d0000000-0000-4000-8000-000000000004";
const NOTE: &str = "d0000000-0000-4000-8000-000000000005";
const PLAIN: &str = "d0000000-0000-4000-8000-000000000007";
const PLAIN_FILE: &str = "d0000000-0000-4000-8000-000000000008";
const UNHELD: &str = "d0000000-0000-4000-8000-0000000000ff";

type Source = EngineFilesSource<MemoryCredentials>;

struct World {
    _fake: FakeApi,
    first: Arc<FakeAccount>,
    second: Arc<FakeAccount>,
    engine: Arc<Engine<MemoryCredentials>>,
    _dir: tempfile::TempDir,
}

fn put(db: &StateDb, id: &str, path: &str, parent: Option<&str>, kind: ItemKind) {
    let entry = FileEntry {
        file_id: id.to_string(),
        path: path.to_string(),
        status: FileStatus::CloudOnly,
        size_bytes: 10,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: parent.map(str::to_string),
        item_kind: kind,
    };
    db.upsert_file_with_contract(&entry, |contract| {
        contract.item_kind = entry.item_kind.clone();
        contract.parent_id = entry.parent_id.clone();
    })
    .unwrap();
}

/// Two accounts on one fake. The first account's local state holds the Backups tree (this PC's backup and a phone's)
/// and a plain folder.
fn world(tag: &str) -> World {
    let (first, _) = FakeAccount::register(&format!("places-one-{tag}@example.eu"), PASSWORD, TEST_PHRASE);
    let (second, _) = FakeAccount::register(&format!("places-two-{tag}@example.eu"), PASSWORD, OTHER_PHRASE);
    let (first, second) = (Arc::new(first), Arc::new(second));
    let (to_first, to_second) = (account_handler(first.clone()), account_handler(second.clone()));
    let (second_email, second_token) = (second.email.clone(), second.session_token.clone());
    let signing_in_second = Mutex::new(false);
    let handler: Handler = Arc::new(move |request: &Request| {
        let second_asked = match request.bearer() {
            Some(token) => token == second_token,
            None => {
                let mut signing_in = signing_in_second.lock().unwrap();
                if let Some(email) = request.json().get("email").and_then(|email| email.as_str()) {
                    *signing_in = email == second_email;
                }
                *signing_in
            }
        };
        if second_asked {
            to_second(request)
        } else {
            to_first(request)
        }
    });
    let fake = FakeApi::start(handler);
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: format!("files-places-slot-{tag}"),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: fake.base.clone(),
    };
    let engine = Arc::new(
        Engine::open(
            crate::shell::identity::engine_identity(),
            config,
            host,
            MemoryCredentials::default(),
        )
        .expect("the engine opens"),
    );
    engine.restore();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    put(&db, ROOT, "/Backups", None, ItemKind::Folder);
    db.mark_backups_root_for_test(ROOT).unwrap();
    put(&db, THIS_PC, "/Backups/Invented Laptop", Some(ROOT), ItemKind::Folder);
    put(&db, OTHER_PC, "/Backups/Invented Phone", Some(ROOT), ItemKind::Folder);
    put(
        &db,
        DOCS,
        "/Backups/Invented Laptop/Documents",
        Some(THIS_PC),
        ItemKind::Folder,
    );
    put(
        &db,
        NOTE,
        "/Backups/Invented Laptop/Documents/note.txt",
        Some(DOCS),
        ItemKind::File,
    );
    put(&db, PLAIN, "/Plain", None, ItemKind::Folder);
    put(&db, PLAIN_FILE, "/Plain/plain.txt", Some(PLAIN), ItemKind::File);
    db.set_owner(&Identity::new(Some(&first.user_id), Some(&first.email)))
        .unwrap();
    db.set_backup_layout(&BackupLayout {
        root_id: ROOT.into(),
        device_folder_id: THIS_PC.into(),
        device_folder_name: "Invented Laptop".into(),
    })
    .unwrap();
    drop(db);
    engine.use_local_state_for_test(path);
    World {
        _fake: fake,
        first,
        second,
        engine,
        _dir: dir,
    }
}

fn sign_in(engine: &Engine<MemoryCredentials>, account: &FakeAccount, phrase: &str) {
    engine
        .sign_in_password(&account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(phrase.to_string()))
        .expect("the phrase unlocks");
}

fn first_signed_in(tag: &str) -> (World, Source) {
    let world = world(tag);
    sign_in(&world.engine, &world.first, TEST_PHRASE);
    let source = EngineFilesSource::new(world.engine.clone());
    (world, source)
}

fn row(id: &str, parent: Option<&str>, is_folder: bool) -> DriveItem {
    DriveItem {
        id: id.to_string(),
        parent_id: parent.map(str::to_string),
        name: "an invented name".to_string(),
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

/// The next read waits on its worker before it asks the engine anything, until the first is sent on; the second says
/// its answer is in the source's inbox.
fn hold_next_call(source: &mut Source) -> (mpsc::Sender<()>, mpsc::Receiver<()>) {
    let ((release, hold), (ended, has_ended)) = (mpsc::channel(), mpsc::channel());
    source.places.next_call = Some((hold, ended));
    (release, has_ended)
}

fn watch_next_call(source: &mut Source) -> mpsc::Receiver<()> {
    let (release, ended) = hold_next_call(source);
    release.send(()).expect("the hook is set");
    ended
}

/// The window shows `folder`: read it, wait for the answer, and take it in.
fn read_now(source: &mut Source, folder: Option<&str>) {
    let ended = watch_next_call(source);
    source.folder_shown(folder);
    ended.recv_timeout(WAIT).expect("the read ended");
    source.changes();
}

#[test]
fn protection_answers_for_the_item_the_folder_and_a_dropped_on_row() {
    let (_world, mut source) = first_signed_in("answers");
    assert_eq!(
        source.protection(Some(ROOT), None),
        None,
        "nothing before the engine answered"
    );
    read_now(&mut source, Some(ROOT));
    // The folder itself, and each row.
    assert_eq!(source.protection(Some(ROOT), None), Some(Protection::BackupsRoot));
    assert_eq!(
        source.protection(Some(ROOT), Some(&row(THIS_PC, Some(ROOT), true))),
        Some(Protection::DeviceBackup(THIS_PC.into()))
    );
    // A drop on a row of the shown folder: the row is the destination.
    assert_eq!(
        source.protection(Some(THIS_PC), None),
        Some(Protection::DeviceBackup(THIS_PC.into()))
    );
    assert_eq!(
        source.protection(Some(OTHER_PC), None),
        Some(Protection::DeviceBackup(OTHER_PC.into()))
    );
    // A row nothing is known of, and not in the shown folder: nothing.
    assert_eq!(source.protection(Some(UNHELD), None), None);
    // Inside a device's backup, and outside the tree.
    read_now(&mut source, Some(DOCS));
    assert_eq!(
        source.protection(Some(DOCS), None),
        Some(Protection::DeviceBackup(THIS_PC.into()))
    );
    assert_eq!(
        source.protection(Some(DOCS), Some(&row(NOTE, Some(DOCS), false))),
        Some(Protection::DeviceBackup(THIS_PC.into()))
    );
    read_now(&mut source, Some(PLAIN));
    assert_eq!(source.protection(Some(PLAIN), None), None);
    assert_eq!(
        source.protection(Some(PLAIN), Some(&row(PLAIN_FILE, Some(PLAIN), false))),
        None
    );
    // The top of Files: Backups is the one protected row.
    read_now(&mut source, None);
    assert_eq!(source.protection(None, None), None);
    assert_eq!(
        source.protection(None, Some(&row(ROOT, None, true))),
        Some(Protection::BackupsRoot)
    );
    assert_eq!(source.protection(None, Some(&row(PLAIN, None, true))), None);
}

#[test]
fn a_row_the_read_does_not_hold_takes_the_folders_place() {
    let (_world, mut source) = first_signed_in("missing");
    read_now(&mut source, Some(ROOT));
    // A row that arrived after the read, in the Backups folder: inside the tree, never "nothing".
    assert_eq!(
        source.protection(Some(ROOT), Some(&row(UNHELD, Some(ROOT), true))),
        Some(Protection::BackupsRoot)
    );
    read_now(&mut source, Some(DOCS));
    assert_eq!(
        source.protection(Some(DOCS), Some(&row(UNHELD, Some(DOCS), false))),
        Some(Protection::DeviceBackup(THIS_PC.into()))
    );
    // Outside the tree a missing row is outside it too, and a row of another folder is nothing.
    read_now(&mut source, Some(PLAIN));
    assert_eq!(
        source.protection(Some(PLAIN), Some(&row(UNHELD, Some(PLAIN), false))),
        None
    );
    read_now(&mut source, Some(ROOT));
    assert_eq!(
        source.protection(Some(ROOT), Some(&row(UNHELD, Some(PLAIN), false))),
        None
    );
}

#[test]
fn places_of_an_earlier_account_answer_nothing() {
    let (world, mut source) = first_signed_in("earlier");
    // The answer is in the inbox, and the account changes before the window takes it in.
    let ended = watch_next_call(&mut source);
    source.folder_shown(Some(ROOT));
    ended.recv_timeout(WAIT).expect("the read ended");
    world.engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&world.engine, &world.second, OTHER_PHRASE);
    assert_eq!(source.changes().len(), 0);
    assert_eq!(
        source.protection(Some(ROOT), None),
        None,
        "nothing of the first account's tree"
    );
    let held = Some(THIS_PC);
    assert_eq!(source.backup_entry(&row(held.unwrap(), Some(ROOT), true)), None);
}

#[test]
fn an_answer_for_a_folder_no_longer_asked_is_dropped() {
    let (_world, mut source) = first_signed_in("stale-folder");
    let (release, ended) = hold_next_call(&mut source);
    source.folder_shown(Some(ROOT));
    // The window moved on before the first read ended: its answer is for a folder that is not shown any more.
    let second = watch_next_call(&mut source);
    source.folder_shown(Some(PLAIN));
    second.recv_timeout(WAIT).expect("the second read ended");
    release.send(()).expect("the first worker waits");
    ended.recv_timeout(WAIT).expect("the first read ended");
    source.changes();
    assert_eq!(source.protection(Some(ROOT), None), None);
    assert_eq!(
        source.protection(Some(PLAIN), None),
        None,
        "outside the tree: nothing, and held"
    );
    assert!(
        source
            .places
            .held
            .as_ref()
            .is_some_and(|held| held.folder.as_deref() == Some(PLAIN))
    );
}

#[test]
fn this_pcs_backup_row_says_this_pc_in_the_adapter_and_others_say_nothing() {
    let (_world, mut source) = first_signed_in("entry");
    assert_eq!(
        source.backup_entry(&row(THIS_PC, Some(ROOT), true)),
        None,
        "nothing before the answer"
    );
    read_now(&mut source, Some(ROOT));
    let this = source
        .backup_entry(&row(THIS_PC, Some(ROOT), true))
        .expect("this PC's backup");
    assert_eq!(this.status.as_deref(), Some("This PC"));
    assert_eq!(this.kind, BackupKind::Laptop);
    assert!(!this.managed);
    assert_eq!(source.backup_entry(&row(OTHER_PC, Some(ROOT), true)), None);
    assert_eq!(source.backup_entry(&row(UNHELD, Some(ROOT), true)), None);
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

#[test]
fn a_rename_before_the_places_answered_is_refused_by_the_engine() {
    let (world, mut source) = first_signed_in("rename");
    std::fs::create_dir_all(world.engine.sync_root().expect("a sync folder")).unwrap();
    let _running = hold_running(&world.engine);
    // Nothing was told and nothing answered: the window knows no place, and sends the change.
    assert_eq!(source.protection(Some(DOCS), Some(&row(NOTE, Some(DOCS), false))), None);
    let doomed = row(NOTE, Some(DOCS), false);
    assert_eq!(source.rename(&doomed, "renamed.txt"), Err(ActionError::Protected));
    assert_eq!(
        source.move_to(std::slice::from_ref(&doomed), Some(PLAIN)),
        Err(ActionError::Protected)
    );
    // The row is as it was, in the state.
    let state = world.engine.local_state().expect("the state");
    let (path, parent) = state
        .with(|db| {
            let entry = db.get_file(NOTE).unwrap().expect("the row");
            (entry.path, entry.parent_id)
        })
        .unwrap();
    assert_eq!(path, "/Backups/Invented Laptop/Documents/note.txt");
    assert_eq!(parent.as_deref(), Some(DOCS));
}

// ── The Files window ─────────────────────────────────────────────────────────────────────────────

/// Every element's text that reads exactly `words`.
fn texts_of(rt: &rune::Runtime<crate::State>, words: &str) -> usize {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting && e.text.as_deref() == Some(words))
        .count()
}

#[test]
fn this_pcs_backup_row_says_this_pc_and_others_say_nothing() {
    let world = world("window");
    let mut rt = window(&world.engine);
    sign_in(&world.engine, &world.first, TEST_PHRASE);
    settle(&mut rt);
    rt.update(|s, _| s.open_folder(Some(ROOT.to_string())));
    until_drawn(&mut rt, "Invented Phone");
    until_drawn(&mut rt, "This PC");
    let now = drawn(&rt);
    assert!(now.contains("Invented Laptop"), "the window draws:\n{now}");
    assert_eq!(
        texts_of(&rt, "This PC"),
        1,
        "exactly one row says it is this PC's:\n{now}"
    );
}
