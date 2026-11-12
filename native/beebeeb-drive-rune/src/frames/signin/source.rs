//! `SignInSource`: what the sign-in frames ask of the engine, and the board's fixture.
//!
//! The enum names mirror `src-tauri/src/auth_v2/gate.rs` (`GateState`, `NextAfterSignIn`,
//! `SignInProblem`), so the engine lane implements this trait without translating. The engine
//! works off the UI thread: every `begin_*` call returns at once, and its result arrives later
//! through [`SignInSource::poll`] as a [`SignInEvent`]. The fixture answers on the next `poll`.
//!
//! Secrets cross this trait as `&str` borrowed from a secure `TextEdit` (wiped on drop). An
//! implementation must not log, store or clone them beyond the call.

// The whole contract is written down, also the values and calls of frames that are listed as not
// built in `FRAMES.md`: the engine lane implements against this file.
#![allow(dead_code)]

use std::collections::VecDeque;

/// `auth_v2::gate::GateState`: which screen the window shows before Files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateState {
    Ready,
    /// DW-A02 / DW-A03.
    SignedOut,
    /// DW-0009.
    Locked,
    /// DW-A23.
    NeedsRecoveryPhrase,
    /// DW-0014.
    NoPlan,
    KeyStoreUnavailable,
}

/// `auth_v2::gate::NextAfterSignIn`: what follows a sign-in step that succeeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextAfterSignIn {
    /// The vault is unlocked: the product's own first step (DW-0201).
    Drive,
    /// The vault key is not on this PC: the 12 words, or another device (DW-A23).
    RecoveryPhrase,
    /// A code is needed first (DW-A26 / DW-A28).
    TwoFactor,
    Reauthenticated,
    AccountMismatch,
}

/// `auth_v2::gate::SignInProblem`: the sign-in form's error state (DW-A19 to DW-A22).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInProblem {
    /// DW-A19. One message for a wrong email and a wrong password.
    Invalid,
    /// DW-A22: the fifth wrong try in a row.
    LockedHint,
    /// DW-A20.
    Offline,
    /// DW-A21.
    RateLimited {
        seconds: u32,
    },
    StaleKey,
    Other,
}

/// The account another Beebeeb app on this device is signed in to (the Continue as card).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedSession {
    pub display_name: String,
    pub initials: String,
    pub email: String,
    /// The app that holds the session, as the card names it: "Drive".
    pub held_by: String,
}

/// A device-approval request this PC started (DW-A07).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRequest {
    /// The code the other device types: "K7QM-4WTD". Shown to the person; it is not a credential
    /// by itself (the other device still proves itself), but it never goes into a log.
    pub code: String,
    /// The QR code's dark modules, row by row. It opens the request and does not hold the code.
    pub qr: Vec<Vec<bool>>,
    pub seconds_left: u32,
    /// This PC's name as the other device will see it: "Anna-Laptop".
    pub asking_as: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelloOutcome {
    Succeeded(NextAfterSignIn),
    /// DW-A05.
    NotRecognized,
    /// DW-A06.
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceStatus {
    /// DW-A08. `by` is the approving device's name: "Anna's iPhone".
    Approved { by: String },
    /// DW-A09.
    Denied { by: String },
    /// DW-A10.
    Expired,
    /// DW-A08's error state: the other device approved, and the key payload did not decrypt
    /// here. Nothing was signed in.
    KeysNotReceived { by: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeKind {
    /// Six digits from the authenticator app (DW-A26).
    Authenticator,
    /// An eight-digit backup code (DW-A28).
    Backup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TwoFactorProblem {
    /// DW-A27.
    WrongCode,
    /// The partial sign-in ran out: back to the password.
    Expired,
    Offline,
    RateLimited,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhraseProblem {
    /// DW-A24 / DW-A48. `bad_word`: the first word that is not in the word list, if the engine
    /// can tell (a checksum failure cannot).
    WrongPhrase {
        bad_word: Option<usize>,
    },
    TooManyTries,
    Offline,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasskeyOutcome {
    /// DW-A30: the passkey proved the person; the vault still needs the password here.
    NeedsPassword,
    Succeeded(NextAfterSignIn),
    Cancelled,
    Unavailable,
}

/// What the strength meter under a new password shows (`.kit-strength`): how many of its four
/// bars are lit and the engine's words for it ("Strong."). The rules are the engine's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordStrength {
    /// 1 to 4.
    pub bars: u8,
    pub words: String,
}

/// What a `begin_*` call answers with, later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignInEvent {
    SignedIn(Result<NextAfterSignIn, SignInProblem>),
    /// The password was proven and the account turned out to be deleted (DW-A63). Never sent
    /// before the password is proven (`gate.rs`: the form must not say an account existed).
    AccountDeleted,
    Hello(HelloOutcome),
    DeviceRequested(Result<DeviceRequest, SignInProblem>),
    /// One second passed on a waiting request.
    DeviceTick {
        seconds_left: u32,
    },
    Device(DeviceStatus),
    /// The waiting request's socket dropped (`false`) or is back (`true`). The countdown goes
    /// on either way; the request is lost only when the code expires (DW-A07).
    DeviceConnection {
        connected: bool,
    },
    /// The wait after `SignInProblem::RateLimited`, once a second; `0` when it is over (DW-A21).
    RetryAfter {
        seconds: u32,
    },
    TwoFactor(Result<NextAfterSignIn, TwoFactorProblem>),
    PhraseRestored(Result<NextAfterSignIn, PhraseProblem>),
    Passkey(PasskeyOutcome),
    VaultUnlocked(Result<NextAfterSignIn, SignInProblem>),
    /// Recovery, step 1 (DW-A47): the phrase re-derived this account's key.
    RecoveryPhraseVerified(Result<(), PhraseProblem>),
    /// Recovery, last step (DW-A49).
    PasswordSet(Result<(), SignInProblem>),
}

pub trait SignInSource {
    fn gate(&self) -> GateState;
    /// `None`: no other Beebeeb app is signed in here, or Windows Hello is not set up (DW-A03).
    fn shared_session(&self) -> Option<SharedSession>;
    /// Engine: `auth_v2::sign_in` (OPAQUE login + vault unlock).
    fn begin_sign_in(&mut self, email: &str, password: &str);
    /// Engine: Windows Hello (`UserConsentVerifier`), then the shared credential store entry.
    fn begin_hello(&mut self);
    /// The person left while the Windows Hello prompt was up: dismiss it. No `Hello` event of
    /// that attempt may arrive after this call.
    fn cancel_hello(&mut self);
    /// Engine: `auth_v2::device_request_start`; ticks and the outcome follow as events.
    fn begin_device_request(&mut self);
    /// Engine: `auth_v2::device_request_cancel`. No event of that request may arrive after this call.
    fn cancel_device_request(&mut self);
    /// Engine: `auth_v2::verify_two_factor`.
    fn begin_two_factor(&mut self, code: &str, kind: CodeKind);
    /// Engine: `auth_v2::unlock_with_recovery_phrase`.
    fn begin_restore(&mut self, words: &[&str]);
    /// Engine: the system passkey prompt (WebAuthn), then `auth_v2::passkey_sign_in`.
    fn begin_passkey(&mut self);
    /// Engine: `auth_v2::unlock_vault` with the password, after a passkey.
    fn begin_unlock(&mut self, password: &str);
    /// Engine: `auth_v2::recovery_verify_phrase`.
    fn begin_recovery_verify(&mut self, email: &str, words: &[&str]);
    /// Engine: `auth_v2::recovery_set_password`.
    fn begin_set_password(&mut self, password: &str);
    /// The next finished answer, if any.
    fn poll(&mut self) -> Option<SignInEvent>;
    /// Added by lane f-signin-rest (a default, so an implementation written before it compiles):
    /// the person went back from a step after the password (the code, the twelve words, the
    /// vault password). The partial session is discarded (`auth_v2`: drop the partial token) and
    /// no `TwoFactor`, `PhraseRestored` or `VaultUnlocked` event of it may arrive after this call.
    fn discard_partial_sign_in(&mut self) {}
    /// Added by lane f-signin-rest (a default): how strong a password being chosen is, by the
    /// same rules as sign-up (DW-A49). Local and at once (it runs on every keystroke); `None`:
    /// no verdict, the meter stays empty.
    fn password_strength(&self, _password: &str) -> Option<PasswordStrength> {
        None
    }
    /// Added by lane f-signin-rest (a default): the person left while Windows' passkey dialog
    /// was up: dismiss it. No `Passkey` event of that attempt may arrive after this call.
    fn cancel_passkey(&mut self) {}
}

// ---------------------------------------------------------------------------
// The board's fixture
// ---------------------------------------------------------------------------

/// The board's sample values, in one place.
pub mod board {
    pub const NAME: &str = "Anna Visser";
    pub const INITIALS: &str = "AV";
    pub const EMAIL: &str = "anna@northwind.studio";
    pub const HELD_BY: &str = "Drive";
    pub const DEVICE: &str = "Anna-Laptop";
    pub const OTHER_DEVICE: &str = "Anna's iPhone";
    pub const CODE: &str = "K7QM-4WTD";
    pub const SECONDS_LEFT: u32 = 4 * 60 + 12;
    /// The length of the password the board draws (14 bullets). The fixture never holds a real one.
    pub const PASSWORD_LEN: usize = 14;
    pub const WORDS: [&str; 12] = [
        "harbor", "maple", "quiet", "lantern", "orbit", "copper", "meadow", "ribbon", "atlas", "velvet", "thunder",
        "pebble",
    ];
    /// DW-A24's third word.
    pub const WRONG_WORD: &str = "quite";
    pub const TOTP: &str = "482019";
    pub const BACKUP_CODE: &str = "48201395";
}

/// What the fixture answers to each call. A test (or a scene) sets the answers it wants.
#[derive(Debug, Clone)]
pub struct Script {
    pub gate: GateState,
    pub shared_session: bool,
    pub sign_in: SignInAnswer,
    pub hello: HelloOutcome,
    pub device: Option<DeviceStatus>,
    pub passkey: PasskeyOutcome,
    /// What a two-factor code gets instead of the check against the board's code (DW-A27 draws
    /// the board's own six digits as the wrong ones).
    pub two_factor_problem: Option<TwoFactorProblem>,
    /// `true`: answers wait until [`FixtureSignInSource::release`] (the board's in-flight frames).
    pub hold: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInAnswer {
    Next(NextAfterSignIn),
    Problem(SignInProblem),
    AccountDeleted,
}

impl Default for Script {
    fn default() -> Self {
        Script {
            gate: GateState::SignedOut,
            shared_session: true,
            sign_in: SignInAnswer::Next(NextAfterSignIn::TwoFactor),
            hello: HelloOutcome::Succeeded(NextAfterSignIn::Drive),
            device: None,
            passkey: PasskeyOutcome::NeedsPassword,
            two_factor_problem: None,
            hold: false,
        }
    }
}

/// The fixture: the board's account, answering as the board's flow does. No network, no secrets:
/// a password is right when it is [`board::PASSWORD_LEN`] characters long, a phrase when it is
/// the board's twelve words.
#[derive(Default)]
pub struct FixtureSignInSource {
    pub script: Script,
    queue: VecDeque<SignInEvent>,
    /// The calls made, by name: tests assert on them. Never the arguments.
    #[cfg(test)]
    pub calls: std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>,
}

impl FixtureSignInSource {
    pub fn new(script: Script) -> Self {
        FixtureSignInSource {
            script,
            ..Default::default()
        }
    }

    #[cfg(test)]
    fn called(&mut self, name: &'static str) {
        self.calls.borrow_mut().push(name);
    }

    #[cfg(not(test))]
    fn called(&mut self, _name: &'static str) {}

    /// Let held answers through.
    pub fn release(&mut self) {
        self.script.hold = false;
    }

    /// Queue an event as if the engine sent it (a tick, the other device's answer).
    pub fn push(&mut self, event: SignInEvent) {
        self.queue.push_back(event);
    }

    fn after_proof(&self) -> NextAfterSignIn {
        NextAfterSignIn::Drive
    }

    pub fn request() -> DeviceRequest {
        DeviceRequest {
            code: board::CODE.into(),
            qr: super::qr::board_modules(),
            seconds_left: board::SECONDS_LEFT,
            asking_as: board::DEVICE.into(),
        }
    }
}

impl SignInSource for FixtureSignInSource {
    fn gate(&self) -> GateState {
        self.script.gate
    }

    fn shared_session(&self) -> Option<SharedSession> {
        self.script.shared_session.then(|| SharedSession {
            display_name: board::NAME.into(),
            initials: board::INITIALS.into(),
            email: board::EMAIL.into(),
            held_by: board::HELD_BY.into(),
        })
    }

    fn begin_sign_in(&mut self, _email: &str, _password: &str) {
        self.called("sign_in");
        self.queue.push_back(match self.script.sign_in {
            SignInAnswer::Next(next) => SignInEvent::SignedIn(Ok(next)),
            SignInAnswer::Problem(problem) => SignInEvent::SignedIn(Err(problem)),
            SignInAnswer::AccountDeleted => SignInEvent::AccountDeleted,
        });
    }

    fn begin_hello(&mut self) {
        self.called("hello");
        self.queue.push_back(SignInEvent::Hello(self.script.hello));
    }

    fn cancel_hello(&mut self) {
        self.called("cancel_hello");
        self.queue.retain(|e| !matches!(e, SignInEvent::Hello(_)));
    }

    fn begin_device_request(&mut self) {
        self.called("device_request");
        self.queue.push_back(SignInEvent::DeviceRequested(Ok(Self::request())));
        // The scripted answer is for one request: a new code waits, as a real one does.
        if let Some(status) = self.script.device.take() {
            self.queue.push_back(SignInEvent::Device(status));
        }
    }

    fn cancel_device_request(&mut self) {
        self.called("cancel_device_request");
        self.queue.retain(|e| {
            !matches!(
                e,
                SignInEvent::Device(_)
                    | SignInEvent::DeviceTick { .. }
                    | SignInEvent::DeviceRequested(_)
                    | SignInEvent::DeviceConnection { .. }
            )
        });
    }

    fn begin_two_factor(&mut self, code: &str, kind: CodeKind) {
        self.called("two_factor");
        let right = match kind {
            CodeKind::Authenticator => board::TOTP,
            CodeKind::Backup => board::BACKUP_CODE,
        };
        self.queue
            .push_back(SignInEvent::TwoFactor(match self.script.two_factor_problem {
                Some(problem) => Err(problem),
                None if code == right => Ok(self.after_proof()),
                None => Err(TwoFactorProblem::WrongCode),
            }));
    }

    fn begin_restore(&mut self, words: &[&str]) {
        self.called("restore");
        self.queue.push_back(SignInEvent::PhraseRestored(
            check_phrase(words).map(|()| self.after_proof()),
        ));
    }

    fn begin_passkey(&mut self) {
        self.called("passkey");
        self.queue.push_back(SignInEvent::Passkey(self.script.passkey));
    }

    fn begin_unlock(&mut self, password: &str) {
        self.called("unlock");
        self.queue.push_back(SignInEvent::VaultUnlocked(
            if password.chars().count() == board::PASSWORD_LEN {
                Ok(self.after_proof())
            } else {
                Err(SignInProblem::Invalid)
            },
        ));
    }

    fn begin_recovery_verify(&mut self, _email: &str, words: &[&str]) {
        self.called("recovery_verify");
        self.queue
            .push_back(SignInEvent::RecoveryPhraseVerified(check_phrase(words)));
    }

    fn begin_set_password(&mut self, _password: &str) {
        self.called("set_password");
        self.queue.push_back(SignInEvent::PasswordSet(Ok(())));
    }

    fn poll(&mut self) -> Option<SignInEvent> {
        if self.script.hold {
            return None;
        }
        self.queue.pop_front()
    }

    fn discard_partial_sign_in(&mut self) {
        self.called("discard_partial_sign_in");
        self.queue.retain(|e| {
            !matches!(
                e,
                SignInEvent::TwoFactor(_) | SignInEvent::PhraseRestored(_) | SignInEvent::VaultUnlocked(_)
            )
        });
    }

    fn password_strength(&self, password: &str) -> Option<PasswordStrength> {
        // The board draws one verdict, for its own password: the fixture has no other.
        (password.chars().count() == board::PASSWORD_LEN).then(|| PasswordStrength {
            bars: 4,
            words: "Strong.".into(),
        })
    }

    fn cancel_passkey(&mut self) {
        self.called("cancel_passkey");
        self.queue.retain(|e| !matches!(e, SignInEvent::Passkey(_)));
    }
}

fn check_phrase(words: &[&str]) -> Result<(), PhraseProblem> {
    if words == board::WORDS {
        return Ok(());
    }
    let bad_word = words
        .iter()
        .zip(board::WORDS)
        .position(|(typed, right)| *typed != right);
    Err(PhraseProblem::WrongPhrase { bad_word })
}
