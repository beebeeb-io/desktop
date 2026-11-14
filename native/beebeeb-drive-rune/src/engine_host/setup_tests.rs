//! The first run on the real engine (`EngineSetupSource`): against a double of the engine handle (what each step
//! reads and sends, every refusal, another session, another account), in the running app's window (the gate makes
//! and drops the first run), and the engine's own first-run calls once against a real handle on the fake API.
//! Every name is invented.

use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::time::{Duration, Instant};

use beebeeb_engine::app::SyncStartRefused;
use rune::{Env, Runtime, Tx};

use super::*;
use crate::State;
use crate::shell::actions::{EngineActions, FixtureEngine};
use crate::shell::app::GateState as ShellGate;
use crate::shell::host::{self, Shell};
use crate::shell::platform::fake::FakePlatform;

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
    stops: AtomicUsize,
    wakes: AtomicUsize,
    /// A write waits here until the test lets it go (a change that is on its way).
    held: Mutex<()>,
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
        stops: AtomicUsize::new(0),
        wakes: AtomicUsize::new(0),
        held: Mutex::new(()),
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
        let _held = self.held.lock().unwrap();
        if session != self.stamp().0 {
            return Err(SetupRefused::SessionMoved);
        }
        self.marked.lock().unwrap().push(session);
        *self.writes.lock().unwrap()
    }
    fn set_notifications_for(&self, session: SessionGeneration, on: bool) -> Result<(), SetupRefused> {
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
    fn stop_sync(&self) {
        self.stops.fetch_add(1, Ordering::SeqCst);
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
    let source = EngineSetupSource::new(engine.clone(), hooks(&asked, platform.clone(), first_run.clone()));
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
fn only_the_steps_something_answers_for_are_offered() {
    let set = set();
    let offered: Vec<Step> = Step::STEPS
        .into_iter()
        .filter(|step| set.source.offers(*step))
        .collect();
    assert_eq!(offered, [Step::Explorer, Step::Notifications, Step::Login]);
    assert!(!set.source.opens_settings() && set.source.answers_later());
    // On an OS without File Explorer the step is not there either.
    let mut other = hooks(&set.asked, set.platform.clone(), set.first_run.clone());
    other.explorer_step = false;
    let source = EngineSetupSource::new(set.engine.clone(), other);
    assert!(!source.offers(Step::Explorer) && source.offers(Step::Login));
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
fn file_explorer_is_drawn_from_the_engines_view_and_a_refused_start_is_not_adding() {
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
fn try_again_asks_for_one_start_and_restarts_a_sync_that_runs() {
    let mut set = set();
    // Sync did not start: one start is asked for, and nothing is stopped.
    set.engine.set(
        Start::Refused(SyncStartRefused::FolderUnreadable),
        EngineExplorer::Unknown,
    );
    set.source.retry_explorer();
    settle(&mut set, 1);
    assert_eq!(
        (
            set.asked.starts.load(Ordering::SeqCst),
            set.engine.stops.load(Ordering::SeqCst)
        ),
        (1, 0)
    );
    // Sync runs and Windows refused the entry: only a start asks Windows, so it is stopped and started again.
    set.engine.set(Start::Running, EngineExplorer::Failed { code: None });
    set.source.retry_explorer();
    settle(&mut set, 2);
    assert_eq!(
        (
            set.asked.starts.load(Ordering::SeqCst),
            set.engine.stops.load(Ordering::SeqCst)
        ),
        (2, 1)
    );
    // A start that could not be asked for changes nothing here: the view still says what the engine says.
    set.asked.start_refused.store(true, Ordering::SeqCst);
    set.source.retry_explorer();
    settle(&mut set, 3);
    assert_eq!(set.source.explorer(), Explorer::Failed { code: String::new() });
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
    set.source.show_in_explorer();
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
    set.source.show_in_explorer();
    std::thread::sleep(Duration::from_millis(50));
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
        set.source.set_notifications(true);
        set.source.set_open_at_login(true);
        set.source.finish();
        set.source.retry_explorer();
        std::thread::sleep(Duration::from_millis(50));
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

        // The session moves while the change is on its way: the answer is dropped.
        let mut set = set_on(fake(), FakePlatform::default());
        let held = set.engine.held.lock().unwrap();
        set.source.set_notifications(true);
        set.source.finish();
        *set.engine.stamp.lock().unwrap() = moved_to.clone();
        drop(held);
        settle(&mut set, 2);
        assert!(set.source.take_outcomes().is_empty(), "{moved_to:?}");
        assert!(set.first_run.due(), "{moved_to:?}");
        assert!(set.engine.marked.lock().unwrap().is_empty(), "{moved_to:?}");
    }
    // Locked (the gate is not Ready): the same.
    let mut set = set();
    set.engine.view.lock().unwrap().gate = GateState::Locked;
    set.source.set_notifications(true);
    set.source.finish();
    std::thread::sleep(Duration::from_millis(50));
    assert!(set.engine.notified.lock().unwrap().is_empty() && set.engine.marked.lock().unwrap().is_empty());
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

/// The running app: a gate that shows the first run makes its steps for that session; only the steps something
/// answers for are there; each switch is drawn once it was answered; "Open my Beebeeb" ends it.
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
    // The list at the left: the three steps, none of the four nothing answers for.
    for step in ["Beebeeb in File Explorer", "Notifications", "Open at login"] {
        assert!(says(&rt, step), "{step}");
    }
    for step in [
        "Add a passkey",
        "Lock Beebeeb",
        "Back up your folders",
        "Thumbnails and previews",
        "2,431",
        "20 GB",
    ] {
        assert!(!says(&rt, step), "{step}");
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
        assert!(!says(&rt, row), "no row for a step this first run did not have: {row}");
    }
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
    press(&mut rt, "Continue");
    assert!(says(&rt, "Allow notifications"), "on step two");
    // Locked: nothing of the first run stays.
    fixture.set_gate_for_scene(ShellGate::Locked);
    wait_for(&mut rt, "dropped", |rt| rt.state.setup.is_none());
    // Unlocked again (another session): the first run starts over, for that session.
    *engine.stamp.lock().unwrap() = (session(2), Some(FIRST.into()));
    fixture.set_gate_for_scene(ShellGate::FirstRun);
    wait_for(&mut rt, "made again", |rt| rt.state.setup.is_some());
    assert_eq!(rt.state.setup.as_ref().map(|ui| ui.step), Some(Step::Explorer));
    press(&mut rt, "Continue");
    // Another account under the same gate: from the first step again, and its presses are its own session's.
    *engine.stamp.lock().unwrap() = (session(3), Some(SECOND.into()));
    fixture.set_account(SECOND);
    wait_for(&mut rt, "the second account's", |rt| {
        rt.state.setup.as_ref().is_some_and(|ui| ui.step == Step::Explorer)
    });
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

// ---------------------------------------------------------------------------
// The engine's own calls, once, on a real handle against the fake API
// ---------------------------------------------------------------------------

mod glue {
    use beebeeb_engine::app::EngineConfig;
    use beebeeb_engine::config::{CONFIG_DIR_OVERRIDE, DesktopConfig};
    use beebeeb_engine::host::ChannelHost;
    use beebeeb_engine::identity::TEST;
    use beebeeb_engine::signin::memory_store::MemoryCredentials;
    use beebeeb_engine::test_api::FakeApi;
    use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
    use zeroize::Zeroizing;

    use super::*;

    const PASSWORD: &str = "an invented password for setup";

    type Messages = std::sync::mpsc::Receiver<beebeeb_engine::host::HostMessage>;

    /// A handle on a fake API of its own, signed in and unlocked.
    fn signed_in() -> (Arc<Engine<MemoryCredentials>>, FakeApi, String) {
        let (engine, api, phrase, _messages) = signed_in_with_messages();
        (engine, api, phrase)
    }

    fn signed_in_with_messages() -> (Arc<Engine<MemoryCredentials>>, FakeApi, String, Messages) {
        let (account, phrase) = FakeAccount::register(FIRST, PASSWORD, TEST_PHRASE);
        let account = Arc::new(account);
        let api = FakeApi::start(account_handler(account.clone()));
        let (host, messages) = ChannelHost::new(|| {});
        let config = EngineConfig {
            account_id: "setup-test-slot".into(),
            state_dir: crate::engine_host::tests::process_state_dir(),
            api_base: api.base.clone(),
        };
        let engine = Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("opens"));
        engine.restore();
        engine
            .sign_in_password(FIRST, Zeroizing::new(PASSWORD.into()))
            .expect("the fake accepts the password");
        engine
            .unlock_with_phrase(Zeroizing::new(phrase.clone()))
            .expect("the phrase unlocks");
        (engine, api, phrase, messages)
    }

    /// The gate the shell reads from the live engine: the first-run steps for a signed-in, unlocked session of a
    /// PC that still has its first run to do, only once the app's start wired it, and the main window after.
    #[test]
    fn the_live_engines_gate_shows_the_first_run_only_while_it_is_due_and_wired() {
        use crate::engine_host::LiveEngine;
        use crate::engine_host::open::Opened;
        use crate::shell::actions::EngineActions;
        let (engine, _api, _phrase, messages) = signed_in_with_messages();
        let opened = Opened {
            engine: engine.clone(),
            identity: &TEST,
            gate: engine.gate(),
        };
        let live = Arc::new(LiveEngine::new(&opened, messages));
        assert_eq!(live.session().gate, ShellGate::Ready, "nobody wired a first run");
        in_own_settings(|| live.wire_first_run());
        assert_eq!(live.session().gate, ShellGate::FirstRun, "due, signed in and unlocked");
        // Locked: the lock screen, never the first run.
        engine.lock();
        assert_eq!(live.session().gate, ShellGate::Locked);
        // The first run ends (the adapter's flag, through the hooks the app hands it): the main window.
        let hooks = live.setup_hooks(Arc::new(FakePlatform::default()));
        hooks.first_run.end();
        assert_eq!(live.session().gate, ShellGate::Locked);
        assert!(!hooks.first_run.due());
        // A PC that had its first run: wiring it shows none.
        in_own_settings(|| {
            DesktopConfig::update_for(&TEST, |config| {
                config.known_folder_onboarding_seen = true;
                (true, ())
            })
            .expect("written");
            live.wire_first_run();
        });
        assert!(!hooks.first_run.due());
    }

    /// The settings file of this test alone (the process's is shared with every other test).
    fn in_own_settings<T>(run: impl FnOnce() -> T) -> T {
        let dir = tempfile::tempdir().expect("a folder for the settings file");
        CONFIG_DIR_OVERRIDE.sync_scope(dir.path().to_path_buf(), run)
    }

    #[test]
    fn the_first_run_flag_and_the_notifications_switch_are_written_for_the_session_of_the_press_only() {
        let (engine, _api, phrase) = signed_in();
        in_own_settings(|| {
            let file = || DesktopConfig::load_for(&TEST).expect("the settings file reads");
            let turn = engine.session_turn();
            // A PC that never had a first run: it is due, and nothing was written by asking.
            assert_eq!(Engine::first_run_due(&*engine), Ok(true));
            assert!(
                file().notify_allow,
                "precondition: notifications are allowed by default"
            );
            // The switch, both ways, for this session.
            assert_eq!(engine.set_notifications_allowed_for(turn, false), Ok(()));
            assert!(!file().notify_allow);
            assert_eq!(engine.set_notifications_allowed_for(turn, true), Ok(()));
            assert!(file().notify_allow);
            assert_eq!(
                Engine::first_run_due(&*engine),
                Ok(true),
                "a switch does not end the first run"
            );

            // Another session (a lock and an unlock): a press stamped with the old one writes nothing.
            engine.lock();
            assert_eq!(
                engine.set_notifications_allowed_for(turn, false),
                Err(SetupRefused::SessionMoved)
            );
            assert_eq!(engine.mark_first_run_done_for(turn), Err(SetupRefused::SessionMoved));
            engine
                .unlock_with_phrase(Zeroizing::new(phrase.clone()))
                .expect("the phrase unlocks");
            assert_eq!(
                engine.set_notifications_allowed_for(turn, false),
                Err(SetupRefused::SessionMoved)
            );
            assert_eq!(engine.mark_first_run_done_for(turn), Err(SetupRefused::SessionMoved));
            assert!(file().notify_allow, "nothing was written");
            assert_eq!(Engine::first_run_due(&*engine), Ok(true), "nothing was written");

            // The session of now: the first run is done, and stays done (also for the adapter's own glue).
            let now = engine.session_turn();
            assert_eq!(SetupEngine::mark_done_for(&*engine, now), Ok(()));
            assert_eq!(SetupEngine::first_run_due(&*engine), Ok(false));
            assert!(
                file().known_folder_onboarding_seen,
                "the settings file's own show-once flag"
            );
            assert_eq!(engine.mark_first_run_done_for(now), Ok(()), "twice is the same");
            assert_eq!(SetupEngine::set_notifications_for(&*engine, now, false), Ok(()));
            assert!(!file().notify_allow);
            // The glue's stamp is the handle's session and account.
            assert_eq!(SetupEngine::stamp(&*engine), (now, Some(FIRST.to_string())));
            assert_eq!(SetupEngine::view(&*engine).gate, GateState::Ready);
        });
        // A PC that went through the first run in the app before this one (its flag is set): not due.
        in_own_settings(|| {
            DesktopConfig::update_for(&TEST, |config| {
                config.known_folder_onboarding_seen = true;
                (true, ())
            })
            .expect("written");
            assert_eq!(Engine::first_run_due(&*engine), Ok(false));
        });
    }
}
