//! The first-time guide's "done" mark on the real engine (`frames::signin::setup::guide_marks`): once per account
//! on this PC, kept in the account's own local state by the engine ([`Engine::guide_mark_for`],
//! [`Engine::mark_guide_done_for`]).
//!
//! Session rule: a question and a mark carry the session and the account they were made under, taken on the
//! window's thread; the engine reads and writes for that session only, and an answer is taken in only while that
//! session and account are still the engine's. The engine functions take no account: whose mark it is, is the
//! session's.
//!
//! A mark that could not be kept (nothing has synced on this PC yet, so there is no state proven to be the
//! account's to write into) is held for that ACCOUNT and asked again, at most once per [`RETRY_AFTER`], while a
//! session of the same account is in memory. It is not kept across a restart of the app: see the PR record.
//!
//! Thread rule: every engine call runs on a worker of its own; the window's poll ([`GuideMarks::answer`]) takes the
//! answers in. Must not: log an account, an id or a path; only a refusal's fixed word.

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::app::owned_state::guide_mark::GuideMark;
use beebeeb_engine::app::{CredentialStores, Engine, LocalStateRefused};

use super::log;
use super::stamp::{Stamp, account_of, stamp};
use crate::frames::signin::setup::guide_marks::{GuideMarks, Known};

/// A mark that could not be kept is tried again after this long.
pub const RETRY_AFTER: Duration = Duration::from_secs(30);

enum Answer {
    Read {
        asked: Stamp,
        mark: GuideMark,
    },
    Kept {
        asked: Stamp,
        result: Result<(), LocalStateRefused>,
    },
}

pub struct EngineGuideMarks<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    /// The answer to the last question, for the session and account it was asked under.
    known: Option<(Stamp, Known)>,
    /// The account whose mark is still to be kept, and when it was last tried.
    owed: Option<(String, Instant)>,
    /// A mark is on its way.
    keeping: bool,
    answers: mpsc::Sender<Answer>,
    inbox: mpsc::Receiver<Answer>,
}

impl<C: CredentialStores + 'static> EngineGuideMarks<C> {
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            engine,
            known: None,
            owed: None,
            keeping: false,
            answers,
            inbox,
        }
    }

    fn is_now(&self, asked: &Stamp) -> bool {
        self.engine.has_session() && *asked == stamp(&self.engine)
    }

    fn spawn(&self, name: &str, work: impl FnOnce(&Engine<C>) -> Answer + Send + 'static) -> bool {
        let (engine, answers) = (self.engine.clone(), self.answers.clone());
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let _ = answers.send(work(&engine));
                engine.wake_host();
            })
            .is_ok()
    }

    /// Send the mark for the session in memory, when it is `account`'s.
    fn keep(&mut self, account: &str) {
        let asked = stamp(&self.engine);
        if !self.engine.has_session() || asked.1.as_deref() != Some(account) {
            return;
        }
        self.owed = Some((account.to_string(), Instant::now()));
        let session = asked.0;
        self.keeping = self.spawn("guide-mark", move |engine| Answer::Kept {
            result: engine.mark_guide_done_for(session),
            asked,
        });
        if !self.keeping {
            log("guide: could not start a thread to keep the mark");
        }
    }

    fn take_in(&mut self) {
        while let Ok(answer) = self.inbox.try_recv() {
            match answer {
                Answer::Read { asked, mark } => {
                    let known = match mark {
                        GuideMark::Done => Known::Done,
                        GuideMark::NotDone => Known::NotDone,
                        GuideMark::NotKnown(refused) => {
                            log(format!("guide: the mark was not read ({})", refused.kind()));
                            Known::NotKnown
                        }
                    };
                    // Made for a session or an account that is not the engine's now: never used.
                    if self.is_now(&asked) {
                        self.known = Some((asked, known));
                    }
                }
                Answer::Kept { asked, result } => {
                    self.keeping = false;
                    match result {
                        // Kept: nothing is owed for that account any more.
                        Ok(()) => {
                            if self.owed.as_ref().map(|(account, _)| account.as_str()) == asked.1.as_deref() {
                                self.owed = None;
                            }
                        }
                        Err(refused) => log(format!("guide: the mark was not kept ({})", refused.kind())),
                    }
                }
            }
        }
    }
}

impl<C: CredentialStores + 'static> GuideMarks for EngineGuideMarks<C> {
    fn ask(&mut self, _account: Option<&str>) {
        self.known = None;
        if !self.engine.has_session() {
            return;
        }
        let asked = stamp(&self.engine);
        // The account finished the guide in this run and the mark is still to be kept: it is done.
        if self.owed.as_ref().map(|(account, _)| account.as_str()) == asked.1.as_deref() && asked.1.is_some() {
            self.known = Some((asked, Known::Done));
            return;
        }
        let session = asked.0;
        let started = self.spawn("guide-mark-read", move |engine| Answer::Read {
            mark: engine.guide_mark_for(session),
            asked,
        });
        if !started {
            log("guide: could not start a thread to read the mark");
        }
    }

    fn answer(&mut self, _account: Option<&str>) -> Option<Known> {
        self.take_in();
        // A mark still owed: asked again while a session of its account is here, not more often than the wait.
        if !self.keeping
            && let Some((account, tried)) = self.owed.clone()
            && tried.elapsed() >= RETRY_AFTER
        {
            self.keep(&account);
        }
        let (asked, known) = self.known.take()?;
        self.is_now(&asked).then_some(known)
    }

    fn mark_done(&mut self, _account: Option<&str>) {
        // The engine's own account, read here on the window's thread at the press.
        let Some(account) = account_of(&self.engine).filter(|_| self.engine.has_session()) else {
            return;
        };
        self.keep(&account);
    }
}

#[cfg(test)]
impl<C: CredentialStores + 'static> EngineGuideMarks<C> {
    /// Whether a mark is still to be kept.
    pub(crate) fn owes(&self) -> bool {
        self.owed.is_some()
    }
    pub(crate) fn busy(&self) -> bool {
        self.keeping
    }
    /// The wait before the next try is over.
    pub(crate) fn wait_is_over(&mut self) {
        if let Some((_, tried)) = &mut self.owed {
            *tried = Instant::now() - RETRY_AFTER;
        }
    }
}

#[cfg(test)]
#[path = "guide_tests.rs"]
mod tests;
