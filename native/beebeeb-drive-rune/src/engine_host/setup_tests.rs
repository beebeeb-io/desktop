//! The first run on the real engine (`EngineSetupSource`): against a double of the engine handle (what each step
//! reads and sends, every refusal, another session, another account), in the running app's window (the gate makes
//! and drops the first run), and the engine's own first-run calls once against a real handle on the fake API.
//! Every name is invented.

use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::time::{Duration, Instant};

use rune::{Env, Runtime, Tx};

use super::*;
use crate::State;
use crate::shell::actions::{EngineActions, FixtureEngine};
use crate::shell::app::GateState as ShellGate;
use crate::shell::host::{self, Shell};
use crate::shell::platform::fake::FakePlatform;

#[path = "setup_folder_tests.rs"]
mod folder_tests;
#[path = "setup_glue_tests.rs"]
mod glue;

/// The folder a first run suggests, in these tests.
const SUGGESTED: &str = "/invented/home/Beebeeb";
const FIRST: &str = "setup-first@example.eu";
const SECOND: &str = "setup-second@example.eu";

/// A double of the engine handle: a test sets what it answers and reads what it was sent.
struct Fake {
    view: Mutex<SyncView>,
    stamp: Mutex<Stamp>,
    due: Mutex<Result<bool, SetupRefused>>,
    /// What the two writes answer.
    writes: Mutex<Result<(), SetupRefused>>,
    /// Every session the first run was marked done for.
    marked: Mutex<Vec<SessionGeneration>>,
    /// Every notifications write: its session and its value.
    notified: Mutex<Vec<(SessionGeneration, bool)>>,
    folder: Mutex<Option<PathBuf>>,
    /// The sync folder the settings name.
    settings_folder: Mutex<Result<Option<PathBuf>, SetupRefused>>,
    /// What the engine's rule answers for a folder, and every folder it was asked about.
    rule: Mutex<Result<(), SyncFolderRefused>>,
    checked: Mutex<Vec<PathBuf>>,
    /// What a start answers, and every folder a start was made in.
    start_answer: Mutex<Result<SyncStart, SyncStartRefused>>,
    started: Mutex<Vec<PathBuf>>,
    /// Every folder that was written to the settings.
    kept: Mutex<Vec<PathBuf>>,
    /// How often Windows was asked again for the folder's entry.
    entry_asks: AtomicUsize,
    wakes: AtomicUsize,
    /// A write waits here until the test lets it go (a change that is on its way).
    held: Mutex<()>,
    /// The next write panics on its worker.
    panics: AtomicBool,
}

fn session(number: u64) -> SessionGeneration {
    SessionGeneration::for_test(number)
}

fn fake() -> Arc<Fake> {
    Arc::new(Fake {
        view: Mutex::new(SyncView {
            gate: GateState::Ready,
            ..SyncView::default()
        }),
        stamp: Mutex::new((session(1), Some(FIRST.into()))),
        due: Mutex::new(Ok(true)),
        writes: Mutex::new(Ok(())),
        marked: Mutex::default(),
        notified: Mutex::default(),
        folder: Mutex::new(Some(PathBuf::from("an-invented-folder"))),
        // A PC whose settings name the sync folder: the first run has nothing to ask.
        settings_folder: Mutex::new(Ok(Some(PathBuf::from("an-invented-folder")))),
        rule: Mutex::new(Ok(())),
        checked: Mutex::default(),
        start_answer: Mutex::new(Ok(SyncStart::Started)),
        started: Mutex::default(),
        kept: Mutex::default(),
        entry_asks: AtomicUsize::new(0),
        wakes: AtomicUsize::new(0),
        held: Mutex::new(()),
        panics: AtomicBool::new(false),
    })
}

impl Fake {
    fn set(&self, start: Start, explorer: EngineExplorer) {
        let mut view = self.view.lock().unwrap();
        view.start = start;
        view.explorer = explorer;
    }
}

impl SetupEngine for Fake {
    fn view(&self) -> SyncView {
        *self.view.lock().unwrap()
    }
    fn stamp(&self) -> Stamp {
        self.stamp.lock().unwrap().clone()
    }
    fn wake(&self) {
        self.wakes.fetch_add(1, Ordering::SeqCst);
    }
    fn first_run_due(&self) -> Result<bool, SetupRefused> {
        *self.due.lock().unwrap()
    }
    // As the engine: a write for a session that is not the one in memory writes nothing.
    fn mark_done_for(&self, session: SessionGeneration) -> Result<(), SetupRefused> {
        assert!(
            !self.panics.swap(false, Ordering::SeqCst),
            "a test asked this worker to panic"
        );
        let _held = self.held.lock().unwrap();
        if session != self.stamp().0 {
            return Err(SetupRefused::SessionMoved);
        }
        self.marked.lock().unwrap().push(session);
        *self.writes.lock().unwrap()
    }
    fn set_notifications_for(&self, session: SessionGeneration, on: bool) -> Result<(), SetupRefused> {
        assert!(
            !self.panics.swap(false, Ordering::SeqCst),
            "a test asked this worker to panic"
        );
        let _held = self.held.lock().unwrap();
        if session != self.stamp().0 {
            return Err(SetupRefused::SessionMoved);
        }
        self.notified.lock().unwrap().push((session, on));
        *self.writes.lock().unwrap()
    }
    fn sync_folder(&self) -> Option<PathBuf> {
        self.folder.lock().unwrap().clone()
    }
    fn ask_explorer_again(&self) {
        self.entry_asks.fetch_add(1, Ordering::SeqCst);
    }
    fn settings_folder(&self) -> Result<Option<PathBuf>, SetupRefused> {
        self.settings_folder.lock().unwrap().clone()
    }
    fn suggested_folder(&self) -> PathBuf {
        PathBuf::from(SUGGESTED)
    }
    fn check_folder(&self, folder: &Path) -> Result<(), SyncFolderRefused> {
        let _held = self.held.lock().unwrap();
        self.checked.lock().unwrap().push(folder.to_path_buf());
        *self.rule.lock().unwrap()
    }
    fn start(&self, folder: PathBuf) -> Result<SyncStart, SyncStartRefused> {
        self.started.lock().unwrap().push(folder);
        *self.start_answer.lock().unwrap()
    }
    // As the engine: written only when the settings name none.
    fn keep_folder(&self, folder: &Path) -> Result<bool, SetupRefused> {
        (*self.writes.lock().unwrap())?;
        let mut named = self.settings_folder.lock().unwrap();
        if !matches!(*named, Ok(None)) {
            return Ok(false);
        }
        *named = Ok(Some(folder.to_path_buf()));
        self.kept.lock().unwrap().push(folder.to_path_buf());
        Ok(true)
    }
}

/// What the hooks were asked, for a test to read.
#[derive(Default)]
struct Asked {
    starts: AtomicUsize,
    /// What a start answers.
    start_refused: AtomicBool,
    /// How often the window was asked to look again later.
    looks: AtomicUsize,
}

static OPENED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

fn hooks(asked: &Arc<Asked>, platform: Arc<FakePlatform>, first_run: Arc<FirstRun>) -> Hooks {
    let asked = asked.clone();
    let starts = asked.clone();
    Hooks {
        start_sync: Arc::new(move || {
            let asked = &starts;
            asked.starts.fetch_add(1, Ordering::SeqCst);
            !asked.start_refused.load(Ordering::SeqCst)
        }),
        open_folder: |folder| {
            OPENED.lock().unwrap().push(folder.to_path_buf());
            true
        },
        platform,
        first_run,
        explorer_step: true,
        look_later: {
            let asked = asked.clone();
            Arc::new(move |_| {
                asked.looks.fetch_add(1, Ordering::SeqCst);
            })
        },
    }
}

struct Set {
    engine: Arc<Fake>,
    asked: Arc<Asked>,
    platform: Arc<FakePlatform>,
    first_run: Arc<FirstRun>,
    source: EngineSetupSource<Fake>,
}

fn set_on(engine: Arc<Fake>, platform: FakePlatform) -> Set {
    let (asked, platform, first_run) = (Arc::<Asked>::default(), Arc::new(platform), Arc::<FirstRun>::default());
    first_run.read(&*engine);
    let mut source = EngineSetupSource::new(engine.clone(), hooks(&asked, platform.clone(), first_run.clone()));
    // The source reads, on a worker, whether the folder has to be asked for: taken in before a test begins, so
    // a test counts its own workers from zero.
    let began = Instant::now();
    while engine.wakes.load(Ordering::SeqCst) < 1 {
        assert!(began.elapsed() < Duration::from_secs(10), "the folder was not read");
        std::thread::sleep(Duration::from_millis(5));
    }
    source.poll();
    engine.wakes.store(0, Ordering::SeqCst);
    Set {
        engine,
        asked,
        platform,
        first_run,
        source,
    }
}

fn set() -> Set {
    set_on(fake(), FakePlatform::default())
}

/// Poll until the workers' answers are in: `wakes` workers have finished (at most 10 s).
fn settle(set: &mut Set, wakes: usize) -> bool {
    let began = Instant::now();
    while set.engine.wakes.load(Ordering::SeqCst) < wakes {
        assert!(began.elapsed() < Duration::from_secs(10), "a worker did not answer");
        std::thread::sleep(Duration::from_millis(5));
    }
    set.source.poll()
}

#[test]
fn only_the_steps_something_answers_for_are_available() {
    let set = set();
    let available: Vec<Step> = Step::STEPS
        .into_iter()
        .filter(|step| set.source.available(*step))
        .collect();
    assert_eq!(available, [Step::Explorer, Step::Notifications, Step::Login]);
    assert!(!set.source.opens_settings() && set.source.answers_later());
    // On an OS without File Explorer that step is not available either, and no folder is asked for.
    let mut other = hooks(&set.asked, set.platform.clone(), set.first_run.clone());
    other.explorer_step = false;
    *set.engine.settings_folder.lock().unwrap() = Ok(None);
    let source = EngineSetupSource::new(set.engine.clone(), other);
    assert!(!source.available(Step::Explorer) && source.available(Step::Login));
    assert_eq!(source.folder_ask(), FolderAsk::Settled);
    assert_eq!(
        set.engine.wakes.load(Ordering::SeqCst),
        0,
        "no worker read the settings"
    );
    // What is not bound claims nothing: no folder, no weight, no estimate, no limit, no passkey.
    assert!(set.source.folders().is_empty());
    assert_eq!(set.source.weight(PreviewSize::K4, PreviewFormat::Heic), None);
    assert_eq!(
        (set.source.original_weight(), set.source.space_limit()),
        (String::new(), String::new())
    );
    assert_eq!(set.source.fit_line(PreviewSize::K4, PreviewFormat::Heic), "");
    assert!(
        set.source.sample().bundled,
        "never said to be one of the account's photos"
    );
}

#[test]
fn step_5_follows_the_view() {
    let mut set = set();
    let failed = |code: &str| Explorer::Failed { code: code.into() };
    let refused = Start::Refused(SyncStartRefused::OtherAccountsData);
    for (start, explorer, drawn) in [
        (Start::NotAsked, EngineExplorer::Unknown, Explorer::Adding),
        (Start::Starting, EngineExplorer::Unknown, Explorer::Adding),
        (Start::Running, EngineExplorer::Adding, Explorer::Adding),
        (Start::Running, EngineExplorer::Added, Explorer::Added),
        // Windows' code, written as Windows writes an HRESULT; and no code at all.
        (
            Start::Running,
            EngineExplorer::Failed {
                code: Some(0x8007_017C_u32 as i32),
            },
            failed("0x8007017C"),
        ),
        (Start::Running, EngineExplorer::Failed { code: None }, failed("")),
        // Windows refused to register the folder: DW-0280 with Windows' code as text, whatever the entry said.
        (
            Start::Refused(SyncStartRefused::RegistrationFailed {
                code: Some(0x8007_0005_u32 as i32),
            }),
            EngineExplorer::Unknown,
            failed("0x80070005"),
        ),
        (
            Start::Refused(SyncStartRefused::RegistrationFailed { code: None }),
            EngineExplorer::Adding,
            failed(""),
        ),
        // The engine refused the start: nothing is being added, whatever the entry said before.
        (refused, EngineExplorer::Unknown, Explorer::NotSyncing),
        (refused, EngineExplorer::Adding, Explorer::NotSyncing),
        (
            Start::Refused(SyncStartRefused::FolderOfAnotherProvider),
            EngineExplorer::Unknown,
            Explorer::NotSyncing,
        ),
        (
            Start::Refused(SyncStartRefused::FolderMissing),
            EngineExplorer::Unknown,
            Explorer::NotSyncing,
        ),
    ] {
        set.engine.set(start, explorer);
        assert_eq!(set.source.explorer(), drawn, "{start:?}, {explorer:?}");
    }
    // A start that never came: "adding" only for the moment a start that was just asked for needs; after it,
    // sync is off, and the step says so (with its Try again, which asks for a start and waits again).
    assert_eq!(
        set.asked.looks.load(Ordering::SeqCst),
        1,
        "the window looks again after that moment"
    );
    set.engine.set(Start::NotAsked, EngineExplorer::Unknown);
    assert_eq!(set.source.explorer(), Explorer::Adding);
    set.source.start_asked_at = Instant::now() - START_GRACE;
    assert_eq!(set.source.explorer(), Explorer::NotSyncing);
    assert!(set.source.poll(), "the window draws it");
    set.source.retry_explorer();
    assert_eq!(set.source.explorer(), Explorer::Adding, "a start was just asked for");
    assert_eq!(set.asked.looks.load(Ordering::SeqCst), 2);
    settle(&mut set, 1);
    // The poll says so when the view moved, once.
    set.engine.set(Start::Running, EngineExplorer::Adding);
    set.source.poll();
    assert!(!set.source.poll(), "nothing moved");
    set.engine.set(Start::Running, EngineExplorer::Added);
    assert!(set.source.poll(), "File Explorer finished: the window draws again");
    assert!(!set.source.poll());
}

#[test]
fn try_again_asks_windows_again_and_never_stops_a_sync_that_runs() {
    let mut set = set();
    // Sync did not start: one start is asked for, and Windows is not asked.
    set.engine.set(
        Start::Refused(SyncStartRefused::FolderUnreadable),
        EngineExplorer::Unknown,
    );
    set.source.retry_explorer();
    settle(&mut set, 1);
    assert_eq!(
        (
            set.asked.starts.load(Ordering::SeqCst),
            set.engine.entry_asks.load(Ordering::SeqCst)
        ),
        (1, 0)
    );
    // Sync runs and Windows refused the entry: Windows is asked again; no start is asked for.
    set.engine.set(Start::Running, EngineExplorer::Failed { code: None });
    set.source.retry_explorer();
    settle(&mut set, 2);
    assert_eq!(
        (
            set.asked.starts.load(Ordering::SeqCst),
            set.engine.entry_asks.load(Ordering::SeqCst)
        ),
        (1, 1)
    );
    // A start that could not be asked for changes nothing here: the view still says what the engine says.
    set.engine.set(
        Start::Refused(SyncStartRefused::FolderUnreadable),
        EngineExplorer::Failed { code: None },
    );
    set.asked.start_refused.store(true, Ordering::SeqCst);
    set.source.retry_explorer();
    settle(&mut set, 3);
    assert!(matches!(
        set.source.explorer(),
        Explorer::Failed { .. } | Explorer::NotSyncing
    ));
}

#[test]
fn show_in_file_explorer_opens_the_sync_folder_and_never_another_sessions() {
    let mut set = set();
    *set.engine.folder.lock().unwrap() = Some(PathBuf::from("the-folder-of-this-test"));
    let opened = || {
        OPENED
            .lock()
            .unwrap()
            .iter()
            .filter(|folder| folder.as_path() == Path::new("the-folder-of-this-test"))
            .count()
    };
    let before = set.source.workers_started();
    set.source.show_in_explorer();
    assert_eq!(
        set.source.workers_started(),
        before + 1,
        "control: this session's press starts a worker"
    );
    settle(&mut set, 1);
    assert_eq!(opened(), 1);
    // No folder is known: nothing is opened.
    *set.engine.folder.lock().unwrap() = None;
    set.source.show_in_explorer();
    settle(&mut set, 2);
    assert_eq!(opened(), 1);
    // Another session: nothing is asked at all.
    *set.engine.folder.lock().unwrap() = Some(PathBuf::from("the-folder-of-this-test"));
    *set.engine.stamp.lock().unwrap() = (session(2), Some(FIRST.into()));
    let before = set.source.workers_started();
    set.source.show_in_explorer();
    assert_eq!(set.source.workers_started(), before, "no worker was started");
    assert_eq!((opened(), set.engine.wakes.load(Ordering::SeqCst)), (1, 2));
}

#[test]
fn notifications_are_set_for_the_session_of_the_press_and_said_only_once_answered() {
    let mut set = set();
    set.source.set_notifications(true);
    assert!(
        set.source.take_outcomes().is_empty(),
        "nothing is said before the engine answered"
    );
    assert!(settle(&mut set, 1));
    assert_eq!(set.engine.notified.lock().unwrap().clone(), [(session(1), true)]);
    assert_eq!(
        set.source.take_outcomes(),
        [Outcome {
            switch: Switch::Notifications,
            done: true,
            now: Some(true)
        }]
    );
    assert!(set.source.take_outcomes().is_empty(), "said once");
    set.source.set_notifications(false);
    settle(&mut set, 2);
    assert_eq!(set.source.take_outcomes()[0].now, Some(false));

    // The engine refuses (the settings file, or the session moved under the write): not done, and not known.
    for refusal in [SetupRefused::Settings, SetupRefused::SessionMoved] {
        *set.engine.writes.lock().unwrap() = Err(refusal);
        let wakes = set.engine.wakes.load(Ordering::SeqCst);
        set.source.set_notifications(true);
        settle(&mut set, wakes + 1);
        assert_eq!(
            set.source.take_outcomes(),
            [Outcome {
                switch: Switch::Notifications,
                done: false,
                now: None
            }],
            "{refusal:?}"
        );
    }
}

#[test]
fn open_at_login_is_the_platforms_entry_and_the_answer_is_what_the_entry_is() {
    let mut set = set();
    set.source.set_open_at_login(true);
    assert!(set.source.take_outcomes().is_empty());
    settle(&mut set, 1);
    assert_eq!(set.platform.autostart_writes(), [true]);
    assert_eq!(
        set.source.take_outcomes(),
        [Outcome {
            switch: Switch::OpenAtLogin,
            done: true,
            now: Some(true)
        }]
    );
    set.source.set_open_at_login(false);
    settle(&mut set, 2);
    assert_eq!(set.platform.autostart_writes(), [true, false]);
    assert_eq!(set.source.take_outcomes()[0].now, Some(false));

    // A platform that cannot change the entry: not done, and what is drawn is not the value that was asked for.
    let mut set = set_on(fake(), FakePlatform::unbuilt());
    set.source.set_open_at_login(true);
    settle(&mut set, 1);
    let outcome = set.source.take_outcomes()[0];
    assert!(!outcome.done);
    assert_ne!(outcome.now, Some(true));
}

#[test]
fn open_my_beebeeb_marks_the_first_run_done_and_only_then_lets_the_main_window_through() {
    let mut set = set();
    assert!(set.first_run.due(), "precondition: this PC has its first run to do");
    let held = set.engine.held.lock().unwrap();
    set.source.finish();
    set.source.poll();
    assert!(set.first_run.due(), "not before the engine answered");
    drop(held);
    settle(&mut set, 1);
    assert_eq!(
        set.engine.marked.lock().unwrap().clone(),
        [session(1)],
        "for the session of the press"
    );
    assert!(!set.first_run.due());

    // The settings file could not be written: the person is not held out of their files (the first run is shown
    // again at the next start, where the file is read).
    let mut set = set_on(fake(), FakePlatform::default());
    *set.engine.writes.lock().unwrap() = Err(SetupRefused::Settings);
    set.source.finish();
    settle(&mut set, 1);
    assert!(!set.first_run.due());

    // Written, and the vault is locked before the window polls again: the first run is over for this PC all the
    // same (the file says so), so an unlock in this run does not show it again.
    let mut set = set_on(fake(), FakePlatform::default());
    set.source.finish();
    let began = Instant::now();
    while set.engine.wakes.load(Ordering::SeqCst) < 1 {
        assert!(began.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(5));
    }
    set.engine.view.lock().unwrap().gate = GateState::Locked;
    set.source.poll();
    assert!(!set.first_run.due());
}

/// A worker that panics answers as a press that was refused: a switch says "not done" (never "Saving" for ever),
/// and "Open my Beebeeb" still lets the person in.
#[test]
fn a_worker_that_panics_still_answers() {
    let mut set = set();
    set.engine.panics.store(true, Ordering::SeqCst);
    set.source.set_notifications(true);
    settle(&mut set, 1);
    assert_eq!(
        set.source.take_outcomes(),
        [Outcome {
            switch: Switch::Notifications,
            done: false,
            now: None
        }]
    );
    set.engine.panics.store(true, Ordering::SeqCst);
    set.source.finish();
    settle(&mut set, 2);
    assert!(set.engine.marked.lock().unwrap().is_empty(), "nothing was written");
    assert!(!set.first_run.due(), "the person is not held on the last page");
}

#[test]
fn whether_a_first_run_is_due_is_read_once_and_a_file_that_does_not_read_is_not_a_first_run() {
    let first_run = FirstRun::default();
    assert!(!first_run.due(), "nobody wired it: no first run");
    let engine = fake();
    for (answer, due) in [
        (Ok(true), true),
        (Ok(false), false),
        (Err(SetupRefused::Settings), false),
    ] {
        *engine.due.lock().unwrap() = answer;
        first_run.read(&*engine);
        assert_eq!(first_run.due(), due, "{answer:?}");
    }
}

/// A press under another session or another account sends nothing; an answer that lands after the session moved
/// is handed to nobody; a switch then says "not done" at once, never nothing.
#[test]
fn another_session_or_account_is_never_written_for_and_never_answered() {
    for moved_to in [
        (session(2), Some(FIRST.to_string())),
        (session(1), Some(SECOND.to_string())),
        (session(1), None),
    ] {
        // The press itself is under the other session.
        let mut set = set();
        *set.engine.stamp.lock().unwrap() = moved_to.clone();
        let before = set.source.workers_started();
        set.source.set_notifications(true);
        set.source.set_open_at_login(true);
        set.source.finish();
        set.source.retry_explorer();
        assert_eq!(
            set.source.workers_started(),
            before,
            "{moved_to:?}: no worker was started"
        );
        assert!(set.engine.notified.lock().unwrap().is_empty(), "{moved_to:?}");
        assert!(set.engine.marked.lock().unwrap().is_empty(), "{moved_to:?}");
        assert!(set.platform.autostart_writes().is_empty(), "{moved_to:?}");
        assert_eq!(set.asked.starts.load(Ordering::SeqCst), 0, "{moved_to:?}");
        assert_eq!(set.engine.wakes.load(Ordering::SeqCst), 0, "no worker ran");
        set.source.poll();
        assert!(
            set.source.take_outcomes().is_empty(),
            "nothing is said to another session"
        );
        assert!(set.first_run.due(), "{moved_to:?}: the first run is not ended for it");
        // Control: a press under the right session starts one worker.
        *set.engine.stamp.lock().unwrap() = (session(1), Some(FIRST.into()));
        set.source.set_notifications(true);
        assert_eq!(set.source.workers_started(), before + 1, "{moved_to:?}: control");
        settle(&mut set, 1);

        // The session moves while the change is on its way: the answer is dropped.
        let mut set = set_on(fake(), FakePlatform::default());
        let held = set.engine.held.lock().unwrap();
        set.source.set_notifications(true);
        set.source.finish();
        *set.engine.stamp.lock().unwrap() = moved_to.clone();
        drop(held);
        settle(&mut set, 2);
        assert!(set.source.take_outcomes().is_empty(), "{moved_to:?}");
        // The engine writes only for the session of the press: a session that moved on wrote nothing, and the
        // first run is not ended. (An account never changes under one session in the engine; the double's
        // second case keeps the session, so its write is the press's own and stands, for this PC.)
        let moved = moved_to.0 != session(1);
        assert_eq!(set.first_run.due(), moved, "{moved_to:?}");
        assert_eq!(set.engine.marked.lock().unwrap().is_empty(), moved, "{moved_to:?}");
        assert_eq!(set.engine.notified.lock().unwrap().is_empty(), moved, "{moved_to:?}");
    }
    // Locked (the gate is not Ready): the same.
    let mut set = set();
    set.engine.view.lock().unwrap().gate = GateState::Locked;
    let before = set.source.workers_started();
    set.source.set_notifications(true);
    set.source.finish();
    assert_eq!(set.source.workers_started(), before, "no worker was started");
    assert!(set.engine.notified.lock().unwrap().is_empty() && set.engine.marked.lock().unwrap().is_empty());
    // Control: unlocked, the same press starts one worker.
    set.engine.view.lock().unwrap().gate = GateState::Ready;
    set.source.set_notifications(true);
    assert_eq!(set.source.workers_started(), before + 1, "control");
    settle(&mut set, 1);
}

// ---------------------------------------------------------------------------
// The window: the first run's own steps, on the adapter
// ---------------------------------------------------------------------------

fn window(
    set: Set,
) -> (
    Runtime<State>,
    Arc<Fake>,
    Arc<FixtureEngine>,
    Arc<FirstRun>,
    Arc<FakePlatform>,
) {
    let Set {
        engine,
        asked,
        platform,
        first_run,
        ..
    } = set;
    let fixture = Arc::new(FixtureEngine::ready());
    fixture.set_account(FIRST);
    let mut state = crate::shell::state::init_with(Shell::fixture_on(fixture.clone(), platform.clone()));
    let (source_engine, source_hooks) = (engine.clone(), hooks(&asked, platform.clone(), first_run.clone()));
    state.setup_source = Some(Box::new(move || {
        Box::new(EngineSetupSource::new(source_engine.clone(), source_hooks.clone()))
    }));
    let mut rt = Runtime::new(
        state,
        crate::view,
        (960.0, 640.0),
        Env {
            system_dark: false,
            scale: 2.0,
        },
    );
    rt.on_key = Some(host::on_key);
    rt.poll = Some(|s: &mut State, seconds: f64, tx: &mut Tx| host::poll(s, seconds, tx));
    rt.frame(0.0);
    (rt, engine, fixture, first_run, platform)
}

fn says(rt: &Runtime<State>, words: &str) -> bool {
    rt.elements().iter().any(|e| {
        !e.exiting
            && (e.text.as_deref().is_some_and(|t| t.contains(words))
                || e.label.as_deref().is_some_and(|l| l.contains(words)))
    })
}

fn wait_for(rt: &mut Runtime<State>, what: &str, done: impl Fn(&Runtime<State>) -> bool) {
    let began = Instant::now();
    while !done(rt) {
        assert!(began.elapsed() < Duration::from_secs(10), "never: {what}");
        std::thread::sleep(Duration::from_millis(20));
        rt.frame(0.1);
    }
}

fn press(rt: &mut Runtime<State>, label: &str) {
    let rect = rt
        .elements()
        .into_iter()
        .find(|e| !e.exiting && e.role == "button" && e.label.as_deref() == Some(label))
        .unwrap_or_else(|| panic!("no button {label}"))
        .rect;
    let (x, y) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.016);
    rt.frame(0.1);
}

/// Through the four steps nothing answers for in this version: each keeps its place and its name, says that it
/// is not in this version, and "Continue" leaves it with nothing asked of the engine.
fn to_file_explorer(rt: &mut Runtime<State>) {
    for step in [
        "Add a passkey",
        "Lock Beebeeb",
        "Back up your folders",
        "Thumbnails and previews",
    ] {
        let heading = rt
            .elements()
            .iter()
            .any(|e| !e.exiting && e.role == "heading" && e.text.as_deref() == Some(step));
        assert!(heading, "the step's own name: {step}");
        assert!(says(rt, "This isn't in this version of Beebeeb yet"), "{step}");
        press(rt, "Continue");
    }
}

/// The running app: a gate that shows the first run makes its steps for that session; the seven steps are there,
/// and the ones nothing answers for say so; each switch is drawn once it was answered; "Open my Beebeeb" ends it.
#[test]
fn the_first_run_in_the_window_shows_its_real_steps_and_ends_in_the_main_window() {
    let set = set();
    set.engine.set(Start::Running, EngineExplorer::Adding);
    let (mut rt, engine, fixture, first_run, platform) = window(set);
    assert!(rt.state.setup.is_none(), "the gate is Ready: no first run");
    let starts = |fixture: &FixtureEngine| {
        fixture
            .calls()
            .iter()
            .filter(|call| **call == crate::shell::actions::FixtureCall::StartSync)
            .count()
    };
    let before = starts(&fixture);
    fixture.set_gate_for_scene(ShellGate::FirstRun);
    wait_for(&mut rt, "the first run", |rt| rt.state.setup.is_some());
    assert_eq!(
        starts(&fixture),
        before + 1,
        "sync is asked to start with the first run: its File Explorer step is about the sync folder"
    );
    // The list at the left: the seven steps.
    for step in Step::STEPS {
        assert!(says(&rt, step.label()), "{}", step.label());
    }
    // None of the board's sample values, and no control of a step nothing answers for.
    for sample in ["2,431", "20 GB", "Turn on backup", "Compare", "Not now"] {
        assert!(!says(&rt, sample), "{sample}");
    }
    to_file_explorer(&mut rt);
    for sample in ["2,431", "20 GB"] {
        assert!(!says(&rt, sample), "{sample}");
    }
    // File Explorer follows the engine's view without a press.
    engine.set(
        Start::Refused(SyncStartRefused::OtherAccountsData),
        EngineExplorer::Unknown,
    );
    wait_for(&mut rt, "sync did not start", |rt| {
        says(rt, "Sync didn't start on this PC")
    });
    assert!(!says(&rt, "Windows didn't finish"), "it was not Windows that refused");
    engine.set(Start::Running, EngineExplorer::Added);
    wait_for(&mut rt, "added", |rt| says(rt, "is in File Explorer"));
    press(&mut rt, "Continue");
    // Notifications, then open at login (the engine's answer is held: nothing is drawn as set before it).
    let held = engine.held.lock().unwrap();
    press(&mut rt, "Allow notifications");
    press(&mut rt, "Continue");
    wait_for(&mut rt, "the last page", |rt| says(rt, "You're set up"));
    assert!(says(&rt, "Notifications, Saving"), "asked, not answered: no value");
    assert!(!says(&rt, "Notifications, On"));
    drop(held);
    wait_for(&mut rt, "the answers", |rt| {
        says(rt, "Notifications, On") && says(rt, "Open at login, On")
    });
    assert_eq!(engine.notified.lock().unwrap().clone(), [(session(1), true)]);
    assert_eq!(platform.autostart_writes(), [true]);
    for row in ["Passkey", "App lock", "Backup", "Previews"] {
        assert!(
            says(&rt, &format!("{row}, Not available yet")),
            "a step nothing answered for claims no value: {row}"
        );
    }
    assert!(says(&rt, "File Explorer, On"));
    assert!(
        !rt.elements().iter().any(|e| !e.exiting && e.role == "link"),
        "the rows are values: nothing opens a setting from here"
    );
    // "Open my Beebeeb": marked done for this session, and the gate's flag lets the main window through.
    assert!(first_run.due());
    press(&mut rt, "Open my Beebeeb");
    wait_for(&mut rt, "the first run ended", |_| !first_run.due());
    assert_eq!(engine.marked.lock().unwrap().clone(), [session(1)]);
    fixture.set_gate_for_scene(ShellGate::Ready);
    wait_for(&mut rt, "the main window", |rt| rt.state.setup.is_none());
}

/// A change that did not happen is said on the last page, and the row never shows the value that was asked for.
#[test]
fn a_switch_that_could_not_be_set_is_said_and_not_drawn_as_set() {
    let set = set_on(fake(), FakePlatform::unbuilt());
    set.engine.set(Start::Running, EngineExplorer::Added);
    *set.engine.writes.lock().unwrap() = Err(SetupRefused::Settings);
    let (mut rt, _engine, fixture, ..) = window(set);
    fixture.set_gate_for_scene(ShellGate::FirstRun);
    wait_for(&mut rt, "the first run", |rt| rt.state.setup.is_some());
    to_file_explorer(&mut rt);
    press(&mut rt, "Continue");
    press(&mut rt, "Allow notifications");
    press(&mut rt, "Continue");
    wait_for(&mut rt, "what was not done", |rt| {
        says(rt, "Couldn't change notifications and open at login")
    });
    assert!(says(&rt, "Notifications, Not changed"));
    assert!(!says(&rt, "Notifications, On") && !says(&rt, "Open at login, On"));
}

/// The first run is one session's: the gate leaving it drops it, and another account under the same gate gets its
/// own, from the first step, with nothing of the first one's.
#[test]
fn the_first_run_is_dropped_with_its_session_and_made_again_for_another_account() {
    let set = set();
    set.engine.set(Start::Running, EngineExplorer::Added);
    let (mut rt, engine, fixture, ..) = window(set);
    fixture.set_gate_for_scene(ShellGate::FirstRun);
    wait_for(&mut rt, "the first run", |rt| rt.state.setup.is_some());
    to_file_explorer(&mut rt);
    press(&mut rt, "Continue");
    assert!(says(&rt, "Allow notifications"), "on the step after File Explorer");
    // Locked: nothing of the first run stays.
    fixture.set_gate_for_scene(ShellGate::Locked);
    wait_for(&mut rt, "dropped", |rt| rt.state.setup.is_none());
    // Unlocked again (another session): the first run starts over, for that session.
    *engine.stamp.lock().unwrap() = (session(2), Some(FIRST.into()));
    fixture.set_gate_for_scene(ShellGate::FirstRun);
    wait_for(&mut rt, "made again", |rt| rt.state.setup.is_some());
    assert_eq!(rt.state.setup.as_ref().map(|ui| ui.step), Some(Step::Passkey));
    to_file_explorer(&mut rt);
    press(&mut rt, "Continue");
    // Another account under the same gate: from the first step again, and its presses are its own session's.
    *engine.stamp.lock().unwrap() = (session(3), Some(SECOND.into()));
    fixture.set_account(SECOND);
    wait_for(&mut rt, "the second account's", |rt| {
        rt.state.setup.as_ref().is_some_and(|ui| ui.step == Step::Passkey)
    });
    to_file_explorer(&mut rt);
    press(&mut rt, "Continue");
    press(&mut rt, "Allow notifications");
    wait_for(&mut rt, "its write", |_| !engine.notified.lock().unwrap().is_empty());
    assert_eq!(engine.notified.lock().unwrap().clone(), [(session(3), true)]);
}

/// "Skip setup" asks nothing of the engine: the last page claims no value for a switch nothing set.
#[test]
fn skip_setup_sets_nothing_and_the_last_page_claims_nothing() {
    let set = set();
    set.engine.set(Start::Running, EngineExplorer::Added);
    let (mut rt, engine, fixture, _first_run, platform) = window(set);
    fixture.set_gate_for_scene(ShellGate::FirstRun);
    wait_for(&mut rt, "the first run", |rt| rt.state.setup.is_some());
    rt.update(|s, _| {
        if let Some(ui) = s.setup.as_mut() {
            ui.skip();
        }
    });
    wait_for(&mut rt, "the last page", |rt| says(rt, "You're set up"));
    assert!(says(&rt, "Notifications, Not set here") && says(&rt, "Open at login, Not set here"));
    assert!(!says(&rt, "Notifications, On") && !says(&rt, "Open at login, On"));
    assert!(engine.notified.lock().unwrap().is_empty() && platform.autostart_writes().is_empty());
}

/// A gate that was moved by a refresh outside the poll (the b's menu refreshes the session too) still gets its
/// first run, and loses it the same way.
#[test]
fn a_gate_change_seen_outside_the_poll_still_makes_and_drops_the_first_run() {
    let set = set();
    set.engine.set(Start::Running, EngineExplorer::Added);
    let (mut rt, _engine, fixture, ..) = window(set);
    fixture.set_gate_for_scene(ShellGate::FirstRun);
    rt.update(|s, _| {
        s.shell.refresh();
        assert_eq!(
            s.shell.app.gate(),
            ShellGate::FirstRun,
            "the refresh moved the gate, not the poll"
        );
    });
    wait_for(&mut rt, "the first run", |rt| rt.state.setup.is_some());
    fixture.set_gate_for_scene(ShellGate::Ready);
    rt.update(|s, _| {
        s.shell.refresh();
    });
    wait_for(&mut rt, "dropped", |rt| rt.state.setup.is_none());
}

/// A first run that ended (the gate went Ready) and is first seen by a refresh outside the poll still asks for
/// one sync start, as `on_gate_change` does.
#[test]
fn a_first_run_that_ended_outside_the_poll_still_asks_for_a_start() {
    let set = set();
    set.engine.set(Start::Running, EngineExplorer::Added);
    let (mut rt, _engine, fixture, ..) = window(set);
    let starts = |fixture: &FixtureEngine| {
        fixture
            .calls()
            .iter()
            .filter(|call| **call == crate::shell::actions::FixtureCall::StartSync)
            .count()
    };
    fixture.set_gate_for_scene(ShellGate::FirstRun);
    rt.update(|s, _| {
        s.shell.refresh();
    });
    wait_for(&mut rt, "the first run", |rt| rt.state.setup.is_some());
    let before = starts(&fixture);
    fixture.set_gate_for_scene(ShellGate::Ready);
    rt.update(|s, _| {
        s.shell.refresh();
    });
    wait_for(&mut rt, "dropped", |rt| rt.state.setup.is_none());
    assert_eq!(
        starts(&fixture),
        before + 1,
        "one start was asked for when the first run ended"
    );
}

/// A window nobody wired a first run into (every scene, every other test) is never moved by this: the frame says
/// "not built yet" for the gate, and no source is made.
#[test]
fn a_window_without_the_wiring_never_starts_a_first_run() {
    let fixture = Arc::new(FixtureEngine::ready());
    let state = crate::shell::state::init_with(Shell::fixture_on(fixture.clone(), Arc::new(FakePlatform::default())));
    let mut rt = Runtime::new(
        state,
        crate::view,
        (960.0, 640.0),
        Env {
            system_dark: false,
            scale: 2.0,
        },
    );
    rt.poll = Some(|s: &mut State, seconds: f64, tx: &mut Tx| host::poll(s, seconds, tx));
    fixture.set_gate_for_scene(ShellGate::FirstRun);
    rt.frame(0.0);
    rt.frame(0.1);
    assert!(rt.state.setup.is_none());
}
