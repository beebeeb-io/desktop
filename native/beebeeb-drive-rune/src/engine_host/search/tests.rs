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

fn found(source: &EngineSearchSource<Host>, query: &str, narrow: Narrow) -> Vec<Hit> {
    match source.search(query, narrow) {
        Answer::Found { hits, loading } => {
            assert_eq!(loading, None);
            hits
        }
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
