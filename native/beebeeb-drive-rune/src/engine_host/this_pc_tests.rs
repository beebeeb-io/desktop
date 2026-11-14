//! This PC on the real engine (`EngineThisPcSource`), against the engine's own state database in a folder of each
//! test's own: the history (transfers, Trash, the changes that did not sync), the problems with the engine's words
//! and Try again through the queue, the kept items with an honest status, Remove from this PC through the pin change,
//! no local state, a read that fails, no name while the vault is locked, the account on every answer, and nothing of
//! another account's local state (task 2488). Every name is invented.

use beebeeb_engine::app::LocalStateRefused;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    /// How many session-bound keep changes the engine refused (the session had moved on).
    kept_for_refused: std::sync::atomic::AtomicUsize,
    /// The engine's session: a test moves it on as a sign-in, a lock or a sign-out does.
    session: AtomicU64,
    /// Runs inside the next read of the local state, on the worker (a test changes the session under it).
    during_read: Mutex<Option<Box<dyn FnOnce(&Fake) + Send>>>,
    /// Runs after the next `set_kept`, on the worker.
    after_kept: Mutex<Option<Box<dyn FnOnce(&Fake) + Send>>>,
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
            kept_for_refused: std::sync::atomic::AtomicUsize::new(0),
            session: AtomicU64::new(1),
            during_read: Mutex::new(None),
            after_kept: Mutex::new(None),
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
    fn session(&self) -> SessionGeneration {
        SessionGeneration::for_test(self.session.load(Ordering::SeqCst))
    }
    fn local_state<T>(&self, read: impl FnOnce(&StateDb) -> T) -> Result<T, LocalStateRefused> {
        let hook = self.during_read.lock().unwrap().take();
        if let Some(hook) = hook {
            hook(self);
        }
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
    fn set_kept_for(&self, session: SessionGeneration, file_id: &str, kept: bool) -> Result<(), String> {
        if session != self.session() {
            self.kept_for_refused.fetch_add(1, Ordering::SeqCst);
            return Err("session moved on".into());
        }
        self.set_kept(file_id, kept)
    }
    fn set_kept(&self, file_id: &str, kept: bool) -> Result<(), String> {
        self.kept_calls.lock().unwrap().push((file_id.to_string(), kept));
        let done = self
            .db()
            .set_recursive_pin(file_id, kept, now())
            .map(|_| ())
            .map_err(|_| "refused".to_string());
        let hook = self.after_kept.lock().unwrap().take();
        if let Some(hook) = hook {
            hook(self);
        }
        done
    }
}

pub(super) fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub(super) fn file(db: &StateDb, id: &str, path: &str, status: FileStatus, size: i64, folder: bool) {
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
    assert_eq!(source.standing(), Standing::Reading, "before the first pump: no answer");
    settle(&mut source);
    assert!(
        source.history().is_empty() && source.kept().is_empty(),
        "no answer yet: nothing"
    );
    assert_eq!(
        source.standing(),
        Standing::Unreadable,
        "and the section says it could not be read, never an empty PC"
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
    assert_eq!(source.standing(), Standing::AnotherAccounts);
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
    assert_eq!(
        source.standing(),
        Standing::AnotherAccounts,
        "said as what it is, never as a PC with nothing on it"
    );
}

/// Lists that are held and then turn out to be another account's (the owner changed under the same sign-in name)
/// go at that answer: nothing of them stays on screen beside "another account's".
#[test]
fn another_accounts_answer_drops_what_was_held() {
    let fake = busy_pc();
    let mut source = EngineThisPcSource::new(fake.clone());
    settle(&mut source);
    assert_eq!(source.kept_count(), 2);
    fake.db()
        .set_owner(&Identity::new(None, Some("other@beebeeb.io")))
        .unwrap();
    source.ask_read();
    settle(&mut source);
    assert_eq!(source.kept_count(), 0);
    assert!(source.history().is_empty());
    assert_eq!(source.standing(), Standing::AnotherAccounts);
}

/// The first answer makes the lists the PC's own; a PC with nothing synced is `Ready` with empty lists.
#[test]
fn standing_is_ready_once_an_answer_is_held() {
    let fake = Fake::new();
    let mut source = EngineThisPcSource::new(fake.clone());
    assert_eq!(source.standing(), Standing::Reading);
    settle(&mut source);
    assert_eq!(source.standing(), Standing::Ready);
    // A lock: nothing is held for the window, so nothing is "ready".
    fake.unlocked.store(false, Ordering::SeqCst);
    assert_eq!(source.standing(), Standing::Reading);
}

/// Holds the worker inside its next read until the returned sender is used (or dropped).
fn hold_next_read(fake: &Fake) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
    let (entered_tx, entered) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel::<()>();
    *fake.during_read.lock().unwrap() = Some(Box::new(move |_| {
        let _ = entered_tx.send(());
        let _ = released.recv_timeout(Duration::from_secs(10));
    }));
    (entered, release)
}

/// The session moves on under the SAME account (a lock and an unlock, a sign-out and a sign-in): what was held
/// goes at the next pump, before the new read answers, and comes back only from a read of the new session.
#[test]
fn a_new_session_of_the_same_account_drops_the_held_answer() {
    let fake = busy_pc();
    let mut source = EngineThisPcSource::new(fake.clone());
    settle(&mut source);
    assert_eq!(source.kept_count(), 2);

    let (entered, release) = hold_next_read(&fake);
    fake.session.fetch_add(1, Ordering::SeqCst);
    assert_eq!(source.kept_count(), 0, "at once: the readers ask the engine's session");
    let pumped = source.pump();
    assert!(pumped.changed);
    entered
        .recv_timeout(Duration::from_secs(10))
        .expect("a new read was asked");
    assert!(source.held.is_none(), "dropped by the pump, not by the answer");
    assert_eq!(source.standing(), Standing::Reading);
    release.send(()).unwrap();
    settle(&mut source);
    assert_eq!(source.kept_count(), 2, "the new session's own read");
}

/// A read that was out while the session moved on: its answer names no session (the worker's check after the
/// work), so no rule of the window could keep it.
#[test]
fn a_read_that_outlived_its_session_answers_for_no_one() {
    let fake = busy_pc();
    let whose = whose_now(&*fake).expect("signed in");
    *fake.during_read.lock().unwrap() = Some(Box::new(|fake: &Fake| {
        fake.session.fetch_add(1, Ordering::SeqCst);
    }));
    let (answers, inbox) = mpsc::channel();
    run(
        &*fake,
        &answers,
        Job {
            ask: Ask::Read,
            whose: whose.clone(),
            generation: 7,
        },
    );
    let answer = inbox.try_recv().expect("every job answers");
    assert_eq!(answer.generation, 7);
    assert_eq!(answer.whose, None, "the session moved on during the read");

    // The same job with no change under it answers for its session.
    run(
        &*fake,
        &answers,
        Job {
            ask: Ask::Read,
            whose: whose_now(&*fake).unwrap(),
            generation: 8,
        },
    );
    let answer = inbox.try_recv().unwrap();
    assert_eq!(answer.whose, whose_now(&*fake));
    assert!(answer.read.is_ok());
}

/// The window's generation alone: an answer asked before the account went away and came back (the same account,
/// the same engine session in this fake) is a stale one, and its refusal is never said.
#[test]
fn an_answer_of_an_earlier_generation_is_dropped_even_for_the_same_session() {
    let fake = busy_pc();
    let mut source = EngineThisPcSource::new(fake.clone());
    settle(&mut source);
    fake.syncing.store(false, Ordering::SeqCst);
    let (entered, release) = hold_next_read(&fake);
    // Refused (sync does not run), then held in the read that follows every change.
    source.remove_from_pc("f-sheet");
    entered.recv_timeout(Duration::from_secs(10)).expect("the job runs");

    *fake.account.lock().unwrap() = None;
    source.pump();
    *fake.account.lock().unwrap() = Some("invented@beebeeb.io".into());
    source.pump();
    release.send(()).unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut said = None;
    loop {
        let pumped = source.pump();
        said = said.or(pumped.refusal);
        if source.idle() {
            said = said.or(source.pump().refusal);
            break;
        }
        assert!(Instant::now() < deadline, "the worker did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(said, None, "a refusal of an earlier generation is not this window's");
    assert_eq!(source.kept_count(), 2, "the current generation's own read is held");
}

/// A worker that ends early says "could not be read" for the session that asked: the section never waits forever.
#[test]
fn a_job_that_ends_early_is_said_as_unreadable() {
    let fake = Fake::new();
    *fake.during_read.lock().unwrap() = Some(Box::new(|_| panic!("an invented failure inside a read")));
    let mut source = EngineThisPcSource::new(fake.clone());
    settle(&mut source);
    assert_eq!(source.standing(), Standing::Unreadable);
    // Opening the section asks again, and the read that works is held.
    source.opened();
    settle(&mut source);
    assert_eq!(source.standing(), Standing::Ready);
}

/// A read refused because the session was not one to read under just then is not "could not be read": what is
/// held stays, and the next opening asks again.
#[test]
fn a_read_refused_for_the_session_keeps_what_is_held_and_is_asked_again() {
    assert_eq!(
        read_for(
            &*busy_pc(),
            &Whose {
                account: "invented@beebeeb.io".into(),
                session: SessionGeneration::for_test(99),
            }
        ),
        Err(NoRead::NotNow),
        "a session that is not the engine's"
    );
    let fake = busy_pc();
    let mut source = EngineThisPcSource::new(fake.clone());
    settle(&mut source);
    source.inbox_for_test(Answer {
        generation: source.generation,
        whose: source.seen.clone(),
        refused: None,
        read: Err(NoRead::NotNow),
    });
    source.pump();
    assert_eq!(source.standing(), Standing::Ready, "the lists stay");
    assert_eq!(source.kept_count(), 2);
    assert!(source.read_at.is_none(), "and the next opening reads again");
}

/// A FIRST read answered "not now" (nothing held) is asked again by itself, shortly after: the window never stays on
/// "reading" until the person reopens the pane.
#[test]
fn a_first_read_answered_not_now_is_asked_again() {
    let fake = busy_pc();
    let mut source = EngineThisPcSource::new(fake.clone());
    settle(&mut source);
    source.held = None;
    source.inbox_for_test(Answer {
        generation: source.generation,
        whose: source.seen.clone(),
        refused: None,
        read: Err(NoRead::NotNow),
    });
    let pumped = source.pump();
    assert_eq!(source.standing(), Standing::Reading, "nothing held yet");
    assert!(source.retry_at.is_some(), "a new read is due");
    assert!(pumped.busy, "the window keeps polling until it is asked");
    source.retry_at = Some(Instant::now() - Duration::from_millis(1));
    settle(&mut source);
    assert_eq!(source.standing(), Standing::Ready, "the second read answered");
    assert!(source.retry_at.is_none());
}

/// The one rule, by its parts: the generation, an answer that names a session, and that session being the window's.
#[test]
fn counts_needs_the_generation_and_the_same_session() {
    let a = Whose {
        account: "invented@beebeeb.io".into(),
        session: SessionGeneration::for_test(1),
    };
    let later = Whose {
        session: SessionGeneration::for_test(2),
        ..a.clone()
    };
    let other = Whose {
        account: "other@beebeeb.io".into(),
        ..a.clone()
    };
    assert!(counts(3, 3, Some(&a), Some(&a)));
    assert!(!counts(2, 3, Some(&a), Some(&a)), "an earlier generation");
    assert!(!counts(3, 3, None, Some(&a)), "an answer for no one");
    assert!(!counts(3, 3, None, None), "nobody signed in");
    assert!(
        !counts(3, 3, Some(&a), Some(&later)),
        "the same account, a later session"
    );
    assert!(!counts(3, 3, Some(&a), Some(&other)), "another account");
}

/// Remove from this PC, item by item, under the session of the press: once the session moves on, no further item
/// is touched, and the window is told not everything was removed.
#[test]
fn remove_stops_when_the_session_moves_on() {
    let fake = busy_pc();
    let whose = whose_now(&*fake).unwrap();
    *fake.after_kept.lock().unwrap() = Some(Box::new(|fake: &Fake| {
        fake.session.fetch_add(1, Ordering::SeqCst);
    }));
    let (answers, inbox) = mpsc::channel();
    run(
        &*fake,
        &answers,
        Job {
            ask: Ask::Release(vec!["f-sheet".into(), "d-shoot".into()]),
            whose,
            generation: 1,
        },
    );
    assert_eq!(
        *fake.kept_calls.lock().unwrap(),
        [("f-sheet".to_string(), false)],
        "the second item is not touched under a later session"
    );
    let answer = inbox.try_recv().unwrap();
    assert_eq!(answer.refused, Some(NOT_REMOVED));
    assert_eq!(answer.whose, None);
}

/// Try again under a session that moved on changes nothing in the queue.
#[test]
fn retry_under_a_later_session_changes_nothing() {
    let fake = busy_pc();
    let whose = whose_now(&*fake).unwrap();
    fake.session.fetch_add(1, Ordering::SeqCst);
    assert_eq!(retry(&*fake, &whose, &["op-1".to_string()]), None);
    let due = fake.db().unsent_changes(0, 10, now()).unwrap().items;
    assert_eq!(
        status::sync_problems(&due).len(),
        1,
        "still a problem: nothing was retried"
    );
    // Under its own session it is retried.
    assert_eq!(
        retry(&*fake, &whose_now(&*fake).unwrap(), &["op-1".to_string()]),
        Some(0)
    );
}

/// A snapshot in a log line or a failed assertion: counts only, never a name, a folder or an id.
#[test]
fn a_snapshot_prints_counts_and_no_name() {
    let fake = busy_pc();
    let snapshot = read(&fake.db(), now()).unwrap();
    assert!(!snapshot.history.is_empty() && !snapshot.kept.is_empty());
    let printed = format!("{snapshot:?} {:?}", Some(&snapshot));
    for secret in ["Report", "Rates", "Invented", "Shoot", "f-sheet", "op-1", "Loose"] {
        assert!(!printed.contains(secret), "a snapshot's Debug names {secret}");
    }
    assert!(
        printed.contains("history: 4") && printed.contains("kept: 2"),
        "{printed}"
    );
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

/// The switch decides the source, both ways: off gives `Unwired` (no worker, no read, no sidebar count, P2-1), on
/// gives the engine's, which reads. The running app's is on.
#[test]
fn the_switch_decides_the_source() {
    let mut off = source_when(false, busy_pc());
    off.pump();
    assert!(!off.wired());
    assert!(off.kept().is_empty());
    assert_eq!(off.kept_count(), 0);

    let mut on = source_when(true, busy_pc());
    assert!(on.wired());
    let deadline = Instant::now() + Duration::from_secs(10);
    while on.kept_count() != 2 {
        on.pump();
        assert!(Instant::now() < deadline, "the engine's source did not read");
        std::thread::sleep(Duration::from_millis(5));
    }

    assert!(TURNED_ON, "This PC answers in the running app");
    assert!(source_for(busy_pc()).wired());
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

/// Remove from this PC goes through the session-bound keep change: a session that moves on between the window's
/// own check and the engine's call changes nothing (the engine refuses it itself).
#[test]
fn remove_is_refused_by_the_engine_when_the_session_moves_after_the_check() {
    let fake = busy_pc();
    let whose = whose_now(&*fake).unwrap();
    *fake.during_read.lock().unwrap() = Some(Box::new(|fake: &Fake| {
        fake.session.fetch_add(1, Ordering::SeqCst);
    }));
    let (answers, inbox) = mpsc::channel();
    run(
        &*fake,
        &answers,
        Job {
            ask: Ask::Release(vec!["f-sheet".into()]),
            whose,
            generation: 1,
        },
    );
    assert_eq!(
        fake.kept_for_refused.load(Ordering::SeqCst),
        1,
        "the engine was asked under the old session"
    );
    assert!(fake.kept_calls.lock().unwrap().is_empty(), "and changed nothing");
    assert_eq!(inbox.try_recv().unwrap().refused, Some(NOT_REMOVED));
}
