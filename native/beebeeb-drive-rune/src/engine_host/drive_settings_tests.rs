//! Drive's settings panes on the real engine (`EngineDriveSettings`), against the engine's own state database in a
//! folder of each test's own and the shell's fake platform: Storage insights (ready, empty, Beebeeb unreachable, the
//! file list unreadable, locked), the Sync pane's reads and changes, Free up space, Keep on this PC's tree and Apply,
//! and every pane left unwired drawing the shell's frame, never a sample value. Every name is invented.

use beebeeb_engine::app::LocalStateRefused;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use beebeeb_engine::account_binding::Identity;
use beebeeb_engine::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use beebeeb_engine::sync_view::{Explorer, Start, SyncView};
use rune::{Env, Runtime};

use super::*;
use crate::State;
use crate::frames::settings::drive::BoardDrive;
use crate::frames::settings::drive::Control;
use crate::shell::platform::fake::FakePlatform;

const GB: i64 = 1_000_000_000;

/// Repair, turn off, the speed test, the support bundle and the reads that wait for their pane.
#[path = "drive_settings_tools_tests.rs"]
mod tools_tests;

/// The transfer limits and the Bandwidth read.
#[path = "drive_settings_limits_tests.rs"]
mod limits;

/// Holds the worker inside one call of the fake (the `at`-th, counted from 1) until the test lets it go: an
/// answer in flight, with no clock deciding the order.
#[derive(Default)]
struct Hold {
    at: AtomicUsize,
    passes: AtomicUsize,
    /// (the worker is inside, the test let it go)
    state: Mutex<(bool, bool)>,
    moved: Condvar,
}

const HOLD_FOR: Duration = Duration::from_secs(30);

impl Hold {
    fn arm_at(&self, at: usize) {
        self.at.store(at, Ordering::SeqCst);
    }

    /// The fake's side: the armed pass waits here.
    fn pass(&self) {
        if self.passes.fetch_add(1, Ordering::SeqCst) + 1 != self.at.load(Ordering::SeqCst) {
            return;
        }
        let mut state = self.state.lock().unwrap();
        state.0 = true;
        self.moved.notify_all();
        let (state, timed_out) = self.moved.wait_timeout_while(state, HOLD_FOR, |s| !s.1).unwrap();
        drop(state);
        assert!(!timed_out.timed_out(), "the test never let the held call go");
    }

    /// The test's side: wait until the worker is inside the armed pass.
    fn reached(&self) {
        let state = self.state.lock().unwrap();
        let (_state, timed_out) = self.moved.wait_timeout_while(state, HOLD_FOR, |s| !s.0).unwrap();
        assert!(!timed_out.timed_out(), "the worker never reached the held call");
    }

    fn release(&self) {
        self.state.lock().unwrap().1 = true;
        self.moved.notify_all();
    }
}

struct Fake {
    /// After a read of the local state, before its answer leaves the engine.
    after_read: Hold,
    /// Inside the call that asks Beebeeb for the use.
    in_usage: Hold,
    pause_fails: AtomicBool,
    _dir: tempfile::TempDir,
    path: PathBuf,
    view: Mutex<SyncView>,
    usage: Mutex<Result<(i64, i64), String>>,
    broken: AtomicBool,
    syncing: AtomicBool,
    paused: Mutex<Vec<bool>>,
    kept_calls: Mutex<Vec<(String, bool)>>,
    freed: Mutex<u32>,
    account: Mutex<Option<String>>,
    /// The engine's session: a test moves it on as a lock or a sign-in does.
    session: std::sync::atomic::AtomicU64,
    /// The settings file's limits, and every change the engine took.
    limits: Mutex<TransferLimits>,
    limit_changes: Mutex<Vec<TransferLimitsChange>>,
    /// Reads of the limits (the Bandwidth pane's).
    limit_reads: AtomicUsize,
    /// What the engine answers a repair and a turn-off, and how often each was asked (with the session).
    explorer_answer: Mutex<Result<(), ExplorerChangeRefused>>,
    explorer_calls: Mutex<Vec<(&'static str, SessionGeneration)>>,
    /// What each speed step answers (`None`: the step fails offline), and the steps asked.
    speed_answers: Mutex<Vec<Option<SpeedFigure>>>,
    speed_asked: Mutex<Vec<SpeedStep>>,
    /// What an export answers.
    bundle_answer: Mutex<Result<SavedBundle, BundleRefused>>,
}

impl Fake {
    fn new() -> Arc<Fake> {
        Self::with_account("invented@beebeeb.io")
    }

    fn with_account(email: &str) -> Arc<Fake> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let db = StateDb::open(&path).unwrap();
        db.set_owner(&Identity::new(None, Some(email))).unwrap();
        Arc::new(Fake {
            after_read: Hold::default(),
            in_usage: Hold::default(),
            pause_fails: AtomicBool::new(false),
            _dir: dir,
            path,
            view: Mutex::new(SyncView {
                gate: GateState::Ready,
                start: Start::Running,
                explorer: Explorer::Added,
                ..SyncView::default()
            }),
            usage: Mutex::new(Ok((240 * GB, 1_000 * GB))),
            broken: AtomicBool::new(false),
            syncing: AtomicBool::new(true),
            paused: Mutex::new(Vec::new()),
            kept_calls: Mutex::new(Vec::new()),
            freed: Mutex::new(0),
            account: Mutex::new(Some(email.to_string())),
            session: std::sync::atomic::AtomicU64::new(1),
            limits: Mutex::new(TransferLimits::default()),
            limit_changes: Mutex::new(Vec::new()),
            limit_reads: AtomicUsize::new(0),
            explorer_answer: Mutex::new(Ok(())),
            explorer_calls: Mutex::new(Vec::new()),
            speed_answers: Mutex::new(Vec::new()),
            speed_asked: Mutex::new(Vec::new()),
            bundle_answer: Mutex::new(Err(BundleRefused::NoQueue)),
        })
    }

    fn db(&self) -> StateDb {
        StateDb::open(&self.path).unwrap()
    }

    fn gate(&self, gate: GateState) {
        self.view.lock().unwrap().gate = gate;
    }
}

impl DriveEngine for Fake {
    fn view(&self) -> SyncView {
        self.view.lock().unwrap().clone()
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
        let answer = read(&db);
        self.after_read.pass();
        Ok(answer)
    }
    fn storage_info(&self) -> Result<(i64, i64), String> {
        self.in_usage.pass();
        self.usage.lock().unwrap().clone()
    }
    fn set_paused(&self, paused: bool) -> Result<(), String> {
        if self.pause_fails.load(Ordering::SeqCst) {
            return Err("refused".into());
        }
        self.paused.lock().unwrap().push(paused);
        self.view.lock().unwrap().paused = paused;
        Ok(())
    }
    fn syncing(&self) -> bool {
        self.syncing.load(Ordering::SeqCst)
    }
    fn set_kept(&self, file_id: &str, kept: bool) -> Result<(), String> {
        self.kept_calls.lock().unwrap().push((file_id.to_string(), kept));
        let now = chrono::Utc::now().timestamp();
        self.db()
            .set_recursive_pin(file_id, kept, now)
            .map(|_| ())
            .map_err(|_| "refused".into())
    }
    fn free_up_space(&self) -> Result<u64, String> {
        *self.freed.lock().unwrap() += 1;
        Ok(0)
    }
    fn local_settings(&self) -> Result<(i64, bool), String> {
        Ok((50 * GB, true))
    }
    fn session(&self) -> SessionGeneration {
        SessionGeneration::for_test(self.session.load(Ordering::SeqCst))
    }
    fn limits(&self) -> Result<TransferLimits, String> {
        self.limit_reads.fetch_add(1, Ordering::SeqCst);
        Ok(*self.limits.lock().unwrap())
    }
    /// As the engine: only under the session in memory.
    fn set_limits(&self, session: SessionGeneration, change: TransferLimitsChange) -> Result<TransferLimits, String> {
        if session != DriveEngine::session(self) {
            return Err("the session changed".into());
        }
        self.limit_changes.lock().unwrap().push(change);
        let mut limits = self.limits.lock().unwrap();
        if let Some(limit) = change.upload_kib_per_s {
            limits.upload_kib_per_s = limit;
        }
        if let Some(limit) = change.download_kib_per_s {
            limits.download_kib_per_s = limit;
        }
        if let Some(limit) = change.downloaded_copies_bytes {
            limits.downloaded_copies_bytes = limit;
        }
        Ok(*limits)
    }
    fn repair_explorer(&self, session: SessionGeneration) -> Result<(), ExplorerChangeRefused> {
        self.explorer_calls.lock().unwrap().push(("repair", session));
        *self.explorer_answer.lock().unwrap()
    }
    fn turn_off_explorer(&self, session: SessionGeneration) -> Result<(), ExplorerChangeRefused> {
        self.explorer_calls.lock().unwrap().push(("turn off", session));
        *self.explorer_answer.lock().unwrap()
    }
    fn speed_step(
        &self,
        _session: SessionGeneration,
        step: SpeedStep,
        progress: &(dyn Fn(u32, u32) + Sync),
    ) -> Result<SpeedFigure, AccountCallError> {
        self.speed_asked.lock().unwrap().push(step);
        progress(1, 3);
        let mut answers = self.speed_answers.lock().unwrap();
        if answers.is_empty() {
            return Err(AccountCallError::Offline);
        }
        answers.remove(0).ok_or(AccountCallError::Offline)
    }
    fn export_bundle(&self, _session: SessionGeneration) -> Result<SavedBundle, BundleRefused> {
        self.bundle_answer.lock().unwrap().clone()
    }
}

fn file(db: &StateDb, id: &str, path: &str, size: i64, folder: bool) {
    let parent = path.rsplit_once('/').map(|(p, _)| format!("d-{}", p.to_lowercase()));
    let entry = FileEntry {
        file_id: id.to_string(),
        path: path.to_string(),
        status: FileStatus::CloudOnly,
        size_bytes: size,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: parent,
        item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
    };
    db.upsert_file_with_contract(&entry, |contract| {
        contract.parent_id = entry.parent_id.clone();
        contract.item_kind = entry.item_kind.clone();
    })
    .unwrap();
}

/// An account with media, documents and other files in two folders (one inside the other), and ten more videos
/// so the largest list has more than it shows.
fn account() -> Arc<Fake> {
    let fake = Fake::new();
    let db = fake.db();
    file(&db, "d-shoot", "Shoot", 0, true);
    file(&db, "d-shoot/raw", "Shoot/Raw", 0, true);
    file(&db, "f-clip", "Shoot/Raw/Clip one.mov", 4 * GB, false);
    file(&db, "f-photo", "Shoot/Frame.heic", 3_000_000, false);
    file(&db, "d-papers", "Papers", 0, true);
    file(&db, "f-tax", "Papers/Taxes 2025.pdf", 212_000_000, false);
    file(&db, "f-zip", "Archive.zip", 2_600_000_000, false);
    for n in 0..10 {
        file(
            &db,
            &format!("f-v{n}"),
            &format!("Shoot/Take {n}.mp4"),
            100_000_000 + n,
            false,
        );
    }
    db.set_recursive_pin("d-papers", true, 1_794_000_000).unwrap();
    fake
}

/// A source whose Insights and Sync panes were just opened, with the reads still out (nothing is read before a pane
/// opens, so a test that holds a read in flight opens the panes itself).
fn opened_source(fake: &Arc<Fake>, platform: Arc<FakePlatform>) -> EngineDriveSettings<Fake> {
    let mut source = EngineDriveSettings::new(fake.clone(), platform);
    source.opened(Pane::Sync);
    source.opened(Pane::Insights);
    source
}

/// A source whose Sync and Storage insights panes were opened (nothing is read before a pane opens).
fn source(fake: &Arc<Fake>, platform: Arc<FakePlatform>) -> EngineDriveSettings<Fake> {
    let mut source = EngineDriveSettings::new(fake.clone(), platform);
    source.opened(Pane::Insights);
    source.opened(Pane::Sync);
    settle(&mut source);
    source
}

fn settle(source: &mut EngineDriveSettings<Fake>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (_, more) = source.pump(0.0);
        if !more && source.idle() {
            source.pump(0.0);
            return;
        }
        assert!(Instant::now() < deadline, "the worker did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn ready(source: &EngineDriveSettings<Fake>) -> InsightsData {
    match source.insights() {
        Insights::Ready(data) => (**data).clone(),
        other => panic!("insights are ready, not {other:?}"),
    }
}

/// Storage insights: the account's use from Beebeeb, the kinds largest first, the largest files with the rest
/// counted, every figure in the board's words.
#[test]
fn insights_work_out_the_kinds_and_the_largest_files() {
    let fake = account();
    let source = source(&fake, Arc::default());
    assert!(source.wired(Pane::Insights));
    let data = ready(&source);
    assert_eq!((data.used.as_str(), data.total.as_str()), ("240 GB", "1 TB"));
    assert_eq!(data.used_says, "24% used");
    assert_eq!(data.free_says, "760 GB free");
    assert_eq!(data.plan, "1 TB plan");
    assert_eq!(data.totals, "14 files · 7.8 GB");
    let kinds: Vec<(&str, &str)> = data.kinds.iter().map(|k| (k.name.as_str(), k.says.as_str())).collect();
    assert_eq!(
        kinds,
        [
            ("Media", "5 GB · 12 files"),
            ("Other", "2.6 GB · 1 file"),
            ("Documents", "212 MB · 1 file")
        ]
    );
    assert_eq!(data.kinds[0].share, 1.0);
    assert_eq!(data.largest[0].path, "Shoot/Raw/Clip one.mov");
    assert_eq!(data.largest[0].size, "4 GB");
    assert_eq!(data.largest[1].kind, "Other");
    assert_eq!(data.largest.len(), 7);
    assert_eq!(data.largest_says, "top 14");
    assert_eq!(data.more, Some(("And 7 more, down to ".into(), "3 MB".into())));
}

/// No files and nothing used: the empty state, never a page of zeros.
#[test]
fn insights_of_an_empty_account_are_empty() {
    let fake = Fake::new();
    *fake.usage.lock().unwrap() = Ok((0, 1_000 * GB));
    let source = source(&fake, Arc::default());
    assert_eq!(source.insights(), &Insights::Empty);
}

/// Beebeeb unreachable, another refusal, the file list unreadable: each its own fixed sentence; Try again loads
/// (DW-0783b) and then answers.
#[test]
fn insights_that_cannot_load_say_why_and_try_again_reloads() {
    let fake = account();
    *fake.usage.lock().unwrap() = Err("offline".into());
    let mut source = source(&fake, Arc::default());
    assert_eq!(source.insights(), &Insights::Failed(INSIGHTS_OFFLINE.into()));

    *fake.usage.lock().unwrap() = Err("could not read storage".into());
    source.reload_insights();
    assert_eq!(source.insights(), &Insights::Loading);
    settle(&mut source);
    assert_eq!(source.insights(), &Insights::Failed(INSIGHTS_FAILED.into()));

    *fake.usage.lock().unwrap() = Ok((240 * GB, 1_000 * GB));
    fake.broken.store(true, Ordering::SeqCst);
    source.reload_insights();
    settle(&mut source);
    assert_eq!(source.insights(), &Insights::Failed(INSIGHTS_FAILED.into()));

    fake.broken.store(false, Ordering::SeqCst);
    source.reload_insights();
    settle(&mut source);
    assert_eq!(ready(&source).used, "240 GB");
}

/// Locked or signed out: no file name, at once and after; an unlock reads again.
#[test]
fn a_locked_vault_shows_no_insights_and_no_folders() {
    let fake = account();
    let mut source = source(&fake, Arc::default());
    assert!(matches!(source.keep(), Keep::Ready { .. }));
    fake.gate(GateState::Locked);
    assert_eq!(source.keep(), Keep::Failed, "at once");
    settle(&mut source);
    assert_eq!(source.insights(), &Insights::Failed(INSIGHTS_LOCKED.into()));
    assert_eq!(source.keep, Keep::Failed, "gone from memory");
    assert_eq!(source.sync().downloaded, "");

    // Unlocked again: only the panes the person opens read for the new session.
    fake.gate(GateState::Ready);
    source.pump(0.0);
    source.opened(Pane::Insights);
    source.opened(Pane::Sync);
    settle(&mut source);
    assert_eq!(ready(&source).used, "240 GB");
    assert!(matches!(source.keep(), Keep::Ready { .. }));
}

/// The Sync pane's reads: where sync stands from the handle's view, Open at login from the platform, the sizes on
/// this PC; Pause, Resume and Open at login go to the engine and the platform.
#[test]
fn the_sync_pane_reads_the_view_and_changes_go_to_the_engine_and_the_platform() {
    let fake = account();
    let platform = Arc::new(FakePlatform::default());
    let mut source = source(&fake, platform.clone());
    let sync = source.sync();
    assert!(sync.in_explorer);
    assert!(!sync.open_at_login);
    assert_eq!(sync.limit_gb, 50);
    assert_eq!(sync.offline, "1 item · 212 MB");
    assert_eq!(sync.downloaded, "0 bytes");
    assert_eq!(
        sync.status,
        SyncStatus::Running(crate::ui::status_line::CHECKING.to_string())
    );

    source.set_paused(true);
    settle(&mut source);
    assert_eq!(*fake.paused.lock().unwrap(), [true]);
    assert_eq!(source.sync().status, SyncStatus::Paused);
    source.set_paused(false);
    settle(&mut source);
    assert_ne!(source.sync().status, SyncStatus::Paused);

    source.set_open_at_login(true);
    settle(&mut source);
    assert!(platform.autostart_enabled().unwrap());
    assert!(source.sync().open_at_login);
}

/// Free up space reaches the engine for downloaded copies only; previews have no engine call.
#[test]
fn free_up_space_reaches_the_engine_for_downloaded_copies_only() {
    let fake = account();
    let mut source = source(&fake, Arc::default());
    source.clear(Kept::Previews);
    settle(&mut source);
    assert_eq!(*fake.freed.lock().unwrap(), 0);
    source.clear(Kept::Downloaded);
    settle(&mut source);
    assert_eq!(*fake.freed.lock().unwrap(), 1);
}

/// Keep on this PC: the tree by path, each folder with what it holds, its size and whether it is kept.
#[test]
fn keep_lists_the_folder_tree_with_sizes_and_the_kept_set() {
    let fake = account();
    let source = source(&fake, Arc::default());
    let Keep::Ready { folders, total_mb } = source.keep() else {
        panic!("the tree loaded");
    };
    assert_eq!(total_mb.round(), 7815.0);
    let names: Vec<&str> = folders.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["Papers", "Shoot"]);
    assert!(folders[0].kept && !folders[1].kept);
    assert_eq!(folders[0].count, "1 item");
    assert_eq!(folders[1].count, "12 items");
    assert!(folders[1].expandable);
    assert_eq!(folders[1].children[0].name, "Raw");
    assert_eq!(folders[1].children[0].mb, 4_000.0);
}

/// Apply: only the folders named, through the pin change, only while sync runs; the tree is read again after.
#[test]
fn apply_keep_changes_only_the_folders_named_while_sync_runs() {
    let fake = account();
    let mut source = source(&fake, Arc::default());
    fake.syncing.store(false, Ordering::SeqCst);
    source.apply_keep(&["d-shoot".into()], &[]);
    settle(&mut source);
    assert!(fake.kept_calls.lock().unwrap().is_empty());

    fake.syncing.store(true, Ordering::SeqCst);
    source.apply_keep(&["d-shoot".into()], &["d-papers".into()]);
    settle(&mut source);
    assert_eq!(
        *fake.kept_calls.lock().unwrap(),
        [("d-shoot".to_string(), true), ("d-papers".to_string(), false)]
    );
    let Keep::Ready { folders, .. } = source.keep() else {
        panic!("the tree loaded");
    };
    assert!(!folders[0].kept && folders[1].kept);
}

/// Every pane this source leaves unwired draws the shell's "not built yet" frame and no pane of values; the same
/// panes on the board's fixture draw values (the check can fail). Storage insights draws its own page.
#[test]
fn every_unwired_pane_draws_no_sample_value() {
    for (frame, pane) in [("DW-0712", Pane::Storage), ("DW-0744", Pane::Previews)] {
        let engine = drawn(
            frame,
            Box::new(EngineDriveSettings::new(account(), Arc::new(FakePlatform::default()))),
        );
        assert!(!engine.state.settings.wired(pane), "{frame}");
        assert!(has(&engine, "not-built"), "{frame}: the shell's frame");
        let texts = texts(&engine);
        for sample in ["240 GB", "18.4 GB", "1 TB", "50 GB", "Anna", "3 items"] {
            assert!(!texts.iter().any(|t| t.contains(sample)), "{frame} draws {sample}");
        }
        let board = drawn(frame, Box::new(BoardDrive::new()));
        assert!(!has(&board, "not-built"), "{frame}: the fixture draws its pane");
    }
    for (frame, pane) in [
        ("DW-0783", Pane::Insights),
        ("DW-0746", Pane::Sync),
        ("DW-0785", Pane::Bandwidth),
        ("DW-0790", Pane::Diagnostics),
    ] {
        let engine = drawn(
            frame,
            Box::new(EngineDriveSettings::new(account(), Arc::new(FakePlatform::default()))),
        );
        assert!(engine.state.settings.wired(pane), "{frame}");
        assert!(!has(&engine, "not-built"), "{frame} draws its own pane");
        let texts = texts(&engine);
        for sample in ["18.4 GB", "Anna", "3 items", "35.2", "94.0"] {
            assert!(!texts.iter().any(|t| t.contains(sample)), "{frame} draws {sample}");
        }
    }
}

fn drawn(frame: &'static str, drive: Box<dyn DriveSettings>) -> Runtime<State> {
    let mut state = crate::init();
    state.settings.drive = drive;
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, crate::view, (958.0, 1100.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.poll = Some(crate::shell::host::poll);
    rt.frame(0.0);
    crate::frames::settings::go(&mut rt.state, frame);
    for _ in 0..3 {
        rt.frame(0.016);
        rt.settle_all();
    }
    rt
}

fn has(rt: &Runtime<State>, key: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn texts(rt: &Runtime<State>) -> Vec<String> {
    rt.elements().into_iter().filter_map(|e| e.text).collect()
}

/// A change of account clears what was held (P2-5: nothing of account A shows under account B).
#[test]
fn a_change_of_account_clears_the_held_answer() {
    let fake = account();
    let mut source = source(&fake, Arc::default());
    assert!(matches!(source.keep(), Keep::Ready { .. }));
    assert!(source.usage.is_some());

    // Switch to account B.
    *fake.account.lock().unwrap() = Some("other@beebeeb.io".into());
    assert_eq!(
        source.insights(),
        &Insights::Loading,
        "at once, before a pump: nothing of A"
    );
    assert_eq!(source.keep(), Keep::Failed, "at once, before a pump");
    assert_eq!(source.storage().used, "", "at once, before a pump");
    source.pump(0.0);
    assert_eq!(source.keep(), Keep::Failed, "A's tree is gone");
    assert!(source.usage.is_none(), "A's usage is gone");
    assert!(source.on_pc.is_none(), "A's local sizes are gone");
    assert_eq!(source.insights, Insights::Loading, "A's insights are gone from memory");

    // The source asked a new read; B's state DB is A's, so nothing shows (task 2488).
    settle(&mut source);
    assert!(source.on_pc.is_none(), "B owns no local state");
}

/// Task 2488: a local state database owned by another account reads as nothing for insights and the folder tree.
#[test]
fn another_accounts_local_state_reads_as_nothing() {
    // Account B's fake.
    let fake = Fake::with_account("other@beebeeb.io");
    let db = fake.db();
    file(&db, "d-papers", "Papers", 0, true);
    file(&db, "f-tax", "Papers/Taxes 2025.pdf", 212_000_000, false);
    db.set_recursive_pin("d-papers", true, 1_794_000_000).unwrap();

    // Sign in as account A.
    *fake.account.lock().unwrap() = Some("invented@beebeeb.io".into());
    let source = source(&fake, Arc::default());
    assert_eq!(source.keep(), Keep::Failed, "A never sees B's folders");
    assert!(source.on_pc.is_none(), "A never sees B's local sizes");
}

/// A refused Apply (sync not running) sends a refusal the window can draw (P2-6).
#[test]
fn a_refused_apply_sends_a_toast() {
    let fake = account();
    let mut source = source(&fake, Arc::default());
    fake.syncing.store(false, Ordering::SeqCst);
    source.apply_keep(&["d-shoot".into()], &[]);
    settle(&mut source);
    assert_eq!(source.take_toast(), Some(KEEP_NOT_SYNCING));
    // Taken once.
    assert_eq!(source.take_toast(), None);
}

/// `opened(Insights)` reads again when what is held is older than `INSIGHTS_FRESH_FOR` (P2-3).
#[test]
fn opened_insights_reads_again_when_stale() {
    let fake = account();
    let mut source = source(&fake, Arc::default());
    assert!(matches!(source.insights(), Insights::Ready(_)));

    // Fresh: opened does not ask again.
    source.opened(Pane::Insights);
    assert!(!source.insights_reading, "fresh: no new read");

    // Make stale.
    source.insights_at = Some(Instant::now() - INSIGHTS_FRESH_FOR - Duration::from_secs(1));
    source.opened(Pane::Insights);
    assert!(source.insights_reading, "stale: a new read is asked");
}

/// The words a page shows for sizes, counts and long paths.
#[test]
fn sizes_counts_and_paths_read_as_the_board_writes_them() {
    assert_eq!(size_words(240 * GB), "240 GB");
    assert_eq!(size_words(221_600_000_000), "221.6 GB");
    assert_eq!(size_words(940_000_000), "940 MB");
    assert_eq!(size_words(1_000 * GB), "1 TB");
    assert_eq!(size_words(512), "512 bytes");
    assert_eq!(count_words(3_140, "file", "files"), "3,140 files");
    assert_eq!(count_words(1, "item", "items"), "1 item");
    let long = "Backups/Invented-Laptop/Desktop/Screen recording 14 Oct.mov";
    let shown = cut_middle(long);
    assert!(
        shown.ends_with("Screen recording 14 Oct.mov") && shown.contains('\u{2026}'),
        "{shown}"
    );
    assert_eq!(shown.chars().count(), 49);
    assert_eq!(cut_middle("Short/path.txt"), "Short/path.txt");
}

/// Wait until the worker has nothing left (at most 30 s), without pumping: the test decides when the window looks.
fn worker_done(pending: &AtomicUsize) {
    let deadline = Instant::now() + HOLD_FOR;
    while pending.load(Ordering::SeqCst) > 0 {
        assert!(Instant::now() < deadline, "the worker did not finish");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn frames(rt: &mut Runtime<State>) {
    for _ in 0..3 {
        rt.frame(0.016);
        rt.settle_all();
    }
}

/// Names and figures of the first account (`account()`), as Storage insights draws them.
const OF_THE_FIRST: [&str; 5] = ["Archive.zip", "Clip one.mov", "Taxes 2025.pdf", "240 GB", "Take 9.mp4"];

/// Insights worked out for one account and landing after a lock: dropped, in the window and in memory.
#[test]
fn insights_landing_after_a_lock_are_dropped() {
    let fake = account();
    // The second read of the local state is the insights' (the first is the local sizes').
    fake.after_read.arm_at(2);
    let mut source = opened_source(&fake, Arc::new(FakePlatform::default()));
    fake.after_read.reached();
    fake.gate(GateState::Locked);
    source.pump(0.0);
    fake.after_read.release();
    settle(&mut source);
    assert_eq!(source.insights(), &Insights::Failed(INSIGHTS_LOCKED.into()));
    assert_eq!(
        source.insights,
        Insights::Failed(INSIGHTS_LOCKED.into()),
        "no name of it in memory"
    );
    assert!(source.usage.is_none() && source.on_pc.is_none());
    assert_eq!(source.keep, Keep::Failed);
}

/// Insights worked out for one account and landing after another account opened the vault: never drawn. Asserted
/// on the element tree, while the second account's own answer is still held back (so nothing has replaced it).
#[test]
fn insights_of_one_account_landing_under_another_are_never_drawn() {
    let fake = account();
    fake.after_read.arm_at(2);
    // The second ask for the use is the second account's.
    fake.in_usage.arm_at(2);
    let source = opened_source(&fake, Arc::new(FakePlatform::default()));
    let pending = source.pending.clone();
    fake.after_read.reached();
    let mut rt = drawn("DW-0783", Box::new(source));

    // The first account locks; another one signs in and unlocks. The window sees each.
    fake.gate(GateState::Locked);
    frames(&mut rt);
    assert!(texts(&rt).iter().any(|t| t == INSIGHTS_LOCKED), "the locked sentence");
    *fake.account.lock().unwrap() = Some("other@beebeeb.io".into());
    *fake.usage.lock().unwrap() = Ok((5 * GB, 100 * GB));
    fake.gate(GateState::Ready);
    frames(&mut rt);

    // The first account's answer lands now; the second account's is held inside its ask for the use.
    fake.after_read.release();
    fake.in_usage.reached();
    frames(&mut rt);
    let drawn_now = texts(&rt);
    for theirs in OF_THE_FIRST {
        assert!(
            !drawn_now.iter().any(|t| t.contains(theirs)),
            "{theirs} of the first account is drawn"
        );
    }
    assert_eq!(rt.state.settings.drive.insights(), &Insights::Loading);
    assert_eq!(rt.state.settings.drive.keep(), Keep::Failed);
    assert_eq!(rt.state.settings.drive.storage().used, "");

    // The second account's own answer: its use, and no file of the first (the local state is not its own).
    fake.in_usage.release();
    worker_done(&pending);
    frames(&mut rt);
    let drawn_now = texts(&rt);
    assert!(drawn_now.iter().any(|t| t.contains("5 GB")), "the second account's use");
    for theirs in OF_THE_FIRST {
        assert!(
            !drawn_now.iter().any(|t| t.contains(theirs)),
            "{theirs} of the first account is drawn"
        );
    }
}

/// Changes one account asked for that the worker reaches after another account signed in: none is made, for
/// anyone.
#[test]
fn a_change_asked_by_one_account_is_not_made_for_the_next() {
    let fake = account();
    let platform = Arc::new(FakePlatform::default());
    // The worker is held in its first read while the changes queue up behind it.
    fake.after_read.arm_at(1);
    let mut source = opened_source(&fake, platform.clone());
    fake.after_read.reached();
    source.set_paused(true);
    source.set_open_at_login(true);
    source.clear(Kept::Downloaded);
    source.apply_keep(&["d-shoot".into()], &["d-papers".into()]);

    *fake.account.lock().unwrap() = Some("other@beebeeb.io".into());
    source.pump(0.0);
    fake.after_read.release();
    settle(&mut source);
    assert!(
        fake.paused.lock().unwrap().is_empty(),
        "sync was paused for another account"
    );
    assert!(!platform.autostart_enabled().unwrap(), "open at login was changed");
    assert_eq!(*fake.freed.lock().unwrap(), 0, "space was freed for another account");
    assert!(
        fake.kept_calls.lock().unwrap().is_empty(),
        "folders were changed for another account"
    );
    assert_eq!(
        source.take_toast(),
        None,
        "the refusal belongs to the account that left"
    );
}

/// The same, landing after a lock: no change is made in a locked vault.
#[test]
fn a_change_the_worker_reaches_after_a_lock_is_not_made() {
    let fake = account();
    let platform = Arc::new(FakePlatform::default());
    fake.after_read.arm_at(1);
    let mut source = opened_source(&fake, platform.clone());
    fake.after_read.reached();
    source.set_paused(true);
    source.set_open_at_login(true);
    fake.gate(GateState::Locked);
    fake.after_read.release();
    // The window has not looked yet: the refusal is of its generation, and is said.
    worker_done(&source.pending);
    assert!(fake.paused.lock().unwrap().is_empty());
    assert!(!platform.autostart_enabled().unwrap());
    settle(&mut source);
    assert!(fake.paused.lock().unwrap().is_empty());
}

/// A change that cannot be asked, or that the engine refuses, is said in the window's toast, never a log line only.
#[test]
fn a_change_that_does_not_happen_is_said() {
    let fake = account();
    let mut source = source(&fake, Arc::default());
    fake.pause_fails.store(true, Ordering::SeqCst);
    source.set_paused(true);
    settle(&mut source);
    assert_eq!(source.take_toast(), Some(PAUSE_FAILED));
    assert_eq!(source.take_toast(), None, "taken once");

    fake.gate(GateState::Locked);
    source.set_paused(true);
    assert_eq!(source.take_toast(), Some(CHANGE_LOCKED), "at once, before a pump");
    settle(&mut source);
    source.clear(Kept::Downloaded);
    assert_eq!(source.take_toast(), Some(CHANGE_LOCKED));
    settle(&mut source);
    assert_eq!(*fake.freed.lock().unwrap(), 0);
    assert!(fake.paused.lock().unwrap().is_empty());
}

/// The app's own glue (`impl DriveEngine for Engine`), against a real engine handle with no one signed in: no
/// account, no local state handed out (and why), no use asked of Beebeeb, no folder kept.
#[test]
fn the_engines_own_glue_hands_out_nothing_with_no_one_signed_in() {
    use beebeeb_engine::app::{Engine, EngineConfig};
    let (host, _messages) = beebeeb_engine::host::ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "drive-settings-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: crate::engine_host::open::ApiBase::engine_default().origin().to_string(),
    };
    let engine = Engine::open(
        &beebeeb_engine::identity::TEST,
        config,
        host,
        beebeeb_engine::signin::memory_store::MemoryCredentials::default(),
    )
    .expect("the engine opens");
    engine.restore();
    assert_eq!(DriveEngine::account(&engine), None);
    assert_ne!(DriveEngine::view(&engine).gate, GateState::Ready);
    assert_eq!(
        DriveEngine::local_state(&engine, |_| ()),
        Err(LocalStateRefused::SignedOut)
    );
    assert!(DriveEngine::storage_info(&engine).is_err());
    assert!(DriveEngine::set_kept(&engine, "an-invented-id", true).is_err());
    assert!(!DriveEngine::syncing(&engine));
    // The limits are this PC's: read without a session, and never changed without the session of a press.
    assert_eq!(DriveEngine::session(&engine), engine.session_turn());
    let found = DriveEngine::limits(&engine).expect("the settings file reads");
    let change = TransferLimitsChange {
        upload_kib_per_s: Some(found.upload_kib_per_s + 1),
        ..TransferLimitsChange::default()
    };
    assert!(DriveEngine::set_limits(&engine, engine.session_turn(), change).is_err());
    assert_eq!(DriveEngine::limits(&engine), Ok(found), "nothing was written");
}
