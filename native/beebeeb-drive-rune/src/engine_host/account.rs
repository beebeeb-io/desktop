//! The account sections on the real engine: [`EngineAccountSource`], the `AccountSource` of the running app.
//!
//! What it answers, and from where:
//! - Account (DW-A51) and the plan over it (DW-0708): the email ([`Engine::account_profile`]), the name the
//!   account shows ([`Engine::account_display_name`]; the email stands in when none is set) and the plan's size,
//!   use, billing period and renewal day ([`Engine::account_subscription`]). "This account uses" lists Drive
//!   only: this app. Which other Beebeeb products hold a session on this PC is not read here.
//! - Devices and sessions (DW-A55): the account's sessions ([`Engine::account_sessions`]), ONE ROW PER SESSION as
//!   Beebeeb lists them (it names no device and no product per session; see the PR record). Sign out (DW-A56)
//!   ends that one session ([`Engine::revoke_account_session`]); "Sign out everywhere else" ends every session
//!   but this one ([`Engine::revoke_other_account_sessions`]). Both are sent once per press, only for a row of
//!   the list that shows, never for this session, and the list is read again after the answer: a row goes only
//!   when Beebeeb's list no longer has it.
//! - Every other account pane (Sign-in and security and all under it, Name, Change email): `PaneState::Unwired`,
//!   the shell's "not built yet" frame. Their methods here do nothing and are not reachable from a drawn pane.
//!
//! Session rule: every answer carries the session ([`Engine::session_turn`]) and the account it was made for; one
//! that is not the current session's and account's is dropped, every accessor checks before it hands anything
//! out, and the window's poll ([`AccountSource::pump`]) drops what it holds the moment the session moves on. A
//! session Beebeeb no longer accepts (401) is remembered: nothing more is asked or sent for it.
//!
//! Thread rule: each call runs on a worker of its own and sends its answer down one channel, taken in by `pump`;
//! the worker wakes the window ([`Engine::wake_host`]). It reads when a pane opens and what it holds is missing
//! or older than [`FRESH_FOR`]; never from a draw.
//!
//! Must not: log an email, a name, a device's name, an id or a server's words; only a call's kind. Nothing is
//! written to disk here.

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::account_dto::{AccountSession, Subscription};
use beebeeb_engine::app::settings_api::AccountCallError;
use beebeeb_engine::app::{CredentialStores, Engine};
use chrono::{DateTime, Local, Utc};

use super::log;
use crate::frames::account::Pane;
use crate::frames::account::approve::{Approval, Denial, Lookup, SignInRequest};
use crate::frames::account::source::{
    AccountSource, CodeRefused, Deletion, DeletionRefused, DeviceKind, EmailRefused, Event, Factor, Identity,
    PaneState, Passkey, Password, Plan, PlanDetails, ProductKind, ProductState, Pumped, Purpose, Refused, Secret,
    Security, Session, Setup, StepUp, Strength, TwoFactor,
};

#[cfg(test)]
#[path = "account_tests.rs"]
mod tests;

/// What a pane holds is not read again, when it opens, for this long.
pub const FRESH_FOR: Duration = Duration::from_secs(30);

/// What the toast says after a sign-out of another device.
pub const SIGNED_OUT: &str = "Signed out. It ends the next time that device contacts Beebeeb.";
pub const SIGNED_OUT_ELSEWHERE: &str = "Signed out everywhere else.";
/// Only after Beebeeb refused (a status it answered): the session stands.
pub const NOT_SIGNED_OUT: &str = "Couldn't sign that device out. Nothing changed.";
/// A timeout or a broken connection: the sign-out may have landed.
pub const UNKNOWN_SIGN_OUT: &str = "Couldn't confirm the sign-out. Checking what Beebeeb has.";
pub const STILL_SIGNING_OUT: &str = "Still signing out. Try again in a moment.";

/// The key of "every session but this one" among the changes on their way (no session's id is this).
const ELSEWHERE: &str = "*";

/// One session as Beebeeb lists it. No `Debug`: it holds a device's name.
#[derive(Clone, PartialEq)]
struct Row {
    id: String,
    kind: DeviceKind,
    /// The kind of client, in the row's words (`kind_of`).
    client: &'static str,
    name: String,
    /// Unix seconds of the last activity, when Beebeeb named one that reads.
    at: Option<i64>,
    country: String,
    current: bool,
}

/// What Account draws. No `Debug`: it holds the email and the name.
#[derive(Clone)]
struct Overview {
    identity: Identity,
    plan: Option<(Plan, Option<PlanDetails>)>,
}

/// What was read, for one session of one account.
struct Held<T> {
    session: SessionGeneration,
    account: Option<String>,
    value: T,
}

/// The session and account a call is made for.
type Stamp = (SessionGeneration, Option<String>);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Overview,
    Sessions,
}

impl Kind {
    fn of(pane: Pane) -> Option<Kind> {
        match pane {
            Pane::Account => Some(Kind::Overview),
            Pane::Sessions => Some(Kind::Sessions),
            _ => None,
        }
    }
}

/// Where the reads of one kind stand.
#[derive(Default)]
struct Reads {
    /// The read on its way, by number.
    reading: Option<u64>,
    /// When the last read answered with a value (a failed read never stamps this).
    read_at: Option<Instant>,
    /// The session the last read failed for.
    failed_for: Option<SessionGeneration>,
    /// The session the last read was asked for.
    asked_for: Option<SessionGeneration>,
}

enum Answer {
    Overview {
        number: u64,
        made_for: Option<Stamp>,
        result: Result<Overview, AccountCallError>,
    },
    Sessions {
        number: u64,
        /// The count of changes Beebeeb had taken when the read began.
        changes: u64,
        made_for: Option<Stamp>,
        result: Result<Vec<Row>, AccountCallError>,
    },
    /// A sign-out of `key` (a session's id, or [`ELSEWHERE`]) sent for the session `sent_for`.
    SignedOut {
        key: String,
        sent_for: SessionGeneration,
        made_for: Option<Stamp>,
        result: Result<(), AccountCallError>,
    },
}

/// Whose account a call is for: the stored email, else the user id (as `settings::account_of`).
fn account_of<C: CredentialStores>(engine: &Engine<C>) -> Option<String> {
    engine.account_email().or_else(|| engine.user_id())
}

fn stamp<C: CredentialStores>(engine: &Engine<C>) -> Stamp {
    (engine.session_turn(), account_of(engine))
}

/// The stamp, when it is still the engine's after the call: else `None` (the answer is dropped).
fn still<C: CredentialStores>(engine: &Engine<C>, before: Stamp) -> Option<Stamp> {
    (stamp(engine) == before).then_some(before)
}

/// "1 TB", "240 GB", "1.5 GB": decimal units, as the board writes sizes.
pub(crate) fn size_words(bytes: i64) -> String {
    const UNITS: [(&str, f64); 4] = [("TB", 1e12), ("GB", 1e9), ("MB", 1e6), ("KB", 1e3)];
    let bytes = bytes.max(0) as f64;
    for (unit, size) in UNITS {
        if bytes >= size {
            let value = bytes / size;
            let rounded = (value * 10.0).round() / 10.0;
            return if rounded >= 100.0 || rounded.fract() == 0.0 {
                format!("{} {unit}", rounded.round() as i64)
            } else {
                format!("{rounded:.1} {unit}")
            };
        }
    }
    format!("{} B", bytes as i64)
}

/// Up to two letters: the first of the first word and of the last.
pub(crate) fn initials_of(name: &str) -> String {
    let mut words = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty());
    let first = words.next().and_then(|w| w.chars().next());
    let last = words.last().and_then(|w| w.chars().next());
    first.into_iter().chain(last).flat_map(char::to_uppercase).collect()
}

/// "now", "12 min ago", "3 hours ago", "5 days ago"; "not recorded" when Beebeeb named no time.
pub(crate) fn ago(at: Option<i64>, now: i64) -> String {
    let Some(at) = at else {
        return "not recorded".into();
    };
    let seconds = (now - at).max(0);
    match seconds {
        0..=59 => "now".into(),
        60..=3_599 => format!("{} min ago", seconds / 60),
        3_600..=7_199 => "1 hour ago".into(),
        7_200..=86_399 => format!("{} hours ago", seconds / 3_600),
        86_400..=172_799 => "1 day ago".into(),
        _ => format!("{} days ago", seconds / 86_400),
    }
}

/// The icon and the words for a session's `device_kind`, as Beebeeb stores it (web, desktop, cli, ios, android,
/// app, api). Beebeeb names no product per session, so the row says the kind of client, never a product's name;
/// `ios` and `android` are a phone's app or its browser, so the words name the system only. A kind this list
/// does not know is "Other client": never a guess.
fn kind_of(device_kind: &str) -> (DeviceKind, &'static str) {
    match device_kind {
        "web" => (DeviceKind::Browser, "Web browser"),
        "desktop" => (DeviceKind::Laptop, "Desktop app"),
        "cli" => (DeviceKind::CommandLine, "Command line"),
        "ios" => (DeviceKind::Phone, "iPhone or iPad"),
        "android" => (DeviceKind::Phone, "Android"),
        "app" => (DeviceKind::Laptop, "Beebeeb app"),
        _ => (DeviceKind::Browser, "Other client"),
    }
}

fn row_of(session: AccountSession) -> Row {
    let country = session
        .country_code
        .map(|code| code.trim().to_ascii_uppercase())
        // Two letters: a country, never anything longer (a city can never reach the pane).
        .filter(|code| code.len() == 2 && code.chars().all(|c| c.is_ascii_alphabetic()))
        .unwrap_or_default();
    let (kind, client) = kind_of(&session.device_kind);
    Row {
        id: session.id,
        kind,
        client,
        name: session.device_name,
        at: session
            .last_active_at
            .as_deref()
            .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
            .map(|at| at.timestamp()),
        country,
        current: session.is_current,
    }
}

/// The plan as Account and the plan dialog draw it. `None`: Beebeeb named no size.
fn plan_of(plan: &Subscription) -> Option<(Plan, Option<PlanDetails>)> {
    if plan.quota_bytes <= 0 {
        return None;
    }
    let (size, used) = (size_words(plan.quota_bytes), size_words(plan.used_bytes));
    let renews = plan
        .current_period_end
        .as_deref()
        .and_then(|end| DateTime::parse_from_rfc3339(end).ok())
        .map(|end| end.with_timezone(&Local).format("%-d %b %Y").to_string());
    let billing = match plan.billing_cycle.to_ascii_lowercase().as_str() {
        "yearly" | "annual" | "year" => Some("Yearly"),
        "monthly" | "month" => Some("Monthly"),
        _ => None,
    };
    let card = Plan {
        title: format!("Drive plan \u{b7} {size}"),
        used: used.clone(),
        rest: match &renews {
            Some(day) => format!("used \u{b7} renews {day}"),
            None => "used".into(),
        },
    };
    // The dialog draws a renewal day and a billing period: without both it is not opened (the card then leads
    // to the plan page on the website, as the trait says).
    let details = renews.zip(billing).map(|(renews, billing)| PlanDetails {
        size,
        used,
        share: (plan.used_bytes.max(0) as f32 / plan.quota_bytes as f32).clamp(0.0, 1.0),
        renews,
        billing: billing.into(),
    });
    Some((card, details))
}

/// Read everything Account draws. The three answers must be one session's.
fn read_overview<C: CredentialStores>(engine: &Engine<C>) -> Result<(SessionGeneration, Overview), AccountCallError> {
    let profile = engine.account_profile()?;
    let name = engine.account_display_name()?;
    let plan = match engine.account_subscription() {
        Ok(plan) => Some(plan),
        Err(kind @ (AccountCallError::SessionEnded | AccountCallError::NotSignedIn)) => return Err(kind),
        // The plan could not be read: the card is not drawn (never a guessed plan).
        Err(_) => None,
    };
    let session = profile.session;
    if name.session != session || plan.as_ref().is_some_and(|plan| plan.session != session) {
        return Err(AccountCallError::NotSignedIn);
    }
    let email = profile.value.email;
    let name = name.value.unwrap_or_else(|| email.clone());
    Ok((
        session,
        Overview {
            identity: Identity {
                initials: initials_of(&name),
                name,
                email,
            },
            plan: plan.and_then(|plan| plan_of(&plan.value)),
        },
    ))
}

/// Run `work` on a worker named `name`; its answer (or `fallback` if it panicked) goes down `answers` and wakes
/// the window. `false`: no worker could start.
fn spawn<C: CredentialStores + 'static>(
    name: &str,
    engine: &Arc<Engine<C>>,
    answers: &mpsc::Sender<Answer>,
    work: impl FnOnce(&Engine<C>) -> Answer + Send + 'static,
    fallback: impl FnOnce() -> Answer + Send + 'static,
) -> bool {
    let (engine, answers) = (engine.clone(), answers.clone());
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let answer =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&engine))).unwrap_or_else(|_| fallback());
            let _ = answers.send(answer);
            engine.wake_host();
        })
        .is_ok()
}

/// The account sections on the real engine. See the module's doc.
pub struct EngineAccountSource<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    overview: Option<Held<Overview>>,
    sessions: Option<Held<Vec<Row>>>,
    overview_reads: Reads,
    sessions_reads: Reads,
    reads: u64,
    /// Counts the sign-outs Beebeeb took: a list read before one of them is stale when it lands.
    changes: u64,
    /// The sign-outs on their way, by session and key: a second press for one of them sends nothing.
    changing: Vec<(SessionGeneration, String)>,
    /// The session Beebeeb answered 401 for: nothing more is asked or sent for it.
    ended: Option<SessionGeneration>,
    /// The pane the window shows, when it is one this source answers for.
    showing: Option<Kind>,
    notice: Option<&'static str>,
    dirty: bool,
    answers: mpsc::Sender<Answer>,
    inbox: mpsc::Receiver<Answer>,
}

impl<C: CredentialStores + 'static> EngineAccountSource<C> {
    /// Nothing is asked of Beebeeb until a pane opens.
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            engine,
            overview: None,
            sessions: None,
            overview_reads: Reads::default(),
            sessions_reads: Reads::default(),
            reads: 0,
            changes: 0,
            changing: Vec::new(),
            ended: None,
            showing: None,
            notice: None,
            dirty: false,
            answers,
            inbox,
        }
    }

    /// Whether the engine holds a session that Beebeeb has not ended.
    fn open(&self) -> bool {
        self.engine.has_session() && self.ended != Some(self.engine.session_turn())
    }

    /// Whether a stamp is the engine's session and account now, with the session open.
    fn is_now(&self, session: SessionGeneration, account: &Option<String>) -> bool {
        self.open() && session == self.engine.session_turn() && *account == account_of(&self.engine)
    }

    fn current<'a, T>(&self, held: &'a Option<Held<T>>) -> Option<&'a T> {
        held.as_ref()
            .filter(|held| self.is_now(held.session, &held.account))
            .map(|held| &held.value)
    }

    fn reads_of(&mut self, kind: Kind) -> &mut Reads {
        match kind {
            Kind::Overview => &mut self.overview_reads,
            Kind::Sessions => &mut self.sessions_reads,
        }
    }

    /// Ask Beebeeb on a worker, unless a read of this kind is on its way or the session is not open.
    fn read(&mut self, kind: Kind) {
        if !self.open() || self.reads_of(kind).reading.is_some() {
            return;
        }
        self.reads += 1;
        let (number, changes, turn) = (self.reads, self.changes, self.engine.session_turn());
        let started = match kind {
            Kind::Overview => spawn(
                "account-overview",
                &self.engine,
                &self.answers,
                move |engine| {
                    let before = stamp(engine);
                    let result = read_overview(engine);
                    let made_for =
                        still(engine, before.clone()).filter(|_| !matches!(&result, Ok((s, _)) if *s != before.0));
                    Answer::Overview {
                        number,
                        made_for,
                        result: result.map(|(_, overview)| overview),
                    }
                },
                move || Answer::Overview {
                    number,
                    made_for: None,
                    result: Err(AccountCallError::Offline),
                },
            ),
            Kind::Sessions => spawn(
                "account-sessions",
                &self.engine,
                &self.answers,
                move |engine| {
                    let before = stamp(engine);
                    let result = engine.account_sessions();
                    let made_for =
                        still(engine, before.clone()).filter(|_| !matches!(&result, Ok(s) if s.session != before.0));
                    Answer::Sessions {
                        number,
                        changes,
                        made_for,
                        result: result.map(|list| list.value.sessions.into_iter().map(row_of).collect()),
                    }
                },
                move || Answer::Sessions {
                    number,
                    changes,
                    made_for: None,
                    result: Err(AccountCallError::Offline),
                },
            ),
        };
        let reads = self.reads_of(kind);
        reads.asked_for = Some(turn);
        if started {
            reads.reading = Some(number);
            reads.failed_for = None;
        } else {
            log("account: could not start a thread for a read");
            reads.failed_for = Some(turn);
        }
        self.dirty = true;
    }

    /// Drop everything of the account, and every mark of a change on its way.
    fn forget(&mut self) {
        self.overview = None;
        self.sessions = None;
        self.overview_reads.read_at = None;
        self.sessions_reads.read_at = None;
        self.changing.clear();
        self.notice = None;
        self.dirty = true;
    }

    /// Beebeeb answered 401 for `session`: when it is the engine's now, nothing more is asked for it.
    fn session_ended(&mut self, made_for: &Option<Stamp>) {
        if let Some((session, _)) = made_for
            && *session == self.engine.session_turn()
        {
            self.ended = Some(*session);
            self.forget();
        }
    }

    /// What a failed read leaves: nothing held, nothing fresh, and the pane's "couldn't load" state.
    fn read_failed(&mut self, kind: Kind, made_for: Option<Stamp>, error: AccountCallError) {
        match error {
            AccountCallError::SessionEnded => return self.session_ended(&made_for),
            // The session went during the call: the poll reads again for the next one.
            AccountCallError::NotSignedIn => return,
            _ => {}
        }
        let Some((session, _)) = made_for.filter(|(s, a)| self.is_now(*s, a)) else {
            return;
        };
        log(format!("account: a read did not answer ({error})"));
        match kind {
            Kind::Overview => self.overview = None,
            Kind::Sessions => self.sessions = None,
        }
        let reads = self.reads_of(kind);
        reads.read_at = None;
        reads.failed_for = Some(session);
    }

    fn take_in(&mut self, answer: Answer) {
        match answer {
            Answer::Overview {
                number,
                made_for,
                result,
            } => {
                if self.overview_reads.reading == Some(number) {
                    self.overview_reads.reading = None;
                }
                match result {
                    Ok(value) => {
                        // Made for a session or an account that is not the current one: never drawn.
                        let Some((session, account)) = made_for.filter(|(s, a)| self.is_now(*s, a)) else {
                            return;
                        };
                        self.overview = Some(Held {
                            session,
                            account,
                            value,
                        });
                        self.overview_reads.read_at = Some(Instant::now());
                        self.overview_reads.failed_for = None;
                    }
                    Err(error) => self.read_failed(Kind::Overview, made_for, error),
                }
            }
            Answer::Sessions {
                number,
                changes,
                made_for,
                result,
            } => {
                if self.sessions_reads.reading == Some(number) {
                    self.sessions_reads.reading = None;
                }
                match result {
                    // A sign-out Beebeeb took after this read began is newer than what the read says.
                    Ok(_) if changes != self.changes => self.read(Kind::Sessions),
                    Ok(value) => {
                        let Some((session, account)) = made_for.filter(|(s, a)| self.is_now(*s, a)) else {
                            return;
                        };
                        self.sessions = Some(Held {
                            session,
                            account,
                            value,
                        });
                        self.sessions_reads.read_at = Some(Instant::now());
                        self.sessions_reads.failed_for = None;
                    }
                    Err(error) => self.read_failed(Kind::Sessions, made_for, error),
                }
            }
            Answer::SignedOut {
                key,
                sent_for,
                made_for,
                result,
            } => {
                self.changing
                    .retain(|(session, k)| !(*session == sent_for && *k == key));
                let mine = made_for.as_ref().is_some_and(|(s, a)| self.is_now(*s, a));
                match result {
                    Ok(()) => {
                        self.changes += 1;
                        if mine {
                            self.notice = Some(if key == ELSEWHERE {
                                SIGNED_OUT_ELSEWHERE
                            } else {
                                SIGNED_OUT
                            });
                            // The row goes when Beebeeb's list no longer has it.
                            self.read(Kind::Sessions);
                        }
                    }
                    Err(AccountCallError::SessionEnded) => self.session_ended(&made_for),
                    // Not sent: the session had moved on.
                    Err(AccountCallError::NotSignedIn) => {}
                    Err(error) => {
                        log(format!("account: a sign-out was not confirmed ({error})"));
                        if mine {
                            if error == AccountCallError::ServerError {
                                self.notice = Some(NOT_SIGNED_OUT);
                            } else {
                                // It may have landed: say so and read what Beebeeb has.
                                self.notice = Some(UNKNOWN_SIGN_OUT);
                                self.changes += 1;
                                self.read(Kind::Sessions);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Send one sign-out on a worker: `key` is a session's id of the list that shows, or [`ELSEWHERE`]. Once per
    /// press; a second press while it is on its way sends nothing and says so. `true`: sent.
    fn sign_out(&mut self, key: &str) -> bool {
        self.notice = None;
        self.dirty = true;
        let Some(held) = self.sessions.as_ref().filter(|h| self.is_now(h.session, &h.account)) else {
            return false;
        };
        let known = if key == ELSEWHERE {
            held.value.iter().any(|row| !row.current)
        } else {
            // Only a row of the list that shows, and never this session.
            held.value.iter().any(|row| row.id == key && !row.current)
        };
        if !known {
            return false;
        }
        let session = held.session;
        if self.changing.iter().any(|(s, k)| *s == session && k == key) {
            self.notice = Some(STILL_SIGNING_OUT);
            return false;
        }
        let (sent, fallback_key) = (key.to_string(), key.to_string());
        let started = spawn(
            "account-sign-out",
            &self.engine,
            &self.answers,
            move |engine| {
                let before = stamp(engine);
                let result = if sent == ELSEWHERE {
                    engine.revoke_other_account_sessions(session).map(|_| ())
                } else {
                    engine.revoke_account_session(session, &sent)
                };
                Answer::SignedOut {
                    key: sent,
                    sent_for: session,
                    made_for: still(engine, before),
                    result,
                }
            },
            move || Answer::SignedOut {
                key: fallback_key,
                sent_for: session,
                made_for: None,
                result: Err(AccountCallError::Offline),
            },
        );
        if started {
            self.changing.push((session, key.to_string()));
        } else {
            log("account: could not start a thread for a sign-out");
            self.notice = Some(NOT_SIGNED_OUT);
        }
        started
    }
}

impl<C: CredentialStores + 'static> AccountSource for EngineAccountSource<C> {
    fn pane_state(&self, pane: Pane) -> PaneState {
        let Some(kind) = Kind::of(pane) else {
            return PaneState::Unwired;
        };
        if !self.engine.has_session() {
            return PaneState::SignedOut;
        }
        let turn = self.engine.session_turn();
        if self.ended == Some(turn) {
            return PaneState::SessionEnded;
        }
        let (held, reads) = match kind {
            Kind::Overview => (self.current(&self.overview).is_some(), &self.overview_reads),
            Kind::Sessions => (self.current(&self.sessions).is_some(), &self.sessions_reads),
        };
        if held {
            PaneState::Ready
        } else if reads.reading.is_none() && reads.failed_for == Some(turn) {
            PaneState::Failed
        } else {
            PaneState::Loading
        }
    }

    fn opened(&mut self, pane: Pane) {
        self.showing = Kind::of(pane);
        let Some(kind) = self.showing else {
            return;
        };
        let held = match kind {
            Kind::Overview => self.current(&self.overview).is_some(),
            Kind::Sessions => self.current(&self.sessions).is_some(),
        };
        let fresh = held && self.reads_of(kind).read_at.is_some_and(|at| at.elapsed() < FRESH_FOR);
        if !fresh {
            self.read(kind);
        }
    }

    fn reload(&mut self, pane: Pane) {
        if let Some(kind) = Kind::of(pane) {
            self.read(kind);
        }
    }

    fn pump(&mut self) -> Pumped {
        let mut changed = std::mem::take(&mut self.dirty);
        let stale = |held: Option<(SessionGeneration, &Option<String>)>| {
            held.is_some_and(|(session, account)| !self.is_now(session, account))
        };
        if stale(self.overview.as_ref().map(|h| (h.session, &h.account)))
            || stale(self.sessions.as_ref().map(|h| (h.session, &h.account)))
            || self
                .changing
                .iter()
                .any(|(session, _)| *session != self.engine.session_turn())
        {
            self.forget();
            changed = true;
        }
        while let Ok(answer) = self.inbox.try_recv() {
            self.take_in(answer);
            changed = true;
        }
        // Another sign-in or an unlock while the pane shows: its own values are read once, without a reopening.
        let turn = self.engine.session_turn();
        if let Some(kind) = self.showing
            && self.open()
            && self.reads_of(kind).reading.is_none()
            && self.reads_of(kind).asked_for != Some(turn)
        {
            self.read(kind);
            changed = true;
        }
        self.dirty = false;
        Pumped {
            changed,
            busy: self.overview_reads.reading.is_some()
                || self.sessions_reads.reading.is_some()
                || !self.changing.is_empty(),
        }
    }

    fn notice(&self) -> Option<&'static str> {
        self.notice
    }

    fn clear_notice(&mut self) {
        self.dirty |= self.notice.take().is_some();
    }

    fn identity(&self) -> Identity {
        self.current(&self.overview)
            .map(|overview| overview.identity.clone())
            .unwrap_or(Identity {
                name: String::new(),
                initials: String::new(),
                email: String::new(),
            })
    }

    /// Drive, this app. Which other Beebeeb products hold a session on this PC is not read here, so none of
    /// them is listed (never "Not signed in" for a product that was not checked).
    fn products(&self) -> Vec<(ProductKind, ProductState)> {
        if self.current(&self.overview).is_some() {
            vec![(ProductKind::Drive, ProductState::ThisApp)]
        } else {
            Vec::new()
        }
    }

    fn plan(&self) -> Option<Plan> {
        self.current(&self.overview)?
            .plan
            .as_ref()
            .map(|(plan, _)| plan.clone())
    }

    fn plan_details(&self) -> Option<PlanDetails> {
        self.current(&self.overview)?.plan.as_ref()?.1.clone()
    }

    fn sessions(&self) -> Vec<Session> {
        let now = Utc::now().timestamp();
        self.current(&self.sessions)
            .map(|rows| {
                rows.iter()
                    .map(|row| Session {
                        id: row.id.clone(),
                        kind: row.kind,
                        name: row.name.clone(),
                        apps: row.client.into(),
                        apps_sentence: row.client.into(),
                        last_active: ago(row.at, now),
                        country: row.country.clone(),
                        this_device: row.current,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn sign_out_device(&mut self, id: &str) -> bool {
        // `ELSEWHERE` is not a session's id: a row can never ask for the plural by its id.
        id != ELSEWHERE && self.sign_out(id)
    }

    fn sign_out_everywhere_else(&mut self) {
        self.sign_out(ELSEWHERE);
    }

    // ---- Not bound: each pane these belong to is `PaneState::Unwired` (the shell's "not built yet" frame), so
    // no drawn control reaches them. Each does nothing and claims nothing.

    /// Sign-in and security (DW-A52) is not bound: its values have no "not known".
    fn security(&self) -> Security {
        Security {
            app_lock: false,
            lock_after: String::new(),
            password_changed: String::new(),
            two_factor: false,
            passkeys: 0,
            sign_in_activity: false,
            sessions: 0,
        }
    }
    fn factors(&self) -> Vec<Factor> {
        Vec::new()
    }
    /// App lock: a lane of its own (Windows Hello).
    fn set_app_lock(&mut self, _on: bool) {}
    /// Signing this app out lives on the unbound Security pane; the sign-in lane owns it.
    fn sign_out_this_app(&mut self) {}
    fn sign_out_all_apps(&mut self) {}
    /// The step-up, and everything it guards (email, password, two-factor, deletion): security lanes of their own.
    fn step_up(&mut self, _password: Password<'_>, _purpose: Purpose) -> Result<StepUp, Refused> {
        Err(Refused::Offline)
    }
    fn set_name(&mut self, _name: &str) -> bool {
        false
    }
    fn start_email_change(&mut self, _confirmation: StepUp, _new_email: &str) -> Result<(), EmailRefused> {
        Err(EmailRefused::Offline)
    }
    fn confirm_email_change(&mut self, _code: &str) -> Result<(), CodeRefused> {
        Err(CodeRefused::Offline)
    }
    fn resend_wait(&self) -> u32 {
        0
    }
    fn send_again(&mut self) -> bool {
        false
    }
    fn password_strength(&self, _candidate: &Password<'_>) -> Strength {
        Strength::TooShort
    }
    fn change_password(&mut self, _confirmation: StepUp, _new: Password<'_>) -> Result<(), Refused> {
        Err(Refused::Offline)
    }
    fn two_factor(&self) -> TwoFactor {
        TwoFactor {
            on: false,
            added: String::new(),
            last_used: String::new(),
            codes_left: 0,
            codes_total: 0,
        }
    }
    fn begin_two_factor(&mut self, _confirmation: Option<StepUp>) -> Result<Setup, Refused> {
        Err(Refused::Offline)
    }
    fn turn_on_two_factor(&mut self, _code: &str) -> Result<Vec<Secret>, CodeRefused> {
        Err(CodeRefused::Offline)
    }
    fn new_backup_codes(&mut self, _code: &str) -> Result<Vec<Secret>, CodeRefused> {
        Err(CodeRefused::Offline)
    }
    fn check_code(&mut self, _code: &str) -> Result<(), CodeRefused> {
        Err(CodeRefused::Offline)
    }
    fn turn_off_two_factor(&mut self, _code: &str) -> Result<(), CodeRefused> {
        Err(CodeRefused::Offline)
    }
    fn passkeys(&self) -> Vec<Passkey> {
        Vec::new()
    }
    fn add_passkey(&mut self) -> Result<(), Refused> {
        Err(Refused::Offline)
    }
    fn rename_passkey(&mut self, _id: &str, _name: &str) -> bool {
        false
    }
    fn remove_passkey(&mut self, _id: &str) -> bool {
        false
    }
    fn activity(&self) -> Option<Vec<(String, Vec<Event>)>> {
        None
    }
    fn start_activity(&mut self) {}
    fn stop_activity(&mut self, _code: &str) -> Result<(), CodeRefused> {
        Err(CodeRefused::Offline)
    }
    fn deletion(&self) -> Deletion {
        Deletion::None
    }
    fn request_deletion(&mut self, confirmation: StepUp, _code: &str) -> Result<(), DeletionRefused> {
        Err(DeletionRefused {
            why: CodeRefused::Offline,
            confirmation: Some(confirmation),
        })
    }
    fn cancel_deletion_request(&mut self) {}
    fn deletion_link_used(&mut self) -> bool {
        false
    }
    fn undo_deletion(&mut self) -> bool {
        false
    }
    // Approving a sign-in from another device: the trait's own "nothing waits" bodies, named here so a reader
    // sees they are not bound.
    fn find_sign_in_request(&mut self, _code: &str) -> Lookup {
        Lookup::NotFound
    }
    fn waiting_sign_in_request(&self, _request: &str) -> Option<SignInRequest> {
        None
    }
    fn approval_answer(&mut self) -> Option<(String, Approval)> {
        None
    }
    fn deny_sign_in(&mut self, _request: &str) -> Denial {
        Denial::Expired
    }
}
