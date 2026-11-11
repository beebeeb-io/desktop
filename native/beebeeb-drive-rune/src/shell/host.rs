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
use super::tray::{LeftClick, MenuAction, TrayInputs, TrayModel, tray_model};
use super::window_state::{self, Observed, SavedWindows};
use super::{registry, surfaces};
use crate::State;
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
            // The flyout reads the inputs themselves (counts), not only what the b shows.
            self.tray_inputs = inputs;
            if let Some(flyout) = &self.wiring.flyout {
                flyout.update(|s, _| s.refresh());
            }
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

    fn toggle_flyout(&self, anchor: Option<flyout::Anchor>) {
        let Some(flyout) = &self.wiring.flyout else { return };
        let since_hide = unpoisoned(&self.wiring.flyout_hidden_at).map(|at| at.elapsed().as_millis() as u64);
        match flyout::on_tray_click(flyout.is_visible(), since_hide) {
            Toggle::Hide => flyout.hide(),
            Toggle::Ignore => {}
            Toggle::Show => {
                if let Some(anchor) = anchor {
                    flyout.set_position(flyout::position(&anchor));
                }
                flyout.update(|s, _| s.refresh());
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
    let outcome = s.shell.app.apply(event);
    sync_sidebar(s);
    crate::frames::settings::sync(s); // what was open inside settings is forgotten when the route leaves it
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
    // The one window is the first-run window (960 x 640) or the main window, by the screen.
    let kind = surfaces::kind_for(&s.shell.app.screen());
    if kind != s.shell.window_kind {
        let saved_main = s.shell.wiring.saved.get(surfaces::MAIN_KEY);
        if let Some(size) = surfaces::size_on_change(s.shell.window_kind, kind, saved_main) {
            tx.resize_window_instant(size);
        }
        s.shell.window_kind = kind;
    }
    if s.sidebar_section == s.shell.sidebar_seen {
        return false;
    }
    let outcome = s
        .shell
        .app
        .apply(Event::Navigate(Route::section(section_for(s.sidebar_section))));
    sync_sidebar(s);
    outcome == Outcome::Changed
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
    let event = match ev.key {
        Key::Left if plain_alt => Some(Event::Back),
        Key::Right if plain_alt => Some(Event::Forward),
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
