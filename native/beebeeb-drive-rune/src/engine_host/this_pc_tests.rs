//! This PC on the real engine (`EngineThisPcSource`), against the engine's own state database in a folder of each
//! test's own: the history (transfers, Trash, the changes that did not sync), the problems with the engine's words
//! and Try again through the queue, the kept items with an honest status, Remove from this PC through the pin change,
//! no local state, a read that fails, no name while the vault is locked, the account on every answer, and nothing of
//! another account's local state (task 2488). Every name is invented.

use beebeeb_engine::app::LocalStateRefused;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beebeeb_engine::account_binding::Identity;
use beebeeb_engine::state_db::{
    FileEntry, FileStatus, ItemKind, LocalActivityEventInput, LocalActivityKind, OperationKind, PendingOperation,
    StateDb, TransferActivityInput,
};

use super::*;

/// The engine handle as the adapter sees it, on a state database in a temp folder.
struct Fake {
    _dir: tempfile::TempDir,
    path: PathBuf,
    unlocked: AtomicBool,
    syncing: AtomicBool,
    /// The state database cannot be opened (an engine error).
    broken: AtomicBool,
    /// The signed-in account (None: signed out).
    account: Mutex<Option<String>>,
    /// What `set_kept` was asked, in order.
    kept_calls: Mutex<Vec<(String, bool)>>,
}

impl Fake {
    fn new() -> Arc<Fake> {
        Self::with_account("invented@beebeeb.io")
    }

    fn with_account(email: &str) -> Arc<Fake> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let db = StateDb::open(&path).unwrap();
        // Record the owner so `owns` can compare it.
        db.set_owner(&Identity::new(None, Some(email))).unwrap();
        Arc::new(Fake {
            _dir: dir,
            path,
            unlocked: AtomicBool::new(true),
            syncing: AtomicBool::new(true),
            broken: AtomicBool::new(false),
            account: Mutex::new(Some(email.to_string())),
            kept_calls: Mutex::new(Vec::new()),
        })
    }

    fn db(&self) -> StateDb {
        StateDb::open(&self.path).unwrap()
    }
}

impl ThisPcEngine for Fake {
    fn unlocked(&self) -> bool {
        self.unlocked.load(Ordering::SeqCst)
    }
    fn account(&self) -> Option<String> {
        self.account.lock().unwrap().clone()
    }
    fn local_state<T>(&self, read: impl FnOnce(&StateDb) -> T) -> Result<T, LocalStateRefused> {
        if self.broken.load(Ordering::SeqCst) {
            return Err(LocalStateRefused::Unreadable);
        }
        let db = StateDb::open(&self.path).map_err(|_| LocalStateRefused::Unreadable)?;
        let session = Identity::new(None, self.account.lock().unwrap().as_deref());
        if !beebeeb_engine::app::state_owner::owned_by(&db, &session) {
            return Err(LocalStateRefused::OtherAccount);
        }
        Ok(read(&db))
    }
    fn change_local_state<T>(&self, change: impl FnOnce(&StateDb) -> T) -> Result<T, LocalStateRefused> {
        self.local_state(change)
    }
    fn syncing(&self) -> bool {
        self.syncing.load(Ordering::SeqCst)
    }
    fn set_kept(&self, file_id: &str, kept: bool) -> Result<(), String> {
        self.kept_calls.lock().unwrap().push((file_id.to_string(), kept));
        self.db()
            .set_recursive_pin(file_id, kept, now())
            .map(|_| ())
            .map_err(|_| "refused".to_string())
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn file(db: &StateDb, id: &str, path: &str, status: FileStatus, size: i64, folder: bool) {
    // The parent, as the existing `put` sets it: the folder whose path holds this one, by id `d-<name>`.
    let parent = path.rsplit_once('/').map(|(p, _)| format!("d-{}", p.to_lowercase()));
    let entry = FileEntry {
        file_id: id.to_string(),
        path: path.to_string(),
        status,
        size_bytes: size,
        modified_at: now() - 3_600,
        content_hash: None,
        remote_updated_at: now() - 3_600,
        parent_id: parent,
        item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
    };
    db.upsert_file_with_contract(&entry, |contract| {
        contract.parent_id = entry.parent_id.clone();
        contract.item_kind = entry.item_kind.clone();
    })
    .unwrap();
}

fn transfer(db: &StateDb, direction: &'static str, name: &str, rel: &str, at: i64) {
    db.record_transfer_activity(TransferActivityInput {
        direction,
        file_id: None,
        file_name: name.to_string(),
        rel_path: Some(rel.to_string()),
        bytes: 1_000,
        occurred_at: at,
    })
    .unwrap();
}

/// A change that stopped after its tries (`gave_up`): a problem with Try again.
fn gave_up(db: &StateDb, op: &str, file_id: &str, at: i64) {
    db.enqueue_operation(&PendingOperation {
        op_id: op.to_string(),
        kind: OperationKind::RenameFile,
        file_id: Some(file_id.to_string()),
        parent_id: None,
        target_path: None,
        metadata_json: None,
        payload_path: None,
        base_version: None,
        base_object_version_id: None,
        attempts: 5,
        max_attempts: 5,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: at,
        updated_at: at,
    })
    .unwrap();
}

/// Let the worker finish, then pump all answers into the source.
fn settle<E: ThisPcEngine>(source: &mut EngineThisPcSource<E>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        source.pump();
        if source.idle() {
            source.pump();
            return;
        }
        assert!(Instant::now() < deadline, "the worker did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn names(changes: &[Change]) -> Vec<&str> {
    changes.iter().map(|c| c.name.as_str()).collect()
}

/// A PC with one of everything: two transfers and a move to Trash today, a change that did not sync, a transfer
/// older than 30 days, a kept folder (one of its files not here yet), a kept file and a file not kept.
fn busy_pc() -> Arc<Fake> {
    let fake = Fake::new();
    let db = fake.db();
    let t = now();
    file(&db, "d-invented", "Invented", FileStatus::Local, 0, true);
    file(
        &db,
        "f-report",
        "Invented/Report draft.pdf",
        FileStatus::Local,
        2_000_000,
        false,
    );
    file(&db, "d-shoot", "Shoot", FileStatus::Local, 0, true);
    file(
        &db,
        "f-clip",
        "Shoot/Clip one.mov",
        FileStatus::Local,
        40_000_000,
        false,
    );
    file(
        &db,
        "f-clip2",
        "Shoot/Clip two.mov",
        FileStatus::CloudOnly,
        60_000_000,
        false,
    );
    file(&db, "f-sheet", "Rates.xlsx", FileStatus::Local, 80_000, false);
    file(&db, "f-loose", "Loose note.md", FileStatus::Local, 900, false);
    db.set_recursive_pin("d-shoot", true, t).unwrap();
    db.set_recursive_pin("f-sheet", true, t).unwrap();
    transfer(&db, "up", "Report draft.pdf", "Invented/Report draft.pdf", t - 60);
    transfer(&db, "down", "Rates.xlsx", "Rates.xlsx", t - 120);
    transfer(&db, "up", "Ancient.pdf", "Ancient.pdf", t - 40 * 86_400);
    db.record_local_activity(LocalActivityEventInput {
        event_type: LocalActivityKind::MovedToTrash,
        file_id: None,
        file_name: "Old list.txt".into(),
        rel_path: Some("Invented/Old list.txt".into()),
        occurred_at: t - 30,
    })
    .unwrap();
    gave_up(&db, "op-1", "f-loose", t - 90);
    fake
}

/// The history: newest first, uploads and downloads with their folder, the move to Trash, the change that did not
/// sync with the engine's words; nothing older than 30 days; every row this PC's own.
#[test]
fn history_lists_transfers_trash_and_problems_newest_first() {
    let fake = busy_pc();
    let mut source = EngineThisPcSource::new(fake.clone());
    settle(&mut source);
    assert_eq!(source.wired(), TURNED_ON);
    let history = source.history();
    assert_eq!(
        names(&history),
        ["Old list.txt", "Report draft.pdf", "Loose note.md", "Rates.xlsx"]
    );
    assert_eq!(
        history[0].what,
        What::MovedToTrash {
            place: "Invented".into()
        }
    );
    assert_eq!(
        history[1].what,
        What::Uploaded {
            place: "Invented".into()
        }
    );
    assert_eq!(history[2].problem(), Some("op-1"));
    assert_eq!(history[3].what, What::Downloaded { place: "Files".into() });
    assert_eq!(history[3].kind, Kind::Sheet);
    assert!(history.iter().all(|c| c.origin == Origin::ThisPc));
    assert!(history.iter().all(|c| c.time.len() == 5), "a recorded time is HH:MM");
}

/// The problems: the queue's, with the engine's own words; Try again makes the change due (the queue's rule) and the
/// row leaves the list on the read after it. A row the list does not hold is never sent to the queue.
#[test]
fn problems_carry_the_engines_words_and_try_again_reaches_the_queue() {
    let fake = busy_pc();
    let mut source = EngineThisPcSource::new(fake.clone());
    settle(&mut source);
    let problems = source.problems();
    assert_eq!(names(&problems), ["Loose note.md"]);
    let What::Failed { problem, reason } = &problems[0].what else {
        panic!("a problem is Failed");
    };
    assert_eq!(problem, "op-1");
    assert_eq!(reason, &Reason::Said("Stopped after 5 tries."));
    assert_eq!(reason.line(), "Stopped after 5 tries.");

    source.retry("not-in-the-list");
    settle(&mut source);
    assert_eq!(
        source.problems().len(),
        1,
        "an id the list does not hold changes nothing"
    );

    source.retry("op-1");
    settle(&mut source);
    assert!(source.problems().is_empty(), "made due: no longer a problem");
    let op = fake.db().get_operation("op-1").unwrap().expect("still queued, now due");
    assert_eq!(op.attempts, 0);
    assert!(source.history().iter().all(|c| c.problem().is_none()));
}

/// Try again on every problem reaches every one of them.
#[test]
fn retry_all_reaches_every_problem() {
    let fake = busy_pc();
    gave_up(&fake.db(), "op-2", "f-report", now() - 10);
    let mut source = EngineThisPcSource::new(fake.clone());
    settle(&mut source);
    assert_eq!(source.problems().len(), 2);
    source.retry_all();
    settle(&mut source);
    assert!(source.problems().is_empty());
}

/// The kept items: the top-most only (the folder, not its files), a folder with its file count, a file with its
/// folder; "Up to date" only when every copy is here.
#[test]
fn kept_lists_the_top_most_kept_items_with_an_honest_status() {
    let fake = busy_pc();
    let mut source = EngineThisPcSource::new(fake.clone());
    settle(&mut source);
    let kept = source.kept();
    assert_eq!(kept.len(), 2, "{kept:?}");
    assert_eq!(kept[0].name, "Shoot");
    assert_eq!(kept[0].kind, Kind::Folder);
    assert_eq!(kept[0].contains, Contains::Items(2));
    assert_eq!(kept[0].bytes, 100_000_000);
    assert_eq!(kept[0].status, KeptStatus::NotOnPcYet, "one clip is not on this PC yet");
    assert_eq!(kept[1].name, "Rates.xlsx");
    assert_eq!(kept[1].contains, Contains::In("Files".into()));
    assert_eq!(kept[1].status, KeptStatus::UpToDate);
    assert_eq!(source.kept_count(), 2);
}

/// Remove from this PC: the pin change, only while sync runs, only for an item the list holds; Remove all reaches
/// each kept item.
#[test]
fn remove_from_pc_unpins_only_while_sync_runs() {
    let fake = busy_pc();
    let mut source = EngineThisPcSource::new(fake.clone());
    settle(&mut source);
    fake.syncing.store(false, Ordering::SeqCst);
    source.remove_from_pc("f-sheet");
    settle(&mut source);
    assert_eq!(source.kept().len(), 2, "sync does not run: nothing changes");
    assert!(fake.kept_calls.lock().unwrap().is_empty());

    fake.syncing.store(true, Ordering::SeqCst);
    source.remove_from_pc("f-loose");
    settle(&mut source);
    assert!(fake.kept_calls.lock().unwrap().is_empty(), "not in the list: not sent");
    source.remove_from_pc("f-sheet");
    settle(&mut source);
    assert_eq!(names_of(&source.kept()), ["Shoot"]);
    assert_eq!(*fake.kept_calls.lock().unwrap(), [("f-sheet".to_string(), false)]);

    source.remove_all_from_pc();
    settle(&mut source);
    assert!(source.kept().is_empty());
}

fn names_of(kept: &[Kept]) -> Vec<&str> {
    kept.iter().map(|k| k.name.as_str()).collect()
}

/// A PC where nothing synced yet: empty lists, never a sample.
#[test]
fn an_empty_pc_has_empty_lists() {
    let fake = Fake::new();
    let mut source = EngineThisPcSource::new(fake);
    settle(&mut source);
    assert_eq!(source.wired(), TURNED_ON);
    assert!(source.history().is_empty() && source.problems().is_empty() && source.kept().is_empty());
}

/// A read that fails keeps the last answer (never an empty list in its place); before any answer, nothing.
#[test]
fn a_read_that_fails_keeps_the_last_answer() {
    let fake = busy_pc();
    fake.broken.store(true, Ordering::SeqCst);
    let mut source = EngineThisPcSource::new(fake.clone());
    settle(&mut source);
    assert!(
        source.history().is_empty() && source.kept().is_empty(),
        "no answer yet: nothing"
    );

    fake.broken.store(false, Ordering::SeqCst);
    source.ask_read();
    settle(&mut source);
    assert_eq!(source.history().len(), 4);

    fake.broken.store(true, Ordering::SeqCst);
    source.ask_read();
    settle(&mut source);
    assert_eq!(source.history().len(), 4, "the last answer stays");
    assert_eq!(source.kept().len(), 2);
}

/// Locked or signed out: no name at all, at once (the readers filter) and after (the held data is dropped by pump).
#[test]
fn a_locked_vault_shows_no_name() {
    let fake = busy_pc();
    let mut source = EngineThisPcSource::new(fake.clone());
    settle(&mut source);
    assert!(!source.history().is_empty());
    fake.unlocked.store(false, Ordering::SeqCst);
    // At once: the readers filter by `unlocked`.
    assert!(source.history().is_empty() && source.problems().is_empty() && source.kept().is_empty());
    assert_eq!(source.kept_count(), 0);
    // After pump: the held data is dropped.
    settle(&mut source);
    assert!(source.held.is_none(), "cleared in memory by pump");
}

/// A change of account clears what was held (P1: nothing of account A shows under account B).
#[test]
fn a_change_of_account_clears_the_held_answer() {
    let fake = busy_pc();
    let mut source = EngineThisPcSource::new(fake.clone());
    settle(&mut source);
    assert_eq!(source.kept_count(), 2, "A's data");

    // Switch to account B.
    *fake.account.lock().unwrap() = Some("other@beebeeb.io".into());
    source.pump();
    assert_eq!(source.kept_count(), 0, "A's answer is gone at once");
    assert!(source.history().is_empty());

    // The source asked for a new read; account B's state DB is not theirs, so nothing shows.
    settle(&mut source);
    assert!(source.kept().is_empty(), "B owns no local state of A's DB");
}

/// Task 2488: a local state database owned by another account reads as nothing.
#[test]
fn another_accounts_local_state_reads_as_nothing() {
    // Account B's fake: owns the DB.
    let fake_b = Fake::with_account("other@beebeeb.io");
    let db = fake_b.db();
    file(&db, "f-secret", "Secret.pdf", FileStatus::Local, 1_000, false);
    db.set_recursive_pin("f-secret", true, now()).unwrap();

    // Sign in as account A: the DB is B's.
    *fake_b.account.lock().unwrap() = Some("invented@beebeeb.io".into());
    let mut source = EngineThisPcSource::new(fake_b.clone());
    settle(&mut source);
    assert!(source.kept().is_empty(), "A never sees B's kept items");
    assert!(source.history().is_empty(), "A never sees B's history");
    assert_eq!(source.kept_count(), 0);
}

/// A refused Remove from this PC (sync not running) sends a refusal the window can draw (P2-6).
#[test]
fn a_refused_remove_sends_a_refusal_through_pump() {
    let fake = busy_pc();
    let mut source = EngineThisPcSource::new(fake.clone());
    settle(&mut source);
    fake.syncing.store(false, Ordering::SeqCst);
    source.remove_from_pc("f-sheet");
    // Wait for the worker, then pump once: the refusal must be in the first drain.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let pumped = source.pump();
        if pumped.refusal.is_some() {
            assert_eq!(pumped.refusal, Some(NOT_SYNCING), "the refusal reaches the window");
            break;
        }
        assert!(Instant::now() < deadline, "no refusal arrived");
        std::thread::sleep(Duration::from_millis(5));
    }
    // Taken once: the next pump has no refusal.
    let pumped2 = source.pump();
    assert_eq!(pumped2.refusal, None);
}

/// A refused Retry sends a refusal when the local state is not this account's (task 2488).
#[test]
fn a_refused_retry_sends_a_refusal() {
    let fake = busy_pc();
    let mut source = EngineThisPcSource::new(fake.clone());
    settle(&mut source);
    assert!(!source.problems().is_empty());

    // Break the DB ownership: the DB is now someone else's.
    fake.db()
        .set_owner(&Identity::new(None, Some("someone-else@beebeeb.io")))
        .unwrap();
    source.retry("op-1");
    // Wait for the worker to refuse, then pump.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let pumped = source.pump();
        if pumped.refusal.is_some() {
            assert_eq!(pumped.refusal, Some(NOT_RETRIED));
            break;
        }
        assert!(Instant::now() < deadline, "no refusal arrived");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// `opened()` asks for a fresh read when the held answer is older than `FRESH_FOR`.
#[test]
fn opened_reads_again_when_the_answer_is_stale() {
    let fake = busy_pc();
    let mut source = EngineThisPcSource::new(fake.clone());
    settle(&mut source);
    assert_eq!(source.kept_count(), 2);

    // The answer is fresh: opened does not ask again.
    source.opened();
    assert!(!source.reading, "fresh: no new read");

    // Make the answer stale.
    source.read_at = Some(Instant::now() - FRESH_FOR - Duration::from_secs(1));
    source.opened();
    assert!(source.reading, "stale: a new read is asked");
}

/// `source_for` with `TURNED_ON = false` gives `Unwired`: no worker, no read, no sidebar count (P2-1).
#[test]
fn source_for_gives_unwired_when_the_constant_is_off() {
    let fake = busy_pc();
    let source = source_for(fake);
    // TURNED_ON is false, so the source is Unwired.
    assert!(!source.wired());
    assert!(source.kept().is_empty());
    assert_eq!(source.kept_count(), 0);
}

/// The words for where a path is, and the glyph for a name: a picture is a document (never the sample picture).
#[test]
fn place_and_kind_follow_the_path_and_the_name() {
    assert_eq!(place_of(Some("A/B/c.txt")), "B");
    assert_eq!(place_of(Some("c.txt")), "Files");
    assert_eq!(place_of(None), "Files");
    assert_eq!(kind_of("IMG_0001.HEIC", false), Kind::Document);
    assert_eq!(kind_of("Talk.M4A", false), Kind::Audio);
    assert_eq!(kind_of("Plan.key", false), Kind::Deck);
    assert_eq!(kind_of("x", true), Kind::Folder);
}
