//! Move to Trash, and its Undo.
//!
//! One path for both kinds of source. A source that answers in the call (a fixture) is drawn at once. A source
//! that answers later (`FilesSource::answers_later`: the engine's) is only ASKED at the press: no row leaves and
//! no row comes back until it said, per item, what Beebeeb did (`Change::Trash`, taken in by
//! [`State::trash_answered`]); rows are found by id then, never by the place they had at the press.
//!
//! Try again of a failed move to Trash asks for exactly the items that were left ([`Ui::again`]), never "what is
//! selected now"; Try again of a failed Undo is Undo again (the rows it was for are held in `Actions::undo`).

use super::Toast;
use super::local::Late;
use super::source::{Action, ActionError, TrashOutcome};
use crate::State;
use crate::source::DriveItem;
use rune::Tx;

/// What the window keeps of moves to and from Trash beside `Actions::undo`. All of it is the session's
/// (`reset::end_session`).
#[derive(Default)]
pub struct Ui {
    /// The rows an Undo was accepted for and that are not answered yet (a source that answers later), with the
    /// places they had: put back when the source says Beebeeb did, held for Undo again when it says it did not.
    pub restoring: Vec<(usize, DriveItem)>,
    /// What Try again of a failed move to Trash asks for: exactly these. `None`: nothing was held (a board scene
    /// set the message, or a protected item was refused), and it is the selection.
    pub again: Option<Vec<String>>,
}

impl State {
    /// Move the selection to Trash, without a question: Undo and Trash both bring it back.
    /// Like the other actions it answers whether anything is to be drawn: a change, or the message of a failure.
    pub fn trash_selection(&mut self, tx: &mut Tx) -> bool {
        if self.needs_session(tx) {
            return true;
        }
        let items = self.selected_items();
        if items.is_empty() {
            return false;
        }
        self.trash_these(items, tx);
        true
    }

    /// Move exactly these rows to Trash.
    fn trash_these(&mut self, items: Vec<DriveItem>, tx: &mut Tx) {
        self.actions.trash.again = None;
        // Refused before the source is asked (`protected.rs`).
        let asked = if self.protects(&items) {
            Err(ActionError::Protected)
        } else {
            self.actions.source.trash(&items)
        };
        let ids: Vec<String> = items.iter().map(|item| item.id.clone()).collect();
        if let Err(why) = asked {
            if why != ActionError::Protected {
                self.actions.trash.again = Some(ids);
            }
            if let Some(stay) = self.failed_of(Action::Trash, why, &items) {
                tx.wake_after(stay);
            }
            return;
        }
        // Accepted, not done: the rows leave when the source says Beebeeb moved them (`State::trash_answered`).
        if self.actions.source.answers_later() {
            return;
        }
        self.actions.undo = self.take_out(&ids);
        self.clear_selection();
        self.show_toast(Toast::Trashed, tx);
    }

    /// Try again of a failed move to Trash: the items it was left for, as far as they are still shown.
    pub(super) fn trash_again(&mut self, tx: &mut Tx) {
        match self.actions.trash.again.take() {
            Some(ids) => {
                let items = self.rows_of(&ids);
                if !items.is_empty() {
                    self.trash_these(items, tx);
                }
            }
            None => {
                self.trash_selection(tx);
            }
        }
    }

    /// Undo the last Move to Trash: every row back where it was (in the order the folder has now).
    pub fn undo(&mut self, tx: &mut Tx) -> bool {
        if self.needs_session(tx) {
            return true;
        }
        let rows = std::mem::take(&mut self.actions.undo);
        if rows.is_empty() {
            return false;
        }
        let items: Vec<DriveItem> = rows.iter().map(|(_, item)| item.clone()).collect();
        if let Err(why) = self.actions.source.restore(&items) {
            self.actions.undo = rows;
            self.failed(Action::Restore, why, tx);
            // Handled: there is a message to draw.
            return true;
        }
        self.actions.toast = None;
        // Accepted, not done: the rows come back when the source says Beebeeb put them back.
        if self.actions.source.answers_later() {
            self.actions.trash.restoring.extend(rows);
            return true;
        }
        self.put_back(rows);
        true
    }

    /// The rows with these ids leave the list. What left, with the places they had: what Undo puts back.
    fn take_out(&mut self, ids: &[String]) -> Vec<(usize, DriveItem)> {
        let rows: Vec<usize> = (0..self.listing.items.len())
            .filter(|&i| ids.contains(&self.listing.items[i].id))
            .collect();
        let mut out: Vec<(usize, DriveItem)> = rows.iter().rev().map(|&i| (i, self.listing.items.remove(i))).collect();
        out.reverse();
        out
    }

    /// Rows that were taken out stand in the list again, selected.
    fn put_back(&mut self, rows: Vec<(usize, DriveItem)>) {
        let ids: Vec<String> = rows.iter().map(|(_, item)| item.id.clone()).collect();
        for (i, item) in rows {
            let at = i.min(self.listing.items.len());
            self.listing.items.insert(at, item);
        }
        self.select_ids(&ids);
        // The rows may have been put in another order since: the old places mean nothing then.
        self.apply_sort();
    }

    /// A source that answers later said what a move to Trash or its Undo did (`Change::Trash`), per item: the rows
    /// Beebeeb moved leave now, the rows it put back stand in the list again, and what was left is said.
    pub(super) fn trash_answered(&mut self, action: Action, items: Vec<TrashOutcome>) {
        let done: Vec<String> = items
            .iter()
            .filter(|item| item.left.is_none())
            .map(|item| item.id.clone())
            .collect();
        let left: Vec<&TrashOutcome> = items.iter().filter(|item| item.left.is_some()).collect();
        if action == Action::Trash {
            // By id: a row that is no longer shown (another folder since) is simply not here to leave.
            let rows = self.take_out(&done);
            if !rows.is_empty() {
                self.actions.undo = rows;
                self.clear_selection();
                self.actions.local.unsaid.push(Late::Done(Action::Trash));
            }
        } else {
            let (answered, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.actions.trash.restoring)
                .into_iter()
                .partition(|(_, item)| items.iter().any(|outcome| outcome.id == item.id));
            self.actions.trash.restoring = waiting;
            let (back, kept): (Vec<_>, Vec<_>) = answered.into_iter().partition(|(_, item)| done.contains(&item.id));
            // Not put back: still in Trash, and still what Undo is for.
            self.actions.undo.extend(kept);
            if !back.is_empty() {
                self.put_back(back);
                self.actions.local.unsaid.push(Late::Done(Action::Restore));
            }
        }
        let Some(first) = left.first().and_then(|outcome| outcome.left) else {
            return;
        };
        let why = if !done.is_empty() {
            ActionError::NotAll
        } else if left.iter().any(|outcome| outcome.left == Some(ActionError::Protected)) {
            ActionError::Protected
        } else {
            first
        };
        self.actions.local.unsaid.push(Late::Left {
            action,
            why,
            ids: left.iter().map(|outcome| outcome.id.clone()).collect(),
            // Try again is for what asking again can change: not a protected item.
            again: left
                .iter()
                .filter(|outcome| outcome.left != Some(ActionError::Protected))
                .map(|outcome| outcome.id.clone())
                .collect(),
        });
    }
}

#[cfg(test)]
#[path = "to_trash_tests.rs"]
mod tests;
