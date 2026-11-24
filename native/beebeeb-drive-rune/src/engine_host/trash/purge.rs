//! Delete for good on the real engine: "Empty Trash" and "Delete permanently" of [`EngineTrashSource`].
//!
//! The press (the dialog's Empty Trash / Delete permanently button, with the account password typed) stamps the
//! session and the account, then ONE worker deletes. One item: [`Engine::purge_trash_for`] (its own route, one proof).
//! More than one: ONE password proof and ONE call for the first 500 ([`Engine::purge_trash_bulk_for`]); the dialog
//! says before the press when more than 500 are listed, and a call never deletes a row it did not name.
//! Both are session-bound. An answer that lands under another
//! session or account than its press touches no row and says nothing; the list is read again instead.
//!
//! Rows leave the list only as the engine reports them gone (a read that began before cannot bring them back), so
//! nothing is drawn as deleted that Beebeeb has not deleted. A wrong password deletes nothing and the dialog stays.
//!
//! Must not: log the password, a name, an id or a server's words; keep the password past the worker's call.

use beebeeb_engine::app::CredentialStores;
use beebeeb_engine::app::trash_bulk_api::{BulkPurged, MAX_BULK_IDS};
use beebeeb_engine::app::trash_purge_api::{PurgeError, Purged};
use zeroize::Zeroizing;

use super::{Answer, EngineTrashSource, Stamp, log, stamp};
use crate::frames::trash::source::{Emptying, Notice, PurgeEnd, TrashSource, TrashTotals};

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
        // More than one item: one proof and one call for the first batch (the dialog said so before the press).
        let bulk = ids.len() > 1;
        let ids: Vec<String> = if bulk {
            ids.into_iter().take(MAX_BULK_IDS).collect()
        } else {
            ids
        };
        let proof = Zeroizing::new(password.to_string());
        let (engine, answers, stamped, work) =
            (self.engine.clone(), self.answers.clone(), pressed.clone(), ids.clone());
        let started = std::thread::Builder::new().name("trash-purge".into()).spawn(move || {
            if bulk {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    engine.purge_trash_bulk_for(stamped.0, &work, proof.as_bytes())
                }))
                .unwrap_or(BulkPurged {
                    error: Some(PurgeError::Failed),
                    ..BulkPurged::default()
                });
                drop(proof);
                let _ = answers.send(Answer::BulkPurged(stamped, result));
                engine.wake_host();
                return;
            }
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

    /// What Empty Trash deletes in one go: the first batch (count, size) and how many stay for another Empty Trash.
    pub(super) fn first_batch(&self) -> (TrashTotals, usize) {
        let Some(listing) = self.current() else {
            return (self.totals(), 0);
        };
        let batch = &listing.rows[..listing.rows.len().min(MAX_BULK_IDS)];
        let totals = TrashTotals {
            items: batch.len(),
            size: crate::source::format_size(batch.iter().map(|listed| listed.bytes).sum()),
        };
        (totals, listing.rows.len() - batch.len())
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
                self.notice = Some(stopped_notice(error, result.done));
                self.purge_end = Some(PurgeEnd::Stopped);
            }
        }
        self.read();
    }
}

impl<C: CredentialStores> EngineTrashSource<C> {
    /// The answer of a bulk delete. What the engine reports deleted leaves the list now (and no read that began
    /// before can bring it back); a folder Beebeeb kept stays listed, with its reason in a notice. A wrong password
    /// deletes nothing and keeps the dialog.
    pub(super) fn bulk_purge_ended(&mut self, pressed: Stamp, result: BulkPurged) {
        if let Some(error) = result.error {
            log(format!("trash: a delete for good stopped ({})", words_of(error)));
        }
        if self.purging.as_ref().is_some_and(|p| p.stamp == pressed) {
            self.purging = None;
        }
        // Under another session or account than its press: no row is touched and nothing of it is drawn.
        if !self.is_current(&pressed) {
            self.read_at = None;
            if !result.deleted.is_empty() && (self.current().is_some() || self.reading.is_some()) {
                self.read();
            }
            return;
        }
        for id in &result.deleted {
            if let Some(listing) = self.listing.as_mut() {
                listing.rows.retain(|listed| &listed.row.id != id);
            }
            self.restored.insert(id.clone(), self.reads);
        }
        self.purge_end = Some(match result.error {
            Some(PurgeError::WrongPassword) => PurgeEnd::WrongPassword,
            Some(error) => {
                self.notice = Some(stopped_notice(error, result.deleted.len()));
                PurgeEnd::Stopped
            }
            None if result.kept.is_empty() => PurgeEnd::Done,
            None => {
                let inside = result
                    .kept
                    .iter()
                    .map(|kept| match kept.why {
                        beebeeb_engine::app::trash_bulk_api::KeptWhy::HoldsLiveItems(n) => n,
                        beebeeb_engine::app::trash_bulk_api::KeptWhy::Loop => 0,
                    })
                    .sum();
                self.notice = Some(Notice::FoldersKept {
                    done: result.deleted.len(),
                    folders: result.kept.len(),
                    inside,
                });
                PurgeEnd::Stopped
            }
        });
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
        PurgeError::NotConfirmed => "not confirmed",
        PurgeError::Failed => "refused",
    }
}

/// The notice for a delete that stopped: an unconfirmed one is never worded as "nothing was deleted" or "the rest are
/// still in Trash" (it may have gone through).
fn stopped_notice(error: PurgeError, done: usize) -> Notice {
    match error {
        PurgeError::NotConfirmed => Notice::PurgeNotConfirmed { done },
        error => Notice::PurgeStopped {
            done,
            why: why_of(error),
        },
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
        PurgeError::NotConfirmed => "The connection dropped. Beebeeb may have deleted them; check Trash.".into(),
        PurgeError::Failed => "Beebeeb did not delete them.".into(),
    }
}
