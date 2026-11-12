//! Sign-in, the gate and the first run (the board's `.au-mac` window, 960 x 640): what Drive
//! shows before Files. `FRAMES.md` beside this file lists every frame of the group and which
//! are built.
//!
//! [`Ui`] is the window's state; every control changes it through one of its methods, and the
//! engine's answers arrive through [`Ui::pump`] ([`source::SignInSource::poll`]). What leaves the
//! group (the web sign-up, recovery, the product's first step) is an [`Intent`] the shell takes
//! with [`Ui::take_intents`].
//!
//! Secrets: the password lives in ONE secure `TextEdit`, made in [`Ui::new`] and never replaced
//! (bullets, no accessibility text, wiped on drop), and is handed to the source as a borrowed
//! `&str`. Nothing here logs, formats or copies it; it is cleared once a sign-in succeeded.
//!
//! The shell registers the group with one line (`shell/registry.rs`: [`frame`], [`on_key`]).
//! Two hooks wait for a caller in `shell/host.rs`: [`poll`] (the engine's answers that arrive
//! between presses) and [`on_window_focus`] (a shown password is hidden when the window is left).

pub mod icons;
pub mod qr;
pub mod setup;
pub mod source;
pub mod steps;
mod view;
mod view_recovery;
mod view_steps;

#[cfg(test)]
mod tests;

use std::time::Duration;

use rune::{Cx, El, Key, KeyEvent, SecretCopy, TextEdit, Tx};

use crate::State;
use crate::shell::app::{Event, GateState};
use crate::shell::host;
use source::{
    DeviceRequest, DeviceStatus, FixtureSignInSource, HelloOutcome, NextAfterSignIn, PhraseProblem, Script,
    SharedSession, SignInAnswer, SignInEvent, SignInProblem, SignInSource, TwoFactorProblem, board,
};
use steps::Pictured;

pub use view::view;

/// The text inputs' keys (`Tx::focus`, tests).
pub const EMAIL_KEY: &str = "signin-email";
pub const PASSWORD_KEY: &str = "signin-password";

/// The screen's title (`ui::au::heading`): focus moves to it when a result replaces the screen.
pub const TITLE_KEY: &str = "title";
/// The first button of a screen's actions: "Try Windows Hello again" on DW-A05 and DW-A06.
pub const FIRST_BUTTON_KEY: &str = "btn-0";

/// The client's own count of wrong tries in a row after which the lock is explained (DW-A22).
const LOCK_HINT_AFTER: u8 = 5;
/// A device request lives 300 s on the server (DW-A07).
pub const REQUEST_SECONDS: u32 = 300;

/// Which screen the window shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    /// DW-A01.
    Welcome,
    /// DW-A02 / DW-A03, and their states DW-A18 to DW-A22.
    SignIn,
    /// DW-A04; DW-A05 and DW-A06 once Windows Hello answered.
    ContinueAs(Option<HelloFailure>),
    /// DW-A07; DW-A10 when `expired`.
    Device { expired: bool },
    /// DW-A08.
    Approved { by: String },
    /// DW-A08's error state: approved, and the keys did not arrive.
    KeysNotReceived,
    /// DW-A09.
    Denied { by: String },
    /// DW-A26 and DW-A27; DW-A28 when `backup`.
    TwoFactor { backup: bool },
    /// DW-A23 and DW-A24: the twelve words on a device that holds no keys.
    Phrase,
    /// DW-A25: no phrase and no signed-in device.
    NoPhrase,
    /// DW-A30: a passkey proved the person; the vault still needs the password.
    UnlockVault,
    /// DW-A45.
    Forgot,
    /// DW-A46.
    CannotRecover,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelloFailure {
    /// DW-A05.
    NotRecognized,
    /// DW-A06.
    Cancelled,
}

/// Where the person goes when they leave this group. The shell opens it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// Sign-up, at its email step: DW-A31 (DW-A31 to DW-A44, not built in this group yet).
    CreateAccount,
    /// A sign-in step succeeded and what follows is outside this group: the frame `gate.rs`
    /// names for it (DW-0201). The two-factor code and the twelve words are screens of this group.
    Next(NextAfterSignIn),
    /// "Recover with phrase" (DW-A45): recovery, at its email step DW-A47 (not built yet).
    RecoverWithPhrase,
    /// DW-A25's rows: Drive's own dialogs (cancel billing; erase all files; delete the account).
    CancelBilling,
    EraseAllFiles,
    DeleteAccount,
    /// The support address (DW-A46): a message to it in the person's mail app.
    WriteToSupport,
}

pub struct Ui {
    source: Box<dyn SignInSource>,
    pub screen: Screen,
    shared: Option<SharedSession>,
    pub email: TextEdit,
    pub password: TextEdit,
    /// The password round trips and the vault unlock are running (DW-A18).
    pub busy: bool,
    pub problem: Option<SignInProblem>,
    failures: u8,
    /// The Windows Hello prompt is up and its answer has not come.
    hello_waiting: bool,
    request: Option<DeviceRequest>,
    seconds_left: u32,
    /// The waiting request's connection dropped (DW-A07: the pill reads "Reconnecting…").
    reconnecting: bool,
    /// The pairing code on the clipboard, while this screen put it there.
    code_copy: Option<SecretCopy>,
    /// The element that takes keyboard focus once this change is on screen.
    focus: Option<&'static str>,
    intents: Vec<Intent>,
    /// The password was right and the account is deleted (DW-A63).
    deleted: bool,
    /// The six digits of the authenticator code: ONE secure field, made in `Ui::new`.
    pub code: TextEdit,
    /// The eight digits of a backup code: secure, shown as typed (DW-A28).
    pub backup_code: TextEdit,
    verifying: bool,
    code_problem: Option<TwoFactorProblem>,
    /// The twelve words: secure fields, shown as typed (DW-A23), made in `Ui::new`.
    pub words: [TextEdit; 12],
    restoring: bool,
    phrase_problem: Option<PhraseProblem>,
    /// Windows' passkey dialog is up and its answer has not come (DW-A29).
    passkey_waiting: bool,
    unlocking: bool,
    unlock_problem: Option<SignInProblem>,
    /// The field a scene draws holding the keyboard (`steps::Pictured`).
    pictured: Option<Pictured>,
}

impl Ui {
    pub fn new(source: Box<dyn SignInSource>) -> Self {
        let shared = source.shared_session();
        Ui {
            // The first screen is skipped when another Beebeeb app is signed in here (DW-A01's rule).
            screen: if shared.is_some() {
                Screen::SignIn
            } else {
                Screen::Welcome
            },
            source,
            shared,
            email: TextEdit::new(""),
            // The one place the password field is made. Scenes and tests fill THIS field.
            password: TextEdit::new("").secure(),
            busy: false,
            problem: None,
            failures: 0,
            hello_waiting: false,
            request: None,
            seconds_left: 0,
            reconnecting: false,
            code_copy: None,
            focus: None,
            intents: Vec::new(),
            deleted: false,
            // The one place each of these secret fields is made. Scenes and tests fill THESE.
            code: TextEdit::new("").secure(),
            backup_code: TextEdit::new("").secure().revealed(true),
            verifying: false,
            code_problem: None,
            words: std::array::from_fn(|_| TextEdit::new("").secure().revealed(true)),
            restoring: false,
            phrase_problem: None,
            passkey_waiting: false,
            unlocking: false,
            unlock_problem: None,
            pictured: None,
        }
    }

    /// The account of the password just proven is deleted (DW-A63).
    pub fn deleted(&self) -> bool {
        self.deleted
    }

    pub fn pictured(&self) -> Option<Pictured> {
        self.pictured
    }

    /// What the shell should open, oldest first.
    #[cfg_attr(not(test), allow(dead_code))] // waits for its reader in the shell (PR body)
    pub fn take_intents(&mut self) -> Vec<Intent> {
        std::mem::take(&mut self.intents)
    }

    pub fn shared(&self) -> Option<&SharedSession> {
        self.shared.as_ref()
    }

    pub fn request(&self) -> Option<&DeviceRequest> {
        self.request.as_ref()
    }

    pub fn seconds_left(&self) -> u32 {
        self.seconds_left
    }

    pub fn reconnecting(&self) -> bool {
        self.reconnecting
    }

    /// The seconds left of the per-network wait (DW-A21), while it runs.
    pub fn retry_wait(&self) -> Option<u32> {
        match self.problem {
            Some(SignInProblem::RateLimited { seconds }) if seconds > 0 => Some(seconds),
            _ => None,
        }
    }

    /// Sign in is possible once the email looks like an address and a password is typed (DW-A02).
    pub fn can_submit(&self) -> bool {
        let email = self.email.text.trim();
        let looks_like_address = email
            .split_once('@')
            .is_some_and(|(name, host)| !name.is_empty() && host.contains('.') && !host.ends_with('.'));
        looks_like_address
            && !self.password.text.is_empty()
            && !self.busy
            && !self.passkey_waiting
            && self.retry_wait().is_none()
    }

    fn go(&mut self, intent: Intent) {
        self.intents.push(intent);
    }

    /// A link out of the group (create an account). Off while a sign-in runs: "Back and the
    /// other paths are disabled while it runs" (DW-A18).
    pub fn leave(&mut self, intent: Intent) {
        if self.busy || self.passkey_waiting {
            return;
        }
        self.go(intent);
    }

    /// "Open Drive" on DW-A08.
    pub fn open_drive(&mut self) {
        if matches!(self.screen, Screen::Approved { .. }) {
            self.go(Intent::Next(NextAfterSignIn::Drive));
        }
    }

    /// Whichever wait this screen holds is over: the prompt is dismissed, the request closed,
    /// the code forgotten here and on the clipboard.
    fn drop_waits(&mut self) {
        if self.hello_waiting {
            self.source.cancel_hello();
            self.hello_waiting = false;
        }
        if matches!(self.screen, Screen::Device { .. }) {
            // Leaving the screen closes the socket and forgets the code (DW-A07).
            self.source.cancel_device_request();
        }
        self.request = None;
        self.reconnecting = false;
        if let Some(copy) = self.code_copy.take() {
            copy.clear_now();
        }
        self.drop_steps();
    }

    pub fn open_sign_in(&mut self) {
        if self.busy {
            return;
        }
        // A password typed to unlock after a passkey is not the sign-in form's.
        if self.screen == Screen::UnlockVault {
            self.password.clear();
        }
        self.drop_waits();
        self.screen = Screen::SignIn;
    }

    /// The Sign in button and Enter.
    pub fn submit(&mut self) {
        if !self.can_submit() {
            return;
        }
        self.busy = true;
        self.problem = None;
        self.deleted = false;
        self.source.begin_sign_in(self.email.text.trim(), &self.password.text);
        self.pump();
    }

    pub fn toggle_password(&mut self) {
        let shown = self.password.is_revealed();
        self.password.set_revealed(!shown);
    }

    /// The Continue as card's button, and "Try again": Windows Hello is asked every time.
    pub fn continue_as(&mut self) {
        // One prompt at a time: a second press while Windows is asking does nothing.
        if self.busy || self.shared.is_none() || self.hello_waiting || self.passkey_waiting {
            return;
        }
        self.screen = Screen::ContinueAs(None);
        self.hello_waiting = true;
        self.source.begin_hello();
        self.pump();
    }

    /// "Sign in with another device", "Ask again" and "Get a new code".
    pub fn ask_other_device(&mut self) {
        if self.busy || self.passkey_waiting {
            return;
        }
        self.drop_waits();
        self.seconds_left = 0;
        self.screen = Screen::Device { expired: false };
        self.source.begin_device_request();
        self.pump();
    }

    /// Take every answer the engine has ready. `true`: at least one arrived.
    pub fn pump(&mut self) -> bool {
        let mut any = false;
        while let Some(event) = self.source.poll() {
            self.apply(event);
            any = true;
        }
        any
    }

    /// Hand the focus this change asked for to Rune.
    fn settle(&mut self, tx: &mut Tx) {
        if let Some(key) = self.focus.take() {
            tx.focus(key, tx.focus_visible());
        }
    }

    /// Ctrl+C on the screen with the code (DW-A07): the code goes to the clipboard, kept out
    /// of clipboard histories, and is cleared when the code expires or the screen is left.
    fn copy_code(&mut self, tx: &mut Tx) -> bool {
        if self.screen != (Screen::Device { expired: false }) {
            return false;
        }
        let Some(request) = &self.request else { return false };
        let life = Duration::from_secs(u64::from(self.seconds_left.max(1)));
        self.code_copy = Some(tx.copy_secret(request.code.clone(), life));
        true
    }

    fn apply(&mut self, event: SignInEvent) {
        let waiting_for_device = self.screen == Screen::Device { expired: false };
        match event {
            // An answer nobody is waiting for (a screen already left, a request already closed)
            // changes nothing: each arm below names the wait it answers.
            SignInEvent::SignedIn(Ok(next)) if self.busy => {
                self.busy = false;
                self.failures = 0;
                self.problem = None;
                // The password did its work: it does not stay in memory behind the next screens.
                self.password.clear();
                self.proceed(next);
            }
            SignInEvent::SignedIn(Err(problem)) if self.busy => {
                self.busy = false;
                if problem == SignInProblem::Invalid {
                    self.failures = self.failures.saturating_add(1);
                }
                // The lock is invisible on the wire: after the fifth wrong try here, explain it.
                self.problem = Some(
                    if problem == SignInProblem::Invalid && self.failures >= LOCK_HINT_AFTER {
                        SignInProblem::LockedHint
                    } else {
                        problem
                    },
                );
                // "The field with the problem keeps focus" (DW-A18 to DW-A22). The button that was
                // pressed took no focus while it ran, so focus is put back: in the password
                // field, where Enter tries again (offline too: a press re-checks the connection).
                self.focus = Some(PASSWORD_KEY);
            }
            SignInEvent::AccountDeleted if self.busy => {
                // Said plainly, once, under the field whose password was right (DW-A63). The
                // form stays as it is: Sign in tries again.
                self.busy = false;
                self.problem = None;
                self.deleted = true;
                self.focus = Some(PASSWORD_KEY);
            }
            SignInEvent::RetryAfter { seconds } => {
                if let Some(SignInProblem::RateLimited { .. }) = self.problem {
                    // The wait is over: the message goes and Sign in works again (DW-A21).
                    self.problem = (seconds > 0).then_some(SignInProblem::RateLimited { seconds });
                }
            }
            SignInEvent::Hello(outcome) if self.hello_waiting && self.screen == Screen::ContinueAs(None) => {
                self.hello_waiting = false;
                match outcome {
                    HelloOutcome::Succeeded(next) => self.proceed(next),
                    HelloOutcome::NotRecognized => {
                        self.screen = Screen::ContinueAs(Some(HelloFailure::NotRecognized));
                        // "Focus returns to Try again" (DW-A05).
                        self.focus = Some(FIRST_BUTTON_KEY);
                    }
                    HelloOutcome::Cancelled => {
                        self.screen = Screen::ContinueAs(Some(HelloFailure::Cancelled));
                        self.focus = Some(FIRST_BUTTON_KEY);
                    }
                }
            }
            SignInEvent::DeviceRequested(Ok(request)) if waiting_for_device && self.request.is_none() => {
                self.seconds_left = request.seconds_left;
                self.request = Some(request);
            }
            SignInEvent::DeviceRequested(Err(problem)) if waiting_for_device && self.request.is_none() => {
                self.drop_waits();
                self.problem = Some(problem);
                self.screen = Screen::SignIn;
            }
            SignInEvent::DeviceTick { seconds_left } if waiting_for_device => self.seconds_left = seconds_left,
            SignInEvent::DeviceConnection { connected } if waiting_for_device => self.reconnecting = !connected,
            SignInEvent::Device(status) if waiting_for_device => {
                self.reconnecting = false;
                match status {
                    DeviceStatus::Approved { by } => {
                        self.screen = Screen::Approved { by };
                        // "Focus moves to the heading" (DW-A08).
                        self.focus = Some(TITLE_KEY);
                    }
                    DeviceStatus::KeysNotReceived { .. } => {
                        self.screen = Screen::KeysNotReceived;
                        self.focus = Some(TITLE_KEY);
                    }
                    DeviceStatus::Denied { by } => self.screen = Screen::Denied { by },
                    DeviceStatus::Expired => {
                        self.seconds_left = 0;
                        self.screen = Screen::Device { expired: true };
                    }
                }
                // The request is over either way: its code is spent, here and on the clipboard.
                if let Some(copy) = self.code_copy.take() {
                    copy.clear_now();
                }
            }
            // The steps after the password (`steps.rs`), or an answer of a screen already left.
            other => self.apply_step(other),
        }
    }
}

fn ui(s: &mut State) -> Option<&mut Ui> {
    s.signin.as_mut()
}

/// A control's handler (`El::on_press_tx`): `f` on the sign-in state, then the engine's ready
/// answers, then the focus the change asked for.
pub fn act(f: fn(&mut Ui)) -> impl Fn(&mut State, &mut Tx) + 'static {
    move |s: &mut State, tx: &mut Tx| {
        if let Some(ui) = ui(s) {
            ui.pictured = None;
            f(ui);
            ui.pump();
            ui.settle(tx);
        }
    }
}

/// The state a signed-out window starts from, until the engine lane passes the real source:
/// the fixture with no other Beebeeb app signed in, so no person's name is ever invented.
pub fn start() -> Option<Ui> {
    Some(Ui::new(Box::new(FixtureSignInSource::new(Script {
        shared_session: false,
        ..Script::default()
    }))))
}

/// The registry's line for `FrameGroup::SignIn`: the whole first-run window.
pub fn frame(s: &State, cx: &mut Cx<State>) -> El {
    match &s.signin {
        Some(ui) => view(ui, cx),
        None => view::closed(cx),
    }
}

/// The engine's answers that arrive between presses (a tick, the other device's answer, a
/// sign-in that took a second). `true`: the view must run again. For `shell::host::poll`.
#[cfg_attr(not(test), allow(dead_code))] // waits for its one line in `shell/host.rs` (PR body)
pub fn poll(s: &mut State, tx: &mut Tx) -> bool {
    let Some(ui) = ui(s) else { return false };
    let changed = ui.pump();
    ui.settle(tx);
    changed
}

/// The window lost or got focus. Leaving it hides a password that was shown, so the app
/// switcher's picture of the window holds bullets (DW-A02). For `App::on_focus` in the shell.
#[cfg_attr(not(test), allow(dead_code))] // waits for its one line in `shell/mod.rs` (PR body)
pub fn on_window_focus(s: &mut State, focused: bool) {
    if let (false, Some(ui)) = (focused, ui(s))
        && ui.password.is_revealed()
    {
        ui.password.set_revealed(false);
    }
}

/// Escape goes back where a screen has a back link. Return is Rune's: it presses the screen's
/// default button (Sign in, from either field), and Space presses the focused control.
///
/// Ctrl+C on the screen with the code copies the code (DW-A07).
pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(ui) = ui(s) else { return false };
    if ev.key == Key::Escape {
        // Escape goes where the screen's back link goes.
        match ui.screen {
            Screen::Welcome | Screen::SignIn | Screen::Approved { .. } => return false,
            Screen::NoPhrase => ui.lost_phrase(false),
            Screen::CannotRecover => ui.cannot_recover(false),
            _ => ui.open_sign_in(),
        }
        ui.settle(tx);
        return true;
    }
    if ev.key == Key::Command('c') && !ev.mods.alt && !ev.mods.shift {
        return ui.copy_code(tx);
    }
    false
}

// ---------------------------------------------------------------------------
// Scenes: each board frame, reached through the same calls a person's presses make
// ---------------------------------------------------------------------------

fn fixture(script: Script) -> Ui {
    let mut ui = Ui::new(Box::new(FixtureSignInSource::new(script)));
    // The board draws the form filled in. The fixture's password is fourteen x's, never a real one.
    ui.email.replace_text(board::EMAIL);
    ui.password.replace_text("x".repeat(board::PASSWORD_LEN));
    ui
}

fn failing(problem: SignInProblem) -> Ui {
    let mut ui = fixture(Script {
        sign_in: SignInAnswer::Problem(problem),
        ..Script::default()
    });
    ui.submit();
    ui
}

/// The state of one board frame, by its id. `None`: not a frame of this group (or not built).
pub fn scene(frame: &str) -> Option<Ui> {
    let no_session = || Script {
        shared_session: false,
        ..Script::default()
    };
    let hello = |outcome| {
        let mut ui = fixture(Script {
            hello: outcome,
            hold: outcome == HelloOutcome::Succeeded(NextAfterSignIn::Drive),
            ..Script::default()
        });
        ui.continue_as();
        ui
    };
    let device = |status: Option<DeviceStatus>| {
        let mut ui = fixture(Script {
            device: status,
            ..Script::default()
        });
        ui.ask_other_device();
        ui
    };
    Some(match frame {
        "DW-A01" => fixture(no_session()),
        "DW-A02" => fixture(Script::default()),
        "DW-A03" => {
            let mut ui = fixture(no_session());
            ui.open_sign_in();
            ui
        }
        // The system's Windows Hello prompt is up: no answer yet.
        "DW-A04" => hello(HelloOutcome::Succeeded(NextAfterSignIn::Drive)),
        "DW-A05" => hello(HelloOutcome::NotRecognized),
        "DW-A06" => hello(HelloOutcome::Cancelled),
        "DW-A07" => device(None),
        "DW-A08" => device(Some(DeviceStatus::Approved {
            by: board::OTHER_DEVICE.into(),
        })),
        "DW-A09" => device(Some(DeviceStatus::Denied {
            by: board::OTHER_DEVICE.into(),
        })),
        "DW-A10" => device(Some(DeviceStatus::Expired)),
        "DW-A18" => {
            let mut ui = fixture(Script {
                hold: true,
                ..Script::default()
            });
            ui.submit();
            ui
        }
        "DW-A19" => failing(SignInProblem::Invalid),
        "DW-A20" => failing(SignInProblem::Offline),
        "DW-A21" => failing(SignInProblem::RateLimited { seconds: 30 }),
        "DW-A22" => failing(SignInProblem::LockedHint),
        "DW-A63" => {
            let mut ui = fixture(Script {
                sign_in: SignInAnswer::AccountDeleted,
                ..Script::default()
            });
            ui.submit();
            ui
        }
        // Four digits typed: the fifth box is next.
        "DW-A26" => {
            let mut ui = two_factor(None);
            ui.code.replace_text(&board::TOTP[..4]);
            ui
        }
        "DW-A27" => {
            let mut ui = two_factor(Some(TwoFactorProblem::WrongCode));
            ui.code.replace_text(board::TOTP);
            ui.code_changed();
            ui
        }
        "DW-A28" => {
            let mut ui = two_factor(None);
            ui.switch_code(true);
            ui.backup_code.replace_text(board::BACKUP_CODE);
            ui.pictured = Some(Pictured::BackupCode);
            ui
        }
        // Five words typed: the sixth box holds the keyboard.
        "DW-A23" => {
            let mut ui = phrase(&board::WORDS[..5]);
            ui.pictured = Some(Pictured::Word(5));
            ui
        }
        "DW-A24" => {
            let mut words = board::WORDS;
            words[2] = board::WRONG_WORD;
            let mut ui = phrase(&words);
            ui.restore();
            ui
        }
        "DW-A25" => {
            let mut ui = phrase(&[]);
            ui.lost_phrase(true);
            ui
        }
        // Windows' passkey dialog is up: no answer yet.
        "DW-A29" => {
            let mut ui = fixture(Script {
                hold: true,
                ..no_session()
            });
            ui.open_sign_in();
            ui.use_passkey();
            ui
        }
        "DW-A30" => {
            let mut ui = fixture(no_session());
            ui.open_sign_in();
            ui.use_passkey();
            ui.password.replace_text("x".repeat(board::PASSWORD_LEN));
            ui.pictured = Some(Pictured::VaultPassword);
            ui
        }
        "DW-A45" => {
            let mut ui = fixture(Script::default());
            ui.forgot();
            ui
        }
        "DW-A46" => {
            let mut ui = fixture(Script::default());
            ui.forgot();
            ui.cannot_recover(true);
            ui
        }
        _ => return None,
    })
}

/// The password was right and a code is asked for (DW-A26); `problem` is what the code gets.
fn two_factor(problem: Option<TwoFactorProblem>) -> Ui {
    let mut ui = fixture(Script {
        two_factor_problem: problem,
        ..Script::default()
    });
    ui.submit();
    ui
}

/// The password was right and this PC holds no keys (DW-A23), with `words` typed.
fn phrase(words: &[&str]) -> Ui {
    let mut ui = fixture(Script {
        sign_in: SignInAnswer::Next(NextAfterSignIn::RecoveryPhrase),
        ..Script::default()
    });
    ui.submit();
    for (i, word) in words.iter().enumerate() {
        ui.words[i].replace_text(*word);
    }
    ui
}

/// The frames of this group whose scenes live here (`frames::scenes`); DW-A01 to DW-A22 are
/// named in `shell::SCENES`.
pub const FRAMES: &[(&str, fn(&mut State))] = &[
    ("DW-A23", |s| set_scene(s, "DW-A23")),
    ("DW-A24", |s| set_scene(s, "DW-A24")),
    ("DW-A25", |s| set_scene(s, "DW-A25")),
    ("DW-A26", |s| set_scene(s, "DW-A26")),
    ("DW-A27", |s| set_scene(s, "DW-A27")),
    ("DW-A28", |s| set_scene(s, "DW-A28")),
    ("DW-A29", |s| set_scene(s, "DW-A29")),
    ("DW-A30", |s| set_scene(s, "DW-A30")),
    ("DW-A45", |s| set_scene(s, "DW-A45")),
    ("DW-A46", |s| set_scene(s, "DW-A46")),
    ("DW-A63", |s| set_scene(s, "DW-A63")),
];

/// Put the window in a board frame's state (`shell/mod.rs` `SCENES`): signed out, so the
/// registry draws this group, and the frame's own state.
pub fn set_scene(s: &mut State, frame: &str) {
    host::dispatch(s, Event::Gate(GateState::SignedOut));
    s.signin = scene(frame);
}
