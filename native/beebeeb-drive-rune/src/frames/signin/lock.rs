//! The two lock levels in the window (core's contract `2026-11-13-two-level-lock-design.md`, section 7).
//!
//! The rules are core's, reached through the engine; nothing here opens a lock. This file is what the window
//! keeps while the session's gate is closed by a lock:
//!
//! - the SCREEN lock (DW-0009): Unlock asks for Windows Hello ([`SignInSource::begin_hello`]); its answer is a
//!   sentence under the button, never a silent nothing. When Windows Hello cannot answer, the account's password
//!   is offered: DW-A30's screen with the lock's words, in front of the lock screen.
//! - the VAULT lock: DW-A30's screen with the lock's words, then the code step when the account has one.
//!
//! Which of the two holds is the source's word ([`SignInSource::gate`]); the shell's gate says only "locked".
//! A lock that goes (the engine opened it, a sign-out) takes everything typed here with it.

use super::source::{GateState as SourceGate, HelloOutcome};
use super::{PASSWORD_KEY, Screen, Ui};

/// The key of the lock screen's Unlock button (`before_files`).
pub const UNLOCK_KEY: &str = "gate-btn-0";

/// Which lock holds the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Screen,
    Vault,
}

/// What Windows Hello's last answer leaves on the lock screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Said {
    /// Not recognised after Windows' own retries, or the sensor was busy.
    NotRecognized,
    /// No prompt could be shown on this PC.
    Unavailable,
}

impl Said {
    pub fn words(self) -> &'static str {
        match self {
            Said::NotRecognized => "Windows Hello didn't recognise you. Try again, or use your password.",
            Said::Unavailable => "Windows Hello isn't available on this PC. Use your password to unlock.",
        }
    }
}

#[derive(Default)]
pub(super) struct Lock {
    /// The lock as last followed; `None`: no lock holds the window.
    level: Option<Level>,
    /// The Windows Hello prompt of the lock screen is up and its answer has not come.
    hello_waiting: bool,
    said: Option<Said>,
    /// The password screen is in front of the lock screen (the screen lock's other way in).
    password: bool,
    /// Windows Hello was asked by itself once for this lock (DW-0009: "starts by itself when the window opens").
    auto_asked: bool,
}

impl Ui {
    /// The lock that holds the window, as last followed.
    pub fn lock_level(&self) -> Option<Level> {
        self.lock.level
    }

    /// The lock screen's sentence under its button.
    pub fn lock_said(&self) -> Option<Said> {
        self.lock.said
    }

    pub fn lock_hello_waiting(&self) -> bool {
        self.lock.hello_waiting
    }

    /// The password screen (and its code step) shows instead of the lock screen.
    pub fn lock_shows_password(&self) -> bool {
        match self.lock.level {
            Some(Level::Vault) => true,
            Some(Level::Screen) => self.lock.password,
            None => false,
        }
    }

    /// The lock's words for DW-A30's screen; `None`: the screen is the passkey step's, as the board draws it.
    pub fn lock_words(&self) -> Option<Level> {
        self.lock.level.filter(|_| self.lock_shows_password())
    }

    /// The shell's gate was read: `locked` while a lock closes it. The level is the source's. `true`: what the
    /// window shows changed.
    pub(super) fn follow_lock(&mut self, locked: bool) -> bool {
        let level = locked.then(|| match self.source.gate() {
            SourceGate::VaultClosed => Level::Vault,
            _ => Level::Screen,
        });
        let before = std::mem::replace(&mut self.lock.level, level);
        if before == level {
            return false;
        }
        if self.lock.hello_waiting {
            self.source.cancel_hello();
        }
        let on_password_step = matches!(self.screen, Screen::UnlockVault | Screen::TwoFactor { .. });
        match level {
            // A password that is on its way closes the vault before it opens it (the engine's strong unlock
            // from a locked screen): its screen stays, and its answer is still waited for.
            Some(Level::Vault) if before == Some(Level::Screen) && self.lock.password && on_password_step => {
                self.lock = Lock {
                    level,
                    ..Lock::default()
                };
            }
            Some(Level::Vault) => {
                self.forget_steps();
                self.lock = Lock {
                    level,
                    ..Lock::default()
                };
                self.screen = Screen::UnlockVault;
                self.focus = Some(PASSWORD_KEY);
            }
            Some(Level::Screen) | None => {
                self.forget_steps();
                self.lock = Lock {
                    level,
                    ..Lock::default()
                };
                self.screen = Screen::SignIn;
            }
        }
        true
    }

    /// Unlock on the lock screen, the flyout's "Unlock with Windows Hello", and the prompt that starts by itself.
    /// One prompt at a time: a press while one is up asks nothing.
    pub fn unlock_screen(&mut self) {
        if self.lock.level != Some(Level::Screen) || self.lock.password || self.lock.hello_waiting {
            return;
        }
        self.lock.hello_waiting = true;
        self.lock.said = None;
        self.source.begin_hello();
        self.pump();
    }

    /// The window came to the front while the screen is locked: Windows Hello starts by itself, once per lock.
    pub(super) fn lock_window_focused(&mut self) {
        if self.lock.level == Some(Level::Screen) && !self.lock.auto_asked {
            self.lock.auto_asked = true;
            self.unlock_screen();
        }
    }

    /// "Use your password" on the lock screen: the password screen comes in front. A prompt that is up goes.
    pub fn lock_use_password(&mut self) {
        if self.lock.level != Some(Level::Screen) || self.lock.password {
            return;
        }
        if std::mem::take(&mut self.lock.hello_waiting) {
            self.source.cancel_hello();
        }
        self.lock.password = true;
        self.password.clear();
        self.unlock_problem = None;
        self.screen = Screen::UnlockVault;
        self.focus = Some(PASSWORD_KEY);
    }

    /// Back and Escape while a lock holds the window: from the code to the password, from the screen lock's
    /// password to the lock screen. Never to the sign-in form (an account is signed in: the form could only
    /// refuse). A step that is on its way is not left. `false`: no lock holds the window.
    pub(super) fn lock_back(&mut self) -> bool {
        let Some(level) = self.lock.level else { return false };
        let on_code = matches!(self.screen, Screen::TwoFactor { .. });
        // A closed vault's password has nothing behind it; a step that is on its way is not left.
        if self.step_running() || !self.lock_shows_password() || (level == Level::Vault && !on_code) {
            return true;
        }
        // A proven password that waits for its code is forgotten, and what was typed goes.
        self.source.discard_partial_sign_in();
        self.password.clear();
        self.code.clear();
        self.backup_code.clear();
        self.unlock_problem = None;
        self.code_problem = None;
        if on_code {
            self.screen = Screen::UnlockVault;
            self.focus = Some(PASSWORD_KEY);
        } else {
            self.lock.password = false;
            self.screen = Screen::SignIn;
            self.focus = Some(UNLOCK_KEY);
        }
        true
    }

    /// Windows Hello answered the lock screen's prompt. A success moves nothing here: the engine opened the
    /// lock, and the gate says so.
    pub(super) fn lock_hello_answered(&mut self, outcome: HelloOutcome) {
        self.lock.hello_waiting = false;
        self.lock.said = match outcome {
            HelloOutcome::Succeeded(_) | HelloOutcome::Cancelled => None,
            HelloOutcome::NotRecognized => Some(Said::NotRecognized),
            HelloOutcome::Unavailable => Some(Said::Unavailable),
        };
        if self.lock.said.is_some() {
            self.focus = Some(UNLOCK_KEY);
        }
    }
}

/// Scenes of the lock's states the board has no frame for (core's contract, section 7: "no frame yet"), for a
/// render to look at: `--scene LOCK-...`. Fixture data only; none has a golden, so none is gated.
pub const SCENES: &[(&str, fn(&mut crate::State))] = &[
    ("LOCK-hello-unavailable", |s| lock_scene(s, false, false)),
    ("LOCK-screen-password", |s| lock_scene(s, false, true)),
    ("LOCK-vault-closed", |s| lock_scene(s, true, false)),
];

fn lock_scene(s: &mut crate::State, vault: bool, password: bool) {
    use super::source::{FixtureSignInSource, Script};
    crate::shell::host::dispatch(s, crate::shell::app::Event::Gate(crate::shell::app::GateState::Locked));
    let mut ui = Ui::new(Box::new(FixtureSignInSource::new(Script {
        gate: if vault {
            SourceGate::VaultClosed
        } else {
            SourceGate::Locked
        },
        hello: HelloOutcome::Unavailable,
        ..Script::default()
    })));
    ui.follow_lock(true);
    if !vault {
        ui.unlock_screen();
    }
    if password {
        ui.lock_use_password();
    }
    s.signin = Some(ui);
}
