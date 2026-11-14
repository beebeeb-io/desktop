//! App state and routing as one explicit machine. Pure: no Rune, no OS, no clock.
//!
//! The board allows two surfaces at a time: the one window and the flyout (DW-1201, "At most
//! two Beebeeb surfaces"). Settings, the viewer and the conflict choice are states of the main
//! window, never windows of their own (DW-0701: "never a separate window"). Sign-in and the
//! first-run steps are drawn in the first-run window (960 x 640, `surfaces::WindowKind::FirstRun`),
//! which comes before the main window and is never open beside it; this machine names those
//! states too ([`Screen::SignIn`], [`GateState::FirstRun`]), and `surfaces::kind_for` says
//! which window a screen belongs to.
//!
//! What the window shows is [`AppMachine::screen`], decided in this order:
//! 1. the session gate ([`GateState`]): signed out shows sign-in, the other not-ready states
//!    show their gate frame, and nothing of the account is reachable behind them;
//! 2. settings mode, when open;
//! 3. the Drive route (sidebar section, folder, open item), with back/forward history.

/// The sidebar's sections, in the board's order (DW-0401): five of the account, two of "This PC".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Files,
    Shared,
    Photos,
    Backups,
    Trash,
    SyncHistory,
    AvailableOffline,
}

impl Section {
    pub const ALL: [Section; 7] = [
        Section::Files,
        Section::Shared,
        Section::Photos,
        Section::Backups,
        Section::Trash,
        Section::SyncHistory,
        Section::AvailableOffline,
    ];

    /// The sidebar's words.
    pub fn label(self) -> &'static str {
        match self {
            Section::Files => "Files",
            Section::Shared => "Shared",
            Section::Photos => "Photos",
            Section::Backups => "Backups",
            Section::Trash => "Trash",
            Section::SyncHistory => "Sync history",
            Section::AvailableOffline => "Available offline",
        }
    }

    /// The word a deep link uses (`beebeeb://open/<slug>`).
    pub fn slug(self) -> &'static str {
        match self {
            Section::Files => "files",
            Section::Shared => "shared",
            Section::Photos => "photos",
            Section::Backups => "backups",
            Section::Trash => "trash",
            Section::SyncHistory => "sync-history",
            Section::AvailableOffline => "available-offline",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Section> {
        Section::ALL.into_iter().find(|s| s.slug() == slug)
    }
}

/// The settings sidebar (DW-0701): Settings, then Drive's own sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SettingsPane {
    General,
    Account,
    Security,
    DevicesAndSessions,
    Notifications,
    Storage,
    Backups,
    SyncAndFileExplorer,
    ThumbnailsAndPreviews,
    Bandwidth,
}

impl SettingsPane {
    pub const ALL: [SettingsPane; 10] = [
        SettingsPane::General,
        SettingsPane::Account,
        SettingsPane::Security,
        SettingsPane::DevicesAndSessions,
        SettingsPane::Notifications,
        SettingsPane::Storage,
        SettingsPane::Backups,
        SettingsPane::SyncAndFileExplorer,
        SettingsPane::ThumbnailsAndPreviews,
        SettingsPane::Bandwidth,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SettingsPane::General => "General",
            SettingsPane::Account => "Account",
            SettingsPane::Security => "Security",
            SettingsPane::DevicesAndSessions => "Devices and sessions",
            SettingsPane::Notifications => "Notifications",
            SettingsPane::Storage => "Storage",
            SettingsPane::Backups => "Backups",
            SettingsPane::SyncAndFileExplorer => "Sync and File Explorer",
            SettingsPane::ThumbnailsAndPreviews => "Thumbnails and previews",
            SettingsPane::Bandwidth => "Bandwidth",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            SettingsPane::General => "general",
            SettingsPane::Account => "account",
            SettingsPane::Security => "security",
            SettingsPane::DevicesAndSessions => "devices",
            SettingsPane::Notifications => "notifications",
            SettingsPane::Storage => "storage",
            SettingsPane::Backups => "backups",
            SettingsPane::SyncAndFileExplorer => "sync",
            SettingsPane::ThumbnailsAndPreviews => "previews",
            SettingsPane::Bandwidth => "bandwidth",
        }
    }

    pub fn from_slug(slug: &str) -> Option<SettingsPane> {
        SettingsPane::ALL.into_iter().find(|p| p.slug() == slug)
    }
}

/// An item's id as the engine gives it. Opaque here.
pub type ItemId = String;

/// Where the Drive part of the window is: a section, optionally inside a folder, optionally
/// with one item open in the viewer (DW-0408, DW-1501).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub section: Section,
    pub folder: Option<ItemId>,
    pub item: Option<ItemId>,
}

impl Route {
    pub fn section(section: Section) -> Route {
        Route {
            section,
            folder: None,
            item: None,
        }
    }

    pub fn folder(section: Section, folder: impl Into<ItemId>) -> Route {
        Route {
            section,
            folder: Some(folder.into()),
            item: None,
        }
    }

    /// This place with `item` open in the viewer.
    pub fn with_item(&self, item: impl Into<ItemId>) -> Route {
        Route {
            item: Some(item.into()),
            ..self.clone()
        }
    }

    /// Whether the route names something of one account (a folder or an item), as opposed to
    /// a place every account has.
    pub fn names_account_data(&self) -> bool {
        self.folder.is_some() || self.item.is_some()
    }
}

/// What the main window may show, as the session decides. The names are
/// `src-tauri/src/auth_v2/gate.rs`'s `GateState`, variant for variant, so the session lane's
/// type converts into this one. It reaches the machine through `EngineActions::gate_state`
/// (`actions.rs`), which the session lane implements; until then the fixture engine answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GateState {
    /// Signed in, vault unlocked, plan present.
    Ready,
    /// No session at all: sign-in (DW-A02).
    SignedOut,
    /// Signed in, the vault key is on this PC: the lock screen (DW-0009).
    Locked,
    /// Signed in, no vault key on this PC: the recovery phrase (DW-A23).
    NeedsRecoveryPhrase,
    /// Signed in and unlocked, the account has no plan (DW-0014).
    NoPlan,
    /// Signed in, the credential store could not say whether the key is there.
    KeyStoreUnavailable,
    /// Signed in and unlocked, setup on this PC is not finished: the first-run steps (DW-0201
    /// to DW-0208, "A window of its own between sign-in and the main window"). Not a variant
    /// of the engine's gate (`gate.rs`), which is Ready: the engine host shows this one while the
    /// settings' `native_setup_done` is not set (`engine_host::setup::FirstRun`).
    FirstRun,
}

/// The account's id as the session gives it (the user id). Opaque here; never logged.
pub type AccountId = String;

/// What the session lane reports: the gate, and WHOSE session it is. The account is part of
/// the input so that a change of account is seen even when no sign-out was observed between
/// two samples (`Ready(A)` then `Ready(B)`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub gate: GateState,
    /// `None` exactly when signed out.
    pub account: Option<AccountId>,
}

impl Session {
    pub fn signed_out() -> Session {
        Session {
            gate: GateState::SignedOut,
            account: None,
        }
    }

    pub fn of(gate: GateState, account: impl Into<AccountId>) -> Session {
        if gate == GateState::SignedOut {
            return Session::signed_out();
        }
        Session {
            gate,
            account: Some(account.into()),
        }
    }
}

/// A dialog in front of the window's content. One at a time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dialog {
    /// Two versions of one file: choose (DW-0429). `None`: the first waiting file.
    Conflict { file: Option<ItemId> },
    /// Commands and keyboard shortcuts (DW-1232, Ctrl+/).
    Shortcuts,
    /// A sync start was refused: what happened and how to fix it, where a fix exists (Q3 of the sync-start
    /// contract; no board frame). Its words are the shell's refusal (`Shell::sync_off`).
    SyncOff,
}

/// What the main window shows right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Screen {
    SignIn,
    /// One of the not-ready gate states other than signed out.
    Gate(GateState),
    Drive(Route),
    Settings(SettingsPane),
}

/// Something asked of the window from outside it: the tray, the flyout, a deep link, a second
/// launch. Only ever navigation; nothing here changes data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    Go(Route),
    Settings(SettingsPane),
    ReviewConflict { file: Option<ItemId> },
}

impl Intent {
    /// Whether the intent names one account's data, so it must not survive a sign-out.
    fn names_account_data(&self) -> bool {
        match self {
            Intent::Go(route) => route.names_account_data(),
            Intent::Settings(_) => false,
            Intent::ReviewConflict { .. } => true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A click in the window: a sidebar section, a folder, the path.
    Navigate(Route),
    /// Open one item in the viewer, in the place the window is.
    OpenItem(ItemId),
    /// The toolbar's chevrons, Alt+Left / Alt+Right, the mouse's back and forward buttons.
    Back,
    Forward,
    /// Ctrl+, and the account row's gear.
    OpenSettings(SettingsPane),
    OpenDialog(Dialog),
    /// Esc: closes the front-most thing (dialog, then viewer, then settings).
    Escape,
    /// From outside the window; kept for later while the gate is closed.
    Request(Intent),
    /// The session's gate changed, for the same account.
    Gate(GateState),
    /// What the session lane reports (gate and account). The shell sends this on every
    /// refresh; a change of account clears everything the last account left in the window.
    Session(Session),
}

/// What an event did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Changed,
    /// Nothing to do (already there, no history that way, nothing to close).
    Unchanged,
    /// Kept until the gate opens.
    Deferred,
    /// Not allowed in this state.
    Refused,
}

/// The most places Back remembers. Older ones fall off.
pub const HISTORY_LIMIT: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppMachine {
    gate: GateState,
    account: Option<AccountId>,
    back: Vec<Route>,
    current: Route,
    forward: Vec<Route>,
    settings: Option<SettingsPane>,
    dialog: Option<Dialog>,
    pending: Option<Intent>,
}

impl AppMachine {
    /// Starts on Files with an empty history.
    pub fn new(gate: GateState) -> AppMachine {
        AppMachine::for_session(Session::of(gate, ""))
    }

    pub fn for_session(session: Session) -> AppMachine {
        AppMachine {
            gate: session.gate,
            account: session.account,
            back: Vec::new(),
            current: Route::section(Section::Files),
            forward: Vec::new(),
            settings: None,
            dialog: None,
            pending: None,
        }
    }

    pub fn gate(&self) -> GateState {
        self.gate
    }

    pub fn screen(&self) -> Screen {
        match self.gate {
            GateState::SignedOut => Screen::SignIn,
            GateState::Ready => match self.settings {
                Some(pane) => Screen::Settings(pane),
                None => Screen::Drive(self.current.clone()),
            },
            gate => Screen::Gate(gate),
        }
    }

    /// The Drive route, whatever is in front of it.
    pub fn route(&self) -> &Route {
        &self.current
    }

    /// The dialog in front, if the gate is open. The first run has one: the refusal modal (see `apply`).
    pub fn dialog(&self) -> Option<&Dialog> {
        match self.gate {
            GateState::Ready => self.dialog.as_ref(),
            GateState::FirstRun => self.dialog.as_ref().filter(|dialog| **dialog == Dialog::SyncOff),
            _ => None,
        }
    }

    /// Is Settings in front of the Drive route? (Back then closes it.)
    pub fn in_settings(&self) -> bool {
        self.settings.is_some()
    }

    /// Where Back would go, when it is a step in the history.
    pub fn back_target(&self) -> Option<&Route> {
        self.back.last()
    }

    /// Where Forward would go.
    pub fn forward_target(&self) -> Option<&Route> {
        self.forward.last()
    }

    pub fn pending(&self) -> Option<&Intent> {
        self.pending.as_ref()
    }

    /// Whether the back chevron is enabled.
    pub fn can_go_back(&self) -> bool {
        self.gate == GateState::Ready && self.dialog.is_none() && (self.settings.is_some() || !self.back.is_empty())
    }

    pub fn can_go_forward(&self) -> bool {
        self.gate == GateState::Ready && self.dialog.is_none() && self.settings.is_none() && !self.forward.is_empty()
    }

    pub fn apply(&mut self, event: Event) -> Outcome {
        match event {
            Event::Gate(gate) => self.set_gate(gate),
            Event::Session(session) => self.set_session(session),
            Event::Request(intent) => self.request(intent),
            // The first run is signed in and unlocked, and sync starts in it: a start that is refused there
            // opens the same refusal modal ("the first run uses the same modal", the sync-start contract's S2),
            // and Esc closes it. Nothing else moves while the gate is not Ready.
            Event::OpenDialog(Dialog::SyncOff) if self.gate == GateState::FirstRun => {
                self.dialog = Some(Dialog::SyncOff);
                Outcome::Changed
            }
            Event::Escape if self.gate == GateState::FirstRun && self.dialog.is_some() => {
                self.dialog = None;
                Outcome::Changed
            }
            _ if self.gate != GateState::Ready => Outcome::Refused,
            Event::Escape => self.escape(),
            // A dialog is modal: it closes first.
            _ if self.dialog.is_some() => Outcome::Refused,
            Event::Navigate(route) => {
                let left_settings = self.settings.take().is_some();
                if self.push(route) || left_settings {
                    Outcome::Changed
                } else {
                    Outcome::Unchanged
                }
            }
            Event::OpenItem(item) => {
                if self.settings.is_some() {
                    return Outcome::Refused;
                }
                let route = self.current.with_item(item);
                if self.push(route) {
                    Outcome::Changed
                } else {
                    Outcome::Unchanged
                }
            }
            Event::Back => {
                if self.settings.take().is_some() {
                    return Outcome::Changed;
                }
                match self.back.pop() {
                    Some(previous) => {
                        self.forward.push(std::mem::replace(&mut self.current, previous));
                        Outcome::Changed
                    }
                    None => Outcome::Unchanged,
                }
            }
            Event::Forward => {
                if self.settings.is_some() {
                    return Outcome::Unchanged;
                }
                match self.forward.pop() {
                    Some(next) => {
                        self.back.push(std::mem::replace(&mut self.current, next));
                        Outcome::Changed
                    }
                    None => Outcome::Unchanged,
                }
            }
            Event::OpenSettings(pane) => {
                if self.settings.replace(pane) == Some(pane) {
                    Outcome::Unchanged
                } else {
                    Outcome::Changed
                }
            }
            Event::OpenDialog(dialog) => {
                self.dialog = Some(dialog);
                Outcome::Changed
            }
        }
    }

    /// Moves to `route`, remembering where the window was. False when it is already there.
    fn push(&mut self, route: Route) -> bool {
        if route == self.current {
            return false;
        }
        self.back.push(std::mem::replace(&mut self.current, route));
        if self.back.len() > HISTORY_LIMIT {
            self.back.remove(0);
        }
        self.forward.clear();
        true
    }

    fn escape(&mut self) -> Outcome {
        if self.dialog.take().is_some() || self.settings.take().is_some() {
            return Outcome::Changed;
        }
        if self.current.item.is_some() {
            // Leaving the viewer is a step back to the folder it was opened from.
            let folder = Route {
                item: None,
                ..self.current.clone()
            };
            if self.back.last() == Some(&folder) {
                return self.apply(Event::Back);
            }
            self.push(folder);
            return Outcome::Changed;
        }
        Outcome::Unchanged
    }

    fn request(&mut self, intent: Intent) -> Outcome {
        if self.gate == GateState::Ready {
            return self.perform(intent);
        }
        // Signed out there is no account yet: an intent that names one account's data would
        // point the next account at the last one's ids.
        if self.gate == GateState::SignedOut && intent.names_account_data() {
            return Outcome::Refused;
        }
        self.pending = Some(intent);
        Outcome::Deferred
    }

    fn perform(&mut self, intent: Intent) -> Outcome {
        // An intent from outside wins over a dialog the person left open: they asked for
        // something else, in another surface.
        let closed_dialog = self.dialog.take().is_some();
        let outcome = match intent {
            Intent::Go(route) => self.apply(Event::Navigate(route)),
            Intent::Settings(pane) => self.apply(Event::OpenSettings(pane)),
            Intent::ReviewConflict { file } => {
                self.settings = None;
                self.apply(Event::OpenDialog(Dialog::Conflict { file }))
            }
        };
        if closed_dialog && outcome == Outcome::Unchanged {
            Outcome::Changed
        } else {
            outcome
        }
    }

    /// The session as the session lane reports it. Another account than the one the window
    /// was showing (also with no sign-out seen in between) starts from nothing: no place, no
    /// history, no dialog, no settings pane, no waiting intent of any kind.
    fn set_session(&mut self, session: Session) -> Outcome {
        let session = Session {
            // No account is signed out, whatever the gate says.
            account: session.account.filter(|_| session.gate != GateState::SignedOut),
            ..session
        };
        let other_account = match (&self.account, &session.account) {
            (Some(was), Some(now)) => was != now,
            _ => false,
        };
        if other_account {
            self.forget_account();
            self.pending = None;
        }
        let gate = match session.account {
            Some(account) => {
                self.account = Some(account);
                session.gate
            }
            // `set_gate(SignedOut)` forgets the account.
            None => GateState::SignedOut,
        };
        let outcome = self.set_gate(gate);
        if other_account { Outcome::Changed } else { outcome }
    }

    /// Whose session the window shows; `None` when signed out.
    pub fn account(&self) -> Option<&str> {
        self.account.as_deref()
    }

    fn forget_account(&mut self) {
        self.back.clear();
        self.forward.clear();
        self.current = Route::section(Section::Files);
        self.settings = None;
        self.dialog = None;
    }

    fn set_gate(&mut self, gate: GateState) -> Outcome {
        if gate == self.gate {
            return Outcome::Unchanged;
        }
        self.gate = gate;
        match gate {
            GateState::SignedOut => {
                // Nothing of the account outlives its session: not the place, not the
                // history, not a waiting intent that names its data.
                self.account = None;
                self.forget_account();
                if self.pending.as_ref().is_some_and(Intent::names_account_data) {
                    self.pending = None;
                }
            }
            GateState::Ready => {
                if let Some(intent) = self.pending.take() {
                    self.perform(intent);
                }
            }
            // Locked, no plan, no key: the same account comes back to where it was.
            _ => {}
        }
        Outcome::Changed
    }
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
