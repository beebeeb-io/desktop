//! Sign-in activity on the real engine (DW-A77 to DW-A79 and the last three events of the Security page): whether
//! the account records its sign-ins, the newest of them, and the switch that starts the recording.
//!
//! Read from `GET /api/v1/me/activity` ([`Engine::activity_log`]): one answer says whether the log is on and, when
//! it is, what it holds. Only sign-ins and security changes are shown, by an allow-list of Beebeeb's event
//! types; every other type (files, shares, sessions) is dropped here, so no file or item can ever reach the
//! page. A country is its two letters, as in Devices and sessions; a city never exists in this data.
//!
//! Starting the recording is core's own flow, sent by the engine ([`Engine::start_activity_log_for`]); the log is
//! read again after the answer and the switch moves only with that read ("never draw a change before the server
//! made it"). STOPPING it is not done from here: Beebeeb asks for the password's own proof (a step-up token) to
//! erase the log, and this pane's dialog asks for a code. The pane says so and points to the web
//! ([`ActivityPart::can_stop`]).
//!
//! Same session rule as the rest of the account window: an answer for another session or account is dropped; the
//! read is made when a pane that shows it opens and when what is held is older than [`super::FRESH_FOR`], never
//! from a draw. Must not: log an event, a device's name or a country.

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::api_client::{MyActivity, MyActivityEvent};
use beebeeb_engine::app::account_actions_api::ActionError;
use beebeeb_engine::app::{CredentialStores, Engine};
use chrono::{DateTime, Datelike, Utc};

use super::{FRESH_FOR, Stamp, account_of, ago, log, stamp};
use crate::frames::account::Pane;
use crate::frames::account::source::{Event, EventKind, PaneState};

/// The toasts' times, as the account window's.
const STAYS_DONE: Duration = Duration::from_secs(3);
const STAYS_FAILED: Duration = Duration::from_secs(8);

pub(super) const STARTED: &str = "Recording sign-in activity is on.";
pub(super) const NOT_STARTED: &str = "Couldn't turn recording on. Nothing changed.";
pub(super) const NOT_READ: &str = "Couldn't refresh. This is what Beebeeb answered before.";

/// One event with the time it happened (seconds, UTC), when Beebeeb said.
#[derive(Clone)]
pub(super) struct Logged {
    pub(super) at: Option<i64>,
    pub(super) event: Event,
}

/// What one read found: `None`: the account records nothing.
type Log = Option<Vec<Logged>>;

enum Answer {
    Read {
        number: u64,
        made_for: Option<Stamp>,
        result: Result<Log, ActionError>,
    },
    Started {
        stamp: Stamp,
        result: Result<(), ActionError>,
    },
}

struct Held {
    stamp: Stamp,
    value: Log,
}

struct Toast {
    stamp: Stamp,
    words: &'static str,
    failed: bool,
    shown_at: Option<Instant>,
}

pub(super) struct ActivityPart<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    held: Option<Held>,
    reads: u64,
    reading: Option<u64>,
    read_at: Option<Instant>,
    failed_for: Option<SessionGeneration>,
    asked_for: Option<SessionGeneration>,
    /// A pane that shows the log is on screen: Sign-in activity, or the Security page (its last three).
    showing: bool,
    /// The recording is being switched on (by the stamp it was pressed under).
    starting: Option<Stamp>,
    toast: Option<Toast>,
    tx: mpsc::Sender<Answer>,
    rx: mpsc::Receiver<Answer>,
}

impl<C: CredentialStores + 'static> ActivityPart<C> {
    pub(super) fn new(engine: Arc<Engine<C>>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            engine,
            held: None,
            reads: 0,
            reading: None,
            read_at: None,
            failed_for: None,
            asked_for: None,
            showing: false,
            starting: None,
            toast: None,
            tx,
            rx,
        }
    }

    fn open(&self) -> bool {
        self.engine.has_session() && !self.engine.account_session_refused()
    }

    fn is_now(&self, stamp_of: &Stamp) -> bool {
        self.open() && stamp_of.0 == self.engine.session_turn() && stamp_of.1 == account_of(&self.engine)
    }

    fn current(&self) -> Option<&Log> {
        self.held
            .as_ref()
            .filter(|held| self.is_now(&held.stamp))
            .map(|held| &held.value)
    }

    /// Whether a pane of this source is the Sign-in activity page or the Security page.
    pub(super) fn shows(pane: Pane) -> bool {
        matches!(pane, Pane::Activity | Pane::Security)
    }

    pub(super) fn opened(&mut self, pane: Pane) {
        self.showing = Self::shows(pane);
        if self.showing && !(self.current().is_some() && self.read_at.is_some_and(|at| at.elapsed() < FRESH_FOR)) {
            self.read();
        }
    }

    pub(super) fn closed(&mut self) {
        self.showing = false;
    }

    pub(super) fn reload(&mut self) {
        self.read();
    }

    /// Where the Sign-in activity page stands.
    pub(super) fn pane_state(&self) -> PaneState {
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

    /// The page's periods, newest first; `None` while the account records nothing (or is not read).
    pub(super) fn periods(&self) -> Option<Vec<(String, Vec<Event>)>> {
        let events = self.current()?.as_ref()?;
        Some(periods_of(events, Utc::now()))
    }

    /// The Security page's part: `None` not read, `Some(None)` off, `Some(Some(events))` on.
    pub(super) fn facts(&self) -> Option<Option<Vec<Event>>> {
        self.current().map(|log| {
            log.as_ref()
                .map(|events| events.iter().map(|logged| logged.event.clone()).collect())
        })
    }

    /// Whether recording can be switched OFF from here: no (see the module's doc).
    pub(super) fn can_stop(&self) -> bool {
        false
    }

    fn read(&mut self) {
        if !self.open() || self.reading.is_some() {
            return;
        }
        self.reads += 1;
        let number = self.reads;
        let turn = self.engine.session_turn();
        let (engine, tx) = (self.engine.clone(), self.tx.clone());
        let started = std::thread::Builder::new()
            .name("account-activity".into())
            .spawn(move || {
                let before = stamp(&engine);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.activity_log()))
                    .unwrap_or(Err(ActionError::Failed));
                let made_for = (stamp(&engine) == before)
                    .then_some(before.clone())
                    .filter(|_| !matches!(&result, Ok(answer) if answer.session != before.0));
                let _ = tx.send(Answer::Read {
                    number,
                    made_for,
                    result: result.map(|answer| log_of(&answer.value, Utc::now())),
                });
                engine.wake_host();
            })
            .is_ok();
        self.asked_for = Some(turn);
        if started {
            self.reading = Some(number);
            self.failed_for = None;
        } else {
            log("account: could not start a thread for the activity");
            self.failed_for = Some(turn);
        }
    }

    /// DW-A77: switch the recording on. Once per press; the switch moves with the read that follows the answer.
    pub(super) fn start(&mut self) {
        if self.starting.is_some() || !self.open() {
            return;
        }
        // Only an account that records nothing can be switched on.
        if !matches!(self.current(), Some(None)) {
            return;
        }
        let asked_as = stamp(&self.engine);
        let (engine, tx, session) = (self.engine.clone(), self.tx.clone(), asked_as.0);
        let stamp_in = asked_as.clone();
        let started = std::thread::Builder::new()
            .name("account-activity-on".into())
            .spawn(move || {
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.start_activity_log_for(session)))
                        .unwrap_or(Err(ActionError::Failed));
                let _ = tx.send(Answer::Started {
                    stamp: stamp_in,
                    result,
                });
                engine.wake_host();
            })
            .is_ok();
        if started {
            self.starting = Some(asked_as);
        } else {
            log("account: could not start a thread for the recording");
            self.say(NOT_STARTED, true);
        }
    }

    fn say(&mut self, words: &'static str, failed: bool) {
        self.toast = Some(Toast {
            stamp: stamp(&self.engine),
            words,
            failed,
            shown_at: None,
        });
    }

    /// The window's poll. `(changed, busy, wake_in)`.
    pub(super) fn pump(&mut self) -> (bool, bool, Option<Duration>) {
        let mut changed = false;
        // What is held for a session or account that is over goes at once.
        if self.held.as_ref().is_some_and(|held| !self.is_now(&held.stamp)) {
            self.held = None;
            self.read_at = None;
            changed = true;
        }
        if self.toast.as_ref().is_some_and(|toast| !self.is_now(&toast.stamp)) {
            self.toast = None;
            changed = true;
        }
        while let Ok(answer) = self.rx.try_recv() {
            changed = true;
            match answer {
                Answer::Read {
                    number,
                    made_for,
                    result,
                } => {
                    if self.reading == Some(number) {
                        self.reading = None;
                    }
                    // Made for another session or account: never drawn; the read after the change is the poll's.
                    let Some(made_for) = made_for.filter(|stamp_of| self.is_now(stamp_of)) else {
                        continue;
                    };
                    match result {
                        Ok(value) => {
                            self.held = Some(Held { stamp: made_for, value });
                            self.read_at = Some(Instant::now());
                            self.failed_for = None;
                        }
                        Err(error) => {
                            log(format!("account: the activity did not answer ({})", error.words()));
                            self.read_at = None;
                            self.failed_for = Some(made_for.0);
                        }
                    }
                }
                Answer::Started { stamp: sent, result } => {
                    if self.starting.as_ref() == Some(&sent) {
                        self.starting = None;
                    }
                    if !self.is_now(&sent) {
                        continue;
                    }
                    match result {
                        Ok(()) => {
                            self.say(STARTED, false);
                            // The switch moves with what Beebeeb says now.
                            self.read_at = None;
                            self.read();
                        }
                        Err(error) => {
                            log(format!(
                                "account: the recording was not switched on ({})",
                                error.words()
                            ));
                            self.say(NOT_STARTED, true);
                        }
                    }
                }
            }
        }
        // Another sign-in while a pane shows: read once for it.
        let turn = self.engine.session_turn();
        if self.showing && self.open() && self.reading.is_none() && self.asked_for != Some(turn) {
            self.read();
            changed = true;
        }
        let toast_before = self.toast.is_some();
        let wake_in = self.fade();
        changed |= toast_before != self.toast.is_some();
        (changed, self.reading.is_some() || self.starting.is_some(), wake_in)
    }

    /// Time the toast out; how long until it goes, when it shows.
    fn fade(&mut self) -> Option<Duration> {
        let toast = self.toast.as_mut()?;
        let shown_at = *toast.shown_at.get_or_insert_with(Instant::now);
        let stays = if toast.failed { STAYS_FAILED } else { STAYS_DONE };
        match stays.checked_sub(shown_at.elapsed()) {
            Some(left) if !left.is_zero() => Some(left),
            _ => {
                self.toast = None;
                None
            }
        }
    }

    pub(super) fn notice(&self) -> Option<(&'static str, bool)> {
        if let Some(toast) = self.toast.as_ref().filter(|toast| self.is_now(&toast.stamp)) {
            return Some((toast.words, toast.failed));
        }
        // A read that failed over a log that is drawn: said, with the button.
        (self.showing && self.current().is_some() && self.reading.is_none() && self.failed_for.is_some())
            .then_some((NOT_READ, true))
    }

    /// Whether the notice is the "couldn't refresh" one, which has its Try again.
    pub(super) fn notice_is_stale(&self) -> bool {
        self.toast.is_none() && self.notice().is_some()
    }

    pub(super) fn clear(&mut self) {
        self.toast = None;
    }
}

/// The log of one answer: `None` when the account records nothing; else the events that are sign-ins and
/// security changes, newest first.
pub(super) fn log_of(activity: &MyActivity, now: DateTime<Utc>) -> Log {
    if !activity.opted_in {
        return None;
    }
    let mut events: Vec<Logged> = activity
        .events
        .iter()
        .filter_map(|event| event_of(event, now.timestamp()))
        .collect();
    // Beebeeb sends the newest first; a stable sort keeps its order for events of the same second. An event
    // with no time goes last.
    events.sort_by(|a, b| b.at.unwrap_or(i64::MIN).cmp(&a.at.unwrap_or(i64::MIN)));
    Some(events)
}

/// One event, or `None` for a type this page does not show. Only these types are shown: everything else (a file,
/// a share, a session) is never on this page.
fn event_of(event: &MyActivityEvent, now: i64) -> Option<Logged> {
    let failed = event.outcome.as_deref() == Some("failure");
    let (kind, what) = match event.event_type.as_str() {
        "auth.signin.passkey" if !failed => (EventKind::SignIn, "Signed in with a passkey".to_string()),
        "auth.login" | "auth.login.success" | "auth.login.succeeded" | "auth.signin" | "user.login" if !failed => {
            (EventKind::SignIn, "Signed in".to_string())
        }
        "auth.login.failed" => (EventKind::WrongPassword, "A sign-in with a wrong password".to_string()),
        "user.signup" | "user.signup.passkey" | "user.created" => (EventKind::AccountCreated, "Account created".into()),
        "account.password_change" | "auth.password.changed" | "auth.password_changed" | "user.password_change" => {
            (EventKind::PasswordChanged, "Password changed".to_string())
        }
        _ => return None,
    };
    let at = DateTime::parse_from_rfc3339(&event.created_at)
        .ok()
        .map(|at| at.timestamp());
    // The device as the audit log noted it, short and on one line; a country is two letters.
    let device = event
        .device
        .as_deref()
        .map(|device| device.chars().filter(|c| !c.is_control()).take(48).collect::<String>())
        .map(|device| device.trim().to_string())
        .filter(|device| !device.is_empty());
    let what = match (kind, device) {
        (EventKind::SignIn, Some(device)) => format!("{what} on {device}"),
        _ => what,
    };
    let country = event
        .country_code
        .as_deref()
        .map(str::trim)
        .filter(|code| code.len() == 2 && code.chars().all(|c| c.is_ascii_alphabetic()))
        .map(str::to_uppercase);
    let when = ago(at, now);
    let when_where = match country {
        Some(country) => format!("{when} · {country}"),
        None => when,
    };
    Some(Logged {
        at,
        event: Event { kind, what, when_where },
    })
}

/// The page's periods from the newest-first events: "Today", the event's month of this year, else "Earlier" (also
/// for an event Beebeeb gave no time for). Days are UTC days.
pub(super) fn periods_of(events: &[Logged], now: DateTime<Utc>) -> Vec<(String, Vec<Event>)> {
    let mut periods: Vec<(String, Vec<Event>)> = Vec::new();
    for logged in events {
        let title = match logged.at.and_then(|at| DateTime::from_timestamp(at, 0)) {
            Some(at) if at.date_naive() == now.date_naive() => "Today".to_string(),
            Some(at) if at.year() == now.year() => at.format("%B").to_string(),
            _ => "Earlier".to_string(),
        };
        match periods.last_mut() {
            Some((last, rows)) if *last == title => rows.push(logged.event.clone()),
            _ => periods.push((title, vec![logged.event.clone()])),
        }
    }
    periods
}
