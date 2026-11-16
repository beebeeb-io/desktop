//! Review of #464, P3-1: what comes from outside the window while the away lock's wake cover is up (the engine has not
//! said whether the wake locks) waits for the cover's end: a folder picked in a picker left open across the sleep, a
//! link, the b's Settings…. Once the cover is gone it goes on, checked again; a lock, a sign-out or another account
//! drops it. The cover comes as in the running app: the tray's resume of a wake, whose look the test holds and then
//! answers. Every name is invented.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use rune::{Env, Runtime};

use super::*;
use crate::frames::backups::BackupsUi;
use crate::frames::backups::source::{BackupsRoot, BackupsSource, Device, FixtureSource, Folder, NotBackedUp, Status};
use crate::frames::global::sync_off::SyncOff;
use crate::shell::actions::{ActionError, Away, AwayAnswer, AwayJob, FixtureEngine, HostActions, ShellActions};
use crate::shell::app::{GateState, Screen, Section, Session, SettingsPane};
use crate::shell::host::{self, Shell, Wiring, poll};
use crate::shell::platform::TrayEvent;
use crate::shell::platform::fake::FakePlatform;
use crate::shell::tray::{MenuAction, TrayInputs};
use crate::shell::{deeplink, registry, state};

const ENV: Env = Env {
    system_dark: false,
    scale: 2.0,
};

/// The folder the person picks (invented).
const FOLDER: &str = "C:\\Users\\anna\\Projects";

/// The fixture engine's actions with an away look: each look waits while `hold` is set (at most ten seconds), then
/// answers. With `lock` set the engine locks first, as core's `lock_after_away` does, and the answer says so. The
/// engine's lock is open while its gate is Ready.
struct Waking {
    inner: Arc<dyn ShellActions>,
    engine: Arc<FixtureEngine>,
    hold: Arc<AtomicBool>,
    lock: Arc<AtomicBool>,
}

impl ShellActions for Waking {
    fn session(&self) -> Session {
        self.inner.session()
    }
    fn start_sync(&self) -> Result<(), ActionError> {
        self.inner.start_sync()
    }
    fn stop_for_quit(&self) {
        self.inner.stop_for_quit()
    }
    fn resume_after_cancelled_quit(&self) {
        self.inner.resume_after_cancelled_quit()
    }
    fn tray_inputs(&self) -> TrayInputs {
        self.inner.tray_inputs()
    }
    fn pause_sync(&self) -> Result<(), ActionError> {
        self.inner.pause_sync()
    }
    fn resume_sync(&self) -> Result<(), ActionError> {
        self.inner.resume_sync()
    }
    fn open_in_file_explorer(&self) -> Result<(), ActionError> {
        self.inner.open_in_file_explorer()
    }
    fn autostart_enabled(&self) -> bool {
        self.inner.autostart_enabled()
    }
    fn set_autostart(&self, enabled: bool) -> Result<(), ActionError> {
        self.inner.set_autostart(enabled)
    }
    fn set_gate_for_scene(&self, gate: GateState) {
        self.inner.set_gate_for_scene(gate)
    }
    fn sync_refusal(&self) -> Option<SyncOff> {
        self.inner.sync_refusal()
    }
    fn away_job(&self, _seen: Away) -> Option<AwayJob> {
        let (engine, hold, lock) = (self.engine.clone(), self.hold.clone(), self.lock.clone());
        Some(Box::new(move || {
            let began = Instant::now();
            while hold.load(Ordering::SeqCst) && began.elapsed() < Duration::from_secs(10) {
                std::thread::sleep(Duration::from_millis(2));
            }
            let locked = lock.load(Ordering::SeqCst);
            if locked {
                engine.set_gate(GateState::Locked);
            }
            AwayAnswer {
                locked,
                check_again_in: (!locked).then_some(Duration::from_secs(60)),
            }
        }))
    }
    fn screen_open(&self) -> bool {
        self.inner.session().gate == GateState::Ready
    }
}

/// Backups over the board's fixture, counting the folders asked of the engine.
struct Recording {
    inner: FixtureSource,
    adds: Arc<Mutex<Vec<String>>>,
}

impl BackupsSource for Recording {
    fn root(&self) -> BackupsRoot {
        self.inner.root()
    }
    fn status(&self) -> Status {
        self.inner.status()
    }
    fn folders(&self) -> Vec<Folder> {
        self.inner.folders()
    }
    fn devices(&self) -> Vec<Device> {
        self.inner.devices()
    }
    fn not_backed_up(&self) -> Vec<NotBackedUp> {
        self.inner.not_backed_up()
    }
    fn mail_backup_on(&self) -> bool {
        self.inner.mail_backup_on()
    }
    fn back_up_now(&mut self) {
        self.inner.back_up_now()
    }
    fn set_folder(&mut self, id: &str, on: bool) {
        self.inner.set_folder(id, on)
    }
    fn add_folder(&mut self, path: &str) -> bool {
        self.adds.lock().unwrap().push(path.to_owned());
        true
    }
    fn remove_folder(&mut self, id: &str, move_copy_to_trash: bool) {
        self.inner.remove_folder(id, move_copy_to_trash)
    }
    fn retry(&mut self, id: &str) {
        self.inner.retry(id)
    }
    fn retry_all(&mut self) {
        self.inner.retry_all()
    }
    fn delete_device_backup(&mut self, id: &str, code: &str) -> bool {
        self.inner.delete_device_backup(id, code)
    }
}

/// The window as the running app holds it: signed in and unlocked, app lock on (the engine's first look answered that
/// an idle time could lock), the shell's poll acting on its queues, Backups counting what it is asked to add.
struct Window {
    rt: Runtime<State>,
    engine: Arc<FixtureEngine>,
    hold: Arc<AtomicBool>,
    lock: Arc<AtomicBool>,
    adds: Arc<Mutex<Vec<String>>>,
}

impl Window {
    fn open() -> Window {
        let engine = Arc::new(FixtureEngine::ready());
        let platform: Arc<dyn Platform> = Arc::new(FakePlatform::default());
        let (hold, lock) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let actions = Arc::new(Waking {
            inner: Arc::new(HostActions::new(engine.clone(), platform.clone())),
            engine: engine.clone(),
            hold: hold.clone(),
            lock: lock.clone(),
        });
        let mut state = state::init_with(Shell::new(actions, platform, Wiring::default()));
        let adds = Arc::new(Mutex::new(Vec::new()));
        state.backups = BackupsUi::new(Some(Box::new(Recording {
            inner: FixtureSource::board(),
            adds: adds.clone(),
        })));
        let mut rt = Runtime::new(state, registry::view, (1280.0, 800.0), ENV);
        rt.poll = Some(poll);
        poll_drains_for_this_test();
        rt.frame(0.0);
        let mut window = Window {
            rt,
            engine,
            hold,
            lock,
            adds,
        };
        window.frames_until("the away lock's first look answered", |rt| {
            rt.state.shell.away.answered >= 1 && session_gate::open(&rt.state)
        });
        window
    }

    /// Frames (each runs the shell's poll) until `done`; fails after ten seconds, saying `what`.
    fn frames_until(&mut self, what: &str, mut done: impl FnMut(&Runtime<State>) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(&self.rt) {
            assert!(Instant::now() < deadline, "{what}");
            self.rt.frame(0.016);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// A few more polls, for something that must NOT happen.
    fn a_few_frames(&mut self) {
        for _ in 0..12 {
            self.rt.frame(0.016);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// The PC woke from three minutes' sleep: the tray's resume, said as `on_tray` says it. Its look is held, so the
    /// window stays covered until the test answers it.
    fn wake(&mut self) {
        self.hold.store(true, Ordering::SeqCst);
        self.rt.update(|s, tx| {
            host::on_tray(
                s,
                TrayEvent::Resumed {
                    away: Duration::from_secs(3 * 60),
                },
                tx,
            )
        });
        assert!(registry::covered(&self.rt.state), "the resume covers the window");
    }

    /// The wake's look answers that it locked nothing: the cover goes.
    fn nothing_locked(&mut self) {
        self.hold.store(false, Ordering::SeqCst);
        self.frames_until("the cover lifts", |rt| !registry::covered(&rt.state));
    }

    /// The wake's look answers that it locked the app: the lock screen.
    fn locked(&mut self) {
        self.lock.store(true, Ordering::SeqCst);
        self.hold.store(false, Ordering::SeqCst);
        self.frames_until("the window shows the lock", |rt| {
            rt.state.shell.app.gate() == GateState::Locked
        });
        self.a_few_frames();
    }

    /// The person unlocks again (the same account): the session is open, nothing covers.
    fn unlocked(&mut self) {
        self.lock.store(false, Ordering::SeqCst);
        self.engine.set_gate(GateState::Ready);
        self.frames_until("the window is open again", |rt| {
            session_gate::open(&rt.state) && !rt.state.shell.away.covers()
        });
        self.a_few_frames();
    }

    fn adds(&self) -> Vec<String> {
        self.adds.lock().unwrap().clone()
    }

    /// Opens a folder picker for Backups (pressed while the session was open), which stays open until the test closes it
    /// on [`FOLDER`].
    fn pick(&mut self) -> mpsc::Sender<()> {
        let (close, closed) = mpsc::channel::<()>();
        let picker = move || {
            let _ = closed.recv();
            Ok(Some(PathBuf::from(FOLDER)))
        };
        assert!(pick_folder_with(&self.rt.state, FolderFor::Backups, picker));
        close
    }

    /// The person picks the folder under the cover: frames until the shell's poll took the picker's answer.
    fn picked_under_the_cover(&mut self, close: &mpsc::Sender<()>) {
        close.send(()).unwrap();
        self.frames_until("the picker's answer is taken", |_| !a_folder_picker_is_open());
        self.a_few_frames();
        assert!(registry::covered(&self.rt.state), "still covered");
        assert!(self.adds().is_empty(), "nothing is added to Backups under the cover");
        assert!(a_picked_folder_waits(), "the folder waits for the cover's end");
    }

    /// The window shows `section`.
    fn shows(&self, section: Section) -> bool {
        matches!(self.rt.state.shell.app.screen(), Screen::Drive(route) if route.section == section)
    }
}

/// P3-1: a folder picked in a picker left open across a sleep, while the window is under the wake cover, waits; once
/// the wake's look said it locked nothing, the folder goes to Backups, once.
/// Mutants: in `take_picked_folder`, hand the folder over whatever the cover (drop the `under_the_wake_cover` branch)
/// -> red at "nothing is added to Backups under the cover"; in `host::poll_screen`, delete the
/// `intents::after_the_cover` call -> red at "added once the cover is gone".
#[test]
fn wake_cover_holds_a_picked_folder_until_it_lifts() {
    let mut w = Window::open();
    let close = w.pick();
    w.wake();
    w.picked_under_the_cover(&close);
    w.nothing_locked();
    assert_eq!(w.adds(), vec![FOLDER.to_owned()], "added once the cover is gone");
    assert!(!a_picked_folder_waits());
    w.a_few_frames();
    assert_eq!(w.adds().len(), 1, "once");
}

/// P3-1: the wake locked the app. The folder picked under the cover goes with the lock, and nothing is added after the
/// unlock either.
/// Mutant: in `after_the_cover`'s release of the folder, drop the session check (hand it over once the window is not
/// covered) -> red at "the lock drops it" (the lock screen is not the cover, and the folder reaches Backups).
#[test]
fn wake_cover_drops_a_held_folder_on_a_lock() {
    let mut w = Window::open();
    let close = w.pick();
    w.wake();
    w.picked_under_the_cover(&close);
    w.locked();
    assert!(
        w.adds().is_empty() && !a_picked_folder_waits(),
        "the lock drops it: {:?}",
        w.adds()
    );
    w.unlocked();
    assert!(w.adds().is_empty(), "nothing is added after the unlock: {:?}", w.adds());
}

/// P3-1: the account signs out under the cover (and the same account signs in again before the cover lifts): the
/// folder is dropped at the sign-out.
/// Mutant: drop the session check in the release (as above) -> red at "the sign-out drops it".
#[test]
fn wake_cover_drops_a_held_folder_on_a_sign_out() {
    let mut w = Window::open();
    let close = w.pick();
    w.wake();
    w.picked_under_the_cover(&close);
    w.engine.set_gate(GateState::SignedOut);
    w.frames_until("the window sees the sign-out", |rt| {
        rt.state.shell.app.gate() == GateState::SignedOut
    });
    w.a_few_frames();
    assert!(
        w.adds().is_empty() && !a_picked_folder_waits(),
        "the sign-out drops it: {:?}",
        w.adds()
    );
    w.unlocked();
    assert!(
        w.adds().is_empty(),
        "nothing is added for the new session: {:?}",
        w.adds()
    );
}

/// P3-1: another account is signed in while the cover is still up: the folder picked for the first is dropped, never
/// added to the other's backups when the cover lifts.
/// Mutant: drop the account check in the release (compare the session only) -> red at "nothing is added to the other
/// account's backups".
#[test]
fn wake_cover_drops_a_held_folder_for_another_account() {
    let mut w = Window::open();
    let close = w.pick();
    w.wake();
    w.picked_under_the_cover(&close);
    w.engine.set_account("another-account");
    w.frames_until("the window shows the other account", |rt| {
        rt.state.shell.app.account() == Some("another-account")
    });
    w.a_few_frames();
    assert!(registry::covered(&w.rt.state), "the cover is still up");
    w.nothing_locked();
    w.a_few_frames();
    assert!(
        w.adds().is_empty(),
        "nothing is added to the other account's backups: {:?}",
        w.adds()
    );
    assert!(!a_picked_folder_waits());
}

/// The sweep: a `beebeeb://` link that comes under the cover navigates nothing behind it, and goes on once the cover
/// is gone.
/// Mutant: in `host::request`, delete the `under_the_wake_cover` block -> red at "nothing is navigated behind the
/// cover".
#[test]
fn wake_cover_holds_a_link_until_it_lifts() {
    let link = deeplink::link("open/photos");
    let mut w = Window::open();
    assert!(w.shows(Section::Files));
    w.wake();
    w.rt.update(|s, tx| host::on_open_url(s, &link, tx));
    w.a_few_frames();
    assert!(w.shows(Section::Files), "nothing is navigated behind the cover");
    w.nothing_locked();
    assert!(w.shows(Section::Photos), "the link goes on once the cover is gone");
}

/// The lead's ruling (2026-11-16 05:05): a link only opens a view, so a link that came under the cover follows the route
/// machine's rule when the wake locks: kept across the lock and opened after the same account unlocks, checked again;
/// another account or a sign-out drops it. A later link, which came while locked, wins over it.
/// Mutants: in `release_the_ask`, drop the `if link && same_account …` wait (a link goes with the lock like any ask)
/// -> red at "the same account unlocks: the link opens"; drop `same_account` from that wait -> red at "another account:
/// dropped"; in `host::request_as`, delete `intents::a_later_ask_came()` -> red at "the later link wins".
#[test]
fn wake_cover_keeps_a_link_across_a_lock_for_the_same_account() {
    let link = deeplink::link("open/photos");
    let mut w = Window::open();
    w.wake();
    w.rt.update(|s, tx| host::on_open_url(s, &link, tx));
    w.locked();
    assert!(
        !w.shows(Section::Photos) && an_ask_waits(),
        "kept while locked, nothing opened"
    );
    w.unlocked();
    assert!(w.shows(Section::Photos), "the same account unlocks: the link opens");
    assert!(!an_ask_waits());

    // Another account is signed in at the lock: dropped.
    let mut w = Window::open();
    w.wake();
    w.rt.update(|s, tx| host::on_open_url(s, &link, tx));
    w.locked();
    w.engine.set_account("another-account");
    w.unlocked();
    assert_eq!(w.rt.state.shell.app.account(), Some("another-account"));
    assert!(
        !w.shows(Section::Photos) && !an_ask_waits(),
        "another account: dropped ({:?})",
        w.rt.state.shell.app.screen()
    );

    // A sign-out while locked: dropped, also when the same account signs in again.
    let mut w = Window::open();
    w.wake();
    w.rt.update(|s, tx| host::on_open_url(s, &link, tx));
    w.locked();
    w.engine.set_gate(GateState::SignedOut);
    w.frames_until("the window sees the sign-out", |rt| {
        rt.state.shell.app.gate() == GateState::SignedOut
    });
    assert!(!an_ask_waits(), "the sign-out drops it");
    w.unlocked();
    assert!(!w.shows(Section::Photos), "nothing opens at the next sign-in");

    // A later link, which came while locked (the route machine keeps it): it wins over the one held under the cover.
    let mut w = Window::open();
    w.wake();
    w.rt.update(|s, tx| host::on_open_url(s, &link, tx));
    w.locked();
    w.rt.update(|s, tx| host::on_open_url(s, &deeplink::link("open/trash"), tx));
    w.unlocked();
    assert!(
        w.shows(Section::Trash),
        "the later link wins: {:?}",
        w.rt.state.shell.app.screen()
    );
}

/// The sweep: the b's Settings… under the cover opens nothing behind it, and opens Settings once the cover is gone. It
/// opens a pane, so a cover that became the lock drops it (only a link waits for the unlock).
/// Mutants: in `host::request`, delete the `under_the_wake_cover` block -> red at "Settings does not open behind the
/// cover"; in `release_the_ask`, keep every ask across a lock (drop `link &&` from the wait) -> red at "the lock drops
/// it".
#[test]
fn wake_cover_holds_the_trays_settings_until_it_lifts() {
    let mut w = Window::open();
    w.wake();
    w.rt.update(|s, tx| host::on_menu(s, MenuAction::Settings, tx));
    w.a_few_frames();
    assert!(
        w.shows(Section::Files),
        "Settings does not open behind the cover: {:?}",
        w.rt.state.shell.app.screen()
    );
    w.nothing_locked();
    assert_eq!(
        w.rt.state.shell.app.screen(),
        Screen::Settings(SettingsPane::General),
        "Settings, once the cover is gone"
    );

    let mut w = Window::open();
    w.wake();
    w.rt.update(|s, tx| host::on_menu(s, MenuAction::Settings, tx));
    w.locked();
    assert!(!an_ask_waits(), "the lock drops it");
    w.unlocked();
    assert!(
        w.shows(Section::Files),
        "nothing opens at the unlock: {:?}",
        w.rt.state.shell.app.screen()
    );
}

/// The sweep: the flyout's Review… under the cover (its `carry_out` asks the main window through `host::request`) opens
/// no conflict dialog behind it, and opens it once the cover is gone.
/// Mutant: in `host::request`, delete the `under_the_wake_cover` block -> red at "no dialog opens behind the cover".
#[test]
fn wake_cover_holds_the_flyouts_review_until_it_lifts() {
    use crate::shell::app::{Dialog, Intent as Ask};
    let mut w = Window::open();
    w.wake();
    w.rt.update(|s, _| {
        host::request(s, Ask::ReviewConflict { file: None });
    });
    w.a_few_frames();
    assert_eq!(w.rt.state.shell.app.dialog(), None, "no dialog opens behind the cover");
    w.nothing_locked();
    assert_eq!(
        w.rt.state.shell.app.dialog(),
        Some(&Dialog::Conflict { file: None }),
        "the conflict dialog, once the cover is gone"
    );
}
