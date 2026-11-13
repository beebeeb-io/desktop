//! The account sections on the real engine: [`EngineAccountSource`], the `AccountSource` of the running app.
//!
//! What it answers, and from where:
//! - Account (DW-A51) and the plan over it (DW-0708): the email, the name the account shows (the email stands in
//!   when none is set or it could not be read) and the plan's size, use, billing period and how its period ends,
//!   in ONE engine call ([`Engine::account_basics`]: three requests at the same time on one client). The period's
//!   end is said in the words of the plan's status as Beebeeb sends it ([`plan_of`]): a status this file does not
//!   know gets no date, and "renews" is said only for a plan Beebeeb says is charged again, for a day that is
//!   still ahead. "This account uses" lists Drive only: this app. Which other Beebeeb products hold a
//!   session on this PC is not read here.
//! - Devices and sessions (DW-A55): the account's sessions ([`Engine::account_sessions`]), ONE ROW PER SESSION as
//!   Beebeeb lists them (it names no device and no product per session; see the PR record). Sign out (DW-A56)
//!   ends that one session ([`Engine::end_account_session`]); "Sign out everywhere else" ends every session
//!   but this one ([`Engine::revoke_other_account_sessions`]). Both are sent once per press, only for a row of
//!   the list that shows, never for this session, and the list is read again after EVERY answer (taken, already
//!   gone, refused, not known): a row goes only when Beebeeb's list no longer has it. A sign-out Beebeeb did not
//!   take is said with a way to send it again ([`AccountSource::notice_action`]); for "everywhere else" that sends
//!   again only for the sessions that were listed at the confirm, one by one, never for a session made since.
//! - What it says ([`AccountSource::notice`]): the toasts wait in a line, each for its own time
//!   ([`STAYS_DONE`], [`STAYS_FAILED`]), and a thing that was not done is never replaced by one that worked. A
//!   read that failed over values that were already drawn keeps them, and the pane says it could not refresh,
//!   with Try again, for as long as that is so.
//! - Every other account pane (Sign-in and security and all under it, Name, Change email): `PaneState::Unwired`.
//!   The view says "not built yet" for them inside the settings shell, and does not open Name or Change email
//!   from the live Account pane (their rows are the values alone). Their methods here do nothing and are not
//!   reachable from a drawn pane.
//!
//! Session rule: every answer carries the session ([`Engine::session_turn`]) and the account it was made for; one
//! that is not the current session's and account's is dropped, every accessor checks before it hands anything
//! out, and the window's poll ([`AccountSource::pump`]) drops what it holds the moment the session moves on.
//! "Your session ended" is the engine's word alone ([`Engine::account_session_refused`]: its run of 401s, which
//! this file's calls count in): while it says so nothing is asked or sent and nothing of the account is held, and
//! when its next call works the pane reads again. One 401 on a call of this file is a call that failed, with
//! Try again, as it is for the engine's own loop.
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
use beebeeb_engine::app::account_api::SessionEnd;
use beebeeb_engine::app::settings_api::AccountCallError;
use beebeeb_engine::app::{CredentialStores, Engine};
use chrono::Utc;

use super::log;
use crate::frames::account::Pane;
use crate::frames::account::approve::{Approval, Denial, Lookup, SignInRequest};
use crate::frames::account::source::{
    AccountSource, CodeRefused, Deletion, DeletionRefused, DeviceKind, EmailRefused, Event, Factor, Identity,
    PaneState, Passkey, Password, Plan, PlanDetails, ProductKind, ProductState, Pumped, Purpose, Refused, Secret,
    Security, Session, Setup, StepUp, Strength, TwoFactor,
};

#[path = "account_said.rs"]
mod said;
#[cfg(test)]
#[path = "account_tests.rs"]
mod tests;
#[path = "account_values.rs"]
mod values;

use said::*;
use values::*;

/// What a pane holds is not read again, when it opens, for this long.
pub const FRESH_FOR: Duration = Duration::from_secs(30);

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
    /// The session the last read failed for. With a value still held (and drawn) for that session, the pane
    /// says it could not refresh.
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
    /// A sign-out sent for the session `sent_for`. `about.ids`: the sessions it was for; after a failure, the
    /// ones that were not ended.
    SignedOut {
        about: About,
        sent_for: SessionGeneration,
        made_for: Option<Stamp>,
        /// How many sessions Beebeeb said it ended, when they were sent by id.
        ended: usize,
        /// "Everywhere else", sent again by the toast's button: by id, for the confirmed sessions only.
        one_by_one: bool,
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
    /// Counts the sign-outs that were answered: a list read before one of them is stale when it lands.
    changes: u64,
    /// Counts the sign-outs Beebeeb said it took. `sessions_as_of`: this count when the list that is held was
    /// read. A list from before a sign-out that was taken is never kept over a read that failed.
    taken: u64,
    sessions_as_of: u64,
    /// The sign-outs on their way, by session and key: a second press for one of them sends nothing.
    changing: Vec<(SessionGeneration, String)>,
    /// The engine held the session as ended at the last poll.
    was_refused: bool,
    /// The pane the window shows, when it is one this source answers for.
    showing: Option<Kind>,
    notices: Line,
    /// How sign-outs ended that the person has not been told: answered after they left Devices and sessions,
    /// or still waiting in the line when they left. Only what was not done: said when that pane shows again (a
    /// toast belongs to the pane it is about).
    later: Line,
    dirty: bool,
    /// Tests: the next worker panics before it asks anything.
    #[cfg(test)]
    panic_next: Arc<std::sync::atomic::AtomicBool>,
    /// Tests: how far the toasts' clock is ahead of the real one.
    #[cfg(test)]
    ahead: Duration,
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
            taken: 0,
            sessions_as_of: 0,
            changing: Vec::new(),
            was_refused: false,
            showing: None,
            notices: Line::new(),
            later: Line::new(),
            dirty: false,
            #[cfg(test)]
            panic_next: Arc::default(),
            #[cfg(test)]
            ahead: Duration::ZERO,
            answers,
            inbox,
        }
    }

    /// The engine holds the session as one Beebeeb no longer accepts: the one source of "your session ended".
    fn refused(&self) -> bool {
        self.engine.has_session() && self.engine.account_session_refused()
    }

    /// Whether the engine holds a session that it does not hold as ended.
    fn open(&self) -> bool {
        self.engine.has_session() && !self.engine.account_session_refused()
    }

    /// The toasts' clock.
    fn now(&self) -> Instant {
        #[cfg(test)]
        return Instant::now() + self.ahead;
        #[cfg(not(test))]
        Instant::now()
    }

    /// Put a toast in the line, for the session that is the engine's now.
    fn say(&mut self, said: Said) {
        line_up(&mut self.notices, self.engine.session_turn(), said);
        self.dirty = true;
    }

    /// The toast that shows, when it was said in the session that is the engine's now.
    fn said(&self) -> Option<&Said> {
        self.notices
            .front()
            .filter(|(session, _)| self.engine.has_session() && *session == self.engine.session_turn())
            .map(|(_, said)| said)
    }

    /// A list read after the last answer landed: every toast about a sign-out follows what the list says, the
    /// ones that wait too. None of its sessions is listed any more: they are signed out, whatever the answer
    /// was, and there is nothing to send again. One is still listed although the result was not known: it was
    /// not signed out, and the toast offers to send it again.
    fn words_follow(&mut self, list: &[Row]) {
        let turn = self.engine.session_turn();
        let others = list.iter().any(|row| !row.current);
        for (session, said) in self.notices.iter_mut().chain(self.later.iter_mut()) {
            let Some(about) = said.about.clone().filter(|_| *session == turn) else {
                continue;
            };
            if !about.listed_in(list) {
                *said = Said::done(match (about.elsewhere(), others) {
                    (false, _) => IS_SIGNED_OUT,
                    // The list has sessions that were not confirmed (made since): not "everywhere else".
                    (true, true) => SIGNED_OUT_THOSE,
                    (true, false) => SIGNED_OUT_ELSEWHERE,
                });
            } else if said.words == UNKNOWN_SIGN_OUT || said.words == UNCONFIRMED_SIGN_OUT {
                *said = Said::failed(about.not_signed_out(), Some(about), true);
            }
        }
        // Only what was not done is said on return.
        self.later.retain(|(_, said)| said.failed);
        self.dirty = true;
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
        if !self.open() || self.reads_of(kind).reading.is_some() {
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
                    let result = engine.account_basics();
                    let made_for =
                        still(engine, before.clone()).filter(|_| !matches!(&result, Ok(s) if s.session != before.0));
                    Answer::Overview {
                        number,
                        made_for,
                        result: result.map(|basics| overview_of(basics.value, Utc::now())),
                    }
                }),
                move || Answer::Overview {
                    number,
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
                    let result = engine.account_sessions();
                    let made_for =
                        still(engine, before.clone()).filter(|_| !matches!(&result, Ok(s) if s.session != before.0));
                    Answer::Sessions {
                        number,
                        changes,
                        made_for,
                        result: result.map(|list| list.value.sessions.into_iter().map(row_of).collect()),
                    }
                }),
                move || Answer::Sessions {
                    number,
                    changes,
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

    /// Drop everything of the account, every mark of a change on its way, and every toast.
    fn forget(&mut self) {
        self.overview = None;
        self.sessions = None;
        self.overview_reads.read_at = None;
        self.sessions_reads.read_at = None;
        self.changing.clear();
        self.notices.clear();
        self.later.clear();
        self.dirty = true;
    }

    /// A read failed (a 401 among them: one 401 is a call that failed, and "your session ended" is the
    /// engine's word, `refused`). What was drawn for this session and account STAYS, and is not fresh: the pane
    /// says it could not refresh and offers to try again (`stale_words`). With nothing held it is the pane's
    /// "couldn't load" state. One list is never kept: the one from before a sign-out Beebeeb took.
    fn read_failed(&mut self, kind: Kind, made_for: Option<Stamp>, error: AccountCallError) {
        // Made for a session that went during the call: the poll reads again for the next one. One the engine
        // refused while the session stayed (a session step held the lock for a moment) is a read that failed.
        let Some((session, _)) = made_for.filter(|(s, a)| self.is_now(*s, a)) else {
            return;
        };
        log(format!("account: a read did not answer ({error})"));
        if kind == Kind::Sessions {
            if self.sessions_as_of != self.taken {
                self.sessions = None;
            }
            // The check after a sign-out whose result is not known has itself failed: no longer "checking".
            for (_, said) in self.notices.iter_mut().chain(self.later.iter_mut()) {
                if said.words == UNKNOWN_SIGN_OUT {
                    said.words = UNCONFIRMED_SIGN_OUT;
                }
            }
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
                        // A part that failed keeps what was drawn for it (this session's, this account's).
                        let value = match self.current(&self.overview) {
                            Some(before) => value.over(before),
                            None => value,
                        };
                        // A read with a part that failed is not fresh: the next opening reads.
                        self.overview_reads.read_at = value.whole().then(Instant::now);
                        self.overview_reads.failed_for = None;
                        self.overview = Some(Held {
                            session,
                            account,
                            value,
                        });
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
                    // A sign-out that was answered after this read began is newer than what the read says: it
                    // is read again while the pane shows, and when it opens otherwise.
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
                        self.sessions_as_of = self.taken;
                        self.sessions_reads.read_at = Some(Instant::now());
                        self.sessions_reads.failed_for = None;
                    }
                    Err(error) => self.read_failed(Kind::Sessions, made_for, error),
                }
            }
            Answer::SignedOut {
                about,
                sent_for,
                made_for,
                ended,
                one_by_one,
                result,
            } => {
                self.changing
                    .retain(|(session, key)| !(*session == sent_for && *key == about.key));
                // Beebeeb said it ended a session: the list from before is never drawn as the list of now.
                let took = ended > 0 || matches!(result, Ok(SessionEnd::Ended));
                if took {
                    self.taken += 1;
                }
                // Made for a session that is not the current one (or one the engine holds as ended): nothing
                // is said. A sign-out Beebeeb took still makes every list read before it stale.
                if !made_for.as_ref().is_some_and(|(s, a)| self.is_now(*s, a)) {
                    if took {
                        self.changes += 1;
                    }
                    return;
                }
                // What a press said before anything was sent ("still signing out") is over with the answer.
                self.notices.retain(|(_, said)| said.about.is_some() || !said.failed);
                // Not sent (a session step held the lock for a moment): nothing changed at Beebeeb.
                let sent = ended > 0 || result != Err(AccountCallError::NotSignedIn);
                let said = match result {
                    Ok(SessionEnd::Ended) if about.elsewhere() => {
                        // Sent one by one for the sessions that were confirmed: "everywhere else" only when
                        // the list holds no other.
                        let others = one_by_one
                            && self.current(&self.sessions).is_some_and(|rows| {
                                rows.iter().any(|row| !row.current && !about.ids.contains(&row.id))
                            });
                        Said::done(if others { SIGNED_OUT_THOSE } else { SIGNED_OUT_ELSEWHERE })
                    }
                    Ok(SessionEnd::Ended) => Said::done(SIGNED_OUT),
                    // Beebeeb has no such session. That is "signed out" only once its list no longer has
                    // the session (`words_follow`): until then it is not known, and said so.
                    Ok(SessionEnd::AlreadyGone) => Said::failed(UNKNOWN_SIGN_OUT, Some(about), false),
                    Err(AccountCallError::NotSignedIn) if ended == 0 => Said::failed(NOT_SENT, Some(about), true),
                    Err(error) => {
                        log(format!("account: a sign-out was not confirmed ({error})"));
                        let words = if ended > 0 {
                            NOT_ALL_SIGNED_OUT
                        } else {
                            about.not_signed_out()
                        };
                        match error {
                            // Beebeeb answered with an error (a gateway's 5xx can hide a sign-out that was
                            // taken, so "nothing changed" is not claimed), with one 401, or the session was
                            // busy part of the way: it can be sent again.
                            AccountCallError::ServerError
                            | AccountCallError::SessionEnded
                            | AccountCallError::NotSignedIn => Said::failed(words, Some(about), true),
                            // A timeout, a broken connection, an answer that does not read: it may have
                            // landed. The list that is read next says which.
                            _ => Said::failed(UNKNOWN_SIGN_OUT, Some(about), false),
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
                    // The person left Devices and sessions before the answer. A toast belongs to the pane
                    // it is about, and nothing is read for a pane that does not show: what was not done is
                    // said when the pane shows again, where the list is read.
                    line_up(&mut self.later, self.engine.session_turn(), said);
                }
            }
        }
    }

    /// Send one sign-out on a worker: `key` is a session's id of the list that shows, or [`ELSEWHERE`]. Once per
    /// press; a second press while it is on its way sends nothing and says so, and so does a press that is
    /// refused before anything is sent. `true`: sent.
    ///
    /// `again`: the toast's button, with what its toast was about. "Everywhere else" is then sent one by one, for
    /// the sessions that were listed at the confirm and that the list still has: never for one made since (the
    /// one request for all of them would end those too, and nobody confirmed that).
    fn sign_out(&mut self, key: &str, again: Option<About>) -> bool {
        // The person did something else: what worked is no longer said, nor anything about this sign-out. What
        // was not done for ANOTHER sign-out stays in the line.
        self.notices
            .retain(|(_, said)| said.failed && said.about.as_ref().is_some_and(|about| about.key != key));
        self.dirty = true;
        if !self.open() {
            return false;
        }
        let one_by_one = key == ELSEWHERE && again.is_some();
        let retry = again.is_some();
        let held = self
            .sessions
            .as_ref()
            .filter(|held| self.is_now(held.session, &held.account));
        let Some(held) = held else {
            // No list to send against. From the toast's button the toast keeps its button: it works again
            // once the list has been read.
            let said = match again {
                Some(about) => Said::failed(NOT_SENT, Some(about), true),
                None => Said::failed(NOT_SENT, None, false),
            };
            self.say(said);
            return false;
        };
        let session = held.session;
        let others = || held.value.iter().filter(|row| !row.current);
        // Only rows of the list that shows, never this session, and for the button only the confirmed ones.
        let confirmed = match again {
            Some(about) => about.ids,
            None if key == ELSEWHERE => others().map(|row| row.id.clone()).collect(),
            None => vec![key.to_string()],
        };
        let targets: Vec<String> = others()
            .filter(|row| confirmed.contains(&row.id))
            .map(|row| row.id.clone())
            .collect();
        if targets.is_empty() {
            let said = if !retry {
                Said::failed(NOT_SENT, None, false)
            } else if key != ELSEWHERE {
                // The list no longer has what the toast was about: there is nothing left to send.
                Said::done(IS_SIGNED_OUT)
            } else if others().next().is_some() {
                Said::done(SIGNED_OUT_THOSE)
            } else {
                Said::done(SIGNED_OUT_ELSEWHERE)
            };
            self.say(said);
            return false;
        }
        if self.changing.iter().any(|(s, k)| *s == session && k == key) {
            self.say(Said::failed(STILL_SIGNING_OUT, None, false));
            return false;
        }
        let about = About {
            key: key.to_string(),
            ids: targets,
        };
        let (sent, fallback) = (about.clone(), about.clone());
        // As for a read: a worker that panics answers for this session, as a sign-out that is not known.
        let asked_as = stamp(&self.engine);
        let started = spawn(
            "account-sign-out",
            &self.engine,
            &self.answers,
            self.work(move |engine| {
                let before = stamp(engine);
                let mut about = sent;
                let (result, ended) = if !about.elsewhere() {
                    let result = engine.end_account_session(session, &about.key);
                    (result, usize::from(result == Ok(SessionEnd::Ended)))
                } else if !one_by_one {
                    let result = engine.revoke_other_account_sessions(session).map(|_| SessionEnd::Ended);
                    (result, 0)
                } else {
                    // Each is checked against the session by the engine as it is sent; each has its own
                    // outcome. After a refusal that holds for the rest (no session, a 401) nothing more is sent.
                    let (mut ended, mut left, mut first) = (0, Vec::new(), None);
                    let mut ids = about.ids.iter();
                    for id in ids.by_ref() {
                        match engine.end_account_session(session, id) {
                            Ok(SessionEnd::Ended) => ended += 1,
                            Ok(SessionEnd::AlreadyGone) => {}
                            Err(error) => {
                                left.push(id.clone());
                                first.get_or_insert(error);
                                if matches!(error, AccountCallError::NotSignedIn | AccountCallError::SessionEnded) {
                                    break;
                                }
                            }
                        }
                    }
                    left.extend(ids.cloned());
                    match first {
                        Some(error) => {
                            about.ids = left;
                            (Err(error), ended)
                        }
                        None => (Ok(SessionEnd::Ended), ended),
                    }
                };
                Answer::SignedOut {
                    about,
                    sent_for: session,
                    made_for: still(engine, before),
                    ended,
                    one_by_one,
                    result,
                }
            }),
            move || Answer::SignedOut {
                about: fallback,
                sent_for: session,
                made_for: Some(asked_as),
                ended: 0,
                one_by_one,
                result: Err(AccountCallError::Offline),
            },
        );
        if started {
            self.changing.push((session, key.to_string()));
        } else {
            log("account: could not start a thread for a sign-out");
            self.say(Said::failed(NOT_SENT, Some(about), true));
        }
        started
    }

    /// What the pane says, with no toast up, about values it draws that are not what Beebeeb has now: a read
    /// failed and what was read before stands, or a part of Account was never read. It stays for as long as that
    /// is so (it is not a toast: it does not fade), and its button reads again.
    fn stale_words(&self) -> Option<&'static str> {
        let turn = self.engine.session_turn();
        match self.showing? {
            Kind::Overview => {
                let overview = self.current(&self.overview)?;
                let reads = &self.overview_reads;
                if reads.reading.is_some() {
                    None
                } else if overview.name_from == Part::Missing || overview.plan_from == Part::Missing {
                    Some(NOT_ALL_READ)
                } else if !overview.whole() || reads.failed_for == Some(turn) {
                    Some(NOT_REFRESHED)
                } else {
                    None
                }
            }
            Kind::Sessions => {
                self.current(&self.sessions)?;
                let reads = &self.sessions_reads;
                (reads.reading.is_none() && reads.failed_for == Some(turn)).then_some(NOT_REFRESHED)
            }
        }
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
        if self.refused() {
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
        // Sign-outs that were not done, and not said yet: said now, over their own pane.
        if kind == Kind::Sessions {
            let turn = self.engine.session_turn();
            for (session, said) in std::mem::take(&mut self.later) {
                if self.open() && session == turn {
                    self.say(said);
                }
            }
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
    }

    fn reload(&mut self, pane: Pane) {
        if let Some(kind) = Kind::of(pane) {
            self.read(kind);
        }
    }

    fn pump(&mut self) -> Pumped {
        let mut changed = std::mem::take(&mut self.dirty);
        let refused = self.refused();
        let stale = |held: Option<(SessionGeneration, &Option<String>)>| {
            held.is_some_and(|(session, account)| !self.is_now(session, account))
        };
        // The session moved on, or the engine holds it as ended: nothing of the account stays, no toast either.
        if stale(self.overview.as_ref().map(|h| (h.session, &h.account)))
            || stale(self.sessions.as_ref().map(|h| (h.session, &h.account)))
            || self
                .changing
                .iter()
                .any(|(session, _)| *session != self.engine.session_turn())
            || (refused && !(self.changing.is_empty() && self.notices.is_empty() && self.later.is_empty()))
        {
            self.forget();
            changed = true;
        }
        while let Ok(answer) = self.inbox.try_recv() {
            self.take_in(answer);
            changed = true;
        }
        let turn = self.engine.session_turn();
        let refused = self.refused();
        if refused != self.was_refused {
            self.was_refused = refused;
            changed = true;
            if !refused {
                // The engine held the session as ended, and its next call worked: the session is asked again.
                self.overview_reads.asked_for = None;
                self.sessions_reads.asked_for = None;
            }
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
        // The toasts: one of another session is never shown; the first one shows, for its time from the
        // moment it first showed, and then the next.
        let now = self.now();
        let before = self.notices.len() + self.later.len();
        self.notices.retain(|(session, _)| *session == turn);
        self.later.retain(|(session, _)| *session == turn);
        let mut wake_in = None;
        while let Some((_, said)) = self.notices.front_mut() {
            let shown_at = *said.shown_at.get_or_insert(now);
            let left = said.stays().saturating_sub(now.saturating_duration_since(shown_at));
            if left.is_zero() {
                self.notices.pop_front();
            } else {
                wake_in = Some(left);
                break;
            }
        }
        changed |= self.notices.len() + self.later.len() != before;
        self.dirty = false;
        Pumped {
            changed,
            busy: self.overview_reads.reading.is_some()
                || self.sessions_reads.reading.is_some()
                || !self.changing.is_empty(),
            // Nothing wakes the window when the engine's flag clears: look again while a pane says so.
            watch: self.showing.is_some() && refused,
            wake_in,
        }
    }

    fn notice(&self) -> Option<&'static str> {
        match self.said() {
            Some(said) => Some(said.words),
            None => self.stale_words(),
        }
    }

    fn notice_failed(&self) -> bool {
        match self.said() {
            Some(said) => said.failed,
            None => self.stale_words().is_some(),
        }
    }

    fn notice_action(&self) -> Option<&'static str> {
        match self.said() {
            Some(said) => (said.again && said.about.is_some()).then_some(TRY_AGAIN),
            None => self.stale_words().map(|_| TRY_AGAIN),
        }
    }

    /// The toast's button: send the sign-out the toast is about again, through every check a first press goes
    /// through (and for "everywhere else", only for the sessions that were confirmed). With no toast up, the
    /// "couldn't refresh" button: read the pane again.
    fn run_notice_action(&mut self) {
        match self.said().map(|said| said.about.clone().filter(|_| said.again)) {
            Some(Some(about)) => {
                let key = about.key.clone();
                self.sign_out(&key, Some(about));
            }
            Some(None) => {}
            None => {
                if let Some(kind) = self.showing.filter(|_| self.stale_words().is_some()) {
                    self.read(kind);
                }
            }
        }
    }

    /// The window shows another pane: the toasts go. A sign-out that was not done and whose toast never showed
    /// (it waited behind another) is kept for when Devices and sessions shows again.
    fn clear_notice(&mut self) {
        self.dirty |= !self.notices.is_empty();
        for (session, said) in std::mem::take(&mut self.notices) {
            if said.failed && said.about.is_some() && said.shown_at.is_none() {
                line_up(&mut self.later, session, said);
            }
        }
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
        id != ELSEWHERE && self.sign_out(id, None)
    }

    fn sign_out_everywhere_else(&mut self) {
        self.sign_out(ELSEWHERE, None);
    }

    // ---- Not bound: each pane these belong to is `PaneState::Unwired` (it says "not built yet", and none of
    // its controls is drawn), so no drawn control reaches them. Each does nothing and claims nothing.

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
