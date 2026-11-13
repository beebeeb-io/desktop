//! An open file at the engine (`Engine::open_file_facts`, `Engine::open_file_in_memory`), against the engine's own
//! doubles: two invented accounts on the fake API on loopback, each file's content encrypted by core's chunk
//! encryptor under that account's key (`FakeAccount::add_file_with_content`), a state database of each test's own
//! (`Engine::use_local_state_for_test`) recorded as one account's. Sessions are put in memory as a sign-in and an
//! unlock leave them. Every name, key and byte is invented.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::Duration;

use super::super::{Engine, EngineConfig};
use super::*;
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use crate::test_api::{FakeApi, Handler, Reply, Request};

type TestEngine = Engine<MemoryCredentials>;

const NOTE: &str = "a0000000-0000-4000-8000-000000000401";
const EMPTY: &str = "a0000000-0000-4000-8000-000000000402";
const FOLDER: &str = "a0000000-0000-4000-8000-000000000403";
const NOT_ON_SERVER: &str = "a0000000-0000-4000-8000-000000000404";
const THEIRS: &str = "a0000000-0000-4000-8000-000000000501";

const ANSWER: u8 = 0;
const SERVER_ERROR: u8 = 1;
const SESSION_ENDED: u8 = 2;

const NO_LIMIT: u64 = 1 << 20;

/// Three chunks of the fake's size and a part of a fourth.
fn note_text() -> Vec<u8> {
    (0..3_500u32).map(|i| b'a' + (i % 23) as u8).collect()
}

/// Holds every chunk request until released, and says when one arrived.
#[derive(Default)]
struct Hold {
    on: AtomicBool,
    released: Mutex<bool>,
    wake: Condvar,
    arrived: Mutex<Option<mpsc::Sender<()>>>,
}

impl Hold {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }

    fn wait(&self) {
        if !self.on.load(Ordering::SeqCst) {
            return;
        }
        if let Some(arrived) = self.arrived.lock().unwrap().as_ref() {
            let _ = arrived.send(());
        }
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.wake.wait(released).unwrap();
        }
    }
}

struct Fake {
    api: FakeApi,
    first: Arc<FakeAccount>,
    second: Arc<FakeAccount>,
    /// How a chunk request answers.
    chunk_mode: Arc<AtomicU8>,
    hold: Arc<Hold>,
    /// A file's row names no size (a legacy row).
    no_size: Arc<AtomicBool>,
}

fn is_chunk(req: &Request) -> bool {
    req.method == "GET" && req.path.contains("/chunks/")
}

fn fake(tag: &str) -> Fake {
    let (first, _) = FakeAccount::register(&format!("{tag}-one@example.eu"), "an invented password", TEST_PHRASE);
    // Another phrase: another vault key, so the second account cannot open the first's files.
    let (second, _) = FakeAccount::register(
        &format!("{tag}-two@example.eu"),
        "an invented password",
        "legal winner thank year wave sausage worth useful legal winner thank yellow",
    );
    let (first, second) = (Arc::new(first), Arc::new(second));
    assert_ne!(first.master_key, second.master_key);
    first.add_file_with_content(NOTE, None, "Invented note.txt", &note_text());
    first.add_file_with_content(EMPTY, None, "Invented empty.txt", b"");
    second.add_file_with_content(THEIRS, None, "Second account plan.txt", b"the second account's words");
    let (one, two): (Handler, Handler) = (account_handler(first.clone()), account_handler(second.clone()));
    let second_token = second.session_token.clone();
    let (chunk_mode, hold) = (Arc::new(AtomicU8::new(ANSWER)), Arc::new(Hold::default()));
    let no_size = Arc::new(AtomicBool::new(false));
    let (mode_in, hold_in, no_size_in) = (chunk_mode.clone(), hold.clone(), no_size.clone());
    let api = FakeApi::start(Arc::new(move |req: &Request| {
        let rest = if req.bearer() == Some(second_token.as_str()) {
            &two
        } else {
            &one
        };
        if is_chunk(req) {
            hold_in.wait();
            match mode_in.load(Ordering::SeqCst) {
                SERVER_ERROR => return Reply::json(500, serde_json::json!({ "error": "a server body" })),
                SESSION_ENDED => return Reply::json(401, serde_json::json!({ "error": "a server body" })),
                _ => {}
            }
        }
        let reply = rest(req);
        if no_size_in.load(Ordering::SeqCst) && req.route() == format!("GET /api/v1/files/{NOTE}") {
            let mut row: serde_json::Value = serde_json::from_slice(&reply.body).expect("the fake's row");
            row.as_object_mut().expect("a row").remove("size_bytes");
            return Reply::ok(row);
        }
        reply
    }));
    Fake {
        api,
        first,
        second,
        chunk_mode,
        hold,
        no_size,
    }
}

fn engine_at(base: &str) -> Arc<TestEngine> {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-viewer-slot".into(),
        state_dir,
        api_base: base.to_string(),
    };
    Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"))
}

/// A session in memory, as a sign-in and an unlock of `account` leave it.
fn signed_in(engine: &TestEngine, account: &FakeAccount) {
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

fn row(db: &StateDb, id: &str, path: &str, size: i64, folder: bool) {
    let entry = FileEntry {
        file_id: id.into(),
        path: path.into(),
        status: FileStatus::CloudOnly,
        size_bytes: size,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: None,
        item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
    };
    db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
        .unwrap();
}

/// A state database recorded as `owner`'s, with the first account's rows, and the engine reading it.
fn local_state(engine: &TestEngine, owner: &FakeAccount) -> (tempfile::TempDir, StateDb) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    row(&db, FOLDER, "Invented folder", 0, true);
    row(&db, NOTE, "Invented folder/Deeper/Invented note.txt", 3_500, false);
    row(&db, EMPTY, "Invented empty.txt", 0, false);
    row(&db, NOT_ON_SERVER, "Invented gone.txt", 12, false);
    row(&db, THEIRS, "Second account plan.txt", 26, false);
    db.set_owner(&Identity::new(Some(&owner.user_id), Some(&owner.email)))
        .unwrap();
    engine.use_local_state_for_test(path);
    (dir, db)
}

fn open(engine: &TestEngine, id: &str, max: u64) -> Result<OpenedFile, OpenFileError> {
    engine.open_file_in_memory(id, max, &|_, _| {}, &AtomicBool::new(false))
}

fn chunk_requests(fake: &Fake) -> usize {
    fake.api.requests().iter().filter(|req| is_chunk(req)).count()
}

fn kind(result: Result<OpenedFile, OpenFileError>) -> OpenFileError {
    result.expect_err("no content is handed over")
}

#[test]
fn a_file_opens_in_memory_by_the_download_path_and_nothing_about_it_is_recorded() {
    let fake = fake("opens");
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.first);
    let (_dir, db) = local_state(&engine, &fake.first);

    let facts = engine.open_file_facts(NOTE).expect("the facts");
    assert_eq!(facts.name, "Invented note.txt");
    assert_eq!(facts.folders, ["Invented folder", "Deeper"]);
    assert_eq!(facts.size, 3_500);
    assert_eq!(facts.session, engine.session_turn());

    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_in = seen.clone();
    let opened = engine
        .open_file_in_memory(
            NOTE,
            NO_LIMIT,
            &move |done, total| seen_in.lock().unwrap().push((done, total)),
            &AtomicBool::new(false),
        )
        .expect("the file opens");
    assert_eq!(&*opened.bytes, &note_text()[..], "the decrypted bytes are the file's");
    assert_eq!(opened.session, engine.session_turn());
    assert_eq!(chunk_requests(&fake), 4, "every chunk was asked for once");
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.last(), Some(&(3_500, 3_500)), "progress ends at the whole file");
    assert!(
        seen.windows(2).all(|pair| pair[0].0 <= pair[1].0),
        "progress never goes back"
    );

    // Nothing about the file is recorded: it is as online-only as before, and nothing is cached.
    let after = db.get_file(NOTE).unwrap().unwrap();
    assert_eq!(after.status, FileStatus::CloudOnly);
    assert_eq!(db.kept_rows().unwrap().len(), 0);

    // A `{:?}` of either answer prints no name and no content.
    let printed = format!("{facts:?} {opened:?}");
    assert!(!printed.contains("Invented"), "{printed}");
    assert!(printed.contains("3500"), "{printed}");
}

#[test]
fn an_empty_file_opens_as_no_bytes() {
    let fake = fake("empty");
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.first);
    let _state = local_state(&engine, &fake.first);
    let opened = open(&engine, EMPTY, NO_LIMIT).expect("the empty file opens");
    assert!(opened.bytes.is_empty());
    assert_eq!(engine.open_file_facts(EMPTY).unwrap().size, 0);
}

#[test]
fn a_file_over_the_limit_is_refused_before_a_chunk_is_asked_for() {
    let fake = fake("large");
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.first);
    let _state = local_state(&engine, &fake.first);
    assert_eq!(
        kind(open(&engine, NOTE, 3_499)),
        OpenFileError::TooLarge { size: 3_500 }
    );
    assert_eq!(chunk_requests(&fake), 0);
    // At the limit it opens.
    assert_eq!(open(&engine, NOTE, 3_500).unwrap().bytes.len(), 3_500);
}

#[test]
fn a_row_that_names_no_size_is_held_to_the_limit_once_its_bytes_are_counted() {
    let fake = fake("legacy");
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.first);
    let _state = local_state(&engine, &fake.first);
    fake.no_size.store(true, Ordering::SeqCst);
    // Nothing says how large it is before it is downloaded: it is, and then it is not handed over. No size is
    // claimed for it.
    assert_eq!(kind(open(&engine, NOTE, 3_499)), OpenFileError::OverLimit);
    assert_eq!(chunk_requests(&fake), 4);
    assert_eq!(open(&engine, NOTE, 3_500).unwrap().bytes.len(), 3_500);
}

#[test]
fn a_refusal_of_the_server_has_its_kind_and_hands_over_nothing() {
    let fake = fake("refused");
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.first);
    let _state = local_state(&engine, &fake.first);

    fake.chunk_mode.store(SERVER_ERROR, Ordering::SeqCst);
    assert_eq!(kind(open(&engine, NOTE, NO_LIMIT)), OpenFileError::Failed);
    fake.chunk_mode.store(SESSION_ENDED, Ordering::SeqCst);
    assert_eq!(kind(open(&engine, NOTE, NO_LIMIT)), OpenFileError::SessionEnded);
    fake.chunk_mode.store(ANSWER, Ordering::SeqCst);
    // In the local state, gone from Beebeeb.
    assert_eq!(kind(open(&engine, NOT_ON_SERVER, NO_LIMIT)), OpenFileError::NotFound);
    // Not in the local state at all; and a folder is not a file to open.
    assert_eq!(
        engine.open_file_facts("a0000000-0000-4000-8000-00000000ffff"),
        Err(OpenFileError::NotFound)
    );
    assert_eq!(engine.open_file_facts(FOLDER), Err(OpenFileError::NotFound));
    // And after the refusals the file still opens.
    assert_eq!(open(&engine, NOTE, NO_LIMIT).unwrap().bytes.len(), 3_500);
}

#[test]
fn beebeeb_out_of_reach_is_offline() {
    let fake = fake("offline");
    // A closed loopback port.
    let engine = engine_at("http://127.0.0.1:9");
    signed_in(&engine, &fake.first);
    let _state = local_state(&engine, &fake.first);
    assert_eq!(kind(open(&engine, NOTE, NO_LIMIT)), OpenFileError::Offline);
}

#[test]
fn signed_out_or_locked_nothing_is_read_and_nothing_is_asked() {
    let fake = fake("out");
    let engine = engine_at(&fake.api.base);
    let _state = local_state(&engine, &fake.first);
    assert_eq!(engine.open_file_facts(NOTE), Err(OpenFileError::NotSignedIn));
    assert_eq!(kind(open(&engine, NOTE, NO_LIMIT)), OpenFileError::NotSignedIn);

    signed_in(&engine, &fake.first);
    assert!(engine.open_file_facts(NOTE).is_ok());
    engine.forget_session(SessionTransition::Lock);
    assert_eq!(engine.open_file_facts(NOTE), Err(OpenFileError::NotSignedIn));
    assert_eq!(kind(open(&engine, NOTE, NO_LIMIT)), OpenFileError::NotSignedIn);
    assert_eq!(fake.api.requests().len(), 0, "nothing was asked of Beebeeb");
}

#[test]
fn another_accounts_local_state_answers_nothing_and_its_files_are_never_asked_for() {
    let fake = fake("other");
    let engine = engine_at(&fake.api.base);
    // The local state is the first account's; the second account is signed in.
    let _state = local_state(&engine, &fake.first);
    signed_in(&engine, &fake.second);
    assert_eq!(engine.open_file_facts(NOTE), Err(OpenFileError::NotYours));
    assert_eq!(engine.open_file_facts(THEIRS), Err(OpenFileError::NotYours));
    assert_eq!(kind(open(&engine, NOTE, NO_LIMIT)), OpenFileError::NotYours);
    assert_eq!(kind(open(&engine, THEIRS, NO_LIMIT)), OpenFileError::NotYours);
    assert_eq!(fake.api.requests().len(), 0, "nothing was asked of Beebeeb");
}

#[test]
fn the_second_account_opens_its_own_file_with_its_own_key() {
    let fake = fake("second");
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.second);
    let _state = local_state(&engine, &fake.second);
    assert_eq!(
        &*open(&engine, THEIRS, NO_LIMIT).unwrap().bytes,
        b"the second account's words"
    );
    // The first account's file is not the second's on the server: its row is not served to this token.
    assert_eq!(kind(open(&engine, NOTE, NO_LIMIT)), OpenFileError::NotFound);
}

/// Start an open on a thread of its own while the fake holds every chunk; the answer comes down the channel.
fn open_held(
    fake: &Fake,
    engine: &Arc<TestEngine>,
    cancel: &Arc<AtomicBool>,
) -> mpsc::Receiver<Result<usize, OpenFileError>> {
    let (arrived, chunk_arrived) = mpsc::channel();
    *fake.hold.arrived.lock().unwrap() = Some(arrived);
    fake.hold.on.store(true, Ordering::SeqCst);
    let (answer, answered) = mpsc::channel();
    let (engine, cancel) = (engine.clone(), cancel.clone());
    std::thread::spawn(move || {
        let result = engine.open_file_in_memory(NOTE, NO_LIMIT, &|_, _| {}, &cancel);
        let _ = answer.send(result.map(|opened| opened.bytes.len()));
    });
    chunk_arrived
        .recv_timeout(Duration::from_secs(10))
        .expect("the download asked for a chunk");
    answered
}

#[test]
fn a_cancelled_download_ends_while_beebeeb_still_holds_the_chunk() {
    let fake = fake("cancel");
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.first);
    let _state = local_state(&engine, &fake.first);
    let cancel = Arc::new(AtomicBool::new(false));
    let answered = open_held(&fake, &engine, &cancel);
    // Not answered while the chunk is held and nothing was cancelled.
    assert!(answered.recv_timeout(Duration::from_millis(200)).is_err());
    cancel.store(true, Ordering::SeqCst);
    let answer = answered.recv_timeout(Duration::from_secs(5));
    fake.hold.release();
    assert_eq!(answer, Ok(Err(OpenFileError::Cancelled)));
}

#[test]
fn a_lock_during_the_download_ends_it_and_hands_over_nothing() {
    let fake = fake("lock");
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.first);
    let _state = local_state(&engine, &fake.first);
    let answered = open_held(&fake, &engine, &Arc::new(AtomicBool::new(false)));
    engine.forget_session(SessionTransition::Lock);
    let answer = answered.recv_timeout(Duration::from_secs(5));
    fake.hold.release();
    assert_eq!(answer, Ok(Err(OpenFileError::NotSignedIn)));
}

#[test]
fn content_decrypted_across_a_session_change_is_never_handed_over() {
    let fake = fake("across");
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.first);
    let _state = local_state(&engine, &fake.first);
    // The session is locked and unlocked again as the LAST chunk is decrypted: the download itself finishes with
    // every byte, for a session that is no longer the one it began in.
    let (again, calls) = (engine.clone(), Arc::new(AtomicUsize::new(0)));
    let (first, calls_in) = (fake.first.clone(), calls.clone());
    let result = engine.open_file_in_memory(
        NOTE,
        NO_LIMIT,
        &move |done, total| {
            if done == total && calls_in.fetch_add(1, Ordering::SeqCst) == 0 {
                again.forget_session(SessionTransition::Lock);
                signed_in(&again, &first);
            }
        },
        &AtomicBool::new(false),
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1, "the session changed at the last chunk");
    assert_eq!(kind(result), OpenFileError::NotSignedIn);
    // The same account, signed in again, opens it.
    assert_eq!(open(&engine, NOTE, NO_LIMIT).unwrap().bytes.len(), 3_500);
}

// ── The copy on this computer ───────────────────────────────────────────────────────────────────────

/// The account's folder on this computer for a test, with the first account's rows in the local state; the row of
/// `NOTE` says `status`, and the file is on disk when `on_disk` names its content.
fn with_folder(
    engine: &TestEngine,
    owner: &FakeAccount,
    status: FileStatus,
    on_disk: Option<&[u8]>,
) -> (tempfile::TempDir, tempfile::TempDir, StateDb) {
    let (dir, db) = local_state(engine, owner);
    db.set_status(NOTE, status).unwrap();
    let root = tempfile::tempdir().unwrap();
    engine.use_sync_root_for_test(root.path().to_path_buf());
    if let Some(content) = on_disk {
        let folder = root.path().join("Invented folder").join("Deeper");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("Invented note.txt"), content).unwrap();
    }
    (dir, root, db)
}

fn local(engine: &TestEngine, id: &str, max: u64) -> Result<LocalCopy, OpenFileError> {
    engine.open_local_copy_in_memory(id, max, &AtomicBool::new(false))
}

fn opened(result: Result<LocalCopy, OpenFileError>) -> OpenedLocalCopy {
    match result.expect("the copy is read") {
        LocalCopy::Opened(copy) => copy,
        LocalCopy::NotHere => panic!("the copy on this computer was not read"),
    }
}

#[test]
fn a_file_kept_on_this_computer_opens_from_its_copy_with_beebeeb_out_of_reach() {
    let fake = fake("local");
    // A closed loopback port: nothing can be downloaded.
    let engine = engine_at("http://127.0.0.1:9");
    signed_in(&engine, &fake.first);
    let _kept = with_folder(&engine, &fake.first, FileStatus::Local, Some(&note_text()));

    assert_eq!(engine.open_file_facts(NOTE).unwrap().on_this_pc, Some(OnThisPc::InSync));
    let copy = opened(local(&engine, NOTE, NO_LIMIT));
    assert_eq!(&*copy.bytes, &note_text()[..]);
    assert_eq!(copy.copy, OnThisPc::InSync);
    assert_eq!(copy.session, engine.session_turn());
    // A `{:?}` prints how many bytes, never one.
    let printed = format!("{copy:?}");
    assert!(printed.contains("3500") && !printed.contains("abc"), "{printed}");
}

#[test]
fn a_change_made_here_that_is_not_sent_yet_is_what_the_copy_answers_and_it_says_so() {
    let fake = fake("unsent");
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.first);
    // The row says an upload is on its way; the file on disk is the edited one, not what Beebeeb has.
    let _kept = with_folder(
        &engine,
        &fake.first,
        FileStatus::Uploading,
        Some(b"edited here, not sent yet"),
    );
    assert_eq!(
        engine.open_file_facts(NOTE).unwrap().on_this_pc,
        Some(OnThisPc::NotSentYet)
    );
    let copy = opened(local(&engine, NOTE, NO_LIMIT));
    assert_eq!(&*copy.bytes, b"edited here, not sent yet");
    assert_eq!(copy.copy, OnThisPc::NotSentYet);
    assert_eq!(fake.api.requests().len(), 0, "nothing was asked of Beebeeb");
}

#[test]
fn a_copy_whose_length_is_not_the_synced_size_is_not_called_in_sync() {
    let fake = fake("drift");
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.first);
    // The row still says in sync (the change was not noticed yet): the copy's own length says otherwise.
    let _kept = with_folder(
        &engine,
        &fake.first,
        FileStatus::Local,
        Some(b"shorter than 3,500 bytes"),
    );
    assert_eq!(opened(local(&engine, NOTE, NO_LIMIT)).copy, OnThisPc::NotSentYet);
}

#[test]
fn a_file_that_is_not_wholly_here_is_not_read_and_nothing_is_fetched() {
    let fake = fake("nothere");
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.first);
    // Each state that is not a copy to show, with a file on disk all the same: it is not read.
    for status in [
        FileStatus::CloudOnly,
        FileStatus::Downloading,
        FileStatus::Conflict,
        FileStatus::Error,
        FileStatus::Trashing,
    ] {
        let _kept = with_folder(&engine, &fake.first, status.clone(), Some(&note_text()));
        let facts = engine.open_file_facts(NOTE).unwrap();
        assert_eq!(facts.on_this_pc, None, "{status:?}");
        // Two versions, or a sync that failed: the facts say this computer's copy is not settled with Beebeeb's.
        assert_eq!(
            facts.unsettled_here,
            matches!(status, FileStatus::Conflict | FileStatus::Error),
            "{status:?}"
        );
        assert!(
            matches!(local(&engine, NOTE, NO_LIMIT), Ok(LocalCopy::NotHere)),
            "{status:?}"
        );
    }
    // The row says here, and no file is on disk (or a folder is where the file should be): not read.
    let _kept = with_folder(&engine, &fake.first, FileStatus::Local, None);
    assert!(matches!(local(&engine, NOTE, NO_LIMIT), Ok(LocalCopy::NotHere)));
    let (_dir, root, _db) = with_folder(&engine, &fake.first, FileStatus::Local, None);
    std::fs::create_dir_all(root.path().join("Invented folder/Deeper/Invented note.txt")).unwrap();
    assert!(matches!(local(&engine, NOTE, NO_LIMIT), Ok(LocalCopy::NotHere)));
    assert_eq!(fake.api.requests().len(), 0, "nothing was asked of Beebeeb");
}

#[cfg(unix)]
#[test]
fn a_link_where_the_file_should_be_is_never_followed() {
    let fake = fake("link");
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.first);
    let (_dir, root, _db) = with_folder(&engine, &fake.first, FileStatus::Local, None);
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::write(elsewhere.path().join("other.txt"), b"another file's words").unwrap();
    let folder = root.path().join("Invented folder").join("Deeper");
    std::fs::create_dir_all(&folder).unwrap();
    std::os::unix::fs::symlink(elsewhere.path().join("other.txt"), folder.join("Invented note.txt")).unwrap();
    assert!(matches!(local(&engine, NOTE, NO_LIMIT), Ok(LocalCopy::NotHere)));
    // The check of the file itself, without the check of its name before it: a link and a folder are not a plain
    // file, the file the link leads to is.
    assert_eq!(wholly_here(&folder.join("Invented note.txt")), None);
    assert_eq!(wholly_here(&folder), None);
    assert_eq!(wholly_here(&elsewhere.path().join("other.txt")), Some(20));
}

#[test]
fn a_copy_with_a_change_that_is_not_sent_and_cannot_be_read_is_never_answered_as_not_here() {
    let fake = fake("unsentunread");
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.first);
    // The row says a change made here is on its way, and no file is where it says: "not here" would send the
    // caller to the older version Beebeeb has.
    let _kept = with_folder(&engine, &fake.first, FileStatus::Uploading, None);
    assert_eq!(
        local(&engine, NOTE, NO_LIMIT).map(|_| ()).unwrap_err(),
        OpenFileError::CopyNotReadable
    );
    // A folder where the file should be: the same.
    let (_dir, root, _db) = with_folder(&engine, &fake.first, FileStatus::Uploading, None);
    std::fs::create_dir_all(root.path().join("Invented folder/Deeper/Invented note.txt")).unwrap();
    assert_eq!(
        local(&engine, NOTE, NO_LIMIT).map(|_| ()).unwrap_err(),
        OpenFileError::CopyNotReadable
    );
    // The same row for a copy in sync is "not here": Beebeeb has that version, the download is the way.
    let _kept = with_folder(&engine, &fake.first, FileStatus::Local, None);
    assert!(matches!(local(&engine, NOTE, NO_LIMIT), Ok(LocalCopy::NotHere)));
    assert_eq!(fake.api.requests().len(), 0, "nothing was asked of Beebeeb");
    // In sync, the facts do not call the copy unsettled.
    assert!(!engine.open_file_facts(NOTE).unwrap().unsettled_here);
}

#[test]
fn a_copy_over_the_limit_is_refused_from_its_length_and_at_the_limit_it_opens() {
    let fake = fake("localbig");
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.first);
    let _kept = with_folder(&engine, &fake.first, FileStatus::Local, Some(&note_text()));
    // The size is the copy's own length (asked before anything is read), not where a bounded read would stop.
    assert_eq!(
        local(&engine, NOTE, 1_000).map(|_| ()).unwrap_err(),
        OpenFileError::TooLarge { size: 3_500 }
    );
    assert_eq!(
        local(&engine, NOTE, 3_499).map(|_| ()).unwrap_err(),
        OpenFileError::TooLarge { size: 3_500 }
    );
    assert_eq!(opened(local(&engine, NOTE, 3_500)).bytes.len(), 3_500);
    // An empty copy is no bytes, not "not here".
    let _kept = with_folder(&engine, &fake.first, FileStatus::Local, Some(b""));
    assert!(opened(local(&engine, NOTE, 0)).bytes.is_empty());
}

#[test]
fn the_read_is_bounded_by_itself_when_the_file_grows_after_its_length_was_asked() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("grows.bin");
    std::fs::write(&path, vec![7u8; 5_000]).unwrap();
    let (never, always) = (AtomicBool::new(false), || true);
    // Its length was 100 when asked; it is 5,000 now. One byte more than was asked for is read, never the file.
    assert!(matches!(
        read_bounded(&path, 100, 5_000, &never, &always),
        Err(ReadStopped::Grew)
    ));
    // Its length was within the limit when asked, and it is past the limit now: refused, with one byte over held.
    let Err(ReadStopped::Refused(error)) = read_bounded(&path, 1_000, 1_000, &never, &always) else {
        panic!("a file that grew past the limit was read");
    };
    assert_eq!(error, OpenFileError::OverLimit);
    // As long as it said: read whole. Shorter by now: what is there.
    assert_eq!(
        read_bounded(&path, 5_000, 5_000, &never, &always).ok().map(|b| b.len()),
        Some(5_000)
    );
    assert_eq!(
        read_bounded(&path, 6_000, 6_000, &never, &always).ok().map(|b| b.len()),
        Some(5_000)
    );
    // Cancelled, or the session moved on: nothing.
    assert!(matches!(
        read_bounded(&path, 5_000, 5_000, &AtomicBool::new(true), &always),
        Err(ReadStopped::Refused(OpenFileError::Cancelled))
    ));
    assert!(matches!(
        read_bounded(&path, 5_000, 5_000, &never, &|| false),
        Err(ReadStopped::Refused(OpenFileError::NotSignedIn))
    ));
}

#[test]
fn the_copy_is_read_for_the_session_only() {
    let fake = fake("localsession");
    let engine = engine_at(&fake.api.base);
    let _kept = with_folder(&engine, &fake.first, FileStatus::Local, Some(&note_text()));
    // Signed out: nothing is read.
    assert_eq!(
        local(&engine, NOTE, NO_LIMIT).map(|_| ()).unwrap_err(),
        OpenFileError::NotSignedIn
    );
    // Another account signed in on the first account's local state: nothing of it is read.
    signed_in(&engine, &fake.second);
    assert_eq!(
        local(&engine, NOTE, NO_LIMIT).map(|_| ()).unwrap_err(),
        OpenFileError::NotYours
    );
    // Its own account: read. Not in the local state, or a folder: no such file.
    signed_in(&engine, &fake.first);
    assert_eq!(opened(local(&engine, NOTE, NO_LIMIT)).bytes.len(), 3_500);
    assert_eq!(
        local(&engine, "a0000000-0000-4000-8000-00000000ffff", NO_LIMIT)
            .map(|_| ())
            .unwrap_err(),
        OpenFileError::NotFound
    );
    assert_eq!(
        local(&engine, FOLDER, NO_LIMIT).map(|_| ()).unwrap_err(),
        OpenFileError::NotFound
    );
    // Cancelled before the read: nothing.
    assert_eq!(
        engine
            .open_local_copy_in_memory(NOTE, NO_LIMIT, &AtomicBool::new(true))
            .map(|_| ())
            .unwrap_err(),
        OpenFileError::Cancelled
    );
}
