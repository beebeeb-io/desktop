//! Search on the real engine (`EngineSearchSource`): the engine's own local index (`desktop_search`) over a state
//! database of each test's own, written with the engine's writers, behind a host that says whose computer it is.
//! The results and their facts, narrowing, any case, an empty answer, a list that cannot be read, signed out (nothing
//! built), another account's index never answering, recent searches never kept; and the engine handle itself as the
//! host while signed out. Whose answer: an index or a result that was started in one session and lands after a
//! lock or after another account signed in is dropped (in memory, at the readers and in the element tree), and
//! nothing is started by a read. Every name is invented.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_db::{FileEntry, FileStatus, ItemKind, StateDb};

use rune::{Env, Runtime, TextEdit};

use super::*;
use crate::State;
use crate::frames::shared::parts::table::Kind;

const ME: &str = "search-me@example.eu";
const OTHER: &str = "someone-else@example.eu";

const HOLD_FOR: Duration = Duration::from_secs(30);

/// A host on a state database of its own, recorded as `owner`'s: as the engine's account-bound handle, it builds
/// an index for that account alone, and only while the vault is open.
struct Host {
    account: Mutex<Option<String>>,
    owner: Option<String>,
    /// The engine's session turn: it moves on at every sign-in, lock and unlock.
    turn: AtomicU64,
    locked: AtomicBool,
    /// Hold the next build after it read the list, until the test lets it go: (armed, inside, let go).
    hold: Mutex<(bool, bool, bool)>,
    moved: Condvar,
    db: PathBuf,
    /// The list cannot be read.
    broken: AtomicBool,
    builds: AtomicUsize,
    woken: AtomicUsize,
}

impl Host {
    fn lock(&self) {
        self.locked.store(true, Ordering::SeqCst);
        self.turn.fetch_add(1, Ordering::SeqCst);
    }

    fn unlock(&self) {
        self.locked.store(false, Ordering::SeqCst);
        self.turn.fetch_add(1, Ordering::SeqCst);
    }

    /// Sign out and sign in as `account` (`None`: stay signed out), the vault open.
    fn sign_in_as(&self, account: Option<&str>) {
        *self.account.lock().unwrap() = account.map(str::to_string);
        self.locked.store(false, Ordering::SeqCst);
        self.turn.fetch_add(2, Ordering::SeqCst);
    }

    /// Wait until a worker has sent its answer and woken the window (at most 30 s); the test then decides when the
    /// window looks.
    fn woken_past(&self, before: usize) {
        let began = Instant::now();
        while self.woken.load(Ordering::SeqCst) <= before {
            assert!(began.elapsed() < HOLD_FOR, "no worker answered");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn hold_the_next_build(&self) {
        self.hold.lock().unwrap().0 = true;
    }

    /// Wait until a build is held (it has read the list of the session it began in).
    fn build_is_held(&self) {
        let hold = self.hold.lock().unwrap();
        let (_hold, timed_out) = self.moved.wait_timeout_while(hold, HOLD_FOR, |h| !h.1).unwrap();
        assert!(!timed_out.timed_out(), "no build reached the hold");
    }

    fn let_the_build_go(&self) {
        self.hold.lock().unwrap().2 = true;
        self.moved.notify_all();
    }
}

impl SearchHost for Host {
    type Turn = u64;

    fn account(&self) -> Option<String> {
        self.account.lock().unwrap().clone()
    }

    fn turn(&self) -> u64 {
        self.turn.load(Ordering::SeqCst)
    }

    fn unlocked(&self) -> bool {
        !self.locked.load(Ordering::SeqCst) && self.account.lock().unwrap().is_some()
    }

    fn index(&self) -> Result<LocalSearchIndex, ()> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        if self.broken.load(Ordering::SeqCst) {
            return Err(());
        }
        // As the engine's handle: the list of the account it is recorded for, with the vault open.
        if !self.unlocked() || self.account() != self.owner {
            return Err(());
        }
        let db = StateDb::open(&self.db).map_err(|_| ())?;
        let index = build_local_index(&db).map_err(|_| ());
        let mut hold = self.hold.lock().unwrap();
        if hold.0 {
            hold.0 = false;
            hold.1 = true;
            self.moved.notify_all();
            let (hold, timed_out) = self.moved.wait_timeout_while(hold, HOLD_FOR, |h| !h.2).unwrap();
            drop(hold);
            assert!(!timed_out.timed_out(), "the test never let the held build go");
        }
        index
    }

    fn wake(&self) {
        self.woken.fetch_add(1, Ordering::SeqCst);
    }
}

fn put(db: &StateDb, id: &str, path: &str, status: FileStatus, size: i64, folder: bool) {
    let entry = FileEntry {
        file_id: id.to_string(),
        path: path.to_string(),
        status,
        size_bytes: size,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: None,
        item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
    };
    db.upsert_file_with_contract(&entry, |contract| {
        contract.item_kind = entry.item_kind.clone();
    })
    .unwrap();
}

/// A computer with a few files of `ME`'s.
fn host(account: Option<&str>) -> Arc<Host> {
    let dir = tempfile::tempdir().expect("a temp folder").keep();
    let path = dir.join("state.db");
    let db = StateDb::open(&path).expect("a state database");
    put(&db, "d-legal", "/Documents/Legal", FileStatus::Local, 0, true);
    put(
        &db,
        "f-draft",
        "/Contract draft.pdf",
        FileStatus::CloudOnly,
        1_258_291,
        false,
    );
    put(
        &db,
        "f-signed",
        "/Documents/Legal/Contract signed 2025.pdf",
        FileStatus::Local,
        860_160,
        false,
    );
    put(
        &db,
        "f-sheet",
        "/Documents/contract costs.xlsx",
        FileStatus::Uploading,
        94_208,
        false,
    );
    put(
        &db,
        "f-photo",
        "/Photos/CONTRACT scan.jpg",
        FileStatus::Local,
        3_984_588,
        false,
    );
    put(&db, "f-other", "/notes.txt", FileStatus::Local, 12, false);
    drop(db);
    Arc::new(Host {
        account: Mutex::new(account.map(str::to_string)),
        owner: Some(ME.to_string()),
        turn: AtomicU64::new(1),
        locked: AtomicBool::new(false),
        hold: Mutex::new((false, false, false)),
        moved: Condvar::new(),
        db: path,
        broken: AtomicBool::new(false),
        builds: AtomicUsize::new(0),
        woken: AtomicUsize::new(0),
    })
}

/// Pump until no build and no query is on its way (at most 30 s).
fn settle(source: &mut EngineSearchSource<Host>) {
    let began = Instant::now();
    while source.pump().busy {
        assert!(began.elapsed() < HOLD_FOR, "an answer did not come back");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A source whose Search was opened, with the index it built.
fn opened(host: &Arc<Host>) -> EngineSearchSource<Host> {
    let mut source = EngineSearchSource::new(host.clone());
    source.opened();
    settle(&mut source);
    source
}

/// Ask as the window does when the words or the pill change, and wait for the worker's answer.
fn found(source: &mut EngineSearchSource<Host>, query: &str, narrow: Narrow) -> Vec<Hit> {
    source.ask(query, narrow);
    settle(source);
    match source.search(query, narrow) {
        Answer::Found { hits, loading } => {
            assert_eq!(loading, None);
            hits
        }
        Answer::Searching => panic!("the query was not answered"),
        Answer::Unavailable => panic!("search answered Unavailable"),
    }
}

fn ids(hits: &[Hit]) -> Vec<&str> {
    let mut ids: Vec<_> = hits.iter().map(|h| h.id.as_str()).collect();
    ids.sort();
    ids
}

#[test]
fn names_on_this_pc_are_found_in_any_case_with_where_they_are() {
    let host = host(Some(ME));
    let mut source = opened(&host);
    assert!(source.wired());
    assert!(
        host.woken.load(Ordering::SeqCst) >= 1,
        "the window is woken when the index lands"
    );

    let hits = found(&mut source, "contr", Narrow::All);
    assert_eq!(
        ids(&hits),
        ["f-draft", "f-photo", "f-sheet", "f-signed"],
        "any case; folders are not indexed"
    );
    let draft = hits.iter().find(|h| h.id == "f-draft").unwrap();
    assert_eq!(
        (
            draft.name.as_str(),
            draft.kind,
            draft.size.as_str(),
            draft.place.as_str(),
            draft.pc
        ),
        (
            "Contract draft.pdf",
            Kind::Document,
            "1.2 MB",
            "Files",
            Some(PcState::OnlineOnly)
        )
    );
    let signed = hits.iter().find(|h| h.id == "f-signed").unwrap();
    assert_eq!(signed.place, "Documents \u{203A} Legal");
    assert_eq!(signed.pc, Some(PcState::Downloaded));
    assert!(!signed.changed.is_empty());
    let sheet = hits.iter().find(|h| h.id == "f-sheet").unwrap();
    assert_eq!(
        (sheet.kind, sheet.pc),
        (Kind::Sheet, None),
        "uploading: the PC column says nothing"
    );

    assert_eq!(ids(&found(&mut source, "CONTRACT", Narrow::Photos)), ["f-photo"]);
    assert_eq!(
        ids(&found(&mut source, "contract", Narrow::Documents)),
        ["f-draft", "f-sheet", "f-signed"]
    );
    assert!(found(&mut source, "contract", Narrow::Folders).is_empty());
    assert_eq!(host.builds.load(Ordering::SeqCst), 1, "built once while fresh");
}

#[test]
fn a_results_facts_are_what_the_index_holds() {
    let source = opened(&host(Some(ME)));
    let facts = source.details("f-signed").expect("a known result");
    assert_eq!(facts.kind, "PDF document");
    assert_eq!(facts.size, "840 KB");
    assert_eq!(facts.where_, "Documents \u{203A} Legal");
    assert_eq!(facts.offline_note, "Downloaded \u{00B7} 840 KB");
    assert_eq!(facts.shared, "", "not known here: never \"Only you\"");
    assert_eq!((facts.versions, facts.changed_on, facts.added), (None, None, None));
    assert!(!facts.picture, "no sample picture");
    assert_eq!(
        source.details("f-draft").unwrap().offline_note,
        "Online only now \u{00B7} 1.2 MB"
    );
    assert_eq!(source.details("no-such-item"), None);
}

#[test]
fn no_match_and_an_empty_query_are_answers_with_no_result() {
    let mut source = opened(&host(Some(ME)));
    assert!(found(&mut source, "lisbon", Narrow::All).is_empty());
    assert!(found(&mut source, "   ", Narrow::All).is_empty());
}

#[test]
fn a_list_that_cannot_be_read_is_unavailable_never_no_results() {
    let host = host(Some(ME));
    host.broken.store(true, Ordering::SeqCst);
    let mut source = opened(&host);
    assert!(!source.wired());
    assert_eq!(source.search("contract", Narrow::All), Answer::Unavailable);
    assert_eq!(source.details("f-draft"), None);
    for _ in 0..20 {
        source.search("contract", Narrow::All);
        source.opened();
        source.ask("contract", Narrow::All);
    }
    settle(&mut source);
    assert_eq!(
        host.builds.load(Ordering::SeqCst),
        1,
        "no retry loop: not built again within FRESH_FOR"
    );
}

#[test]
fn signed_out_builds_nothing_and_another_accounts_index_never_answers() {
    let host = host(None);
    let mut source = opened(&host);
    assert_eq!(source.search("contract", Narrow::All), Answer::Unavailable);
    assert!(!source.wired());
    assert_eq!(
        host.builds.load(Ordering::SeqCst),
        0,
        "nothing is read without an account"
    );

    host.sign_in_as(Some(ME));
    assert!(!source.wired());
    source.opened();
    settle(&mut source);
    assert_eq!(found(&mut source, "draft", Narrow::All).len(), 1);

    host.sign_in_as(Some(OTHER));
    assert_eq!(
        source.search("draft", Narrow::All),
        Answer::Unavailable,
        "at once, before a pump: another account's index is not this account's"
    );
    assert!(!source.wired());
    assert_eq!(source.details("f-draft"), None);
    assert!(source.pump().changed, "the window's side drops it");
    assert!(
        source.held.is_none() && source.found.is_none() && source.wanted.is_none(),
        "no name of the first account stays in memory"
    );
}

/// A lock ends the session: the same account's index and results are not handed out, at once, and are gone from
/// memory at the next pump; an unlock is another session, which builds its own when Search opens.
#[test]
fn a_lock_drops_the_index_and_the_results() {
    let host = host(Some(ME));
    let mut source = opened(&host);
    assert_eq!(found(&mut source, "draft", Narrow::All).len(), 1);

    host.lock();
    assert!(!source.wired(), "at once, before a pump");
    assert_eq!(source.search("draft", Narrow::All), Answer::Unavailable);
    assert_eq!(source.details("f-draft"), None);
    assert!(source.pump().changed);
    assert!(source.held.is_none() && source.found.is_none() && source.wanted.is_none());

    host.unlock();
    assert!(!source.wired(), "the session before the lock is not this one");
    assert_eq!(
        host.builds.load(Ordering::SeqCst),
        1,
        "nothing is built until Search opens"
    );
    source.opened();
    settle(&mut source);
    assert_eq!(host.builds.load(Ordering::SeqCst), 2);
    assert_eq!(found(&mut source, "draft", Narrow::All).len(), 1);
}

/// An index whose build began in one session and lands after a lock is never held, not for the unlock after it
/// either.
#[test]
fn an_index_landing_after_a_lock_is_never_held() {
    let host = host(Some(ME));
    host.hold_the_next_build();
    let mut source = EngineSearchSource::new(host.clone());
    source.opened();
    source.ask("draft", Narrow::All);
    assert!(source.wired(), "the first index is on its way");
    assert_eq!(
        source.search("draft", Narrow::All),
        Answer::Searching,
        "loading, never \"not available\" and never \"no results\""
    );
    host.build_is_held();
    host.lock();
    assert!(!source.wired(), "at once");
    assert_eq!(source.search("draft", Narrow::All), Answer::Unavailable);
    source.pump();
    let woken = host.woken.load(Ordering::SeqCst);
    host.let_the_build_go();
    host.woken_past(woken);
    // The one pump that takes the answer in: it is dropped there, not held until a later pump.
    source.pump();
    assert!(source.held.is_none(), "an index of a session that ended is in memory");
    assert!(
        source.wanted.is_none(),
        "what was asked in a session that ended is in memory"
    );
    settle(&mut source);
    assert!(source.held.is_none());
    assert!(source.found.is_none());
    assert!(!source.wired());

    host.unlock();
    settle(&mut source);
    assert!(
        source.held.is_none() && !source.wired(),
        "the unlock is another session"
    );
    assert_eq!(source.search("draft", Narrow::All), Answer::Unavailable);
}

/// The same, landing after another account signed in.
#[test]
fn an_index_landing_after_another_account_signed_in_is_never_held() {
    let host = host(Some(ME));
    host.hold_the_next_build();
    let mut source = EngineSearchSource::new(host.clone());
    source.opened();
    source.ask("draft", Narrow::All);
    host.build_is_held();
    host.sign_in_as(Some(OTHER));
    source.pump();
    let woken = host.woken.load(Ordering::SeqCst);
    host.let_the_build_go();
    host.woken_past(woken);
    source.pump();
    assert!(source.held.is_none(), "the first account's index is in memory");
    assert!(source.wanted.is_none(), "what the first account asked is in memory");
    settle(&mut source);
    assert!(source.held.is_none());
    assert!(source.found.is_none());
    assert!(!source.wired());
    assert_eq!(source.search("draft", Narrow::All), Answer::Unavailable);
    assert_eq!(source.details("f-draft"), None);
}

/// A result whose query began in one session and lands after a lock, or after another account signed in, is
/// never kept. (Whether it lands before or after the window looks, it is taken in after the change.)
#[test]
fn a_result_landing_after_its_session_ended_is_never_kept() {
    for ends in [Host::lock as fn(&Host), |host: &Host| host.sign_in_as(Some(OTHER))] {
        let host = host(Some(ME));
        let mut source = opened(&host);
        source.ask("draft", Narrow::All);
        ends(&host);
        settle(&mut source);
        assert!(source.found.is_none(), "a result of a session that ended is in memory");
        assert!(source.held.is_none());
        assert_eq!(source.search("draft", Narrow::All), Answer::Unavailable);
    }
}

/// Nothing is started by a read: `wired`, `search` and `details` build no index and run no query, however often a
/// draw asks.
#[test]
fn a_read_starts_nothing() {
    let host = host(Some(ME));
    let mut source = EngineSearchSource::new(host.clone());
    for _ in 0..20 {
        assert!(!source.wired());
        assert_eq!(source.search("draft", Narrow::All), Answer::Unavailable);
        assert_eq!(source.details("f-draft"), None);
    }
    assert!(!source.pump().busy);
    assert_eq!(
        host.builds.load(Ordering::SeqCst),
        0,
        "no index is built before Search opens"
    );

    source.opened();
    settle(&mut source);
    let woken = host.woken.load(Ordering::SeqCst);
    for _ in 0..20 {
        assert_eq!(source.search("draft", Narrow::All), Answer::Searching);
    }
    assert!(!source.pump().busy, "no query runs before the window asks for it");
    assert_eq!(host.woken.load(Ordering::SeqCst), woken);
    assert_eq!(host.builds.load(Ordering::SeqCst), 1);
}

fn texts(rt: &Runtime<State>) -> Vec<String> {
    rt.elements().into_iter().filter_map(|e| e.text).collect()
}

/// Frames until the source has no answer on its way (at most 30 s).
fn frames(rt: &mut Runtime<State>) {
    let began = Instant::now();
    loop {
        for _ in 0..3 {
            rt.frame(0.016);
            rt.settle_all();
        }
        if !rt.state.frames.find.source.pump().busy {
            break;
        }
        assert!(began.elapsed() < HOLD_FOR, "an answer did not come back");
        std::thread::sleep(Duration::from_millis(5));
    }
    rt.frame(0.016);
    rt.settle_all();
}

/// In the window, through its own poll and draw: the first account's results are drawn; after a lock, and after
/// another account signed in, no name of the first account is in the element tree.
#[test]
fn the_window_draws_no_result_of_a_session_that_ended() {
    for ends in [Host::lock as fn(&Host), |host: &Host| host.sign_in_as(Some(OTHER))] {
        let host = host(Some(ME));
        let mut state = crate::init();
        state.frames.find.source = Box::new(EngineSearchSource::new(host.clone()));
        state.frames.find.open = true;
        state.search = TextEdit::new("contract");
        let env = Env {
            system_dark: false,
            scale: 2.0,
        };
        let mut rt = Runtime::new(state, crate::view, (958.0, 1100.0), env);
        rt.on_key = Some(crate::shell::host::on_key);
        rt.poll = Some(crate::shell::host::poll);
        frames(&mut rt);
        assert!(
            texts(&rt).iter().any(|t| t.contains("Contract draft.pdf")),
            "the control: the first account's result is drawn"
        );
        assert_eq!(host.builds.load(Ordering::SeqCst), 1, "built when Search opened, once");

        ends(&host);
        frames(&mut rt);
        let drawn = texts(&rt);
        for theirs in ["Contract", "contract costs", "CONTRACT scan", "Legal"] {
            assert!(
                !drawn.iter().any(|t| t.contains(theirs) && t != "contract"),
                "{theirs} of the session that ended is drawn"
            );
        }
        assert!(!rt.state.frames.find.source.wired());
    }
}

#[test]
fn recent_searches_are_never_kept() {
    let mut source = opened(&host(Some(ME)));
    source.remember("contract");
    assert!(
        source.recent().is_empty(),
        "no store for a search term yet: nothing kept"
    );
    source.clear_recent();
    assert!(source.recent().is_empty());
}

#[test]
fn the_engine_handle_signed_out_is_unavailable() {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "search-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: "http://127.0.0.1:9".into(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    let mut source = EngineSearchSource::new(Arc::new(engine));
    source.opened();
    assert!(!source.pump().busy, "signed out: nothing is built");
    assert_eq!(SearchHost::account(&*source.host), None);
    assert!(!SearchHost::unlocked(&*source.host));
    assert!(!source.wired());
    assert_eq!(source.search("contract", Narrow::All), Answer::Unavailable);
}

#[test]
fn a_query_runs_on_a_worker_and_a_result_answers_only_the_query_that_asked() {
    let host = host(Some(ME));
    let mut source = opened(&host);
    let woken = host.woken.load(Ordering::SeqCst);
    source.ask("contract", Narrow::All);
    assert_eq!(
        source.search("contract", Narrow::All),
        Answer::Searching,
        "not answered on the window's thread"
    );
    assert_eq!(source.search("draft", Narrow::All), Answer::Searching);
    let hits = found(&mut source, "draft", Narrow::All);
    assert_eq!(
        ids(&hits),
        ["f-draft"],
        "the answer to \"contract\" never stands for \"draft\""
    );
    assert!(
        host.woken.load(Ordering::SeqCst) > woken,
        "the window is woken when a result lands"
    );
    // The same words on another pill are another query.
    assert_eq!(source.search("draft", Narrow::Photos), Answer::Searching);
    assert!(found(&mut source, "draft", Narrow::Photos).is_empty());
}

/// The engine handle as the host: whose account, the index of its local state, and the wake.
#[test]
fn the_engine_handle_is_a_search_host() {
    let woken = Arc::new(AtomicUsize::new(0));
    let count = woken.clone();
    let (channel, _messages) = ChannelHost::new(move || {
        count.fetch_add(1, Ordering::SeqCst);
    });
    let config = EngineConfig {
        account_id: "search-host-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: "http://127.0.0.1:9".into(),
    };
    let engine = Engine::open(&TEST, config, channel, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    assert_eq!(SearchHost::account(&engine), None, "signed out");
    assert!(!SearchHost::unlocked(&engine), "no session in memory");
    assert_eq!(
        SearchHost::turn(&engine),
        engine.session_turn(),
        "the engine's own turn"
    );
    let before = woken.load(Ordering::SeqCst);
    SearchHost::wake(&engine);
    assert_eq!(woken.load(Ordering::SeqCst), before + 1, "the host's loop is woken");
    // The index goes through the account-bound handle (task 2488): a local state with rows, recorded as an
    // account's, builds nothing while no one is signed in, and the handle says why.
    let dir = tempfile::tempdir().expect("a temp folder");
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).expect("a state database");
    put(
        &db,
        "f-draft",
        "/Contract draft.pdf",
        FileStatus::Local,
        1_258_291,
        false,
    );
    db.set_owner(&beebeeb_engine::account_binding::Identity::new(
        Some("u-invented"),
        Some(ME),
    ))
    .unwrap();
    assert_eq!(
        build_local_index(&db).expect("a direct build").records.len(),
        1,
        "the state has a row"
    );
    drop(db);
    engine.use_local_state_for_test(path);
    assert!(
        SearchHost::index(&engine).is_err(),
        "signed out: no index of the local state"
    );
    assert_eq!(
        engine.local_state_refusal(),
        Some(beebeeb_engine::app::LocalStateRefused::SignedOut)
    );
}

/// While `SEARCH_ON_ENGINE` is false (task 2488), a real build's shell gets the unwired Search, and no part of the
/// adapter runs: none is made, so no index is built and no worker starts.
#[test]
fn a_real_build_shell_gets_the_unwired_search_and_runs_no_part_of_the_adapter() {
    assert!(
        !crate::shell::state::SEARCH_ON_ENGINE,
        "task 2488 decides when Search reads the engine"
    );
    let (channel, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "search-shell-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: "http://127.0.0.1:9".into(),
    };
    let engine = Engine::open(&TEST, config, channel, MemoryCredentials::default()).expect("the engine opens");
    let before = made_here();
    let s = crate::shell::state::init_on_engine(crate::shell::host::Shell::fixture(), Arc::new(engine));
    assert_eq!(made_here(), before, "no Search adapter was made");
    assert!(!s.frames.find.source.wired());
    assert_eq!(
        s.frames.find.source.search("contract", Narrow::All),
        Answer::Unavailable
    );
    assert_eq!(
        s.frames.find.shows("contract"),
        crate::frames::search::Shows::Unavailable
    );
}

/// The window's own forget (`frames::search::poll`): the WINDOW's account changes while the source's host still
/// says the session it had. The window tells the source to forget, so nothing the source held is drawn; what is
/// read next is held back here, so nothing has replaced it.
#[test]
fn the_window_tells_the_source_to_forget_when_its_account_changes() {
    use crate::shell::app::{Event, GateState, Session};
    let host = host(Some(ME));
    let mut state = crate::init();
    state.frames.find.source = Box::new(EngineSearchSource::new(host.clone()));
    state.frames.find.open = true;
    state.search = TextEdit::new("contract");
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, crate::view, (958.0, 1100.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.poll = Some(crate::shell::host::poll);
    frames(&mut rt);
    assert!(
        texts(&rt).iter().any(|t| t.contains("Contract draft.pdf")),
        "the control: the result is drawn"
    );

    host.hold_the_next_build();
    rt.state
        .shell
        .app
        .apply(Event::Session(Session::of(GateState::Ready, "window-other@example.eu")));
    rt.state.frames.find.open = true;
    rt.state.search = TextEdit::new("contract");
    for _ in 0..3 {
        rt.frame(0.016);
        rt.settle_all();
    }
    let drawn = texts(&rt);
    for theirs in ["Contract", "contract costs", "CONTRACT scan", "Legal"] {
        assert!(
            !drawn.iter().any(|t| t.contains(theirs) && t != "contract"),
            "{theirs} is drawn after the window's account changed"
        );
    }
    assert_eq!(
        rt.state.frames.find.source.search("contract", Narrow::All),
        Answer::Searching,
        "forgotten, and read again"
    );
    host.build_is_held();
    host.let_the_build_go();
    frames(&mut rt);
}

/// A `{:?}` of a result, of the index or of one of its records prints no name, no place and no id.
#[test]
fn a_result_and_the_index_print_no_name() {
    let host = host(Some(ME));
    let mut source = opened(&host);
    let hits = found(&mut source, "contract", Narrow::All);
    let index = SearchHost::index(&*host).expect("the index");
    let record = index.records.get("f-signed").expect("a record");
    assert_eq!(record.name, "Contract signed 2025.pdf");
    let printed = format!(
        "{hits:?} {index:?} {record:?} {:?}",
        source.search("contract", Narrow::All)
    );
    for secret in [
        "Contract",
        "contract",
        "CONTRACT",
        "Legal",
        "Documents",
        "f-signed",
        "f-draft",
    ] {
        assert!(!printed.contains(secret), "{secret} is printed: {printed}");
    }
    assert!(printed.contains("Hit") && printed.contains("records: 5"), "{printed}");
}
