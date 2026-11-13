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
    let (mode_in, hold_in) = (chunk_mode.clone(), hold.clone());
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
        rest(req)
    }));
    Fake {
        api,
        first,
        second,
        chunk_mode,
        hold,
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
