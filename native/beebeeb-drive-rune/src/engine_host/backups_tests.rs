//! Backups on the real engine (`EngineBackupsSource`): the root and the folders from the engine's answer, the empty
//! answer, an engine error, signed out, another account's local state, a switch only for the listed folder and the
//! account it was asked for, a refused switch said through `pump`, nothing of one account after a change to another
//! (through the trait alone), an answer that outlived its session dropped, the source off in a real build, and the
//! `impl BackupsEngine for Engine` glue on the engine's own handle. Every name is invented.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beebeeb_engine::app::LocalStateRefused;
use beebeeb_engine::app::backups_api::{BackupFolderRow, BackupsError, BackupsOverview, BackupsRootRow, FolderState};

use super::*;
use crate::frames::backups::source::{BackupsSource, Part, RootAction};

const A: &str = "invented-a@beebeeb.io";
const B: &str = "invented-b@beebeeb.io";

/// The engine handle as the adapter sees it.
struct Fake {
    unlocked: AtomicBool,
    account: Mutex<Option<String>>,
    /// What `overview` answers, per account.
    answers: Mutex<Vec<(String, Result<BackupsOverview, BackupsError>)>>,
    /// What `set_folder` answers.
    switch_answer: Mutex<Result<(), BackupsError>>,
    /// What `set_folder` was asked, with the account signed in then.
    switches: Mutex<Vec<(String, bool, Option<String>)>>,
    /// The next `overview` signs this account in while it reads (an answer that outlives its session).
    swap_during_read: Mutex<Option<String>>,
    /// The runner is passing now / a "Back up now" waits / how many were asked.
    running: AtomicBool,
    requested: AtomicBool,
    run_requests: Mutex<u32>,
}

impl Fake {
    fn new() -> Arc<Fake> {
        Arc::new(Fake {
            unlocked: AtomicBool::new(true),
            account: Mutex::new(Some(A.into())),
            answers: Mutex::new(vec![
                (A.into(), Ok(laptop())),
                (B.into(), Ok(BackupsOverview::default())),
            ]),
            switch_answer: Mutex::new(Ok(())),
            switches: Mutex::new(Vec::new()),
            swap_during_read: Mutex::new(None),
            running: AtomicBool::new(false),
            requested: AtomicBool::new(false),
            run_requests: Mutex::new(0),
        })
    }

    fn answer(&self, account: &str, answer: Result<BackupsOverview, BackupsError>) {
        let mut answers = self.answers.lock().unwrap();
        answers.retain(|(who, _)| who != account);
        answers.push((account.into(), answer));
    }

    fn sign_in(&self, account: Option<&str>) {
        *self.account.lock().unwrap() = account.map(str::to_string);
    }
}

impl BackupsEngine for Fake {
    fn unlocked(&self) -> bool {
        self.unlocked.load(Ordering::SeqCst)
    }
    fn account(&self) -> Option<String> {
        self.account.lock().unwrap().clone()
    }
    fn overview(&self) -> Result<BackupsOverview, BackupsError> {
        let who = self.account();
        let answer = self
            .answers
            .lock()
            .unwrap()
            .iter()
            .find(|(account, _)| Some(account) == who.as_ref())
            .map(|(_, answer)| answer.clone())
            .unwrap_or(Err(BackupsError::Refused(LocalStateRefused::SignedOut)));
        if let Some(next) = self.swap_during_read.lock().unwrap().take() {
            self.sign_in(Some(&next));
        }
        answer
    }
    fn set_folder(&self, key: &str, on: bool) -> Result<(), BackupsError> {
        self.switches.lock().unwrap().push((key.into(), on, self.account()));
        *self.switch_answer.lock().unwrap()
    }
    fn pass_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
    fn pass_requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }
    fn request_pass(&self) {
        *self.run_requests.lock().unwrap() += 1;
        self.requested.store(true, Ordering::SeqCst);
    }
}

fn row(key: &'static str, name: &'static str, on: bool, state: FolderState) -> BackupFolderRow {
    BackupFolderRow {
        key,
        name,
        on,
        state,
        items: Some(214),
        last_pass_at_ms: Some(1_700_000_000_000),
        stop_words: (state == FolderState::Stopped).then(|| "Invented-Laptop's Videos folder was removed.".to_string()),
    }
}

/// Account A's laptop: Documents on, Music off, Videos stopped, Downloads not on this PC.
fn laptop() -> BackupsOverview {
    BackupsOverview {
        root: Some(BackupsRootRow {
            root_id: "0193b1a6-0000-7000-8000-000000000001".into(),
            device_folder: "Invented-Laptop".into(),
        }),
        folders: vec![
            row("documents", "Documents", true, FolderState::On),
            row("music", "Music", false, FolderState::Off),
            row("videos", "Videos", false, FolderState::Stopped),
            row("downloads", "Downloads", false, FolderState::Unavailable),
        ],
    }
}

fn settle<E: BackupsEngine>(source: &mut EngineBackupsSource<E>) -> Pumped {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut seen = Pumped::default();
    loop {
        let pumped = source.pump();
        seen.refusal = seen.refusal.or(pumped.refusal);
        if source.idle() {
            let pumped = source.pump();
            seen.refusal = seen.refusal.or(pumped.refusal);
            return seen;
        }
        assert!(Instant::now() < deadline, "the worker did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn ids(source: &dyn BackupsSource) -> Vec<String> {
    source.folders().into_iter().map(|f| f.id).collect()
}

#[test]
fn the_root_and_the_folders_come_from_the_engines_answer() {
    let fake = Fake::new();
    let mut source = EngineBackupsSource::new(fake.clone());
    assert!(source.folders().is_empty(), "nothing before the first answer");
    source.pump();
    assert!(source.loading(), "a read is out and nothing is held");
    settle(&mut source);
    assert!(!source.loading());
    let root = source.root();
    assert_eq!(root.id, "0193b1a6-0000-7000-8000-000000000001");
    assert_eq!(root.device_folder, "Invented-Laptop");
    assert!(root.is_protected && root.allows(RootAction::Open) && !root.allows(RootAction::Delete));
    assert_eq!(
        ids(&source),
        vec!["documents", "music", "videos"],
        "a folder this PC lacks is left out"
    );
    let folders = source.folders();
    assert!(folders[0].on && !folders[0].chosen_by_you);
    assert_eq!(folders[0].items, 214);
    assert!(folders[0].size.is_empty(), "no size is invented");
    assert!(folders[0].last_backup.is_some());
    assert_eq!(folders[1].last_backup, None, "an off folder has no last backup");
    for part in [
        Part::Root,
        Part::Folders,
        Part::Status,
        Part::NotBackedUp,
        Part::BackUpNow,
        Part::Retry,
    ] {
        assert!(source.wired(part), "{part:?} is bound");
    }
    for part in [
        Part::Devices,
        Part::MailBackup,
        Part::AddFolder,
        Part::RemoveFolder,
        Part::DeleteDevice,
    ] {
        assert!(!source.wired(part), "{part:?} is not bound");
    }
}

#[test]
fn an_empty_answer_has_no_folders_and_a_protected_root_without_an_id() {
    let fake = Fake::new();
    fake.answer(A, Ok(BackupsOverview::default()));
    let mut source = EngineBackupsSource::new(fake);
    settle(&mut source);
    assert!(source.folders().is_empty());
    let root = source.root();
    assert!(root.id.is_empty() && root.device_folder.is_empty());
    assert!(root.is_protected, "fail closed before a layout exists");
    assert_eq!(source.refusal(), None);
}

#[test]
fn an_engine_error_keeps_what_is_held_and_says_so() {
    let fake = Fake::new();
    let mut source = EngineBackupsSource::new(fake.clone());
    settle(&mut source);
    assert_eq!(source.folders().len(), 3);
    fake.answer(A, Err(BackupsError::State));
    // The section opens again after FRESH_FOR: a held answer is not re-read sooner.
    source.read_at = Some(Instant::now() - FRESH_FOR - Duration::from_secs(1));
    source.opened();
    settle(&mut source);
    assert_eq!(source.refusal(), Some(NOT_READ));
    assert_eq!(source.folders().len(), 3, "this account's last answer stays");

    let fresh = Fake::new();
    fresh.answer(A, Err(BackupsError::Settings));
    let mut first = EngineBackupsSource::new(fresh);
    settle(&mut first);
    assert_eq!(first.refusal(), Some(NOT_READ));
    assert!(first.folders().is_empty() && !first.loading());
}

#[test]
fn signed_out_or_locked_asks_nothing_and_shows_nothing() {
    let fake = Fake::new();
    fake.sign_in(None);
    let mut source = EngineBackupsSource::new(fake.clone());
    settle(&mut source);
    assert!(source.folders().is_empty() && source.refusal().is_none() && !source.loading());

    // Signed in and read, then the vault locks: nothing shows at once, before any pump.
    fake.sign_in(Some(A));
    settle(&mut source);
    assert_eq!(source.folders().len(), 3);
    fake.unlocked.store(false, Ordering::SeqCst);
    assert!(source.folders().is_empty());
    assert!(source.root().id.is_empty());
    source.set_folder("music", true);
    assert!(fake.switches.lock().unwrap().is_empty(), "no switch while locked");
}

#[test]
fn another_accounts_local_state_shows_nothing_and_says_why() {
    let fake = Fake::new();
    fake.answer(A, Err(BackupsError::Refused(LocalStateRefused::OtherAccount)));
    let mut source = EngineBackupsSource::new(fake);
    settle(&mut source);
    assert!(source.folders().is_empty());
    assert_eq!(source.refusal(), Some(NOT_YOURS));
}

/// Sign out, sign in as another account: nothing of the first is left, read through the trait alone.
#[test]
fn a_change_of_account_leaves_nothing_of_the_first() {
    let fake = Fake::new();
    let mut source = EngineBackupsSource::new(fake.clone());
    settle(&mut source);
    assert_eq!(source.folders().len(), 3);

    fake.sign_in(None);
    fake.sign_in(Some(B));
    // Before any pump: the readers ask the engine whose account it is.
    assert!(source.folders().is_empty());
    assert!(source.root().device_folder.is_empty());
    settle(&mut source);
    let src: &dyn BackupsSource = &source;
    assert!(src.folders().is_empty());
    assert!(src.root().id.is_empty() && src.root().device_folder.is_empty());
    assert_eq!(src.refusal(), None);
}

#[test]
fn an_answer_that_outlives_its_session_is_dropped() {
    let fake = Fake::new();
    *fake.swap_during_read.lock().unwrap() = Some(B.into());
    let mut source = EngineBackupsSource::new(fake.clone());
    settle(&mut source);
    // A's read ended under B: dropped; B's own read (empty) is what shows.
    assert!(source.folders().is_empty());
    assert!(source.root().id.is_empty());
}

#[test]
fn a_switch_is_sent_once_for_a_listed_folder_and_the_account_it_was_asked_for() {
    let fake = Fake::new();
    let mut source = EngineBackupsSource::new(fake.clone());
    settle(&mut source);
    source.set_folder("music", true);
    source.set_folder("documents", true); // already on: nothing to change
    source.set_folder("downloads", true); // not on this PC
    source.set_folder("not-a-folder", true);
    settle(&mut source);
    assert_eq!(
        *fake.switches.lock().unwrap(),
        vec![("music".to_string(), true, Some(A.to_string()))]
    );
}

#[test]
fn a_switch_asked_for_one_account_never_runs_for_another() {
    let fake = Fake::new();
    let mut source = EngineBackupsSource::new(fake.clone());
    settle(&mut source);
    // Hold the worker on a read so the switch waits behind it, then change the account under it.
    *fake.swap_during_read.lock().unwrap() = Some(B.into());
    source.read_at = None;
    source.opened();
    source.set_folder("music", true);
    let pumped = settle(&mut source);
    assert!(
        fake.switches.lock().unwrap().is_empty(),
        "A's switch did not run under B"
    );
    assert_eq!(pumped.refusal, None, "a refusal of the old generation is not said");
}

#[test]
fn a_refused_switch_is_said_through_pump() {
    let fake = Fake::new();
    let mut source = EngineBackupsSource::new(fake.clone());
    settle(&mut source);
    *fake.switch_answer.lock().unwrap() = Err(BackupsError::Settings);
    source.set_folder("music", true);
    let pumped = settle(&mut source);
    assert_eq!(pumped.refusal, Some(NOT_SWITCHED));
    assert_eq!(fake.switches.lock().unwrap().len(), 1);
}

#[test]
fn the_unbound_parts_answer_nothing_and_send_nothing() {
    let fake = Fake::new();
    let mut source = EngineBackupsSource::new(fake.clone());
    settle(&mut source);
    assert!(source.devices().is_empty() && source.not_backed_up().is_empty() && !source.mail_backup_on());
    assert!(
        !source.mail_backup_offered(),
        "no Mail Backup entry where no mail engine runs"
    );
    assert!(!source.add_folder("/invented/Projects"));
    assert!(!source.delete_device_backup("invented-device", "a password"));
    source.remove_folder("documents", true);
    settle(&mut source);
    assert!(fake.switches.lock().unwrap().is_empty());
    assert_eq!(*fake.run_requests.lock().unwrap(), 0);
    assert_eq!(source.folders().len(), 3, "nothing was removed");
}

#[test]
fn the_card_says_what_the_engine_knows_and_never_a_value_it_did_not_read() {
    use crate::frames::backups::source::Status;
    let fake = Fake::new();
    let mut source = EngineBackupsSource::new(fake.clone());
    assert!(
        matches!(source.status(), Status::Plain { can_run: false, .. }),
        "nothing held: no claim"
    );
    settle(&mut source);
    // Videos stopped itself: not "All backed up".
    assert!(matches!(source.status(), Status::Plain { can_run: true, .. }));
    // Nothing stopped: the oldest last pass of what is on.
    let mut ok = laptop();
    ok.folders.retain(|row| row.state != FolderState::Stopped);
    fake.answer(A, Ok(ok.clone()));
    source.read_at = None;
    source.opened();
    settle(&mut source);
    match source.status() {
        Status::AllBackedUp { last, from_this_pc } => {
            assert!(!last.is_empty());
            assert_eq!(from_this_pc, "", "no count of the files is claimed");
        }
        other => panic!("{other:?}"),
    }
    // A folder that is on and has never passed waits for its first backup.
    ok.folders[0].last_pass_at_ms = None;
    fake.answer(A, Ok(ok.clone()));
    source.read_at = None;
    source.opened();
    settle(&mut source);
    assert!(matches!(source.status(), Status::Plain { ref title, can_run: true, .. } if title.contains("first")));
    // Nothing on at all.
    ok.folders[0].on = false;
    ok.folders[0].state = FolderState::Off;
    fake.answer(A, Ok(ok));
    source.read_at = None;
    source.opened();
    settle(&mut source);
    assert!(matches!(source.status(), Status::Plain { ref title, can_run: false, .. } if title.contains("Nothing")));
    // A pass the runner is running says so, whatever the folders say.
    fake.running.store(true, Ordering::SeqCst);
    assert!(matches!(source.status(), Status::Plain { ref title, .. } if title == "Backing up"));
}

#[test]
fn back_up_now_asks_the_runner_once_and_a_pass_that_ends_is_read_again() {
    let fake = Fake::new();
    let mut source = EngineBackupsSource::new(fake.clone());
    settle(&mut source);
    source.back_up_now();
    source.back_up_now(); // already out: not asked again
    settle(&mut source);
    assert_eq!(*fake.run_requests.lock().unwrap(), 1);
    assert!(
        matches!(source.status(), crate::frames::backups::source::Status::Plain { ref title, .. } if title == "Backing up"),
        "the card says it at once"
    );
    // The runner took it: running, then ended; the source reads again for the new last pass.
    fake.requested.store(false, Ordering::SeqCst);
    fake.running.store(true, Ordering::SeqCst);
    assert!(source.pump().busy, "a running pass keeps the window looking");
    fake.running.store(false, Ordering::SeqCst);
    let pumped = source.pump();
    assert!(pumped.changed, "the card follows the pass that ended");
    settle(&mut source);
    // Nothing on: nothing asked.
    let mut off = laptop();
    off.folders[0].on = false;
    fake.answer(A, Ok(off));
    source.read_at = None;
    source.opened();
    settle(&mut source);
    fake.requested.store(false, Ordering::SeqCst);
    source.back_up_now();
    settle(&mut source);
    assert_eq!(
        *fake.run_requests.lock().unwrap(),
        1,
        "no folder is on: no pass is asked for"
    );
}

#[test]
fn a_late_back_up_now_for_another_account_is_never_asked() {
    let fake = Fake::new();
    let mut source = EngineBackupsSource::new(fake.clone());
    settle(&mut source);
    *fake.swap_during_read.lock().unwrap() = Some(B.into());
    source.read_at = None;
    source.opened();
    source.back_up_now();
    settle(&mut source);
    assert_eq!(*fake.run_requests.lock().unwrap(), 0, "A's ask did not run under B");
}

#[test]
fn what_could_not_back_up_is_the_stopped_folders_in_cores_words_and_try_again_switches_them_on() {
    let fake = Fake::new();
    let mut source = EngineBackupsSource::new(fake.clone());
    settle(&mut source);
    let items = source.not_backed_up();
    assert_eq!(items.len(), 1);
    assert_eq!((items[0].id.as_str(), items[0].name.as_str()), ("videos", "Videos"));
    assert_eq!(items[0].why, "Invented-Laptop's Videos folder was removed.");
    assert!(!items[0].excluded);
    source.retry("music"); // not stopped: nothing
    source.retry("documents");
    settle(&mut source);
    assert!(fake.switches.lock().unwrap().is_empty());
    source.retry_all();
    settle(&mut source);
    assert_eq!(
        *fake.switches.lock().unwrap(),
        vec![("videos".to_string(), true, Some(A.to_string()))]
    );
}

#[test]
fn on_then_off_before_the_first_read_sends_both_and_the_list_shows_the_last_ask() {
    let fake = Fake::new();
    let mut source = EngineBackupsSource::new(fake.clone());
    settle(&mut source);
    source.set_folder("music", true);
    // The read of the first switch has not landed: the list already shows what was asked, so the next press asks
    // for the opposite and it is not dropped as "already off" (review of desktop#384, P2).
    assert!(source.folders().iter().any(|f| f.id == "music" && f.on));
    source.set_folder("music", false);
    assert!(source.folders().iter().any(|f| f.id == "music" && !f.on));
    settle(&mut source);
    assert_eq!(
        *fake.switches.lock().unwrap(),
        vec![
            ("music".to_string(), true, Some(A.to_string())),
            ("music".to_string(), false, Some(A.to_string())),
        ]
    );
    // Both reads landed: what the engine has is shown again (the fake's overview never changed: off).
    assert!(source.folders().iter().any(|f| f.id == "music" && !f.on));
    assert!(
        source.switching.is_empty(),
        "nothing is pending once both were read back"
    );
}

#[test]
fn a_refused_switch_is_not_shown_as_done_once_read_back() {
    let fake = Fake::new();
    let mut source = EngineBackupsSource::new(fake.clone());
    settle(&mut source);
    *fake.switch_answer.lock().unwrap() = Err(BackupsError::Settings);
    source.set_folder("music", true);
    assert!(
        source.folders().iter().any(|f| f.id == "music" && f.on),
        "shown as asked until read back"
    );
    let pumped = settle(&mut source);
    assert_eq!(pumped.refusal, Some(NOT_SWITCHED));
    assert!(
        source.folders().iter().any(|f| f.id == "music" && !f.on),
        "the engine's own value again"
    );
}

#[test]
fn source_for_gives_the_engines_source_now_backups_is_on() {
    assert!(TURNED_ON);
    assert!(source_for(Fake::new()).is_some());
}

#[test]
fn last_backup_words_say_today_or_the_day() {
    use chrono::TimeZone;
    let then = chrono::Local.with_ymd_and_hms(2026, 11, 12, 9, 30, 0).single().unwrap();
    let at = then.timestamp_millis();
    assert_eq!(last_backup_words(at, then.date_naive()), "Today, 09:30");
    let next_day = then.date_naive().succ_opt().unwrap();
    assert_eq!(last_backup_words(at, next_day), "12 Nov, 09:30");
}

/// The glue on the engine's own handle: signed in and locked, both calls are refused by the account-bound handle
/// before anything is read or written.
#[test]
fn the_engine_glue_is_refused_by_the_account_bound_handle_while_locked() {
    let engine = super::super::tests::started().opened.engine.clone();
    assert!(!BackupsEngine::unlocked(&*engine));
    assert_eq!(
        BackupsEngine::overview(&*engine),
        Err(BackupsError::Refused(LocalStateRefused::Locked))
    );
    assert_eq!(
        BackupsEngine::set_folder(&*engine, "music", true),
        Err(BackupsError::Refused(LocalStateRefused::Locked))
    );
    assert_eq!(
        BackupsEngine::set_folder(&*engine, "not-a-folder", true),
        Err(BackupsError::UnknownFolder)
    );
}
