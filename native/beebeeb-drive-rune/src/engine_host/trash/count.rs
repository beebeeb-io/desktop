//! The sidebar's Trash badge: the server's own count ([`Engine::trash_count_for`], `GET /api/v1/files/count`), read
//! on a worker, never by listing and decrypting the Trash.
//!
//! When it is read: when the poll finds a session and no badge of this session and account (so the badge is there
//! from every section, not only from Trash), when the Trash section opens, and after a restore or a delete for good
//! was answered (the badge moves with them). A read that is asked for while one is on its way is made again when
//! that one lands. A failed read keeps the number already shown (it is the last thing Beebeeb said) and is tried again
//! no sooner than [`RETRY_AFTER`]; with none shown there is no badge, never a zero.
//!
//! Session rule (as the list's): the read is made for the session AND the account of the poll that started it
//! ([`super::stamp`]), the engine sends nothing unless the session in memory is still that one, and an answer that
//! lands under another session or account, or after a `forget`, is dropped. The badge is handed out only while it
//! belongs to the current stamp.
//!
//! Must not: read the whole Trash, log a count of another account, or show a number before Beebeeb said it.

use std::time::{Duration, Instant};

use beebeeb_engine::app::CredentialStores;
use beebeeb_engine::app::settings_api::AccountCallError;

use super::{Answer, EngineTrashSource, Stamp, log, stamp};

/// A failed read is not tried again sooner than this (the poll would otherwise ask on every tick).
pub const RETRY_AFTER: Duration = Duration::from_secs(30);

/// The last number Beebeeb said, and the session and account it was said to.
pub(super) struct Badge {
    stamp: Stamp,
    items: usize,
}

/// The read on its way: its number and what it was asked under.
pub(super) struct Counting {
    number: u64,
}

/// Everything the badge keeps, in one field of the source.
#[derive(Default)]
pub(super) struct CountState {
    badge: Option<Badge>,
    counting: Option<Counting>,
    /// Numbers the reads.
    reads: u64,
    /// A read was asked for while one was on its way.
    again: bool,
    /// When the last read failed.
    failed_at: Option<Instant>,
}

impl CountState {
    pub(super) fn busy(&self) -> bool {
        self.counting.is_some()
    }
}

impl<C: CredentialStores> EngineTrashSource<C> {
    /// The badge, while it belongs to the current session and account.
    pub(super) fn badge(&self) -> Option<usize> {
        if !self.engine.has_session() {
            return None;
        }
        let badge = self.counted.badge.as_ref()?;
        (badge.stamp == stamp(&self.engine)).then_some(badge.items)
    }

    /// Ask for the count on a worker, unless one is on its way (then once more when it lands).
    pub(super) fn start_count(&mut self) {
        if !self.engine.has_session() {
            return;
        }
        if self.counted.counting.is_some() {
            self.counted.again = true;
            return;
        }
        self.counted.again = false;
        self.counted.reads += 1;
        let number = self.counted.reads;
        // The session and account of this read, taken on the window's thread before the worker starts.
        let asked = stamp(&self.engine);
        let (engine, answers, session) = (self.engine.clone(), self.answers.clone(), asked.0);
        let sent_for = asked.clone();
        let started = std::thread::Builder::new().name("trash-count".into()).spawn(move || {
            // A panic must not leave the read on its way for good: it answers as an error.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.trash_count_for(session)))
                .unwrap_or(Err(AccountCallError::Offline));
            let _ = answers.send(Answer::Counted(number, sent_for, result));
            engine.wake_host();
        });
        match started {
            Ok(_) => self.counted.counting = Some(Counting { number }),
            Err(_) => {
                log("trash: could not start a thread for the count");
                self.counted.failed_at = Some(Instant::now());
            }
        }
    }

    /// The poll: read the count when there is a session and no badge of this session and account to show, and the
    /// last read did not fail a moment ago.
    pub(super) fn count_if_missing(&mut self) {
        if self.counted.counting.is_some() || !self.engine.has_session() || self.badge().is_some() {
            return;
        }
        if self.counted.failed_at.is_some_and(|at| at.elapsed() < RETRY_AFTER) {
            return;
        }
        self.start_count();
    }

    /// A restore or a delete for good was answered: the count moved (or may have).
    pub(super) fn count_moved(&mut self) {
        self.counted.failed_at = None;
        self.start_count();
    }

    /// An answer of the count read with this number.
    pub(super) fn take_count(&mut self, number: u64, asked: Stamp, result: Result<u64, AccountCallError>) {
        // A `forget` (or a newer read) took the read out: its answer is not this source's any more.
        if self.counted.counting.as_ref().map(|c| c.number) != Some(number) {
            return;
        }
        self.counted.counting = None;
        // Made for a session or account that is gone: dropped, never drawn.
        let current = self.engine.has_session() && asked == stamp(&self.engine);
        match result {
            Ok(items) if current => {
                self.counted.failed_at = None;
                self.counted.badge = Some(Badge {
                    stamp: asked,
                    items: usize::try_from(items).unwrap_or(usize::MAX),
                });
            }
            Ok(_) => {}
            Err(kind) => {
                if kind != AccountCallError::NotSignedIn {
                    log(format!("trash: the count did not load ({kind})"));
                }
                self.counted.failed_at = Some(Instant::now());
            }
        }
        if std::mem::take(&mut self.counted.again) {
            self.counted.failed_at = None;
            self.start_count();
        }
    }

    /// The session or the account changed: no number is kept, and a read on its way is not waited for.
    pub(super) fn forget_count(&mut self) {
        self.counted = CountState {
            reads: self.counted.reads,
            ..CountState::default()
        };
    }
}
