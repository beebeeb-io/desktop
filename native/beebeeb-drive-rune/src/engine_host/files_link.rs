//! The Files window's Copy link and its Share dialog's link, on the real engine: [`LinkCalls`], the part of
//! `EngineFilesSource` behind `copy_link` and `share_link`.
//!
//! - Both are [`Engine::share_link_for`]: a link to ONE file, made (or, for a plain Copy link, found again) by the
//!   engine through core. Bound to the session AND the account of the press: both are read on the window's thread,
//!   at the press, before anything else; the engine sends nothing unless the session in memory is still that one.
//! - The call itself only says "accepted" or why not; the link, or why there is none, arrives through
//!   `FilesSource::changes` (`Change::Link`). Nothing is said and nothing is copied before that. The window puts
//!   the link on the clipboard (only its `Tx` can); here it is held from the worker's answer to the window's next
//!   look, as a `LinkText`, and nowhere else.
//! - An answer that lands under another session or account is dropped: no link, no message, nothing kept of it.
//! - One link per item at a time: while one is on its way, a second press for that item sends nothing and says so.
//! - Refused here, with nothing sent, as "not available yet": a folder, more than one item, and a link whose
//!   choice is "Can view" (core's link has no such limit: whoever holds a link can download the file, so a link
//!   that says otherwise is not made).
//!
//! Thread rule: nothing here blocks the window. Each press has a worker of its own; it sends its answer down one
//! channel and wakes the window ([`Engine::wake_host`]).
//!
//! Must not: log a name, an id or a link; hand a link to anything but `Change::Link`; read the session on a worker.

use std::collections::HashMap;
use std::sync::{Arc, mpsc};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::share_link_api::{LinkChoices, ShareLinkError};
use beebeeb_engine::app::{CredentialStores, Engine};
use zeroize::Zeroizing;

use super::log;
use super::stamp::{Stamp, stamp};
use crate::frames::files::actions_data::{LinkAccess, LinkExpiry, ShareOptions};
use crate::frames::files::link::LinkText;
use crate::frames::files::source::{Action, ActionError, Change};
use crate::source::DriveItem;

#[cfg(test)]
#[path = "files_link_tests.rs"]
mod tests;

/// What a worker sends back: the action, the stamp of its press, the item, and the link or why there is none.
struct Answer {
    action: Action,
    pressed: Stamp,
    id: String,
    link: Result<LinkText, ActionError>,
}

/// See the module's doc.
pub struct LinkCalls<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    answers: mpsc::Sender<Answer>,
    inbox: mpsc::Receiver<Answer>,
    /// The session and account the last look saw: when they move, everything held goes.
    seen: Option<Stamp>,
    /// The items a link is on its way for, each with the session of its press.
    in_flight: HashMap<String, SessionGeneration>,
    /// What the engine answered and the window has not taken yet.
    changes: Vec<Change>,
    /// Tests only: the next call waits on the first, on its worker, before it asks the engine anything, and says on
    /// the second that its answer is in the inbox.
    #[cfg(test)]
    next_call: Option<(mpsc::Receiver<()>, mpsc::Sender<()>)>,
}

impl<C: CredentialStores> LinkCalls<C> {
    /// Starts no thread and reads nothing.
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            engine,
            answers,
            inbox,
            seen: None,
            in_flight: HashMap::new(),
            changes: Vec::new(),
            #[cfg(test)]
            next_call: None,
        }
    }

    /// The session or the account is not the one the last look saw: nothing of the earlier one is kept, here, now
    /// (a link that landed and was not taken is wiped with it).
    fn look(&mut self, now: &Stamp) {
        if self.seen.as_ref() == Some(now) {
            return;
        }
        self.seen = Some(now.clone());
        self.in_flight.clear();
        self.changes.clear();
    }

    /// The plain Copy link of these. `Ok`: accepted; the link arrives through [`Self::take`].
    pub fn copy(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        // The session and account of the press, read before anything else.
        let pressed = stamp(&self.engine);
        self.look(&pressed);
        self.start(Action::CopyLink, pressed, items, None)
    }

    /// A new link to these with the Share dialog's choices. `Ok`: accepted; the link arrives through
    /// [`Self::take`].
    pub fn share(&mut self, items: &[DriveItem], options: &ShareOptions) -> Result<(), ActionError> {
        let pressed = stamp(&self.engine);
        self.look(&pressed);
        // A link cannot be held to "view only": it is not made as if it could.
        if options.access == LinkAccess::View {
            return Err(ActionError::NotYet);
        }
        let choices = LinkChoices {
            expires_in_hours: match options.expires {
                LinkExpiry::Day => Some(24),
                LinkExpiry::Week => Some(24 * 7),
                LinkExpiry::Month => Some(24 * 30),
                LinkExpiry::Never => None,
            },
            passphrase: options.password.as_ref().map(|typed| Zeroizing::new(typed.clone())),
        };
        self.start(Action::ShareLink, pressed, items, Some(choices))
    }

    fn start(
        &mut self,
        action: Action,
        pressed: Stamp,
        items: &[DriveItem],
        choices: Option<LinkChoices>,
    ) -> Result<(), ActionError> {
        if !self.engine.has_session() {
            return Err(ActionError::Unavailable);
        }
        // One file: never the first of several, never a folder.
        let [item] = items else {
            return Err(ActionError::NotYet);
        };
        if item.is_folder {
            return Err(ActionError::NotYet);
        }
        if self.in_flight.contains_key(&item.id) {
            return Err(ActionError::InFlight);
        }
        let (engine, answers) = (self.engine.clone(), self.answers.clone());
        let (stamped, id) = (pressed.clone(), item.id.clone());
        #[cfg(test)]
        let hooks = self.next_call.take();
        let started = std::thread::Builder::new().name("files-link".into()).spawn(move || {
            #[cfg(test)]
            if let Some((hold, _)) = &hooks {
                let _ = hold.recv();
            }
            // The session of the press is handed over and the engine decides (atomically with the token copy): a
            // check here could not be. A panic must not leave the item in flight for good: it answers as failed.
            let session = stamped.0;
            let link = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                engine
                    .share_link_for(session, &id, choices.as_ref())
                    .map(|made| LinkText::new(made.expose()))
                    .map_err(why)
            }))
            .unwrap_or_else(|_| {
                log("files: a link ended early");
                Err(ActionError::Failed)
            });
            let _ = answers.send(Answer {
                action,
                pressed: stamped,
                id,
                link,
            });
            #[cfg(test)]
            if let Some((_, ended)) = &hooks {
                let _ = ended.send(());
            }
            engine.wake_host();
        });
        if started.is_err() {
            log("files: could not start the worker");
            return Err(ActionError::Failed);
        }
        self.in_flight.insert(item.id.clone(), pressed.0);
        Ok(())
    }

    /// Take in what the workers answered. An answer counts only for the session AND the account of its press.
    fn pump(&mut self) {
        let now = stamp(&self.engine);
        self.look(&now);
        let here = self.engine.has_session();
        while let Ok(answer) = self.inbox.try_recv() {
            // The mark of this press goes on every outcome; a mark a later session's press put there stays.
            if self.in_flight.get(&answer.id) == Some(&answer.pressed.0) {
                self.in_flight.remove(&answer.id);
            }
            // The session's turn only moves forward, so a stamp that is current now was current for the whole call.
            if !(here && answer.pressed == now) {
                log("files: the answer of an earlier session was dropped");
                continue;
            }
            self.changes.push(Change::Link {
                action: answer.action,
                answer: answer.link,
            });
        }
    }

    /// Is a link still on its way for the session that is here? Asked after [`Self::take`], which drops the
    /// marks of an earlier session.
    pub fn pending(&self) -> bool {
        !self.in_flight.is_empty()
    }

    /// What the links that were asked for answered since the last call, oldest first.
    pub fn take(&mut self) -> Vec<Change> {
        self.pump();
        std::mem::take(&mut self.changes)
    }
}

/// The engine's refusal in the window's words. Never a word of the server's.
fn why(refused: ShareLinkError) -> ActionError {
    match refused {
        ShareLinkError::NotSignedIn => ActionError::Unavailable,
        ShareLinkError::Offline => ActionError::Offline,
        ShareLinkError::NotAFile => ActionError::NotYet,
        ShareLinkError::HasLink => ActionError::HasLink,
        ShareLinkError::SessionEnded
        | ShareLinkError::NotFound
        | ShareLinkError::KeyUnavailable
        | ShareLinkError::Invalid
        | ShareLinkError::Refused => ActionError::Failed,
    }
}
