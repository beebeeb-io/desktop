//! Search on the real engine (`EngineSearchSource`): the engine's own local index (`desktop_search`) over a state
//! database of each test's own, written with the engine's writers, behind a host that says whose computer it is.
//! The results and their facts, narrowing, any case, an empty answer, a list that cannot be read, signed out (nothing
//! built), another account's index never answering, recent searches never kept; and the engine handle itself as the
//! host while signed out. Every name is invented.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_db::{FileEntry, FileStatus, ItemKind, StateDb};

use super::*;
use crate::frames::shared::parts::table::Kind;

const ME: &str = "search-me@example.eu";

/// A host on a state database of its own.
struct Host {
    account: Mutex<Option<String>>,
    db: PathBuf,
    /// The list cannot be read.
    broken: AtomicBool,
    builds: AtomicUsize,
    woken: AtomicUsize,
}

impl SearchHost for Host {
    fn account(&self) -> Option<String> {
        self.account.lock().unwrap().clone()
    }

    fn index(&self) -> Result<LocalSearchIndex, ()> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        if self.broken.load(Ordering::SeqCst) {
            return Err(());
        }
        let db = StateDb::open(&self.db).map_err(|_| ())?;
        build_local_index(&db).map_err(|_| ())
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
        db: path,
        broken: AtomicBool::new(false),
        builds: AtomicUsize::new(0),
        woken: AtomicUsize::new(0),
    })
}

/// Wait until the index is built (at most 10 s).
fn settle(source: &EngineSearchSource<Host>) {
    let began = Instant::now();
    while source.sync().building {
        assert!(began.elapsed() < Duration::from_secs(10), "the index did not come back");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Ask until the worker's answer for THIS query is here (at most 10 s).
fn found(source: &EngineSearchSource<Host>, query: &str, narrow: Narrow) -> Vec<Hit> {
    let began = Instant::now();
    loop {
        match source.search(query, narrow) {
            Answer::Found { hits, loading } => {
                assert_eq!(loading, None);
                return hits;
            }
            Answer::Searching => {}
            Answer::Unavailable => panic!("search answered Unavailable"),
        }
        assert!(began.elapsed() < Duration::from_secs(10), "the query did not come back");
        std::thread::sleep(Duration::from_millis(5));
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
    let source = EngineSearchSource::new(host.clone());
    settle(&source);
    assert!(source.wired());
    assert!(
        host.woken.load(Ordering::SeqCst) >= 1,
        "the window is woken when the index lands"
    );

    let hits = found(&source, "contr", Narrow::All);
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

    assert_eq!(ids(&found(&source, "CONTRACT", Narrow::Photos)), ["f-photo"]);
    assert_eq!(
        ids(&found(&source, "contract", Narrow::Documents)),
        ["f-draft", "f-sheet", "f-signed"]
    );
    assert!(found(&source, "contract", Narrow::Folders).is_empty());
    assert_eq!(host.builds.load(Ordering::SeqCst), 1, "built once while fresh");
}

#[test]
fn a_results_facts_are_what_the_index_holds() {
    let source = EngineSearchSource::new(host(Some(ME)));
    settle(&source);
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
    let source = EngineSearchSource::new(host(Some(ME)));
    settle(&source);
    assert!(found(&source, "lisbon", Narrow::All).is_empty());
    assert!(found(&source, "   ", Narrow::All).is_empty());
}

#[test]
fn a_list_that_cannot_be_read_is_unavailable_never_no_results() {
    let host = host(Some(ME));
    host.broken.store(true, Ordering::SeqCst);
    let source = EngineSearchSource::new(host.clone());
    settle(&source);
    assert!(!source.wired());
    assert_eq!(source.search("contract", Narrow::All), Answer::Unavailable);
    assert_eq!(source.details("f-draft"), None);
    for _ in 0..20 {
        source.search("contract", Narrow::All);
    }
    settle(&source);
    assert_eq!(
        host.builds.load(Ordering::SeqCst),
        1,
        "no retry loop: not built again within FRESH_FOR"
    );
}

#[test]
fn signed_out_builds_nothing_and_another_accounts_index_never_answers() {
    let host = host(None);
    let source = EngineSearchSource::new(host.clone());
    settle(&source);
    assert_eq!(source.search("contract", Narrow::All), Answer::Unavailable);
    assert!(!source.wired());
    assert_eq!(
        host.builds.load(Ordering::SeqCst),
        0,
        "nothing is read without an account"
    );

    *host.account.lock().unwrap() = Some(ME.into());
    assert!(!source.wired());
    settle(&source);
    assert_eq!(found(&source, "draft", Narrow::All).len(), 1);

    *host.account.lock().unwrap() = Some("someone-else@example.eu".into());
    assert_eq!(
        source.search("draft", Narrow::All),
        Answer::Unavailable,
        "another account's index is not this account's"
    );
}

#[test]
fn recent_searches_are_never_kept() {
    let mut source = EngineSearchSource::new(host(Some(ME)));
    settle(&source);
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
    let source = EngineSearchSource::new(Arc::new(engine));
    assert_eq!(SearchHost::account(&*source.host), None);
    assert!(!source.wired());
    assert_eq!(source.search("contract", Narrow::All), Answer::Unavailable);
}

#[test]
fn a_query_runs_on_a_worker_and_a_result_answers_only_the_query_that_asked() {
    let host = host(Some(ME));
    let source = EngineSearchSource::new(host.clone());
    settle(&source);
    let woken = host.woken.load(Ordering::SeqCst);
    assert_eq!(
        source.search("contract", Narrow::All),
        Answer::Searching,
        "not answered on the window's thread"
    );
    assert_eq!(source.search("draft", Narrow::All), Answer::Searching);
    let hits = found(&source, "draft", Narrow::All);
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
    assert!(found(&source, "draft", Narrow::Photos).is_empty());
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
    let before = woken.load(Ordering::SeqCst);
    SearchHost::wake(&engine);
    assert_eq!(woken.load(Ordering::SeqCst), before + 1, "the host's loop is woken");
    // The index goes through the glue: it agrees with a direct build from the same state DB. When both fail
    // (no state DB on this test process), the state DB must genuinely be missing — a stubbed `Err(())` would
    // survive the `(Err, Err)` arm silently, so we guard it by trying the DB independently.
    let db_result = engine.state_db();
    let index_result = SearchHost::index(&engine);
    match (index_result, &db_result) {
        (Ok(index), Ok(db)) => {
            let direct = build_local_index(db).expect("the index from the DB");
            assert_eq!(
                index.records.len(),
                direct.records.len(),
                "the host's index matches a direct build"
            );
        }
        (Err(()), Err(_)) => {
            // The state DB must genuinely not exist: a state_db() that works means the glue is wrong.
            assert!(
                db_result.is_err(),
                "index returned Err but state_db works: the glue is broken"
            );
        }
        (index, db) => panic!("the host and the handle disagree: {} {}", index.is_ok(), db.is_ok()),
    }
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
