//! Sign-in, the gate and the first run (the board's `.au-mac` window, 960 x 640): what Drive
//! shows before Files. `FRAMES.md` beside this file lists every frame of the group and which
//! are built.
//!
//! [`Ui`] is the window's state; every control changes it through one of its methods, and the
//! engine's answers arrive through [`Ui::pump`] ([`source::SignInSource::poll`]). What leaves the
//! group (the web sign-up, recovery, the product's first step) is an [`Intent`] the shell takes
//! with [`Ui::take_intents`].
//!
//! Secrets: the password lives in a secure `TextEdit` (bullets, no accessibility text, wiped on
//! drop) and is handed to the source as a borrowed `&str`. Nothing here logs or formats it.

pub mod icons;
pub mod qr;
pub mod source;
mod view;

#[cfg(test)]
mod tests;

use rune::{Key, KeyEvent, TextEdit, Tx};

use crate::State;
use source::{
    DeviceRequest, DeviceStatus, FixtureSignInSource, HelloOutcome, NextAfterSignIn, Script, SharedSession,
    SignInAnswer, SignInEvent, SignInProblem, SignInSource, board,
};

pub use view::view;

/// The text inputs' keys (`Tx::focus`, tests).
pub const EMAIL_KEY: &str = "signin-email";
pub const PASSWORD_KEY: &str = "signin-password";

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
    /// DW-A09.
    Denied { by: String },
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
    /// Sign-up happens on the web (DW-A31 is not a desktop frame).
    CreateAccount,
    /// DW-A45.
    ForgotPassword,
    /// DW-A29.
    UsePasskey,
    /// A sign-in step succeeded: the frame `gate.rs` names for it (DW-0201, DW-A23, DW-A26).
    Next(NextAfterSignIn),
    /// DW-A63.
    AccountDeleted,
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
    request: Option<DeviceRequest>,
    seconds_left: u32,
    intents: Vec<Intent>,
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
            password: TextEdit::new("").secure(),
            busy: false,
            problem: None,
            failures: 0,
            request: None,
            seconds_left: 0,
            intents: Vec::new(),
        }
    }

    /// What the shell should open, oldest first.
    #[cfg_attr(not(test), allow(dead_code))] // the shell lane calls it once `src/shell/` is in the base
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

    /// Sign in is possible once the email looks like an address and a password is typed (DW-A02).
    pub fn can_submit(&self) -> bool {
        let email = self.email.text.trim();
        let looks_like_address = email
            .split_once('@')
            .is_some_and(|(name, host)| !name.is_empty() && host.contains('.') && !host.ends_with('.'));
        looks_like_address && !self.password.text.is_empty() && !self.busy
    }

    pub fn go(&mut self, intent: Intent) {
        self.intents.push(intent);
    }

    pub fn open_sign_in(&mut self) {
        if self.busy {
            return;
        }
        if matches!(self.screen, Screen::Device { .. }) {
            // Leaving the screen closes the socket and forgets the code (DW-A07).
            self.source.cancel_device_request();
            self.request = None;
        }
        self.screen = Screen::SignIn;
    }

    /// The Sign in button and Enter.
    pub fn submit(&mut self) {
        if !self.can_submit() {
            return;
        }
        self.busy = true;
        self.problem = None;
        self.source.begin_sign_in(self.email.text.trim(), &self.password.text);
        self.pump();
    }

    pub fn toggle_password(&mut self) {
        let shown = self.password.is_revealed();
        self.password.set_revealed(!shown);
    }

    /// The Continue as card's button, and "Try again": Windows Hello is asked every time.
    pub fn continue_as(&mut self) {
        if self.busy || self.shared.is_none() {
            return;
        }
        self.screen = Screen::ContinueAs(None);
        self.source.begin_hello();
        self.pump();
    }

    /// "Sign in with another device", "Ask again" and "Get a new code".
    pub fn ask_other_device(&mut self) {
        if self.busy {
            return;
        }
        self.request = None;
        self.seconds_left = 0;
        self.screen = Screen::Device { expired: false };
        self.source.begin_device_request();
        self.pump();
    }

    /// Take every answer the engine has ready.
    pub fn pump(&mut self) {
        while let Some(event) = self.source.poll() {
            self.apply(event);
        }
    }

    fn apply(&mut self, event: SignInEvent) {
        match event {
            SignInEvent::SignedIn(Ok(next)) => {
                self.busy = false;
                self.failures = 0;
                self.go(Intent::Next(next));
            }
            SignInEvent::SignedIn(Err(problem)) => {
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
            }
            SignInEvent::AccountDeleted => {
                self.busy = false;
                self.go(Intent::AccountDeleted);
            }
            SignInEvent::Hello(HelloOutcome::Succeeded(next)) => self.go(Intent::Next(next)),
            SignInEvent::Hello(HelloOutcome::NotRecognized) => {
                self.screen = Screen::ContinueAs(Some(HelloFailure::NotRecognized));
            }
            SignInEvent::Hello(HelloOutcome::Cancelled) => {
                self.screen = Screen::ContinueAs(Some(HelloFailure::Cancelled));
            }
            SignInEvent::DeviceRequested(Ok(request)) => {
                self.seconds_left = request.seconds_left;
                self.request = Some(request);
            }
            SignInEvent::DeviceRequested(Err(problem)) => {
                self.problem = Some(problem);
                self.screen = Screen::SignIn;
            }
            SignInEvent::DeviceTick { seconds_left } => self.seconds_left = seconds_left,
            SignInEvent::Device(status) if matches!(self.screen, Screen::Device { expired: false }) => match status {
                DeviceStatus::Approved { by } => self.screen = Screen::Approved { by },
                DeviceStatus::Denied { by } => self.screen = Screen::Denied { by },
                DeviceStatus::Expired => {
                    self.seconds_left = 0;
                    self.screen = Screen::Device { expired: true };
                }
            },
            // Answers of frames this group has not built yet, or of a screen already left.
            _ => {}
        }
    }
}

fn ui(s: &mut State) -> Option<&mut Ui> {
    s.signin.as_mut()
}

/// A control's handler: `f` on the sign-in state, then the engine's ready answers.
pub fn act(f: fn(&mut Ui)) -> impl Fn(&mut State) + 'static {
    move |s: &mut State| {
        if let Some(ui) = ui(s) {
            f(ui);
            ui.pump();
        }
    }
}

/// Escape goes back where a screen has a back link. Return is Rune's: it presses the screen's
/// default button (Sign in, from either field), and Space presses the focused control.
pub fn on_key(s: &mut State, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    let Some(ui) = ui(s) else { return false };
    let has_back = matches!(
        ui.screen,
        Screen::ContinueAs(_) | Screen::Device { .. } | Screen::Denied { .. }
    );
    if ev.key == Key::Escape && has_back {
        ui.open_sign_in();
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// Scenes: each board frame, reached through the same calls a person's presses make
// ---------------------------------------------------------------------------

fn fixture(script: Script) -> Ui {
    let mut ui = Ui::new(Box::new(FixtureSignInSource::new(script)));
    // The board draws the form filled in. The fixture's password is fourteen x's, never a real one.
    ui.email = TextEdit::new(board::EMAIL);
    ui.password = TextEdit::new("x".repeat(board::PASSWORD_LEN)).secure();
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
        _ => return None,
    })
}

/// Put the window in a board frame's state (`main.rs` `SCENES`).
pub fn set_scene(s: &mut State, frame: &str) {
    s.signin = scene(frame);
}
