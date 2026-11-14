//! Delete for good on the real engine: "Empty Trash" and "Delete permanently" of [`EngineTrashSource`].
//!
//! The press (the dialog's Empty Trash / Delete permanently button, with the account password typed) stamps the
//! session and the account, then ONE worker proves the password for each item in turn and deletes it
//! ([`Engine::purge_trash_for`]: session-bound, one single-use token per item). An answer that lands under another
//! session or account than its press touches no row and says nothing; the list is read again instead.
//!
//! Rows leave the list only as the engine reports them gone (a read that began before cannot bring them back), so
//! nothing is drawn as deleted that Beebeeb has not deleted. A wrong password deletes nothing and the dialog stays.
//!
//! Must not: log the password, a name, an id or a server's words; keep the password past the worker's call.

use beebeeb_engine::app::CredentialStores;
use beebeeb_engine::app::trash_purge_api::{PurgeError, Purged};
use zeroize::Zeroizing;

use super::{Answer, EngineTrashSource, Stamp, log, stamp};
use crate::frames::trash::source::{Emptying, Notice, PurgeEnd};

/// A delete for good on its way.
pub(super) struct Purging {
    /// The session and account of the press.
    stamp: Stamp,
    /// The ids, in the order they are deleted.
    ids: Vec<String>,
    /// How many are gone.
    done: usize,
    /// Empty Trash (all of it), not one item: only that one draws the "Emptying Trash" note.
    whole: bool,
}

impl<C: CredentialStores> EngineTrashSource<C> {
    /// The note's numbers while Empty Trash is on its way.
    pub(super) fn emptying_now(&self) -> Option<Emptying> {
        let purging = self.purging.as_ref().filter(|p| p.whole)?;
        Some(Emptying {
            done: purging.done,
            of: purging.ids.len(),
        })
    }

    /// Start the delete: `id` (one item of the current list) or, with `None`, every item of it. `false`: nothing
    /// started (no password, no such item, nothing listed, another delete on its way, or no worker).
    pub(super) fn start_purge(&mut self, id: Option<&str>, password: &str) -> bool {
        if password.is_empty() || self.purging.is_some() {
            return false;
        }
        // The session and account of the press, read BEFORE the list the items are chosen from is looked at.
        let pressed = stamp(&self.engine);
        let Some(listing) = self.current().filter(|listing| listing.session == pressed.0) else {
            return false;
        };
        let ids: Vec<String> = match id {
            Some(id) => listing
                .rows
                .iter()
                .filter(|listed| listed.row.id == id)
                .map(|listed| listed.row.id.clone())
                .collect(),
            None => listing.rows.iter().map(|listed| listed.row.id.clone()).collect(),
        };
        if ids.is_empty() {
            return false;
        }
        let proof = Zeroizing::new(password.to_string());
        let (engine, answers, stamped, work) =
            (self.engine.clone(), self.answers.clone(), pressed.clone(), ids.clone());
        let started = std::thread::Builder::new().name("trash-purge".into()).spawn(move || {
            let progress_to = answers.clone();
            let (progress_stamp, wake) = (stamped.clone(), engine.clone());
            // A panic in this worker must not leave the delete "on its way" for good: it answers as a failure.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                engine.purge_trash_for(stamped.0, &work, proof.as_bytes(), |done| {
                    let _ = progress_to.send(Answer::PurgeProgress(progress_stamp.clone(), done));
                    wake.wake_host();
                })
            }))
            .unwrap_or(Purged {
                done: 0,
                error: Some(PurgeError::Failed),
            });
            drop(proof);
            let _ = answers.send(Answer::Purged(stamped, result));
            engine.wake_host();
        });
        if started.is_err() {
            log("trash: could not start a thread for a delete");
            return false;
        }
        self.purging = Some(Purging {
            stamp: pressed,
            ids,
            done: 0,
            whole: id.is_none(),
        });
        self.dirty = true;
        true
    }

    /// Whether `pressed` is the session and account the window is under now.
    fn is_current(&self, pressed: &Stamp) -> bool {
        self.engine.has_session() && *pressed == stamp(&self.engine)
    }

    /// `done` items are gone: they leave the list, and no read that began before can bring them back.
    pub(super) fn purge_progress(&mut self, pressed: Stamp, done: usize) {
        if !self.is_current(&pressed) {
            return;
        }
        let Some(purging) = self.purging.as_mut().filter(|p| p.stamp == pressed) else {
            return;
        };
        let gone: Vec<String> = purging.ids.iter().take(done).skip(purging.done).cloned().collect();
        purging.done = done.min(purging.ids.len());
        for id in gone {
            if let Some(listing) = self.listing.as_mut() {
                listing.rows.retain(|listed| listed.row.id != id);
            }
            self.restored.insert(id, self.reads);
        }
    }

    pub(super) fn purge_ended(&mut self, pressed: Stamp, result: Purged) {
        if let Some(error) = result.error {
            log(format!("trash: a delete for good stopped ({})", words_of(error)));
        }
        let mine = self.purging.as_ref().is_some_and(|p| p.stamp == pressed);
        if mine {
            self.purging = None;
        }
        // Under another session or account than its press: no row is touched and nothing of it is drawn. Beebeeb may
        // have deleted some: a list on screen or on its way is read again, and the next opening reads again.
        if !self.is_current(&pressed) {
            self.read_at = None;
            if result.done > 0 && (self.current().is_some() || self.reading.is_some()) {
                self.read();
            }
            return;
        }
        match result.error {
            None => self.purge_end = Some(PurgeEnd::Done),
            Some(PurgeError::WrongPassword) if result.done == 0 => self.purge_end = Some(PurgeEnd::WrongPassword),
            Some(error) => {
                self.notice = Some(Notice::PurgeStopped {
                    done: result.done,
                    why: why_of(error),
                });
                self.purge_end = Some(PurgeEnd::Stopped);
            }
        }
        self.read();
    }
}

/// A few fixed words for a log line (never a name, an id or a server's words).
fn words_of(error: PurgeError) -> &'static str {
    match error {
        PurgeError::NotSignedIn => "not signed in",
        PurgeError::SessionEnded => "the session ended",
        PurgeError::WrongPassword => "wrong password",
        PurgeError::RateLimited(_) => "rate limited",
        PurgeError::Offline => "offline",
        PurgeError::Failed => "refused",
    }
}

/// The reason a delete stopped, in the words the view says.
fn why_of(error: PurgeError) -> String {
    match error {
        PurgeError::NotSignedIn => "Beebeeb was locked or signed out.".into(),
        PurgeError::SessionEnded => "Your session ended. Sign in again.".into(),
        PurgeError::WrongPassword => "That password is not right.".into(),
        PurgeError::RateLimited(secs) => format!("Too many tries. Wait {secs} seconds."),
        PurgeError::Offline => "Beebeeb could not be reached.".into(),
        PurgeError::Failed => "Beebeeb did not delete them.".into(),
    }
}
