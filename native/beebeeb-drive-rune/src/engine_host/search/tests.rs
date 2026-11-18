//! Search on the real engine (`EngineSearchSource`): the engine's own local index (`desktop_search`) over a state
//! database of each test's own, written with the engine's writers, behind a host that says whose computer it is.
//! The results and their facts, narrowing, any case, an empty answer, a list that cannot be read, signed out (nothing
//! built), another account's index never answering, the recent searches as the engine keeps them (for the session
//! of the moment only, never asked from a draw); and the engine handle itself as the host while signed out. Whose
//! answer: an index, a result or a recent search that was started in one session and lands after a lock or after
//! another account signed in is dropped (in memory, at the readers and in the element tree), and nothing is started
//! by a read. Every name is invented. The real engine with two accounts: `tests_accounts.rs`.

use std::collections::HashMap;
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
use crate::frames::search::source::RECENT_KEPT;
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
    /// Nothing has synced on this computer yet.
    nothing_synced: AtomicBool,
    builds: AtomicUsize,
    woken: AtomicUsize,
    /// The recent searches each account keeps on this computer (the engine's row per account).
    recents: Mutex<HashMap<String, Vec<String>>>,
    /// How often the recent searches were asked of the host (read, kept or cleared).
    recent_calls: AtomicUsize,
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

    /// As the engine's handle: whose recent searches the session `turn` may read and change. Only the account the
    /// state is recorded for, with the vault open, and only while `turn` is the session of the moment.
    fn recents_of(&self, turn: u64) -> Result<String, ()> {
        self.recent_calls.fetch_add(1, Ordering::SeqCst);
        if turn != self.turn() || !self.unlocked() || self.account() != self.owner {
            return Err(());
        }
        self.account().ok_or(())
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

    fn index(&self) -> Result<LocalSearchIndex, LocalStateRefused> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        if self.broken.load(Ordering::SeqCst) {
            return Err(LocalStateRefused::Unreadable);
        }
        if self.nothing_synced.load(Ordering::SeqCst) {
            return Err(LocalStateRefused::NothingSynced);
        }
        // As the engine's handle: the list of the account it is recorded for, with the vault open; another account's
        // is refused with the engine's own word.
        if !self.unlocked() {
            return Err(LocalStateRefused::Locked);
        }
        if self.account() != self.owner {
            return Err(LocalStateRefused::OtherAccount);
        }
        let db = StateDb::open(&self.db).map_err(|_| LocalStateRefused::Unreadable)?;
        let index = build_local_index(&db).map_err(|_| LocalStateRefused::Unreadable);
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

    fn recent_searches(&self, turn: u64) -> Result<Vec<String>, ()> {
        let account = self.recents_of(turn)?;
        Ok(self.recents.lock().unwrap().get(&account).cloned().unwrap_or_default())
    }

    fn remember_search(&self, turn: u64, query: &str) -> Result<Vec<String>, ()> {
        let account = self.recents_of(turn)?;
        let mut recents = self.recents.lock().unwrap();
        let list = recents.entry(account).or_default();
        list.retain(|earlier| earlier.to_lowercase() != query.to_lowercase());
        list.insert(0, query.to_string());
        list.truncate(RECENT_KEPT);
        Ok(list.clone())
    }

    fn clear_recent_searches(&self, turn: u64) -> Result<(), ()> {
        let account = self.recents_of(turn)?;
        self.recents.lock().unwrap().remove(&account);
        Ok(())
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
        nothing_synced: AtomicBool::new(false),
        builds: AtomicUsize::new(0),
        woken: AtomicUsize::new(0),
        recents: Mutex::new(HashMap::new()),
        recent_calls: AtomicUsize::new(0),
    })
}

/// Pump until no build and no query is on its way (at most 30 s).
fn settle(source: &mut EngineSearchSource<Host>) {
    let began = Instant::now();
    // A refusal that waits to be built again is not an answer on its way.
    while source.pump().busy && (source.building.is_some() || source.asked.is_some() || source.recent_out.is_some()) {
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
        Answer::NotThisAccounts => panic!("search answered NotThisAccounts"),
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
        source.ask("contract", Narrow::All);
        source.pump();
    }
    assert_eq!(
        host.builds.load(Ordering::SeqCst),
        1,
        "no retry loop: not built again before RETRY_AFTER"
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

/// Row S8, the source alone: another account over this state is told why Search is not available
/// (`NotThisAccounts`, the engine's refusal), with the field empty too, and only in that session: once the session
/// moves on (a lock, then the owner's own sign-in) the reason is not said, at once (before a pump), and the window's
/// side drops it at the next pump. A list that cannot be read stays `Unavailable`
/// (`a_list_that_cannot_be_read_is_unavailable_never_no_results`). Mutant: `not_this_accounts` reads the refusal
/// whatever its session (red at "another session, before a pump").
#[test]
fn another_accounts_refusal_is_said_only_to_its_own_session() {
    let host = host(Some(OTHER));
    let mut source = opened(&host);
    assert_eq!(
        host.builds.load(Ordering::SeqCst),
        1,
        "the control: the engine was asked for an index"
    );
    assert!(!source.wired(), "nothing of another account's state is held");
    assert!(source.held.is_none());
    for query in ["draft", "contract", "", "   "] {
        assert_eq!(
            source.search(query, Narrow::All),
            Answer::NotThisAccounts,
            "{query:?}: the second account is not told why"
        );
    }
    assert_eq!(source.details("f-draft"), None);
    assert!(source.recent().is_empty());

    host.lock();
    assert_eq!(
        source.search("draft", Narrow::All),
        Answer::Unavailable,
        "locked, before a pump"
    );
    host.sign_in_as(Some(ME));
    assert_eq!(
        source.search("draft", Narrow::All),
        Answer::Unavailable,
        "another session, before a pump: the reason was the other account's session alone"
    );
    assert!(source.pump().changed, "the window's side drops the reason");
    assert!(
        source.refused.is_none(),
        "a refusal of a session that ended is in memory"
    );
    assert_eq!(source.search("draft", Narrow::All), Answer::Unavailable);
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
    let before_open = host.woken.load(Ordering::SeqCst);
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
    // `opened` also started a read of the recent searches: its wake lands first (the build is held, and no query
    // runs without an index, so it is the only wake there can be), and the wake waited for below is the build's.
    host.woken_past(before_open);
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
    let before_open = host.woken.load(Ordering::SeqCst);
    source.opened();
    source.ask("draft", Narrow::All);
    host.build_is_held();
    host.sign_in_as(Some(OTHER));
    source.pump();
    // As above: the recent read `opened` started has woken the window before the wake of the build is waited for.
    host.woken_past(before_open);
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

/// The recent searches are the engine's, for the session of the moment only: shown once the engine answered, in the
/// order asked, cleared by Clear (the engine deletes them). At a lock and at another account's sign-in they go at once
/// (before a pump) and from memory at the next pump; another account over this state reads and keeps none. A draw
/// never asks the engine.
#[test]
fn recent_searches_come_from_the_engine_for_this_session_only() {
    let host = host(Some(ME));
    let mut source = opened(&host);
    assert!(source.recent().is_empty(), "none kept yet");
    source.remember("contract");
    source.remember("  Lisbon ");
    source.remember("   ");
    assert!(source.recent().is_empty(), "nothing shown before the engine answered");
    settle(&mut source);
    assert_eq!(source.recent(), ["Lisbon", "contract"]);
    let calls = host.recent_calls.load(Ordering::SeqCst);
    for _ in 0..20 {
        let _ = source.recent();
    }
    assert_eq!(
        host.recent_calls.load(Ordering::SeqCst),
        calls,
        "a draw never asks the engine"
    );

    source.clear_recent();
    settle(&mut source);
    assert!(source.recent().is_empty());
    assert!(
        host.recents.lock().unwrap().get(ME).is_none(),
        "the engine deleted them"
    );
    source.remember("invoice 2026");
    settle(&mut source);
    assert_eq!(source.recent(), ["invoice 2026"]);

    // A lock: gone at once, and from memory at the next pump; the unlock reads them again when Search opens.
    host.lock();
    assert!(source.recent().is_empty(), "at once, before a pump");
    assert!(source.pump().changed, "the window's side drops them");
    assert!(source.recent.is_none() && source.recent_next.is_empty());
    host.unlock();
    source.opened();
    settle(&mut source);
    assert_eq!(source.recent(), ["invoice 2026"], "the same account's, read again");

    // Another account over this state: nothing of the first account's, and nothing kept for it either.
    host.sign_in_as(Some(OTHER));
    assert!(source.recent().is_empty(), "at once, before a pump");
    source.opened();
    source.remember("lisbon");
    settle(&mut source);
    assert!(source.recent().is_empty());
    assert!(host.recents.lock().unwrap().get(OTHER).is_none());
    assert_eq!(
        host.recents.lock().unwrap().get(ME).map(Vec::len),
        Some(1),
        "the first account's are untouched"
    );
}

/// Recent searches read or changed in one session that land after a lock, or after another account signed in, are
/// never kept (whether they land before or after the window looks, they are taken in after the change).
#[test]
fn recent_searches_landing_after_their_session_ended_are_never_kept() {
    for ends in [Host::lock as fn(&Host), |host: &Host| host.sign_in_as(Some(OTHER))] {
        let host = host(Some(ME));
        let mut source = opened(&host);
        source.remember("contract");
        ends(&host);
        settle(&mut source);
        assert!(
            source.recent.is_none(),
            "a recent search of a session that ended is in memory"
        );
        assert!(source.recent().is_empty());
    }
}

/// The window shows as many recent searches as the engine keeps (DI-0801: up to 10).
#[test]
fn the_window_and_the_engine_keep_as_many_recent_searches() {
    assert_eq!(beebeeb_engine::app::bindings::search_recent::SEARCHES_KEPT, RECENT_KEPT);
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

/// With `SEARCH_ON_ENGINE` (task 2488 fixed), a real build's shell gets the engine's Search, and nothing of it runs
/// before Search opens: signed out, it says it is not available, builds no index and reads no recent search.
#[test]
fn a_real_build_shell_gets_the_engine_search_and_nothing_runs_before_it_opens() {
    assert!(
        crate::shell::state::SEARCH_ON_ENGINE,
        "task 2488 is fixed: Search reads the engine"
    );
    let (channel, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "search-shell-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: "http://127.0.0.1:9".into(),
    };
    let engine = Engine::open(&TEST, config, channel, MemoryCredentials::default()).expect("the engine opens");
    let before = made_here();
    let mut s = crate::shell::state::init_on_engine(crate::shell::host::Shell::fixture(), Arc::new(engine));
    assert_eq!(made_here(), before + 1, "the shell made the engine's Search");
    assert!(!s.frames.find.source.wired());
    assert!(
        !s.frames.find.source.pump().busy,
        "nothing is built or read before Search opens"
    );
    assert!(s.frames.find.source.recent().is_empty());
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

/// The ways the turn moves on after A searched: a lock; a lock and an unlock (the same account in another turn, with
/// no pump between them); another account's sign-in; a sign-out.
fn turn_moves() -> [(&'static str, fn(&Host)); 4] {
    [
        ("a lock", Host::lock as fn(&Host)),
        ("a lock and an unlock", |host: &Host| {
            host.lock();
            host.unlock();
        }),
        ("another account's sign-in", |host: &Host| host.sign_in_as(Some(OTHER))),
        ("a sign-out", |host: &Host| host.sign_in_as(None)),
    ]
}

/// A's files, by id: once A's turn ended, no reader hands out the facts of one of them.
const MINE: [&str; 5] = ["f-draft", "f-signed", "f-sheet", "f-photo", "f-other"];

/// The reader gate (`current`), alone: once the turn moved on, `wired`, `search` and `details` hand out nothing of the
/// index of the turn that ended, even while it is still in memory because nothing ran `follow` yet (no pump, no ask,
/// no open). The lock and the unlock keep the account: a gate on the account alone hands A's names out there.
/// Mutant (M1): `current` hands out what is held, whatever the session.
#[test]
fn current_never_hands_out_an_ended_turns_index() {
    for (said, move_on) in turn_moves() {
        let host = host(Some(ME));
        let mut source = opened(&host);
        assert_eq!(
            found(&mut source, "contract", Narrow::All).len(),
            4,
            "{said}: the control: A's names are found"
        );
        assert!(source.details("f-draft").is_some(), "{said}: the control");

        move_on(&host);
        assert!(
            source.held.is_some() && source.found.is_some(),
            "{said}: the control: nothing ran follow, A's index and result are still in memory"
        );
        assert!(
            source.current().is_none(),
            "{said}: the index of a turn that ended is handed out"
        );
        assert!(
            !source.wired(),
            "{said}: Search says it answers from an ended turn's index"
        );
        assert_eq!(
            source.search("contract", Narrow::All),
            Answer::Unavailable,
            "{said}: A's results are handed out"
        );
        for id in MINE {
            assert_eq!(source.details(id), None, "{said}: the facts of A's {id} are handed out");
        }
    }
}

/// `follow`, alone: at the first pump after the turn moved on (the window's poll), everything of the turn that ended
/// goes from memory (A's index, result, words, recent searches and the work on its way), and the source belongs to the
/// turn of the moment. The lock and the unlock run with no pump between them: the account is the same, only the turn
/// says the session ended. Mutants (M2a): `follow` drops nothing; (M2b): `follow` compares the account, not the turn.
#[test]
fn follow_forgets_on_turn_change() {
    for (said, move_on) in turn_moves() {
        let host = host(Some(ME));
        let mut source = opened(&host);
        assert_eq!(
            found(&mut source, "contract", Narrow::All).len(),
            4,
            "{said}: the control"
        );
        source.remember("contract");
        settle(&mut source);
        assert_eq!(source.recent(), ["contract"], "{said}: the control");
        assert!(
            source.held.is_some() && source.found.is_some() && source.wanted.is_some() && source.recent.is_some(),
            "{said}: the control: A's state is in memory"
        );

        move_on(&host);
        assert!(
            source.pump().changed,
            "{said}: the window is not told that what it drew went"
        );
        assert!(source.held.is_none(), "{said}: A's index is in memory");
        assert!(source.found.is_none(), "{said}: A's result is in memory");
        assert!(source.wanted.is_none(), "{said}: A's words are in memory");
        assert!(
            source.recent.is_none() && source.recent_next.is_empty(),
            "{said}: A's recent searches are in memory"
        );
        assert!(
            source.building.is_none() && source.asked.is_none() && source.recent_out.is_none(),
            "{said}: work of A's turn is still waited for"
        );
        assert!(
            source.of == session_of(&*host),
            "{said}: the source does not follow the turn of the moment"
        );
    }
}

/// The take-in compare, alone: a build that answers for a turn the window's state does not belong to is dropped:
/// never held, never counted as fresh, and the words asked are not run on it. In the app this happens inside ONE
/// pump: its `follow` saw A's turn, then the turn moved on, and the build ran wholly in the new turn and was drained
/// by the same pump (the job check does not catch it: nothing forgot the build). A test cannot time that, so the
/// answer is handed to `take_in` as that pump hands it: the build the window waits for, A's names, and the session
/// its worker saw before and after (the one after the move; none after a lock or a sign-out). Another account's
/// answer holding A's names stands for an engine whose owner check let them through (task 2488's own layer,
/// `tests_accounts.rs`): this layer alone still drops it. Mutant (M3): the compare in `take_in` deleted.
#[test]
fn a_build_landing_under_a_new_turn_is_dropped() {
    for (said, move_on) in turn_moves() {
        let host = host(Some(ME));
        let theirs = build_local_index(&StateDb::open(&host.db).expect("the state")).expect("A's index");
        assert_eq!(theirs.records.len(), 5, "the control: the answer holds A's names");
        let mut source = EngineSearchSource::new(host.clone());
        source.opened();
        source.ask("contract", Narrow::All);
        let job = source.building.expect("the control: A's build is on its way");

        move_on(&host);
        // No pump: the window's state still belongs to A's turn, and the build is still waited for.
        source.take_in(Landed::Built(job, session_of(&*host), Ok(theirs)));
        assert!(source.held.is_none(), "{said}: an index of another turn is held");
        assert_eq!(source.installed, 0, "{said}: an index of another turn is installed");
        assert!(source.built_at.is_none(), "{said}: a dropped build counts as fresh");
        assert!(
            source.asked.is_none() && source.found.is_none(),
            "{said}: A's words ran on it"
        );
        assert!(!source.wired(), "{said}");
        assert_eq!(
            source.search("contract", Narrow::All),
            Answer::Unavailable,
            "{said}: A's results are handed out"
        );
        for id in MINE {
            assert_eq!(source.details(id), None, "{said}: the facts of A's {id} are handed out");
        }

        // The window's side: what the real workers send is nobody's; the turn of the moment builds its own when Search
        // opens (A after the unlock), and nothing for another account or with no one signed in.
        settle(&mut source);
        assert!(
            source.held.is_none() && source.wanted.is_none(),
            "{said}: A's index or words after the pump"
        );
        source.opened();
        settle(&mut source);
        let owner_now = SearchHost::unlocked(&*host) && SearchHost::account(&*host) == host.owner;
        assert_eq!(
            source.wired(),
            owner_now,
            "{said}: the turn of the moment does not build its own"
        );
    }
}

/// A first refusal while the first sync runs (NothingSynced), then sync is done: Search says what is true meanwhile
/// (not "Not available yet": the files are being got ready), and the next pump after the wait builds again, so
/// results appear without reopening Search.
#[test]
fn a_refusal_during_the_first_sync_is_built_again_when_sync_is_done() {
    let host = host(Some(ME));
    host.nothing_synced.store(true, Ordering::SeqCst);
    let mut source = EngineSearchSource::new(host.clone());
    source.opened();
    source.ask("contract", Narrow::All);
    settle(&mut source);
    assert_eq!(host.builds.load(Ordering::SeqCst), 1);
    source.retry_after = Duration::ZERO;
    assert!(source.held.is_none());
    assert!(source.wired(), "getting ready is not \"not available\"");
    assert_eq!(source.search("contract", Narrow::All), Answer::Searching);

    host.nothing_synced.store(false, Ordering::SeqCst);
    let began = Instant::now();
    while source.held.is_none() {
        source.pump();
        assert!(began.elapsed() < HOLD_FOR, "no build after sync was done");
        std::thread::sleep(Duration::from_millis(5));
    }
    settle(&mut source);
    assert_eq!(ids(&found(&mut source, "contract", Narrow::All)).len(), 4);
}

/// Reopening Search after a refusal builds at once, even within FRESH_FOR; a refusal is not "fresh".
#[test]
fn reopening_search_after_a_refusal_builds_again() {
    let host = host(Some(ME));
    host.nothing_synced.store(true, Ordering::SeqCst);
    let mut source = opened(&host);
    assert!(source.held.is_none());
    host.nothing_synced.store(false, Ordering::SeqCst);
    source.opened();
    settle(&mut source);
    assert_eq!(found(&mut source, "draft", Narrow::All).len(), 1);
}
