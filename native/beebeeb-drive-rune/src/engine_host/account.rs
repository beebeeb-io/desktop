//! The account sections on the real engine: [`EngineAccountSource`], the `AccountSource` of the running app.
//!
//! What it answers, and from where:
//! - Account (DW-A51) and the plan over it (DW-0708): the email, the name the account shows (the email stands in
//!   when none is set or it could not be read) and the plan's size, use, billing period and how its period ends,
//!   in ONE engine call ([`Engine::account_basics`]: three requests at the same time on one client). The period's
//!   end is said in the words of the plan's status as Beebeeb sends it ([`plan_of`]): a status this file does not
//!   know gets no date. "This account uses" lists Drive only: this app. Which other Beebeeb products hold a
//!   session on this PC is not read here.
//! - Devices and sessions (DW-A55): the account's sessions ([`Engine::account_sessions`]), ONE ROW PER SESSION as
//!   Beebeeb lists them (it names no device and no product per session; see the PR record). Sign out (DW-A56)
//!   ends that one session ([`Engine::end_account_session`]); "Sign out everywhere else" ends every session
//!   but this one ([`Engine::revoke_other_account_sessions`]). Both are sent once per press, only for a row of
//!   the list that shows, never for this session, and the list is read again after EVERY answer (taken, already
//!   gone, refused, not known): a row goes only when Beebeeb's list no longer has it. A sign-out Beebeeb did not
//!   take is said with a way to send it again ([`AccountSource::notice_action`]).
//! - Sign-in and security (the one page): only what Beebeeb already tells this file. Whether two-factor is on
//!   comes with the Account read (the profile's own mark; absent: not known), and the number of devices is
//!   the length of the sessions list when this session's list is held. Everything else on the page is `None`
//!   ([`AccountSource::security_facts`]): the score, the password's age, passkeys, sign-in activity, App lock,
//!   signing this app out. The page says "Not known" or "Not available yet" for those and draws no check.
//! - Every other account pane (all the pages under Sign-in and security, Name, Change email):
//!   `PaneState::Unwired`. The view does not open them from a live pane (their rows are the values alone).
//!   Their methods here do nothing and are not reachable from a drawn pane.
//!
//! Session rule: every answer carries the session ([`Engine::session_turn`]) and the account it was made for; one
//! that is not the current session's and account's is dropped, every accessor checks before it hands anything
//! out, and the window's poll ([`AccountSource::pump`]) drops what it holds the moment the session moves on. A
//! session Beebeeb no longer accepts is remembered, and nothing more is asked or sent for it: for good when one
//! of this file's own calls was answered 401; until the engine's next call works when it was the engine that
//! held the session as ended ([`Engine::account_session_refused`], its run of 401s).
//!
//! Thread rule: each call runs on a worker of its own and sends its answer down one channel, taken in by `pump`;
//! the worker wakes the window ([`Engine::wake_host`]). It reads when a pane opens and what it holds is missing
//! or older than [`FRESH_FOR`], and again for a new session only while one of its panes is on screen
//! ([`AccountSource::closed`]); never from a draw. A worker that panicked answers as a call that failed.
//!
//! Must not: log an email, a name, a device's name, an id or a server's words; only a call's kind. Nothing is
//! written to disk here.

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::account_dto::{AccountSession, Subscription};
use beebeeb_engine::app::account_api::{AccountBasics, SessionEnd};
use beebeeb_engine::app::settings_api::AccountCallError;
use beebeeb_engine::app::{CredentialStores, Engine};
use chrono::{DateTime, Local, Utc};

use super::log;
use crate::frames::account::Pane;
use crate::frames::account::approve::{Approval, Denial, Lookup, SignInRequest};
// ONE size formatter: the one behind the app's storage figure.
use crate::frames::account::source::{
    AccountSource, CodeRefused, Deletion, DeletionRefused, DeviceKind, EmailRefused, Event, Factor, Identity,
    PaneState, Passkey, Password, Plan, PlanDetails, ProductKind, ProductState, Pumped, Purpose, Refused, Secret,
    Security, SecurityFacts, Session, Setup, StepUp, Strength, TwoFactor,
};
use crate::frames::global::source::size_words;

#[cfg(test)]
#[path = "account_tests.rs"]
mod tests;
#[path = "account_words.rs"]
mod words;
use words::kind_of;
pub(crate) use words::{ago, initials_of};

/// What a pane holds is not read again, when it opens, for this long.
pub const FRESH_FOR: Duration = Duration::from_secs(30);

/// What the toast says after a sign-out of another device.
pub const SIGNED_OUT: &str = "Signed out. It ends the next time that device contacts Beebeeb.";
pub const SIGNED_OUT_ELSEWHERE: &str = "Signed out everywhere else.";
/// The list Beebeeb has no longer holds the session (it answered 404, or the list read after an answer that was
/// not a yes no longer has it).
pub const IS_SIGNED_OUT: &str = "That session is signed out.";
/// Beebeeb answered with an error (a 5xx, or a refusal): whether it was taken is not claimed; the list is read
/// again, and the toast offers to send it again ([`TRY_AGAIN`]).
pub const NOT_SIGNED_OUT: &str = "Couldn't sign out that session.";
pub const NOT_SIGNED_OUT_ELSEWHERE: &str = "Couldn't sign out your other sessions.";
/// Refused before anything was sent: the list is not current, the row is gone, or the session was busy.
pub const NOT_SENT: &str = "Couldn't sign out. Nothing was sent.";
/// A timeout or a broken connection: the sign-out may have landed.
pub const UNKNOWN_SIGN_OUT: &str = "Couldn't confirm the sign-out. Checking what Beebeeb has.";
pub const STILL_SIGNING_OUT: &str = "Still signing out. Try again in a moment.";
/// The toast's button after a sign-out that was not taken.
pub const TRY_AGAIN: &str = "Try again";

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
    /// Two-factor, as the profile marks it. `None`: Beebeeb did not say.
    two_factor: Option<bool>,
    /// Every part answered. `false`: the name or the plan could not be read; what is drawn stands, but it is
    /// not fresh, so the next opening reads again.
    whole: bool,
}

/// What was read, for one session of one account.
struct Held<T> {
    session: SessionGeneration,
    account: Option<String>,
    value: T,
}

/// The session and account a call is made for.
type Stamp = (SessionGeneration, Option<String>);

/// What the toast says. It is kept with the session it was said in: a toast is never drawn in another session.
struct Said {
    words: &'static str,
    /// The sign-out the words are about while the list can still correct them (a session's id, or
    /// [`ELSEWHERE`]): the next list read after the answer decides (`EngineAccountSource::words_follow`).
    about: Option<String>,
    /// The toast's button sends `about` again.
    again: bool,
    /// Something was not done: drawn as a failure, never with the mark of a thing that worked.
    failed: bool,
}

impl Said {
    fn done(words: &'static str) -> Self {
        Self {
            words,
            about: None,
            again: false,
            failed: false,
        }
    }

    fn failed(words: &'static str, about: Option<String>, again: bool) -> Self {
        Self {
            words,
            about,
            again,
            failed: true,
        }
    }
}

fn not_signed_out(key: &str) -> &'static str {
    if key == ELSEWHERE {
        NOT_SIGNED_OUT_ELSEWHERE
    } else {
        NOT_SIGNED_OUT
    }
}

/// Who found that Beebeeb no longer accepts the session.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EndedBy {
    /// One of this file's own calls was answered 401: for the rest of the session.
    Answer,
    /// The engine held the session as ended and nothing was asked: until the engine's next call works.
    Engine,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Overview,
    Sessions,
}

impl Kind {
    fn of(pane: Pane) -> Option<Kind> {
        match pane {
            // The Security page stands on the Account read (the profile says whether two-factor is on).
            Pane::Account | Pane::Security => Some(Kind::Overview),
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
        /// The engine held the session as ended around the call (so a "session ended" is its finding).
        engine_held: bool,
        result: Result<Overview, AccountCallError>,
    },
    Sessions {
        number: u64,
        /// The count of changes Beebeeb had taken when the read began.
        changes: u64,
        made_for: Option<Stamp>,
        engine_held: bool,
        result: Result<Vec<Row>, AccountCallError>,
    },
    /// A sign-out of `key` (a session's id, or [`ELSEWHERE`]) sent for the session `sent_for`.
    SignedOut {
        key: String,
        sent_for: SessionGeneration,
        made_for: Option<Stamp>,
        engine_held: bool,
        result: Result<SessionEnd, AccountCallError>,
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
///
/// How the period ends is said from `status`, in Beebeeb's own words for it (`active`, `trialing`, `cancelling`,
/// `paused`, `past_due`, `cancelled`), and only for the ones this function knows: a plan that renews, a plan
/// that was cancelled and runs to the end of its period, a trial (Beebeeb sets the period's end to the trial's
/// end), a paused plan. Any other status, or no day: "used" and nothing more. Never "renews" for a plan that
/// does not.
fn plan_of(plan: &Subscription) -> Option<(Plan, Option<PlanDetails>)> {
    if plan.quota_bytes <= 0 {
        return None;
    }
    let size_of = |bytes: i64| size_words(bytes.max(0) as u64);
    let (size, used) = (size_of(plan.quota_bytes), size_of(plan.used_bytes));
    let day = plan
        .current_period_end
        .as_deref()
        .and_then(|end| DateTime::parse_from_rfc3339(end).ok())
        .map(|end| end.with_timezone(&Local).format("%-d %b %Y").to_string());
    // The card's words, and the dialog's row (its label and the day).
    let (rest, dated) = match (plan.status.as_str(), day) {
        ("active", Some(day)) => (format!("used \u{b7} renews {day}"), Some(("Renews", day))),
        ("cancelling", Some(day)) => (format!("used \u{b7} cancels on {day}"), Some(("Cancels on", day))),
        ("trialing", Some(day)) => (format!("used \u{b7} trial ends {day}"), Some(("Trial ends", day))),
        ("paused", _) => ("used \u{b7} paused".into(), None),
        _ => ("used".into(), None),
    };
    let billing = match plan.billing_cycle.to_ascii_lowercase().as_str() {
        "yearly" | "annual" | "year" => Some("Yearly"),
        "monthly" | "month" => Some("Monthly"),
        _ => None,
    };
    let card = Plan {
        title: format!("Drive plan \u{b7} {size}"),
        used: used.clone(),
        rest,
    };
    // The dialog draws a day and a billing period: without both it is not opened (the card then leads to the
    // plan page on the website, as the trait says).
    let details = dated.zip(billing).map(|((day_label, renews), billing)| PlanDetails {
        size,
        used,
        share: (plan.used_bytes.max(0) as f32 / plan.quota_bytes as f32).clamp(0.0, 1.0),
        day_label,
        renews,
        billing: billing.into(),
    });
    Some((card, details))
}

/// What Account draws, from the one read. The profile answered (else the read failed). A name that could not be
/// read is "no name": the email stands in. A plan that could not be read hides the card (never a guessed plan).
/// A 401 on either ends the pane as a 401 does.
fn overview_of(basics: AccountBasics) -> Result<Overview, AccountCallError> {
    if basics.display_name == Err(AccountCallError::SessionEnded)
        || matches!(basics.subscription, Err(AccountCallError::SessionEnded))
    {
        return Err(AccountCallError::SessionEnded);
    }
    let whole = basics.display_name.is_ok() && basics.subscription.is_ok();
    let two_factor = basics.profile.totp_enabled;
    let email = basics.profile.email;
    let name = basics.display_name.ok().flatten();
    // The email standing in for the name gives the initials of its part before the `@`: never a letter of
    // the domain. A name that was set is taken whole.
    let initials = initials_of(
        name.as_deref()
            .unwrap_or_else(|| email.split('@').next().unwrap_or_default()),
    );
    Ok(Overview {
        identity: Identity {
            initials,
            name: name.unwrap_or_else(|| email.clone()),
            email,
        },
        plan: basics.subscription.ok().and_then(|plan| plan_of(&plan)),
        two_factor,
        whole,
    })
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
    /// The session Beebeeb no longer accepts, and who found out: nothing more is asked or sent for it.
    ended: Option<(SessionGeneration, EndedBy)>,
    /// The pane the window shows, when it is one this source answers for.
    showing: Option<Kind>,
    /// The pane on screen is Sign-in and security: it also says how many devices are signed in, so the
    /// sessions are read with it.
    counting_devices: bool,
    notice: Option<(SessionGeneration, Said)>,
    /// How a sign-out ended that was answered after the person left Devices and sessions, when it was not
    /// done: said when that pane shows again (a toast belongs to the pane it is about).
    later: Option<(SessionGeneration, Said)>,
    dirty: bool,
    /// Tests: the next worker panics before it asks anything.
    #[cfg(test)]
    panic_next: Arc<std::sync::atomic::AtomicBool>,
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
            counting_devices: false,
            notice: None,
            later: None,
            dirty: false,
            #[cfg(test)]
            panic_next: Arc::default(),
            answers,
            inbox,
        }
    }

    /// Whether the engine holds a session that Beebeeb has not ended.
    fn open(&self) -> bool {
        self.engine.has_session() && self.ended.map(|(session, _)| session) != Some(self.engine.session_turn())
    }

    /// Remember that Beebeeb no longer accepts `session`, and drop everything of the account.
    fn end(&mut self, session: SessionGeneration, by: EndedBy) {
        self.ended = Some((session, by));
        self.forget();
    }

    /// Whether the engine holds the session as ended: then nothing is asked or sent, and it is remembered as the
    /// engine's finding (cleared when the engine's next call works, `pump`).
    fn refused_by_the_engine(&mut self) -> bool {
        let refused = self.engine.account_session_refused();
        if refused {
            self.end(self.engine.session_turn(), EndedBy::Engine);
        }
        refused
    }

    /// Set the toast, for the session that is the engine's now.
    fn say(&mut self, said: Said) {
        self.notice = Some((self.engine.session_turn(), said));
        self.dirty = true;
    }

    /// The toast, when it was said in the session that is the engine's now.
    fn said(&self) -> Option<&Said> {
        self.notice
            .as_ref()
            .filter(|(session, _)| self.engine.has_session() && *session == self.engine.session_turn())
            .map(|(_, said)| said)
    }

    /// A list read after the last answer landed: a toast about a sign-out follows what the list says. The
    /// session is no longer listed: it is signed out, whatever the answer was, and there is nothing to send
    /// again. It is still listed although the toast said it was gone, or that the result was not known: it was
    /// not signed out, and the toast offers to send it again.
    fn words_follow(&mut self, list: &[Row]) {
        let Some(key) = self.said().and_then(|said| said.about.clone()) else {
            return;
        };
        let elsewhere = key == ELSEWHERE;
        let listed = list.iter().any(|row| !row.current && (elsewhere || row.id == key));
        let words = self.said().map(|said| said.words);
        if !listed {
            self.say(Said::done(if elsewhere { SIGNED_OUT_ELSEWHERE } else { IS_SIGNED_OUT }));
        } else if words == Some(IS_SIGNED_OUT) || words == Some(UNKNOWN_SIGN_OUT) {
            self.say(Said::failed(not_signed_out(&key), Some(key), true));
        }
    }

    /// A worker's work: in a test build it panics first when the test asked for that.
    fn work<F>(&self, work: F) -> impl FnOnce(&Engine<C>) -> Answer + Send + 'static
    where
        F: FnOnce(&Engine<C>) -> Answer + Send + 'static,
    {
        #[cfg(test)]
        let panic_next = self.panic_next.clone();
        move |engine: &Engine<C>| {
            #[cfg(test)]
            if panic_next.swap(false, std::sync::atomic::Ordering::SeqCst) {
                panic!("a test asked this worker to panic");
            }
            work(engine)
        }
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
        if !self.open() || self.reads_of(kind).reading.is_some() || self.refused_by_the_engine() {
            return;
        }
        self.reads += 1;
        let (number, changes, turn) = (self.reads, self.changes, self.engine.session_turn());
        // Taken here, on the window's thread: a worker that panics still answers for this session, as a read
        // that failed (never a pane left on "Loading").
        let asked_as = stamp(&self.engine);
        let started = match kind {
            Kind::Overview => spawn(
                "account-overview",
                &self.engine,
                &self.answers,
                self.work(move |engine| {
                    let before = stamp(engine);
                    let held = engine.account_session_refused();
                    let result = engine.account_basics();
                    let made_for =
                        still(engine, before.clone()).filter(|_| !matches!(&result, Ok(s) if s.session != before.0));
                    Answer::Overview {
                        number,
                        made_for,
                        engine_held: held || engine.account_session_refused(),
                        result: result.and_then(|basics| overview_of(basics.value)),
                    }
                }),
                move || Answer::Overview {
                    number,
                    engine_held: false,
                    made_for: Some(asked_as),
                    result: Err(AccountCallError::Offline),
                },
            ),
            Kind::Sessions => spawn(
                "account-sessions",
                &self.engine,
                &self.answers,
                self.work(move |engine| {
                    let before = stamp(engine);
                    let held = engine.account_session_refused();
                    let result = engine.account_sessions();
                    let made_for =
                        still(engine, before.clone()).filter(|_| !matches!(&result, Ok(s) if s.session != before.0));
                    Answer::Sessions {
                        number,
                        changes,
                        made_for,
                        engine_held: held || engine.account_session_refused(),
                        result: result.map(|list| list.value.sessions.into_iter().map(row_of).collect()),
                    }
                }),
                move || Answer::Sessions {
                    number,
                    changes,
                    engine_held: false,
                    made_for: Some(asked_as),
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
        self.later = None;
        self.dirty = true;
    }

    /// A call answered "the session ended" for `session`: when it is the engine's now, nothing more is asked
    /// for it. The engine holding it as ended (`engine_held`: seen by the worker around its call, or now): its
    /// finding, cleared with its flag. Else Beebeeb answered this call 401: for the rest of the session.
    fn session_ended(&mut self, made_for: &Option<Stamp>, engine_held: bool) {
        if let Some((session, _)) = made_for
            && *session == self.engine.session_turn()
        {
            let by = if engine_held || self.engine.account_session_refused() {
                EndedBy::Engine
            } else {
                EndedBy::Answer
            };
            self.end(*session, by);
        }
    }

    /// What a failed read leaves: nothing held, nothing fresh, and the pane's "couldn't load" state.
    fn read_failed(&mut self, kind: Kind, made_for: Option<Stamp>, engine_held: bool, error: AccountCallError) {
        if error == AccountCallError::SessionEnded {
            return self.session_ended(&made_for, engine_held);
        }
        // Made for a session that went during the call: the poll reads again for the next one. One the engine
        // refused while the session stayed (a session step held the lock for a moment) is a read that failed.
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
                engine_held,
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
                        // A read with a part that failed is drawn, and is not fresh: the next opening reads.
                        self.overview_reads.read_at = value.whole.then(Instant::now);
                        self.overview_reads.failed_for = None;
                        self.overview = Some(Held {
                            session,
                            account,
                            value,
                        });
                    }
                    Err(error) => self.read_failed(Kind::Overview, made_for, engine_held, error),
                }
            }
            Answer::Sessions {
                number,
                changes,
                made_for,
                engine_held,
                result,
            } => {
                if self.sessions_reads.reading == Some(number) {
                    self.sessions_reads.reading = None;
                }
                match result {
                    // A sign-out Beebeeb took after this read began is newer than what the read says: it is
                    // read again while the pane shows, and when it opens otherwise.
                    Ok(_) if changes != self.changes => {
                        self.sessions_reads.read_at = None;
                        if self.showing == Some(Kind::Sessions) {
                            self.read(Kind::Sessions);
                        }
                    }
                    Ok(value) => {
                        let Some((session, account)) = made_for.filter(|(s, a)| self.is_now(*s, a)) else {
                            return;
                        };
                        self.words_follow(&value);
                        self.sessions = Some(Held {
                            session,
                            account,
                            value,
                        });
                        self.sessions_reads.read_at = Some(Instant::now());
                        self.sessions_reads.failed_for = None;
                    }
                    Err(error) => self.read_failed(Kind::Sessions, made_for, engine_held, error),
                }
            }
            Answer::SignedOut {
                key,
                sent_for,
                made_for,
                engine_held,
                result,
            } => {
                self.changing
                    .retain(|(session, k)| !(*session == sent_for && *k == key));
                let mine = made_for.as_ref().is_some_and(|(s, a)| self.is_now(*s, a));
                match result {
                    Err(AccountCallError::SessionEnded) => self.session_ended(&made_for, engine_held),
                    // Made for a session that is not the current one: nothing is said to the next one. A
                    // sign-out Beebeeb took still makes every list read before it stale.
                    result if !mine => {
                        if result.is_ok() {
                            self.changes += 1;
                        }
                    }
                    result => {
                        // Not sent (a session step held the lock for a moment): nothing changed at Beebeeb.
                        let sent = result != Err(AccountCallError::NotSignedIn);
                        let said = match result {
                            Ok(SessionEnd::Ended) if key == ELSEWHERE => Said::done(SIGNED_OUT_ELSEWHERE),
                            Ok(SessionEnd::Ended) => Said::done(SIGNED_OUT),
                            // Beebeeb has no such session: it had ended before. Said as that; the list that
                            // is read next has the last word (`words_follow`).
                            Ok(SessionEnd::AlreadyGone) => Said {
                                about: Some(key),
                                ..Said::done(IS_SIGNED_OUT)
                            },
                            Err(AccountCallError::NotSignedIn) => Said::failed(NOT_SENT, Some(key), true),
                            Err(error) => {
                                log(format!("account: a sign-out was not confirmed ({error})"));
                                if error == AccountCallError::ServerError {
                                    // Beebeeb answered with an error. A gateway's 5xx can hide a sign-out
                                    // that was taken, so "nothing changed" is not claimed.
                                    Said::failed(not_signed_out(&key), Some(key), true)
                                } else {
                                    // A timeout, a broken connection, an answer that does not read: it may
                                    // have landed. The list that is read next says which.
                                    Said::failed(UNKNOWN_SIGN_OUT, Some(key), false)
                                }
                            }
                        };
                        if sent {
                            // A list read before this answer is never drawn after it.
                            self.changes += 1;
                            self.sessions_reads.read_at = None;
                        }
                        if self.showing == Some(Kind::Sessions) {
                            self.say(said);
                            // After every answer: the row goes (or stays) by what Beebeeb's list says now.
                            if sent {
                                self.read(Kind::Sessions);
                            }
                        } else if said.failed {
                            // The person left Devices and sessions before the answer. A toast belongs to
                            // the pane it is about, and nothing is read for a pane that does not show: what
                            // was not done is said when the pane shows again, where the list is read.
                            self.later = Some((self.engine.session_turn(), said));
                        }
                    }
                }
            }
        }
    }

    /// Send one sign-out on a worker: `key` is a session's id of the list that shows, or [`ELSEWHERE`]. Once per
    /// press; a second press while it is on its way sends nothing and says so, and so does a press that is
    /// refused before anything is sent. `true`: sent.
    fn sign_out(&mut self, key: &str) -> bool {
        self.notice = None;
        self.dirty = true;
        if !self.open() || self.refused_by_the_engine() {
            return false;
        }
        // Only a row of the list that shows, and never this session.
        let listed = self
            .sessions
            .as_ref()
            .filter(|held| self.is_now(held.session, &held.account))
            .filter(|held| {
                held.value
                    .iter()
                    .any(|row| !row.current && (key == ELSEWHERE || row.id == key))
            })
            .map(|held| held.session);
        let Some(session) = listed else {
            self.say(Said::failed(NOT_SENT, None, false));
            return false;
        };
        if self.changing.iter().any(|(s, k)| *s == session && k == key) {
            self.say(Said::failed(STILL_SIGNING_OUT, None, false));
            return false;
        }
        let (sent, fallback_key) = (key.to_string(), key.to_string());
        // As for a read: a worker that panics answers for this session, as a sign-out that is not known.
        let asked_as = stamp(&self.engine);
        let started = spawn(
            "account-sign-out",
            &self.engine,
            &self.answers,
            self.work(move |engine| {
                let before = stamp(engine);
                let held = engine.account_session_refused();
                let result = if sent == ELSEWHERE {
                    engine.revoke_other_account_sessions(session).map(|_| SessionEnd::Ended)
                } else {
                    engine.end_account_session(session, &sent)
                };
                Answer::SignedOut {
                    key: sent,
                    sent_for: session,
                    made_for: still(engine, before),
                    engine_held: held || engine.account_session_refused(),
                    result,
                }
            }),
            move || Answer::SignedOut {
                key: fallback_key,
                sent_for: session,
                engine_held: false,
                made_for: Some(asked_as),
                result: Err(AccountCallError::Offline),
            },
        );
        if started {
            self.changing.push((session, key.to_string()));
        } else {
            log("account: could not start a thread for a sign-out");
            self.say(Said::failed(NOT_SENT, Some(key.to_string()), true));
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
        if !self.open() {
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
        self.counting_devices = pane == Pane::Security;
        let Some(kind) = self.showing else {
            return;
        };
        // The Security page's device count: the sessions list, read beside the page's own read.
        if self.counting_devices
            && !(self.current(&self.sessions).is_some()
                && self.sessions_reads.read_at.is_some_and(|at| at.elapsed() < FRESH_FOR))
        {
            self.read(Kind::Sessions);
        }
        // A sign-out that was not done, answered while another pane showed: said now, over its own pane.
        if kind == Kind::Sessions
            && let Some((session, said)) = self.later.take()
            && self.open()
            && session == self.engine.session_turn()
        {
            self.say(said);
        }
        let held = match kind {
            Kind::Overview => self.current(&self.overview).is_some(),
            Kind::Sessions => self.current(&self.sessions).is_some(),
        };
        let fresh = held && self.reads_of(kind).read_at.is_some_and(|at| at.elapsed() < FRESH_FOR);
        if !fresh {
            self.read(kind);
        }
    }

    fn closed(&mut self) {
        self.showing = None;
        self.counting_devices = false;
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
        let turn = self.engine.session_turn();
        // The engine held the session as ended, and its next call worked: the session is asked again.
        let by_the_engine = self.ended == Some((turn, EndedBy::Engine));
        if by_the_engine && !self.engine.account_session_refused() {
            self.ended = None;
            self.overview_reads.asked_for = None;
            self.sessions_reads.asked_for = None;
            changed = true;
        }
        // Another sign-in or an unlock while a pane shows: its own values are read once, without a reopening.
        // Only while one shows: nothing is asked for a pane nobody looks at.
        if let Some(kind) = self.showing
            && self.open()
            && self.reads_of(kind).reading.is_none()
            && self.reads_of(kind).asked_for != Some(turn)
        {
            self.read(kind);
            changed = true;
        }
        // The same for the device count of the Security page.
        if self.counting_devices
            && self.open()
            && self.sessions_reads.reading.is_none()
            && self.sessions_reads.asked_for != Some(turn)
        {
            self.read(Kind::Sessions);
            changed = true;
        }
        self.dirty = false;
        Pumped {
            changed,
            busy: self.overview_reads.reading.is_some()
                || self.sessions_reads.reading.is_some()
                || !self.changing.is_empty(),
            // Nothing wakes the window when the engine's flag clears: look again while a pane says so.
            watch: self.showing.is_some() && self.ended == Some((turn, EndedBy::Engine)),
        }
    }

    fn notice(&self) -> Option<&'static str> {
        self.said().map(|said| said.words)
    }

    fn notice_failed(&self) -> bool {
        self.said().is_some_and(|said| said.failed)
    }

    fn notice_action(&self) -> Option<&'static str> {
        self.said()
            .filter(|said| said.again && said.about.is_some())
            .map(|_| TRY_AGAIN)
    }

    /// Send the sign-out the toast is about again, through every check a first press goes through.
    fn run_notice_action(&mut self) {
        if let Some(key) = self
            .said()
            .filter(|said| said.again)
            .and_then(|said| said.about.clone())
        {
            self.sign_out(&key);
        }
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

    // ---- Not bound: each pane these belong to is `PaneState::Unwired` (it says "not built yet", and none of
    // its controls is drawn), so no drawn control reaches them. Each does nothing and claims nothing.

    /// What the Security page draws: two-factor as the profile of THIS session's account marks it, the number
    /// of devices when this session's list is held, and nothing else. Every other part is not known here.
    fn security_facts(&self) -> SecurityFacts {
        SecurityFacts {
            two_factor: self.current(&self.overview).and_then(|overview| overview.two_factor),
            sessions: self.current(&self.sessions).map(Vec::len),
            ..SecurityFacts::unknown()
        }
    }
    /// The old rows' values have no "not known": the page is drawn from `security_facts`, never from this.
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
