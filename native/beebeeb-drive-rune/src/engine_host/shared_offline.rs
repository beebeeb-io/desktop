//! Make available offline on a file someone else shares (DW-0505 to DW-0412). Part of
//! [`EngineSharedSource`] (`shared.rs`), which hands `keep_offline` here.
//!
//! - The press: for a row of the current session's "Shared with me" list only (an id that is not one sends
//!   nothing). The session is the one the lists were read for, taken on the window's thread before the worker starts.
//! - The call: [`Engine::keep_shared_offline_for`] on a worker of its own, the same pin change Files' own Make
//!   available offline makes. The copy lands where Files' offline copies land; no key or decrypted byte passes here.
//! - The answer: it comes down the adapter's own change channel (`Answer::Changed`), so the rules of a change hold
//!   unchanged: one per id at a time; an answer that lands under another session is dropped (no row, no notice, no
//!   read); a refusal is shown as "the change was not made" (`Notice::ChangeRefused`); nothing is drawn as kept
//!   before the engine answered. After a kept file the lists are read again, as after every change.
//!
//! Must not: log a name, a path or an id; draw a copy as kept that the engine did not keep.

use beebeeb_engine::app::bindings::shared_offline::SharedKeepRefused;

use super::*;

// This file keeps nothing between a press and its answer: the answer rides the adapter's change channel and its
// one-per-id rule is the adapter's `changing`.

impl<C: CredentialStores> EngineSharedSource<C> {
    /// Sent once per press, for a file or folder in the current session's "Shared with me" list. `false`: not such
    /// a row, a change of it is already on its way, or no worker could start.
    pub(super) fn shared_offline_keep_offline(&mut self, id: &str) -> bool {
        let Some(session) = self
            .current()
            .filter(|held| held.with_me.iter().any(|row| row.id == id))
            .map(|held| held.session)
        else {
            return false;
        };
        if self.changing.contains(id) {
            return false;
        }
        let (engine, answers, sent_for) = (self.engine.clone(), self.answers.clone(), id.to_string());
        let started = std::thread::Builder::new().name("shared-keep".into()).spawn(move || {
            let kept = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                engine.keep_shared_offline_for(session, &sent_for)
            }))
            .unwrap_or(Err(SharedKeepRefused::NotKept));
            let done = kept.map_err(|why| match why {
                SharedKeepRefused::NotSignedIn => ShareChangeError::NotSignedIn,
                SharedKeepRefused::NotShared | SharedKeepRefused::NotKept => ShareChangeError::Refused,
            });
            let _ = answers.send(Answer::Changed(sent_for, session, done));
            engine.wake_host();
        });
        if started.is_err() {
            log("shared: could not start a thread for a change");
            return self.refuse(Notice::ChangeRefused);
        }
        self.changing.insert(id.to_string());
        self.dirty = true;
        true
    }
}
