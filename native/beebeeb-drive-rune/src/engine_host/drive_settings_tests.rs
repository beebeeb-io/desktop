//! Drive's settings panes on the real engine (`EngineDriveSettings`), against the engine's own state database in a
//! folder of each test's own and the shell's fake platform: Storage insights (ready, empty, Beebeeb unreachable, the
//! file list unreadable, locked), the Sync pane's reads and changes, Free up space, Keep on this PC's tree and Apply,
//! and every pane left unwired drawing the shell's frame, never a sample value. Every name is invented.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beebeeb_engine::account_binding::Identity;
use beebeeb_engine::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use beebeeb_engine::sync_view::{Explorer, Start, SyncView};
use rune::{Env, Runtime};

use super::*;
use crate::State;
use crate::frames::settings::drive::BoardDrive;
use crate::shell::platform::fake::FakePlatform;

const GB: i64 = 1_000_000_000;

struct Fake {
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
    fn state_db(&self) -> Result<StateDb, String> {
        if self.broken.load(Ordering::SeqCst) {
            return Err("could not open the local state".into());
        }
        StateDb::open(&self.path).map_err(|_| "could not open the local state".to_string())
    }
    fn owns(&self, db: &StateDb) -> bool {
        let account = self.account.lock().unwrap();
        let session = Identity::new(None, account.as_deref());
        beebeeb_engine::app::state_owner::owned_by(db, &session)
    }
    fn storage_info(&self) -> Result<(i64, i64), String> {
        self.usage.lock().unwrap().clone()
    }
    fn set_paused(&self, paused: bool) -> Result<(), String> {
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

fn source(fake: &Arc<Fake>, platform: Arc<FakePlatform>) -> EngineDriveSettings<Fake> {
    let mut source = EngineDriveSettings::new(fake.clone(), platform);
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

    fake.gate(GateState::Ready);
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
    for (frame, pane) in [
        ("DW-0712", Pane::Storage),
        ("DW-0746", Pane::Sync),
        ("DW-0785", Pane::Bandwidth),
        ("DW-0744", Pane::Previews),
        ("DW-0790", Pane::Diagnostics),
    ] {
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
    let insights = drawn(
        "DW-0783",
        Box::new(EngineDriveSettings::new(account(), Arc::new(FakePlatform::default()))),
    );
    assert!(!has(&insights, "not-built"), "Storage insights draws its own page");
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
    source.pump(0.0);
    assert_eq!(source.keep(), Keep::Failed, "A's tree is gone");
    assert!(source.usage.is_none(), "A's usage is gone");
    assert!(source.on_pc.is_none(), "A's local sizes are gone");

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
    let mut source = source(&fake, Arc::default());
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
