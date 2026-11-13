//! What this PC holds of the selection: Keep on this PC, Remove from this PC, Free up space.
//!
//! One path for all three ([`State::change_local`]): ask the source for exactly these items, and for a source
//! that answers later draw nothing until it said, per item, what the engine reports now (`changes.rs`).
//!
//! Try again repeats the SAME direction for the SAME items that were left ([`Ui::retry`]), never "the action,
//! on what is selected now": after a partial Remove the selection is no longer all kept, and a toggle would
//! keep everything again.
//!
//! An answer that comes between presses is said by [`State::say_late`]: its message is woken at its own time
//! by the source (`FilesSource::wake_after`), a refusal of a protected item takes the path an immediate one
//! takes ([`State::failed_of`]), and it never takes a message that still offers Try again away.

use super::Toast;
use super::source::{Action, ActionError};
use crate::State;
use crate::source::DriveItem;
use rune::Tx;

/// What the window keeps of these changes. All of it is the session's (`reset::end_session`).
#[derive(Default)]
pub struct Ui {
    /// What Try again of the message shown repeats: the action, and the ids it was not done for. An empty
    /// list: nothing can be asked again, and no Try again is drawn. `None`: nothing was held (a board scene).
    pub retry: Option<(Action, Vec<String>)>,
    /// What the source answered and the window has not said yet, oldest first.
    pub unsaid: Vec<Late>,
}

/// What an answer that came later has to say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Late {
    /// Done for every item asked.
    Done(Action),
    /// Not done for `ids` (for an action that names no items: empty), and why. `again`: the ones of them that
    /// asking again could change (Try again is for those alone).
    Left {
        action: Action,
        why: ActionError,
        ids: Vec<String>,
        again: Vec<String>,
    },
}

fn is_local(action: Action) -> bool {
    matches!(
        action,
        Action::KeepOffline | Action::RemoveOffline | Action::FreeUpSpace
    )
}

impl State {
    /// Does Make available offline read Remove from this PC? When everything selected already is kept.
    pub fn selection_is_kept(&self) -> bool {
        let items = self.selected_items();
        !items.is_empty()
            && items
                .iter()
                .all(|i| i.pc_status.as_deref() == Some("available_offline"))
    }

    /// Make the selection available offline, or Remove from this PC when all of it already is.
    pub fn toggle_keep_offline(&mut self, tx: &mut Tx) -> bool {
        if self.needs_session(tx) {
            return true;
        }
        let items = self.selected_items();
        if items.is_empty() {
            return false;
        }
        let action = if self.selection_is_kept() {
            Action::RemoveOffline
        } else {
            Action::KeepOffline
        };
        self.change_local(action, items, tx);
        true
    }

    /// Does anything selected have a copy on this PC (downloaded, or kept)? Then Free up space
    /// has something to remove.
    pub fn selection_is_on_this_pc(&self) -> bool {
        self.selected_items()
            .iter()
            .any(|i| matches!(i.pc_status.as_deref(), Some("on_this_pc" | "available_offline")))
    }

    /// Free up space: the copies of the selection on this PC go; the items stay in Beebeeb and
    /// download again when opened. What was kept on this PC is no longer kept.
    pub fn free_up_space(&mut self, tx: &mut Tx) -> bool {
        if self.needs_session(tx) {
            return true;
        }
        let items: Vec<DriveItem> = self
            .selected_items()
            .into_iter()
            .filter(|i| matches!(i.pc_status.as_deref(), Some("on_this_pc" | "available_offline")))
            .collect();
        if items.is_empty() {
            return false;
        }
        self.change_local(Action::FreeUpSpace, items, tx);
        true
    }

    /// One change of what this PC holds, of exactly these items. A refusal holds them for Try again.
    fn change_local(&mut self, action: Action, items: Vec<DriveItem>, tx: &mut Tx) {
        let ids: Vec<String> = items.iter().map(|item| item.id.clone()).collect();
        // Refused before the source is asked (`protected.rs`): the panel's switch and the menu. The switch reads
        // "keep" in both directions for a protected item, as it always did.
        let (asked, said) = if self.protects(&items) {
            let said = if action == Action::FreeUpSpace {
                action
            } else {
                Action::KeepOffline
            };
            (Err(ActionError::Protected), said)
        } else if action == Action::FreeUpSpace {
            (self.actions.source.free_up_space(&items), action)
        } else {
            let keep = action == Action::KeepOffline;
            (self.actions.source.keep_offline(&items, keep), action)
        };
        if let Err(why) = asked {
            // Try again asks for these again, in this direction, whatever is selected then. Not for a protected
            // item: asking again changes nothing.
            let again = if why == ActionError::Protected { Vec::new() } else { ids };
            self.actions.local.retry = Some((said, again));
            if let Some(stay) = self.failed_of(said, why, &items) {
                tx.wake_after(stay);
            }
            return;
        }
        self.forget_undo();
        // Accepted, not done: the rows change when the source says what it did (`State::late_changes`).
        if self.actions.source.answers_later() {
            return;
        }
        // A source that answered in the call (a fixture): what it did is drawn now.
        let status = if action == Action::KeepOffline {
            "available_offline"
        } else {
            "online_only"
        };
        for item in self.listing.items.iter_mut().filter(|item| ids.contains(&item.id)) {
            item.pc_status = Some(status.into());
        }
        match action {
            Action::KeepOffline => self.show_toast(Toast::Keeping, tx),
            Action::FreeUpSpace => self.show_toast(Toast::FreedUp, tx),
            _ => {}
        }
    }

    /// The rows shown that have these ids.
    fn rows_of(&self, ids: &[String]) -> Vec<DriveItem> {
        self.listing
            .items
            .iter()
            .filter(|item| ids.contains(&item.id))
            .cloned()
            .collect()
    }

    /// Is there something Try again of this action would ask for? The items it was left for, still shown.
    /// Nothing held (`None`: a board scene set the message): as before, the selection.
    pub(super) fn local_can_retry(&self, action: Action) -> bool {
        match &self.actions.local.retry {
            Some((held, ids)) => *held == action && self.listing.items.iter().any(|item| ids.contains(&item.id)),
            None => true,
        }
    }

    /// Try again of a change of what this PC holds: the same direction, for the items that were left alone.
    pub(super) fn local_again(&mut self, action: Action, tx: &mut Tx) {
        match self.actions.local.retry.take() {
            Some((held, ids)) => {
                let items = self.rows_of(&ids);
                if held == action && !items.is_empty() {
                    self.change_local(action, items, tx);
                }
            }
            // Nothing was held (a message a board scene set): the selection, as the menu's entry does.
            None if action == Action::FreeUpSpace => {
                self.free_up_space(tx);
            }
            None => {
                self.toggle_keep_offline(tx);
            }
        }
    }

    /// Say what the source answered between presses, oldest first. `true`: something was said.
    ///
    /// A message that still offers Try again is not taken away by an answer nobody pressed for: a failure waits
    /// until it went, a plain "done" is not said at all (the rows show it).
    pub(super) fn say_late(&mut self) -> bool {
        let mut said = false;
        while let Some(next) = self.actions.local.unsaid.first() {
            let busy =
                matches!(self.live_toast(), Some(Toast::Failed(action, why)) if self.offers_try_again(action, why));
            if busy && matches!(next, Late::Left { .. }) {
                break;
            }
            let late = self.actions.local.unsaid.remove(0);
            said |= matches!(late, Late::Left { .. });
            let stay = match late {
                Late::Done(_) if busy => None,
                Late::Done(Action::KeepOffline) => Some(self.set_toast(Toast::Keeping)),
                Late::Done(Action::FreeUpSpace) => Some(self.set_toast(Toast::FreedUp)),
                Late::Done(_) => None,
                Late::Left {
                    action,
                    why,
                    ids,
                    again,
                } => {
                    let items = self.rows_of(&ids);
                    if is_local(action) {
                        self.actions.local.retry = Some((action, again));
                    }
                    // The path an immediate refusal takes, for the items it was about (not the selection).
                    let stay = self.failed_of(action, why, &items);
                    // Never silence after "accepted": where that path has no words for a protected item (a
                    // source without them) and opened no message, the plain one.
                    let silent =
                        stay.is_none() && why == ActionError::Protected && self.actions.protected.alert.is_none();
                    stay.or_else(|| silent.then(|| self.set_toast(Toast::Failed(action, why))))
                }
            };
            if let Some(stay) = stay {
                // No press brought this message: the source wakes the window when its time is over.
                self.actions.source.wake_after(stay);
                said = true;
            }
        }
        said
    }
}
