//! The running shell: the part of the root state that owns the route machine, the b and the
//! two windows' handles, and the hooks Rune calls (keys, close, quit, reopen, links).
//!
//! Frames change the route with [`dispatch`]; anything outside the main window (the tray, the
//! flyout, a link) asks with [`request`]. Nothing else moves the window.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use rune::{AppHandle, Key, KeyEvent, Tx, WindowHandle};

use super::actions::{FixtureEngine, HostActions, ShellActions};
use super::app::{AppMachine, Dialog, Event, Intent, Outcome, Route, Section, SettingsPane};
use super::control::Request;
use super::flyout::{self, Toggle};
use super::flyout_window::FlyoutState;
use super::instance::{self, Forwarded};
use super::platform::{self, Platform, PlatformError, TrayEvent, TrayHandler, TrayIcon};
use super::surfaces::WindowKind;
use super::tray::{EngineState, LeftClick, MenuAction, TrayInputs, TrayModel, tray_model};
use super::window_state::{self, Observed, SavedWindows};
use super::{registry, surfaces};
use crate::State;
use crate::frames::global::sync_off::SyncOff;
use crate::ui::sidebar::SidebarSection;

/// One line for the developer's console. Never a link, a path of the person's, or a secret.
pub(super) fn log(message: impl AsRef<str>) {
    eprintln!("beebeeb shell: {}", message.as_ref());
}

fn unpoisoned<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What the shell needs from outside to run for real. Tests and headless runs leave it empty.
#[derive(Default)]
pub struct Wiring {
    pub main: Option<AppHandle<State>>,
    pub flyout: Option<AppHandle<FlyoutState>>,
    /// The main window's native handle, filled by `App::on_window`.
    pub window: Arc<Mutex<Option<WindowHandle>>>,
    pub flyout_hidden_at: Arc<Mutex<Option<Instant>>>,
    pub saved: SavedWindows,
    /// Where window places are saved; None: nowhere (tooling runs, tests).
    pub state_file: Option<PathBuf>,
}

pub struct Shell {
    /// Where the main window is. Read it freely; change it with [`dispatch`] or [`request`].
    pub app: AppMachine,
    pub actions: Arc<dyn ShellActions>,
    platform: Arc<dyn Platform>,
    tray: Option<Box<dyn TrayIcon>>,
    tray_model: TrayModel,
    tray_inputs: TrayInputs,
    wiring: Wiring,
    /// The sidebar's own selection as the shell last saw it (see [`poll`]).
    sidebar_seen: SidebarSection,
    /// Which kind the one window currently has the size of (`surfaces::kind_for`).
    window_kind: WindowKind,
    /// The quit hook stopped the engine; a cancelled Windows shutdown starts it again.
    stopped_for_quit: bool,
    /// What the platform was last **successfully** told about capture exclusion. `false` at
    /// startup: the platform has not been told anything until the first `poll` calls it.
    capture_excluded: bool,
    /// The gate and the account the last poll saw (`poll` ends the account lane's flows when
    /// the session ends under them).
    session_seen: Option<(super::app::GateState, Option<String>)>,
    /// The last poll saw a screen with a secret (`poll_capture` drops exclusion one poll late).
    secret_seen: bool,
    /// Consecutive failures of the capture-exclusion platform call. Reset on success or when
    /// the desired state changes. Capped at 3: after 3 failures the call is not retried until
    /// the desired state changes (a navigation away and back resets it).
    capture_exclusion_retries: u8,
    /// The b's inputs changed since the flyout last read them. A hidden flyout is not read
    /// ([`Shell::refresh`]); this says it reads them once it shows.
    flyout_behind: bool,
    /// How many reads the shell asked of the flyout ([`Shell::read_flyout`]), for the tests.
    #[cfg(test)]
    flyout_reads: u32,
    /// The refused start the refusal modal shows ([`open_sync_refusal`]).
    sync_off: Option<SyncOff>,
    /// The attempt the refusal modal was last opened for, or seen without a gate to open over: once per attempt.
    refusal_seen: Option<u64>,
    /// How often the refusal modal opened, for the tests.
    #[cfg(test)]
    sync_off_opened: u32,
    /// The engine finished a check since [`poll`] last looked (the b's engine state turned Idle): a list that showed
    /// an error or nothing is read again then ([`read_again_after_a_check`]).
    check_finished: bool,
}

impl Shell {
    pub fn new(actions: Arc<dyn ShellActions>, platform: Arc<dyn Platform>, wiring: Wiring) -> Shell {
        let app = AppMachine::for_session(actions.session());
        let window_kind = surfaces::kind_for(&app.screen());
        let tray_inputs = actions.tray_inputs();
        let tray_model = tray_model(&tray_inputs);
        Shell {
            app,
            actions,
            platform,
            tray: None,
            tray_model,
            tray_inputs,
            wiring,
            sidebar_seen: SidebarSection::Files,
            window_kind,
            stopped_for_quit: false,
            // The platform has not been told anything yet. The first poll that sees a secret
            // route (or any route, if a window handle exists) will apply it.
            capture_excluded: false,
            session_seen: None,
            secret_seen: false,
            capture_exclusion_retries: 0,
            flyout_behind: false,
            #[cfg(test)]
            flyout_reads: 0,
            sync_off: None,
            refusal_seen: None,
            #[cfg(test)]
            sync_off_opened: 0,
            check_finished: false,
        }
    }

    /// A shell on the fixture engine with nothing wired: for tests and headless renders.
    pub fn fixture() -> Shell {
        Shell::fixture_on(Arc::new(FixtureEngine::ready()), platform::current())
    }

    pub fn fixture_on(engine: Arc<FixtureEngine>, platform: Arc<dyn Platform>) -> Shell {
        Shell::new(
            Arc::new(HostActions::new(engine, platform.clone())),
            platform,
            Wiring::default(),
        )
    }

    pub fn tray_model(&self) -> &TrayModel {
        &self.tray_model
    }

    /// Whether the b is IN the notification area right now: the platform's own answer, not
    /// "a tray object exists". Without it, closing the window must quit: there would be no
    /// way back to the app, and no way to quit it.
    pub fn has_tray(&self) -> bool {
        self.tray.as_ref().is_some_and(|tray| tray.present())
    }

    /// Which kind of window the one window is right now (main, or first-run).
    pub fn window_kind(&self) -> WindowKind {
        self.window_kind
    }

    /// The refused start the refusal modal shows.
    pub fn sync_off(&self) -> Option<&SyncOff> {
        self.sync_off.as_ref()
    }

    /// How often the refusal modal opened.
    #[cfg(test)]
    pub(crate) fn sync_off_opened(&self) -> u32 {
        self.sync_off_opened
    }

    /// Asks the engine to start syncing; refused while another agent holds the profile.
    pub fn start_sync(&self) {
        if let Err(error) = self.actions.start_sync() {
            log(format!("sync was not started ({error:?})"));
        }
    }

    /// Reads the session and the engine again and brings the gate, the b and the flyout in
    /// line. The engine lane calls this (through `AppHandle::update`) on every status event.
    /// True when the window's screen changed.
    pub fn refresh(&mut self) -> bool {
        let changed = self.app.apply(Event::Session(self.actions.session())) == Outcome::Changed;
        let inputs = self.actions.tray_inputs();
        if inputs != self.tray_inputs {
            self.check_finished |= inputs.engine == EngineState::Idle && self.tray_inputs.engine != EngineState::Idle;
            // The flyout reads the inputs themselves (counts), not only what the b shows.
            self.tray_inputs = inputs;
            self.flyout_behind = true;
        }
        // A hidden flyout is not read: each read opens this PC's state database and may ask
        // Beebeeb for the storage. The click that opens it reads it first (`toggle_flyout`); a
        // change that came while it opened is read here once it shows.
        if self.flyout_behind && self.wiring.flyout.as_ref().is_some_and(AppHandle::is_visible) {
            self.read_flyout();
        }
        let model = tray_model(&self.tray_inputs);
        if model != self.tray_model {
            self.tray_model = model;
            if let Some(tray) = &self.tray {
                tray.apply(&self.tray_model);
            }
            // Signed out, a click on the b opens the window on sign-in: the flyout has
            // nothing to show for an account that is gone.
            if self.tray_model.left_click == LeftClick::OpenSignIn {
                self.hide_flyout();
            }
        }
        changed
    }

    /// Applies capture exclusion to the window. `desired`: whether the current route shows a
    /// secret. Called from the ONE choke point at the end of `poll`.
    ///
    /// `self.capture_excluded` tracks what the platform was last **successfully** told, not what
    /// the route wants. When `desired != capture_excluded`, the platform is called. On failure
    /// the flag stays at the old value and the call is retried on the next poll, up to 3 times.
    /// A change of `desired` resets the retry counter.
    ///
    /// Content always stays drawn: a failure to exclude is a privacy leak, but not drawing would
    /// deny the person access to their own data.
    pub fn apply_capture_exclusion(&mut self, desired: bool) {
        if desired == self.capture_excluded {
            // Already applied. Reset retries in case a previous desired state was failing.
            self.capture_exclusion_retries = 0;
            return;
        }
        if self.capture_exclusion_retries >= 3 {
            return; // Gave up. Resets when the user navigates away and back.
        }
        let window = unpoisoned(&self.wiring.window);
        let handle = window.as_ref();
        match self.platform.set_capture_exclusion(handle, desired) {
            Ok(()) => {
                self.capture_excluded = desired;
                self.capture_exclusion_retries = 0;
            }
            Err(error) => {
                self.capture_exclusion_retries += 1;
                if self.capture_exclusion_retries == 1 {
                    log(format!("capture exclusion could not be set ({error})"));
                }
            }
        }
    }

    /// Adds the b. Runs on the UI thread once the event loop is up, so the process is
    /// DPI-aware by the time the icon's size is read.
    pub fn start_tray(&mut self) {
        let Some(main) = self.wiring.main.clone() else { return };
        self.attach_tray(Arc::new(move |event: TrayEvent| {
            main.update(move |s: &mut State, tx| on_tray(s, event, tx))
        }));
    }

    /// Whether closing the main window ends the app: when there is no b in the notification
    /// area to come back through (and to quit from).
    pub fn close_quits(&self) -> bool {
        !self.has_tray()
    }

    fn attach_tray(&mut self, handler: TrayHandler) {
        match self.platform.create_tray(&self.tray_model, handler) {
            Ok(tray) => {
                let present = tray.present();
                // Kept also when the icon could not be added: it retries by itself (a timer,
                // and `TaskbarCreated` when Explorer comes back) and reports `Presence`.
                self.tray = Some(tray);
                if !present {
                    log("the b is not in the notification area yet; until it is, closing the window quits");
                    // A start at login has no window; without a b the app would be unreachable.
                    self.open_main();
                }
            }
            Err(error) => {
                match error {
                    PlatformError::NotBuilt(what) => log(format!("not built yet: {what}; closing the window quits")),
                    error => log(format!("the b could not be added ({error}); closing the window quits")),
                }
                // A start at login has no window; without a b the app would be unreachable.
                self.open_main();
            }
        }
    }

    fn open_main(&self) {
        if let Some(main) = &self.wiring.main {
            main.open();
        }
    }

    fn hide_flyout(&self) {
        if let Some(flyout) = &self.wiring.flyout {
            flyout.hide();
        }
    }

    /// Asks the flyout to read the engine and the tray inputs again: one read of the engine and
    /// of this PC's state database, when Rune runs it on the flyout's turn.
    fn read_flyout(&mut self) {
        if let Some(flyout) = &self.wiring.flyout {
            flyout.update(|s, _| s.refresh());
            self.flyout_behind = false;
            #[cfg(test)]
            {
                self.flyout_reads += 1;
            }
        }
    }

    pub(crate) fn toggle_flyout(&mut self, anchor: Option<flyout::Anchor>) {
        let Some(flyout) = self.wiring.flyout.clone() else {
            return;
        };
        let since_hide = unpoisoned(&self.wiring.flyout_hidden_at).map(|at| at.elapsed().as_millis() as u64);
        match flyout::on_tray_click(flyout.is_visible(), since_hide) {
            Toggle::Hide => flyout.hide(),
            Toggle::Ignore => {}
            Toggle::Show => {
                if let Some(anchor) = anchor {
                    flyout.set_position(flyout::position(&anchor));
                }
                // Read before it shows: a hidden flyout was not kept current.
                self.read_flyout();
                flyout.open();
            }
        }
    }

    /// Remembers where the main window is, for the next start. Called before it closes.
    pub fn save_window_place(&mut self) {
        // The first-run window's place is not the main window's.
        if self.window_kind != WindowKind::Main {
            return;
        }
        let Some(path) = self.wiring.state_file.clone() else {
            return;
        };
        let Some(placement) = unpoisoned(&self.wiring.window)
            .as_ref()
            .and_then(WindowHandle::placement)
        else {
            return;
        };
        let now = Observed {
            x: placement.position.0,
            y: placement.position.1,
            w: placement.content_size.0,
            h: placement.content_size.1,
            scale: placement.scale,
            maximized: placement.maximized,
            minimized: placement.minimized,
            full_screen: placement.full_screen,
        };
        let key = surfaces::MAIN_KEY.to_owned();
        let merged = window_state::merge_for_save(self.wiring.saved.get(&key).copied(), now);
        let Some(merged) = merged.filter(|m| self.wiring.saved.get(&key) != Some(m)) else {
            return;
        };
        self.wiring.saved.insert(key, merged);
        if let Err(error) = window_state::save(&path, &self.wiring.saved) {
            log(format!("the window's place was not saved ({})", error.kind()));
        }
    }
}

fn sidebar_for(section: Section) -> Option<SidebarSection> {
    match section {
        Section::Files => Some(SidebarSection::Files),
        Section::Shared => Some(SidebarSection::Shared),
        Section::Photos => Some(SidebarSection::Photos),
        Section::Backups => Some(SidebarSection::Backups),
        Section::Trash => Some(SidebarSection::Trash),
        // `ui::sidebar` has no selection for the two "This PC" rows yet.
        Section::SyncHistory | Section::AvailableOffline => None,
    }
}

fn section_for(sidebar: SidebarSection) -> Section {
    match sidebar {
        SidebarSection::Files => Section::Files,
        SidebarSection::Shared => Section::Shared,
        SidebarSection::Photos => Section::Photos,
        SidebarSection::Backups => Section::Backups,
        SidebarSection::Trash => Section::Trash,
    }
}

/// Points the sidebar's highlight at the route's section.
fn sync_sidebar(s: &mut State) {
    if let Some(sidebar) = sidebar_for(s.shell.app.route().section) {
        s.sidebar_section = sidebar;
    }
    s.shell.sidebar_seen = s.sidebar_section;
}

/// The one way a frame changes what the window shows: a click on a section, a folder, the
/// back chevron, the gear.
pub fn dispatch(s: &mut State, event: Event) -> Outcome {
    // A scene sets the gate on the engine so that `refresh()` reads it back as the authority.
    // `Event::Gate` via dispatch is used only by scene functions; the running app's gate comes
    // through `Event::Session` in `refresh`. The real engine ignores `set_gate_for_scene`.
    if let Event::Gate(gate) = &event {
        s.shell.actions.set_gate_for_scene(*gate);
    }
    let outcome = s.shell.app.apply(event);
    sync_sidebar(s);
    crate::frames::settings::sync(s); // what was open inside settings is forgotten when the route leaves it
    crate::frames::backups::sync(s); // the same for Backups, and its one-time code is wiped
    crate::frames::account::sync(s); // and what was open inside the account sections
    crate::frames::tray::shortcuts::sync(s); // a closed shortcuts list forgets its page
    crate::frames::files::reset::sync(s); // a route that left the Files window closes its dialogs, wiped
    outcome
}

/// Something outside the main window asks it to show a place. It is kept while the gate is
/// closed; the window comes forward either way.
pub fn request(s: &mut State, intent: Intent) -> Outcome {
    let outcome = dispatch(s, Event::Request(intent));
    s.shell.open_main();
    outcome
}

/// Runs at the start of every frame. `ui::sidebar` (not the shell's to edit) selects a
/// section by writing `State::sidebar_section`; this turns that write into a navigation, so
/// the sidebar already routes. A ui lane that calls [`dispatch`] itself makes this a no-op.
pub fn poll(s: &mut State, _seconds: f64, tx: &mut Tx) -> bool {
    let mut changed = poll_screen(s, _seconds, tx);
    changed |= poll_capture(s, tx);
    changed
}

/// Everything a poll does but the capture decision.
fn poll_screen(s: &mut State, _seconds: f64, tx: &mut Tx) -> bool {
    // The open file: the engine's answers, and a route that left it (lane f-viewers).
    let mut changed = crate::frames::viewers::poll(s, tx);
    // The engine's answers that arrive between presses (a sign-in that took a second, the other device).
    changed |= crate::frames::signin::poll(s, tx);
    changed |= crate::frames::settings::poll(s, tx);
    // The speed test's steps (Settings, Bandwidth).
    changed |= crate::frames::settings::bandwidth::poll(s, _seconds, tx);
    // What the sign-in screens asked the shell for.
    changed |= drain_signin_intents(s);
    // "Open my Beebeeb" at the end of the first run starts the first-time guide (DW-0380).
    changed |= crate::frames::signin::setup::guide::poll(s, tx);
    // The session and the tray: the engine may have changed since the last frame.
    let gate_before = s.shell.app.gate();
    changed |= s.shell.refresh();
    let read_by_the_gate = s.shell.app.gate() != gate_before;
    if read_by_the_gate {
        on_gate_change(s);
        changed = true;
    }
    // A refused sync start says so, once per attempt (the refusal modal, Q3).
    changed |= open_sync_refusal(s);
    // The first sync's files, once a check finished (R1.1).
    changed |= read_again_after_a_check(s);
    // A lock, a sign-out or another account ends what was open inside the account sections: a
    // review of another device's sign-in, a typed code or password, a Windows Hello that is up.
    // Compared with what this poll saw last, not with this poll's own refresh: the engine's
    // status events refresh the session between polls too.
    let now = (s.shell.app.gate(), s.shell.app.account().map(str::to_owned));
    if let Some((gate, account)) = s.shell.session_seen.replace(now.clone()) {
        let left_ready = gate == super::app::GateState::Ready && now.0 != gate;
        let other_account = account.is_some() && now.1 != account;
        if left_ready || other_account {
            crate::frames::account::end_flows(s);
            // The same for the Files window: its dialogs (a link's password among them), what
            // Undo and Try again hold, the selection and where Back and Forward go.
            crate::frames::files::reset::end_session(s, now.1 != account);
            changed = true;
        }
        // The rows are the session's too. `on_gate_change` reads them again when THIS poll's
        // refresh moved the gate; another account under a gate that stays Ready, and a change
        // first seen by a refresh between polls, come by here only: the rows are dropped and
        // the root is read again, so no name of the earlier account stays on screen.
        if (gate, account) != now && !read_by_the_gate {
            crate::frames::files::reset::read_the_root_again(s);
            changed = true;
        }
    }
    // Windows Hello's answer to an approval of another device's sign-in (DW-A14). Read after the
    // session: an answer that comes with a lock or a sign-out finds its wait already ended.
    changed |= crate::frames::account::approve::poll(s);
    // The one window is the first-run window (960 x 640) or the main window, by the screen.
    let kind = surfaces::kind_for(&s.shell.app.screen());
    if kind != s.shell.window_kind {
        let saved_main = s.shell.wiring.saved.get(surfaces::MAIN_KEY);
        if let Some(size) = surfaces::size_on_change(s.shell.window_kind, kind, saved_main) {
            tx.resize_window_instant(size);
        }
        s.shell.window_kind = kind;
        changed = true;
    }
    if s.sidebar_section != s.shell.sidebar_seen {
        let outcome = s
            .shell
            .app
            .apply(Event::Navigate(Route::section(section_for(s.sidebar_section))));
        sync_sidebar(s);
        // The sidebar's route event does not go through `dispatch`: the Files window still
        // closes what was open in it (a name being typed in a row is not modal).
        crate::frames::files::reset::sync(s);
        changed |= outcome == Outcome::Changed;
    }
    // A message that went takes what its Try again held with it.
    changed |= crate::frames::files::reset::poll(s);
    // Backups shows its own folder in the Files window (`frames::files::protected`, DW-0422).
    changed |= crate::frames::files::protected::poll(s);
    changed
}

/// The ONE place capture exclusion is decided: every path of the poll (gate change, sign-in
/// intents, sidebar navigation, viewer, an answer of Windows Hello) that may change what is on
/// screen has run. A new path cannot forget this: it runs unconditionally at the end of every poll.
/// The decision combines the route-level check (SignIn, Locked, RecoveryPhrase, FirstRun) with
/// the account pane's per-screen check (change password, 2FA setup, backup codes, the approval code).
///
/// Leaving a secret screen, exclusion drops ONE POLL LATE. The poll runs before the frame it
/// belongs to is built and shown, and a press animates what it removes: on the poll that first
/// sees "no secret" the last shown frame still holds it, and the leaving elements would go on
/// painting it for the length of their animation. So that poll keeps the window excluded and
/// lands every running animation (the leaving elements are gone from the frame it builds); the
/// next poll, which it asks for, tells the platform.
fn poll_capture(s: &mut State, tx: &mut Tx) -> bool {
    let secret = registry::route_shows_secret(&s.shell.app) || {
        let group = registry::group_for(&s.shell.app);
        matches!(
            group,
            registry::FrameGroup::Settings(pane) if crate::frames::account::Pane::root(pane).is_some()
        ) && s.account.shows_secret()
    }
    // A link's password being typed in the Files window's Share dialog (DW-0407).
    || (matches!(registry::group_for(&s.shell.app), registry::FrameGroup::Section(_))
        && crate::frames::files::share::shows_secret(s));
    let leaving = std::mem::replace(&mut s.shell.secret_seen, secret) && !secret;
    if leaving {
        tx.settle();
        tx.wake_after(std::time::Duration::from_millis(1));
    }
    s.shell.apply_capture_exclusion(secret || leaving);
    leaving
}

/// Takes every intent the sign-in screens queued (`signin::Ui::take_intents`) and acts on it. A finished
/// sign-in is not routed from here: the session's gate is the one authority on which screen shows, and
/// [`on_gate_change`] runs when it opens. What the shell cannot do yet is logged, never dropped silently.
/// `true`: something was taken.
pub fn drain_signin_intents(s: &mut State) -> bool {
    use crate::frames::signin::Intent as SignIn;
    let Some(intents) = s.signin.as_mut().map(crate::frames::signin::Ui::take_intents) else {
        return false;
    };
    for intent in &intents {
        match intent {
            // The session is read on this same poll; the gate decides (Ready opens the window on Files).
            SignIn::Next(_) => {}
            // Sign-up is a set of screens of the same window (DW-A31 to DW-A44), where the source has it.
            SignIn::CreateAccount => {
                if !s.signin.as_mut().is_some_and(crate::frames::signin::Ui::start_create) {
                    log("NotWired(sign-in: create an account)");
                }
            }
            SignIn::OpenTerms | SignIn::OpenPrivacy => log("NotWired(sign-up: open a page in the browser)"),
            SignIn::SavePhraseAsFile | SignIn::PrintPhrase => log("NotWired(sign-up: save or print the phrase)"),
            SignIn::WriteToSupport => log("NotWired(sign-in: write to support)"),
            SignIn::CancelBilling | SignIn::EraseAllFiles | SignIn::DeleteAccount => {
                log("NotWired(sign-in: an account dialog)")
            }
        }
    }
    !intents.is_empty()
}

/// The session's gate changed (a sign-in finished, the vault was locked, the account signed out). The
/// folder the window holds was read under the old gate: read it again from the root, so a sign-in shows the
/// account's files and a sign-out shows none of them. A gate that opened also starts sync (refused, and
/// logged, while another agent holds the profile).
fn on_gate_change(s: &mut State) {
    let (listing, list_error) = super::state::load(s.source.as_ref(), None);
    s.listing = listing;
    s.list_error = list_error;
    s.selected = None;
    if s.shell.app.gate() == super::app::GateState::Ready {
        s.shell.start_sync();
    }
}

/// The refusal modal (the owner's ruling Q3 on the sync-start contract): it opens once per refused start attempt,
/// over whatever the window shows, only while the gate is Ready. An attempt refused while the gate is closed is
/// never shown later (a stale refusal); another dialog in front keeps it waiting until that one closes. `true`: it
/// opened.
fn open_sync_refusal(s: &mut State) -> bool {
    let Some(refusal) = s.shell.actions.sync_refusal() else {
        return false;
    };
    if s.shell.refusal_seen == Some(refusal.attempt) {
        return false;
    }
    if s.shell.app.gate() != super::app::GateState::Ready {
        s.shell.refusal_seen = Some(refusal.attempt);
        return false;
    }
    if s.shell.app.dialog().is_some() {
        return false;
    }
    s.shell.refusal_seen = Some(refusal.attempt);
    s.shell.sync_off = Some(refusal);
    #[cfg(test)]
    {
        s.shell.sync_off_opened += 1;
    }
    dispatch(s, Event::OpenDialog(Dialog::SyncOff)) == Outcome::Changed
}

/// The first sync on this PC, as the window sees it (R1.1 of the sync-start contract): the gate opened before anything
/// had synced, so the folder could not be read, or read as empty, and the window said so. Each check the running engine
/// finishes (the b's engine state turns Idle) reads that folder again, so the account's files appear without a click.
/// Only a list that shows an error or nothing is read: a list with rows, a selection and the folder shown are never
/// changed from here. `true`: it was read.
fn read_again_after_a_check(s: &mut State) -> bool {
    let finished = std::mem::take(&mut s.shell.check_finished);
    let shows_nothing = s.list_error.is_some() || s.listing.items.is_empty();
    if !finished || !shows_nothing || s.shell.app.gate() != super::app::GateState::Ready {
        return false;
    }
    let folder = s.listing.folder.id.clone();
    (s.listing, s.list_error) = super::state::load(s.source.as_ref(), folder.as_deref());
    true
}

/// The refusal modal's "Try again": it closes, and one start is asked for (the start worker runs one at a time).
pub fn try_sync_again(s: &mut State) {
    if s.shell.app.dialog() == Some(&Dialog::SyncOff) {
        dispatch(s, Event::Escape);
    }
    s.shell.start_sync();
}

/// The shell's own keys, then the current frame's (DW-1232, DW-1233: Windows' words).
///
/// Alt+Left / Alt+Right: back and forward. Ctrl+,: Settings. Ctrl+/: the shortcuts list.
/// Esc: closes the front-most thing. Everything else goes to the frame.
///
/// A build with the `feedback` feature gives the live feedback mode the key first: its two
/// hotkeys (F8, Shift+F8; none of the shell's or the board's), and every key while its comment
/// mode is on. Otherwise the key goes on to [`shell_keys`] unchanged.
pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    #[cfg(feature = "feedback")]
    return crate::feedback::on_key(s, ev, tx, shell_keys);
    #[cfg(not(feature = "feedback"))]
    shell_keys(s, ev, tx)
}

/// The shell's own shortcuts, then the frame's keys.
fn shell_keys(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let plain_alt = ev.mods.alt && !ev.mods.cmd && !ev.mods.shift;
    let plain_cmd = !ev.mods.alt && !ev.mods.shift;
    // "/" is a shifted key on many layouts (Shift+7 on German): Shift is part of typing it.
    let slash_cmd = !ev.mods.alt;
    // Search shows no place of its own: Back and Forward are off until Esc ends it (DW-0802).
    let searching = crate::frames::search::holds_back(s);
    // While a tip of the first-time guide shows, Esc (Skip), Tab and Enter are its card's (DW-0301).
    if crate::frames::signin::setup::guide::on_key(s, ev, tx) {
        return true;
    }
    let event = match ev.key {
        Key::Left if plain_alt && !searching => Some(Event::Back),
        Key::Right if plain_alt && !searching => Some(Event::Forward),
        Key::Command(',') if plain_cmd => Some(Event::OpenSettings(SettingsPane::General)),
        Key::Command('/') if slash_cmd => Some(Event::OpenDialog(Dialog::Shortcuts)),
        Key::Escape => Some(Event::Escape),
        _ => None,
    };
    if let Some(event) = event {
        let escape = event == Event::Escape;
        match dispatch(s, event) {
            // Esc with nothing of the shell's to close belongs to the frame (a selection, a menu).
            Outcome::Unchanged | Outcome::Refused if escape => {}
            _ => return true,
        }
    }
    // A dialog in front takes the keys; the frame behind it gets none.
    let keys = match registry::dialog_group_for(&s.shell.app) {
        Some(dialog) => registry::dialog_frame(dialog).and_then(|frame| frame.on_key),
        None => registry::frame(registry::group_for(&s.shell.app)).and_then(|frame| frame.on_key),
    };
    match keys {
        Some(keys) => keys(s, ev, tx),
        None => false,
    }
}

/// A click or a menu pick on the b, delivered on the UI thread.
pub fn on_tray(s: &mut State, event: TrayEvent, tx: &mut Tx) {
    match event {
        TrayEvent::Activate { anchor } => match s.shell.tray_model.left_click {
            // Signed out: the window on sign-in, never the flyout (DW-1201).
            LeftClick::OpenSignIn => s.shell.open_main(),
            LeftClick::ToggleFlyout => s.shell.toggle_flyout(anchor),
        },
        TrayEvent::Menu(action) => on_menu(s, action, tx),
        // The icon went (Explorer stopped) while the window is closed: never a live process
        // with no surface. It came back: nothing to do.
        TrayEvent::Presence(false) => {
            let hidden = s.shell.wiring.main.as_ref().is_some_and(|main| !main.is_visible());
            if hidden {
                log("the b left the notification area; opening the window");
                s.shell.open_main();
            }
        }
        TrayEvent::Presence(true) => {}
        TrayEvent::ShutdownCancelled => {
            if std::mem::take(&mut s.shell.stopped_for_quit) {
                s.shell.actions.resume_after_cancelled_quit();
            }
        }
        // `taskkill` without /F, Restart Manager: quit cleanly instead of ignoring it.
        TrayEvent::CloseRequested => tx.quit(),
    }
}

/// One request from the control channel (a later launch, the installer), on the UI thread.
/// Untrusted: `Request` can only be one of three things, and a link only ever navigates.
pub fn on_control(s: &mut State, request: Request, tx: &mut Tx) {
    match request {
        Request::Show => s.shell.open_main(),
        Request::Open(link) => on_open_url(s, &link, tx),
        // The normal quit path: `on_quit` stops the engine, then the app exits.
        Request::QuitForUpdate => {
            s.shell.hide_flyout();
            tx.quit();
        }
    }
}

/// One item of the b's menu (DW-1231).
pub fn on_menu(s: &mut State, action: MenuAction, tx: &mut Tx) {
    s.shell.hide_flyout();
    let result = match action {
        // Brings the window forward where it is; a first open is on Files.
        MenuAction::OpenBeebeeb => {
            s.shell.open_main();
            Ok(())
        }
        MenuAction::OpenInFileExplorer => s.shell.actions.open_in_file_explorer(),
        MenuAction::PauseSyncing => s.shell.actions.pause_sync(),
        MenuAction::ResumeSyncing => s.shell.actions.resume_sync(),
        MenuAction::Settings => {
            request(s, Intent::Settings(SettingsPane::General));
            Ok(())
        }
        MenuAction::Quit => {
            tx.quit();
            Ok(())
        }
    };
    if let Err(error) = result {
        log(format!("the b's menu: {action:?} did not run ({error:?})"));
    }
    if s.shell.refresh() {
        sync_sidebar(s);
    }
}

/// The window is closing. It keeps running in the notification area (DW-1231: "closing the
/// window leaves the b running"); with no b to come back through, closing quits.
pub fn on_close(s: &mut State, tx: &mut Tx) {
    s.shell.save_window_place();
    if s.shell.close_quits() {
        tx.quit();
    }
}

/// The quit hook: every quit comes through here (the b's menu, the installer's
/// `quit-for-update`, a close with no b, a Windows shutdown).
///
/// Rune also runs it when Windows only ASKS to shut down (`WM_QUERYENDSESSION`), and another
/// app may refuse. So nothing here is final: the tray is asked to take its icon away (not
/// dropped: a dropped tray could never come back), and the engine is stopped through the one
/// hook that has a counterpart. If the shutdown is cancelled the tray puts the icon back and
/// sends `TrayEvent::ShutdownCancelled`, which resumes the engine. The icon is finally removed
/// when the shell is dropped with the root state.
pub fn on_quit(s: &mut State, _tx: &mut Tx) {
    s.shell.save_window_place();
    s.shell.actions.stop_for_quit();
    s.shell.stopped_for_quit = true;
    if let Some(tray) = &s.shell.tray {
        tray.remove_for_quit();
    }
}

/// A second launch without a link (Windows), or the Dock icon (macOS): the main window comes
/// forward. True: Rune's default would bring back "the window hidden last", which can be the
/// flyout.
pub fn on_reopen(s: &mut State, _has_visible: bool, _tx: &mut Tx) -> bool {
    s.shell.open_main();
    true
}

/// A `beebeeb://` link: from the first launch's arguments, a later launch's, or the OS.
/// Untrusted: only what `deeplink::parse` allows becomes a request, and a request only shows.
pub fn on_open_url(s: &mut State, url: &str, _tx: &mut Tx) {
    match instance::forwarded(url) {
        Forwarded::Show(intent) => {
            request(s, intent);
        }
        Forwarded::ShowOnly(error) => {
            log(format!("a link was not followed: {}", error.reason()));
            s.shell.open_main();
        }
    }
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
