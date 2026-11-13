//! Copy link, and what a link that was asked for answers.
//!
//! One path for both kinds of source. A source that answers in the call (a fixture) put the link on the clipboard
//! itself, and "Link copied" is said at once. A source that answers later (`FilesSource::answers_later`: the
//! engine's) is only ASKED at the press: nothing is said and nothing is copied until it answered
//! (`Change::Link`). The link then goes to the clipboard here, in [`poll`], because only a `Tx` writes the
//! clipboard; it is held for that one step and nowhere else, and it is never drawn.
//!
//! Try again of a failed Copy link asks for exactly the items of that press ([`Ui::again`]), never "what is
//! selected now"; of a failed link with choices it opens the Share dialog again (`share.rs`), as before.

use super::Toast;
use super::share::Share;
use super::source::{Action, ActionError};
use crate::State;
use crate::source::DriveItem;
use rune::Tx;

/// A share link on its way to the clipboard. Its key part opens the file: it is never drawn, never printed
/// (`Debug` says only that it is a link) and wiped when this is dropped.
#[derive(Clone)]
pub struct LinkText(zeroize::Zeroizing<String>);

impl LinkText {
    pub fn new(link: &str) -> Self {
        Self(zeroize::Zeroizing::new(link.to_string()))
    }

    /// The whole link: for the clipboard and nothing else.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for LinkText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LinkText(a link)")
    }
}

/// What the window keeps of links that were asked for. All of it is the session's (`reset::end_session`).
#[derive(Default)]
pub struct Ui {
    /// What the source answered and this frame's `Tx` has not been given yet: a link for the clipboard, or why
    /// there is none. Emptied by [`poll`] in the same frame it was filled.
    pub landed: Vec<(Action, Result<LinkText, ActionError>)>,
    /// The items of a Copy link that was accepted and is not answered yet.
    pub asked: Option<Vec<String>>,
    /// What Try again of a failed Copy link asks for: exactly these. `None`: nothing was held (a board scene set
    /// the message, or a protected item was refused), and it is the selection.
    pub again: Option<Vec<String>>,
    /// The Share dialog's items and choices (never its password) of a link that was accepted and is not answered
    /// yet: what Try again opens if it fails.
    pub share: Option<Share>,
}

impl State {
    /// Copy the selection's share link. "Link copied" is said once it is on the clipboard, not before.
    pub fn copy_link(&mut self, tx: &mut Tx) -> bool {
        if self.needs_session(tx) {
            return true;
        }
        let items = self.selected_items();
        if items.is_empty() {
            return false;
        }
        self.copy_link_of(items, tx);
        true
    }

    /// Ask for the link of exactly these rows.
    fn copy_link_of(&mut self, items: Vec<DriveItem>, tx: &mut Tx) {
        self.actions.link.again = None;
        if self.protects(&items) {
            self.failed(Action::CopyLink, ActionError::Protected, tx);
            return;
        }
        self.forget_undo();
        let ids: Vec<String> = items.iter().map(|item| item.id.clone()).collect();
        match self.actions.source.copy_link(&items) {
            // Accepted, not made: said, and copied, when the source answered ([`poll`]).
            Ok(()) if self.actions.source.answers_later() => self.actions.link.asked = Some(ids),
            Ok(()) => self.show_toast(Toast::LinkCopied, tx),
            Err(why) => {
                self.actions.link.again = match why {
                    // The first press is still on its way, and its answer is the link: nothing to ask again.
                    ActionError::InFlight => Some(Vec::new()),
                    _ => Some(ids),
                };
                if let Some(stay) = self.failed_of(Action::CopyLink, why, &items) {
                    tx.wake_after(stay);
                }
            }
        }
    }

    /// Try again of a failed Copy link: the items it was asked for, as far as they are still shown.
    pub(super) fn copy_link_again(&mut self, tx: &mut Tx) {
        match self.actions.link.again.take() {
            Some(ids) => {
                let items = self.rows_of(&ids);
                if !items.is_empty() {
                    self.copy_link_of(items, tx);
                }
            }
            None => {
                self.copy_link(tx);
            }
        }
    }

    /// Is there something Try again of a failed Copy link would ask for? The items of that press, still shown.
    /// Nothing held (`None`: a board scene set the message, a protected item): as before, the selection.
    pub(super) fn link_can_retry(&self) -> bool {
        match &self.actions.link.again {
            Some(ids) => self.listing.items.iter().any(|item| ids.contains(&item.id)),
            None => true,
        }
    }
}

/// Every frame, after the source was asked what it did (`shell::host::poll`): a link that landed goes to the
/// clipboard and is said; a link that was not made is said, with what its Try again asks for. `true`: something
/// was said.
pub fn poll(s: &mut State, tx: &mut Tx) -> bool {
    let landed = std::mem::take(&mut s.actions.link.landed);
    let any = !landed.is_empty();
    for (action, answer) in landed {
        let asked = match action {
            Action::ShareLink => None,
            _ => s.actions.link.asked.take(),
        };
        let share = match action {
            Action::ShareLink => s.actions.link.share.take(),
            _ => None,
        };
        match answer {
            Ok(link) => {
                // The one place a link leaves for: the clipboard. Dropped (and wiped) right after.
                tx.copy(link.expose());
                s.show_toast(Toast::LinkCopied, tx);
            }
            Err(why) => {
                if action == Action::ShareLink {
                    // What Try again opens: the same items and choices, without the password.
                    s.actions.retry_share = share;
                } else {
                    s.actions.link.again = asked;
                }
                s.failed(action, why, tx);
            }
        }
    }
    any
}

#[cfg(test)]
#[path = "link_tests.rs"]
mod tests;
