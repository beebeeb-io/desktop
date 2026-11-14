//! The Passkeys pane (DW-A73 to DW-A76) on the real engine: [`PasskeyCalls`], the part of `EngineAccountSource`
//! behind `passkeys`, `add_passkey_confirmed`, `remove_passkey_confirmed`, and the sign-in screen's "Use a passkey".
//!
//! - The list is the engine's read of Beebeeb's own list ([`Engine::passkeys`]), read when the pane opens (and again
//!   when it is older than [`FRESH_FOR`]) and after every change: nothing is drawn before a change was made, a
//!   change that was refused says so (with its kind), and the rows are read again whatever the answer was. A read
//!   that failed never stamps the pane as fresh, and a pane whose re-read failed does not draw rows it cannot vouch
//!   for: it offers Try again.
//! - Adding and removing take the account password (core's step-up, which the server requires): the password is
//!   moved once, into a wiping buffer, to a worker, and into the engine call. Both are bound to the session AND the
//!   account of the press; an answer that lands under another is dropped with its toast.
//! - The system's passkey dialog is the engine's WebAuthn call ([`Engine::add_passkey`]) on a thread of its own,
//!   owned by this process's window ([`host_window`]); the window is never frozen behind it.
//! - Rename: Beebeeb has no route for it, so the menu says nothing about it ([`PasskeyCalls::can_rename`]).
//! - Whether a passkey can unlock the data on a new device is the server's word (`prf`); a list that does not say
//!   draws "Signs you in", never "Sign-in only".
//!
//! Must not: log a password, a passkey's name or id, or a server's words; draw a row of an earlier session.

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::account_api::Stamped;
use beebeeb_engine::app::passkeys_api::{PasskeyAdded, PasskeyProblem, PasskeyRow};
use beebeeb_engine::app::{CredentialStores, Engine};
use chrono::{DateTime, Utc};
use zeroize::Zeroizing;

use super::log;
use super::stamp::{Stamp, stamp};
use crate::frames::account::Pane;
use crate::frames::account::source::{PaneState, Passkey, Password};
use crate::frames::signin::source::PasskeyOutcome;

/// What the pane holds is not read again, when it opens, for this long.
pub const FRESH_FOR: Duration = Duration::from_secs(30);
/// How long a toast of this part stays.
pub const STAYS: Duration = Duration::from_secs(6);

pub const ADDED: &str = "Passkey added.";
pub const REMOVED: &str = "Passkey removed.";
const NOT_ADDED: &str = "The passkey was not added. Nothing was changed.";
const NOT_REMOVED: &str = "The passkey was not removed. Nothing was changed.";
const WRONG: &str = "Incorrect password. Nothing was changed.";
const OFFLINE: &str = "Beebeeb can't be reached. Nothing was changed.";
const CANCELLED: &str = "Adding the passkey was cancelled. Nothing was changed.";
const SLOW_DOWN: &str = "Too many tries. Wait a moment, then try again. Nothing was changed.";
const NO_DIALOG: &str = "Passkeys are not available on this PC. Nothing was changed.";

/// What a change asked of the engine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Change {
    Add,
    Remove,
}

/// One answer from a worker.
enum Answer {
    Read {
        number: u64,
        pressed: Stamp,
        result: Result<Stamped<Vec<PasskeyRow>>, PasskeyProblem>,
    },
    Changed {
        pressed: Stamp,
        change: Change,
        result: Result<Option<PasskeyAdded>, PasskeyProblem>,
    },
}

/// The rows, for one session of one account.
struct Held {
    pressed: Stamp,
    rows: Vec<PasskeyRow>,
}

/// A toast and when it first showed.
struct Said {
    session: SessionGeneration,
    words: &'static str,
    failed: bool,
    at: Instant,
}

/// See the module's doc.
pub struct PasskeyCalls<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    held: Option<Held>,
    /// The read on its way, by number.
    reading: Option<u64>,
    reads: u64,
    /// When the last read answered with rows (a failed read never stamps this).
    read_at: Option<Instant>,
    /// The session the last read failed for.
    failed_for: Option<SessionGeneration>,
    /// A change is on its way (one at a time: the system dialog is one).
    changing: Option<Stamp>,
    said: Option<Said>,
    showing: bool,
    dirty: bool,
    answers: mpsc::Sender<Answer>,
    inbox: mpsc::Receiver<Answer>,
}

fn kind_words(change: Change, problem: PasskeyProblem) -> &'static str {
    match problem {
        PasskeyProblem::WrongPassword => WRONG,
        PasskeyProblem::Offline => OFFLINE,
        PasskeyProblem::Cancelled => CANCELLED,
        PasskeyProblem::RateLimited => SLOW_DOWN,
        PasskeyProblem::Unavailable => NO_DIALOG,
        _ => match change {
            Change::Add => NOT_ADDED,
            Change::Remove => NOT_REMOVED,
        },
    }
}

/// "28 Oct": the day a passkey was added, from the server's time; empty when it does not read.
fn day_of(created_at: &str) -> String {
    DateTime::parse_from_rfc3339(created_at)
        .map(|at| at.with_timezone(&Utc).format("%-d %b").to_string())
        .unwrap_or_default()
}

fn passkey_of(row: &PasskeyRow) -> Passkey {
    Passkey {
        id: row.id.clone(),
        name: row.name.clone(),
        added: day_of(&row.created_at),
        can_unlock: row.prf == Some(true),
        unlock_known: row.prf.is_some(),
    }
}

/// The name a new passkey is given: the person is not asked (the system's dialog says where it is saved), and
/// Beebeeb's own name for it cannot be changed (no route), so it says what it is and when it was made.
fn new_name() -> String {
    format!("Windows PC, {}", chrono::Local::now().format("%-d %b %Y"))
}

impl<C: CredentialStores + 'static> PasskeyCalls<C> {
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            engine,
            held: None,
            reading: None,
            reads: 0,
            read_at: None,
            failed_for: None,
            changing: None,
            said: None,
            showing: false,
            dirty: false,
            answers,
            inbox,
        }
    }

    fn open(&self) -> bool {
        self.engine.has_session() && !self.engine.account_session_refused()
    }

    fn is_now(&self, pressed: &Stamp) -> bool {
        self.open() && *pressed == stamp(&self.engine)
    }

    fn current(&self) -> Option<&Held> {
        self.held.as_ref().filter(|held| self.is_now(&held.pressed))
    }

    pub fn pane_state(&self) -> PaneState {
        if !self.engine.has_session() {
            return PaneState::SignedOut;
        }
        if self.engine.account_session_refused() {
            return PaneState::SessionEnded;
        }
        if self.current().is_some() {
            PaneState::Ready
        } else if self.reading.is_none() && self.failed_for == Some(self.engine.session_turn()) {
            PaneState::Failed
        } else {
            PaneState::Loading
        }
    }

    /// The pane opened: read when nothing fresh is held.
    pub fn opened(&mut self, pane: Pane) {
        self.showing = pane == Pane::Passkeys;
        if !self.showing || !self.open() {
            return;
        }
        let fresh = self.current().is_some() && self.read_at.is_some_and(|at| at.elapsed() < FRESH_FOR);
        if !fresh {
            self.read();
        }
    }

    pub fn closed(&mut self) {
        self.showing = false;
    }

    /// "Try again" on a pane that failed.
    pub fn reload(&mut self) {
        self.read();
    }

    /// The rows, as the pane draws them: only this session's, never a row of an earlier one.
    pub fn passkeys(&self) -> Vec<Passkey> {
        self.current()
            .map(|held| held.rows.iter().map(passkey_of).collect())
            .unwrap_or_default()
    }

    /// Whether this PC can show the system's passkey dialog.
    pub fn can_add(&self) -> bool {
        self.engine.passkeys_supported()
    }

    /// Beebeeb has no route to rename a passkey.
    pub fn can_rename(&self) -> bool {
        false
    }

    /// A read on a worker, stamped at the press. One at a time.
    fn read(&mut self) {
        if self.reading.is_some() || !self.open() {
            return;
        }
        self.reads += 1;
        let number = self.reads;
        let pressed = stamp(&self.engine);
        self.reading = Some(number);
        self.failed_for = None;
        let (engine, answers) = (self.engine.clone(), self.answers.clone());
        let started = std::thread::Builder::new().name("passkeys-read".into()).spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.passkeys()))
                .unwrap_or(Err(PasskeyProblem::Refused));
            let _ = answers.send(Answer::Read {
                number,
                pressed,
                result,
            });
            engine.wake_host();
        });
        if started.is_err() {
            log("passkeys: the read worker did not start");
            self.reading = None;
            self.failed_for = Some(self.engine.session_turn());
        }
    }

    /// "Add a passkey": the password is moved to a worker; the system dialog and the calls run there. `false`: nothing
    /// was started (no session, a change already on its way, no window to own the dialog).
    pub fn add(&mut self, password: Zeroizing<String>) -> bool {
        self.change(Change::Add, None, password)
    }

    /// "Remove" on one passkey, with the password.
    pub fn remove(&mut self, id: &str, password: Zeroizing<String>) -> bool {
        self.change(Change::Remove, Some(id.to_string()), password)
    }

    fn change(&mut self, change: Change, id: Option<String>, password: Zeroizing<String>) -> bool {
        // Read on the window's thread, at the press, before anything else.
        let pressed = stamp(&self.engine);
        if !self.open() || self.changing.is_some() {
            return false;
        }
        // The rows the press was made on are this session's: a press on rows of another session sends nothing.
        if !self.current().is_some_and(|held| held.pressed == pressed) {
            return false;
        }
        let hwnd = match change {
            Change::Add => match host_window() {
                Some(hwnd) => hwnd,
                None => {
                    self.say(pressed.0, NO_DIALOG, true);
                    return false;
                }
            },
            Change::Remove => 0,
        };
        self.changing = Some(pressed.clone());
        let (engine, answers) = (self.engine.clone(), self.answers.clone());
        let (session, mine) = (pressed.0, pressed.clone());
        let started = std::thread::Builder::new()
            .name("passkeys-change".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match change {
                    Change::Add => add_with_dialog(&engine, hwnd, session, password).map(Some),
                    Change::Remove => engine
                        .remove_passkey(session, password, id.as_deref().unwrap_or_default())
                        .map(|()| None),
                }))
                .unwrap_or(Err(PasskeyProblem::Refused));
                let _ = answers.send(Answer::Changed {
                    pressed: mine,
                    change,
                    result,
                });
                engine.wake_host();
            });
        if started.is_err() {
            log("passkeys: the change worker did not start");
            self.changing = None;
            self.say(pressed.0, kind_words(change, PasskeyProblem::Refused), true);
            return false;
        }
        self.dirty = true;
        true
    }

    fn say(&mut self, session: SessionGeneration, words: &'static str, failed: bool) {
        self.said = Some(Said {
            session,
            words,
            failed,
            at: Instant::now(),
        });
        self.dirty = true;
    }

    /// The window's poll. `changed`, `busy`, and how long until a toast goes.
    pub fn pump(&mut self) -> (bool, bool, Option<Duration>) {
        let mut changed = std::mem::take(&mut self.dirty);
        // The session or the account moved on: nothing of this part stays, no toast either.
        let moved = self.held.as_ref().is_some_and(|held| !self.is_now(&held.pressed))
            || self
                .said
                .as_ref()
                .is_some_and(|said| said.session != self.engine.session_turn())
            || self
                .changing
                .as_ref()
                .is_some_and(|pressed| *pressed != stamp(&self.engine));
        if moved {
            self.held = None;
            self.read_at = None;
            self.said = None;
            self.changing = None;
            changed = true;
        }
        while let Ok(answer) = self.inbox.try_recv() {
            self.take_in(answer);
            changed = true;
        }
        // Another sign-in or an unlock while the pane shows: read once, without a reopening.
        if self.showing
            && self.open()
            && self.reading.is_none()
            && self.current().is_none()
            && self.failed_for != Some(self.engine.session_turn())
        {
            self.read();
            changed = true;
        }
        let wake_in = self.said.as_ref().map(|said| STAYS.saturating_sub(said.at.elapsed()));
        if self.said.as_ref().is_some_and(|said| said.at.elapsed() >= STAYS) {
            self.said = None;
            changed = true;
        }
        (changed, self.reading.is_some() || self.changing.is_some(), wake_in)
    }

    fn take_in(&mut self, answer: Answer) {
        match answer {
            Answer::Read {
                number,
                pressed,
                result,
            } => {
                if self.reading != Some(number) {
                    return;
                }
                self.reading = None;
                // Dropped: the answer belongs to a session or an account that is not the one here now.
                if !self.is_now(&pressed) {
                    return;
                }
                match result {
                    Ok(stamped) if stamped.session == pressed.0 => {
                        self.held = Some(Held {
                            pressed,
                            rows: stamped.value,
                        });
                        self.read_at = Some(Instant::now());
                        self.failed_for = None;
                    }
                    _ => {
                        // Rows it cannot vouch for are not drawn: the pane offers Try again. Never fresh.
                        self.held = None;
                        self.read_at = None;
                        self.failed_for = Some(pressed.0);
                    }
                }
            }
            Answer::Changed {
                pressed,
                change,
                result,
            } => {
                if self.changing.as_ref() == Some(&pressed) {
                    self.changing = None;
                }
                if !self.is_now(&pressed) {
                    return;
                }
                let (words, failed) = match (change, &result) {
                    (Change::Add, Ok(_)) => (ADDED, false),
                    (Change::Remove, Ok(_)) => (REMOVED, false),
                    (change, Err(problem)) => (kind_words(change, *problem), true),
                };
                self.say(pressed.0, words, failed);
                // Send, then read again, whatever the answer was: a refused change may still have changed the list
                // (an add whose escrow failed), and the old rows are not vouched for any more.
                self.read_at = None;
                self.read();
            }
        }
    }

    /// The toast, when it is of this session and not over.
    pub fn notice(&self) -> Option<(&'static str, bool)> {
        self.said
            .as_ref()
            .filter(|said| said.session == self.engine.session_turn() && said.at.elapsed() < STAYS)
            .map(|said| (said.words, said.failed))
    }

    pub fn clear_notice(&mut self) {
        self.dirty |= self.said.take().is_some();
    }
}

/// The add, on the worker: the engine's call over the Windows dialog (`hwnd`), off Windows the refusal.
fn add_with_dialog<C: CredentialStores>(
    engine: &Engine<C>,
    hwnd: isize,
    session: SessionGeneration,
    password: Zeroizing<String>,
) -> Result<PasskeyAdded, PasskeyProblem> {
    #[cfg(windows)]
    {
        engine.add_passkey(hwnd, session, password, &new_name())
    }
    #[cfg(not(windows))]
    {
        let _ = (engine, hwnd, session, password, new_name());
        Err(PasskeyProblem::Unavailable)
    }
}

/// The passkey sign-in on a thread of its own: the system dialog over this process's window, then the engine's
/// sign-in. Every outcome is a kind; nothing a server said, no address, no key. An empty address is not asked.
pub fn sign_in_outcome<C: CredentialStores>(engine: &Engine<C>, email: &str) -> PasskeyOutcome {
    #[cfg(windows)]
    {
        let Some(hwnd) = host_window() else {
            return PasskeyOutcome::Unavailable;
        };
        match engine.sign_in_passkey(hwnd, email) {
            Ok(next) => PasskeyOutcome::Succeeded(super::map_next(next)),
            Err(PasskeyProblem::Cancelled) => PasskeyOutcome::Cancelled,
            Err(_) => PasskeyOutcome::Unavailable,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (engine, email);
        PasskeyOutcome::Unavailable
    }
}

/// The top-level window of THIS process that owns the system's passkey dialog: the foreground one when it is ours,
/// else the first visible, unowned one. `None` off Windows, or while the process has none.
pub fn host_window() -> Option<isize> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{HWND, LPARAM};
        use windows_sys::Win32::System::Threading::GetCurrentProcessId;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            EnumWindows, GW_OWNER, GetForegroundWindow, GetWindow, GetWindowThreadProcessId, IsWindowVisible,
        };

        fn pid_of(hwnd: HWND) -> u32 {
            let mut pid = 0u32;
            // SAFETY: `hwnd` is a handle the system gave; the call only writes `pid`.
            unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
            pid
        }

        struct Search {
            pid: u32,
            found: isize,
        }
        unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> i32 {
            // SAFETY: `lparam` is the `Search` this call's caller keeps alive for the whole enumeration.
            let search = unsafe { &mut *(lparam as *mut Search) };
            // SAFETY: plain queries on a handle the enumeration gave.
            let (ours, shown, owned) = unsafe {
                (
                    pid_of(hwnd) == search.pid,
                    IsWindowVisible(hwnd) != 0,
                    !GetWindow(hwnd, GW_OWNER).is_null(),
                )
            };
            if ours && shown && !owned {
                search.found = hwnd as isize;
                return 0;
            }
            1
        }

        // SAFETY: no arguments.
        let pid = unsafe { GetCurrentProcessId() };
        // SAFETY: no arguments.
        let front = unsafe { GetForegroundWindow() };
        if !front.is_null() && pid_of(front) == pid {
            return Some(front as isize);
        }
        let mut search = Search { pid, found: 0 };
        // SAFETY: `each` only reads handles and writes `search`, which outlives the call.
        unsafe { EnumWindows(Some(each), &mut search as *mut Search as LPARAM) };
        (search.found != 0).then_some(search.found)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// A password as the engine takes it: one wiping buffer, made at its full size so it never moves. `None`: empty.
pub fn wiping(password: &Password<'_>) -> Option<Zeroizing<String>> {
    let text = std::str::from_utf8(password.for_opaque())
        .ok()
        .filter(|text| !text.is_empty())?;
    let mut buffer = Zeroizing::new(String::with_capacity(text.len()));
    buffer.push_str(text);
    Some(buffer)
}

/// The passkey methods of `AccountSource`, for `EngineAccountSource` (which keeps a `passkeys: PasskeyCalls<C>`):
/// one line there, so its file stays inside the size limit and every passkey rule is in this file.
macro_rules! passkey_methods {
    () => {
        fn passkeys(&self) -> Vec<$crate::frames::account::source::Passkey> {
            self.passkeys.passkeys()
        }
        /// Adding needs the password (`add_passkey_confirmed`): the pane asks for it, so this is not reached.
        fn add_passkey(&mut self) -> Result<(), $crate::frames::account::source::Refused> {
            Err($crate::frames::account::source::Refused::NotConfirmed)
        }
        /// Beebeeb has no route to rename a passkey: the menu does not offer it (`can_rename_passkey`).
        fn rename_passkey(&mut self, _id: &str, _name: &str) -> bool {
            false
        }
        fn remove_passkey(&mut self, _id: &str) -> bool {
            false
        }
        fn passkeys_need_password(&self) -> bool {
            true
        }
        fn can_add_passkey(&self) -> bool {
            self.passkeys.can_add()
        }
        fn can_rename_passkey(&self) -> bool {
            self.passkeys.can_rename()
        }
        fn add_passkey_confirmed(
            &mut self,
            password: $crate::frames::account::source::Password<'_>,
        ) -> Result<(), $crate::frames::account::source::Refused> {
            match $crate::engine_host::passkeys::wiping(&password) {
                Some(password) if self.passkeys.add(password) => Ok(()),
                _ => Err($crate::frames::account::source::Refused::Offline),
            }
        }
        fn remove_passkey_confirmed(
            &mut self,
            id: &str,
            password: $crate::frames::account::source::Password<'_>,
        ) -> bool {
            $crate::engine_host::passkeys::wiping(&password).is_some_and(|password| self.passkeys.remove(id, password))
        }
    };
}
pub(super) use passkey_methods;

#[cfg(test)]
#[path = "passkeys_tests.rs"]
mod tests;
