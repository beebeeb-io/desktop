//! Move to Trash, and its Undo.
//!
//! One path for both kinds of source. A source that answers in the call (a fixture) is drawn at once. A source
//! that answers later (`FilesSource::answers_later`: the engine's) is only ASKED at the press: no row leaves and
//! no row comes back until it said, per item, what Beebeeb did (`Change::Trash`, taken in by
//! [`State::trash_answered`]); rows are found by id then, never by the place they had at the press.
//!
//! Try again of a failed move to Trash asks for exactly the items that were left ([`Ui::again`]), never "what is
//! selected now"; Try again of a failed Undo is Undo again (the rows it was for are held in `Actions::undo`).
//!
//! Review of 2468 (L316-L321): a new press takes an earlier failure and its Try again away; Undo holds every row
//! moved since the last Undo or change of folder; a move answered after the folder changed is still said, with
//! Undo; a row whose move to Trash is on its way is marked ([`Ui::moving`]) and cannot be renamed or moved;
//! `Actions::undo` alone says what Undo puts back (the engine's adapter only guards it).

use super::Toast;
use super::local::Late;
use super::source::{Action, ActionError, TrashOutcome};
use crate::State;
use crate::source::DriveItem;
use rune::Tx;
use std::collections::{HashMap, HashSet};

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
    /// The rows a move to Trash was accepted for and that are not answered yet (a source that answers later). Each
    /// is marked as on its way while it is here: no new name, no move to another folder ([`State::trash_on_its_way`]).
    /// A change of folder keeps them: a row Beebeeb moved after that is still Undo's.
    pub moving: Vec<Moving>,
    /// The folder each row Undo holds was moved from (by id): an Undo answered while another folder is shown puts
    /// nothing in that one. A row with no entry is the folder shown's.
    pub from: HashMap<String, Option<String>>,
}

/// A row whose move to Trash is on its way: the row, the place it had at the press and the folder it was shown in.
#[derive(Clone, Debug)]
pub struct Moving {
    pub place: usize,
    pub item: DriveItem,
    pub folder: Option<String>,
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
        // A new press: the message of an earlier failed move goes with its Try again, so no Try again is left that
        // would act on what is selected now (review of 2468, L316).
        self.actions.trash.again = None;
        if matches!(self.actions.toast, Some(Toast::Failed(Action::Trash, _))) {
            self.actions.toast = None;
            self.actions.toast_until = None;
        }
        // Refused before the source is asked (`protected.rs`).
        let asked = if self.protects(&items) {
            Err(ActionError::Protected)
        } else {
            self.actions.source.trash(&items)
        };
        let ids: Vec<String> = items.iter().map(|item| item.id.clone()).collect();
        if let Err(why) = asked {
            self.actions.trash.again = match why {
                // Asking again changes nothing for a protected item: as before, Try again is the selection's.
                ActionError::Protected => None,
                // The first press is still on its way, and its answer says what was moved: nothing to ask again.
                ActionError::InFlight => Some(Vec::new()),
                _ => Some(ids),
            };
            if let Some(stay) = self.failed_of(Action::Trash, why, &items) {
                tx.wake_after(stay);
            }
            return;
        }
        // Accepted, not done: the rows leave when the source says Beebeeb moved them (`State::trash_answered`).
        // Until then each is marked as on its way, with its place and its folder (review of 2468, L318, L320).
        if self.actions.source.answers_later() {
            let folder = self.listing.folder.id.clone();
            for item in items {
                let place = self.listing.items.iter().position(|row| row.id == item.id).unwrap_or(0);
                self.actions.trash.moving.push(Moving {
                    place,
                    item,
                    folder: folder.clone(),
                });
            }
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
        let ids: Vec<String> = rows.iter().map(|(_, item)| item.id.clone()).collect();
        self.put_back(rows);
        self.select_ids(&ids);
        true
    }

    /// The rows with these ids leave the list. What left, with the places they had: what Undo puts back.
    fn take_out(&mut self, ids: &[String]) -> Vec<(usize, DriveItem)> {
        let ids: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let rows: Vec<usize> = (0..self.listing.items.len())
            .filter(|&i| ids.contains(self.listing.items[i].id.as_str()))
            .collect();
        let mut out: Vec<(usize, DriveItem)> = rows.iter().rev().map(|&i| (i, self.listing.items.remove(i))).collect();
        out.reverse();
        out
    }

    /// Rows that were taken out stand in the list again, in the order the folder has now. A row that is shown
    /// already (the folder was read again since) is not shown twice.
    fn put_back(&mut self, rows: Vec<(usize, DriveItem)>) {
        for (i, item) in rows {
            if self.listing.items.iter().any(|row| row.id == item.id) {
                continue;
            }
            let at = i.min(self.listing.items.len());
            self.listing.items.insert(at, item);
        }
        // The rows may have been put in another order since: the old places mean nothing then.
        self.apply_sort();
    }

    /// After rows left or came back between presses: the selection is the same ITEMS as before (the ones still
    /// shown), and a name being typed stays on its row, so an open menu or a key acts on what it was opened for.
    fn same_rows(&mut self, chosen: &[String]) {
        self.select_ids(chosen);
        let at = self
            .actions
            .rename
            .as_ref()
            .map(|rename| self.listing.items.iter().position(|item| item.id == rename.id));
        match (at, &mut self.actions.rename) {
            (Some(Some(at)), Some(rename)) => rename.row = at,
            (Some(None), _) => self.actions.rename = None,
            _ => {}
        }
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
        // What is selected now, by id: it may have been chosen since the press, and stays.
        let chosen: Vec<String> = self.selected_items().into_iter().map(|item| item.id).collect();
        let answered: HashSet<&str> = items.iter().map(|outcome| outcome.id.as_str()).collect();
        // `changed`: rows left or came back in the folder shown. `say`: there is a "done" to say.
        let (changed, say) = if action == Action::Trash {
            // Answered, done or left: no longer on its way (review of 2468, L320).
            let (pressed, still): (Vec<Moving>, Vec<Moving>) = std::mem::take(&mut self.actions.trash.moving)
                .into_iter()
                .partition(|moving| answered.contains(moving.item.id.as_str()));
            self.actions.trash.moving = still;
            // By id: the rows still shown leave, with the places they have now.
            let mut moved = self.take_out(&done);
            let shown = !moved.is_empty();
            let here = self.listing.folder.id.clone();
            for (_, item) in &moved {
                self.actions.trash.from.insert(item.id.clone(), here.clone());
            }
            // A row that is no longer shown (another folder since): Beebeeb moved it all the same, so it is said and
            // Undo is for it too, with the place and the folder it had at the press (L318).
            for moving in pressed {
                let id = moving.item.id.clone();
                if done.contains(&id) && !moved.iter().any(|(_, item)| item.id == id) {
                    self.actions.trash.from.insert(id, moving.folder);
                    moved.push((moving.place, moving.item));
                }
            }
            let any = !moved.is_empty();
            // Undo is for every row moved since the last Undo or change of folder, not only this answer's: a
            // partial move and its Try again are undone together (L317).
            self.actions
                .undo
                .retain(|(_, held)| !moved.iter().any(|(_, item)| item.id == held.id));
            self.actions.undo.extend(moved);
            (shown, any)
        } else {
            let put_back: HashSet<&str> = done.iter().map(String::as_str).collect();
            let (mut back, mut kept) = (Vec::new(), Vec::new());
            for row in std::mem::take(&mut self.actions.trash.restoring) {
                if !answered.contains(row.1.id.as_str()) {
                    self.actions.trash.restoring.push(row);
                } else if put_back.contains(row.1.id.as_str()) {
                    back.push(row);
                } else {
                    kept.push(row);
                }
            }
            // Not put back: still in Trash, and still what Undo is for.
            self.actions.undo.extend(kept);
            // Put back in Beebeeb, but moved from another folder than the one shown now: it is back in its own
            // folder, and nothing is put in this one (review of 2468, L318).
            let here = self.listing.folder.id.clone();
            back.retain(|(_, item)| self.actions.trash.from.get(&item.id).is_none_or(|from| *from == here));
            let any = !back.is_empty();
            self.put_back(back);
            (any, any)
        };
        self.forget_folders();
        if changed {
            self.same_rows(&chosen);
        }
        if left.is_empty() {
            if say {
                self.actions.local.unsaid.push(Late::Done(action));
            }
            return;
        }
        // One message per answer: where some were left, that is the one (Ctrl+Z still undoes the rest). Where all
        // were left, the reason that says most, whatever the order of the items (review of 2468, L256.2).
        let why = if !done.is_empty() {
            ActionError::NotAll
        } else {
            super::local::says_most(left.iter().filter_map(|outcome| outcome.left)).unwrap_or(ActionError::Failed)
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

    /// Is a move to Trash of one of these on its way (accepted, not answered)? Rename and Move to are refused on
    /// it, with the words of a change still on its way (review of 2468, L320).
    pub(super) fn trash_on_its_way(&self, items: &[DriveItem]) -> bool {
        items
            .iter()
            .any(|item| self.actions.trash.moving.iter().any(|moving| moving.item.id == item.id))
    }

    /// Only the rows Undo holds, or an Undo waits for, keep the folder they were moved from.
    fn forget_folders(&mut self) {
        let held: HashSet<String> = self
            .actions
            .undo
            .iter()
            .chain(&self.actions.trash.restoring)
            .map(|(_, item)| item.id.clone())
            .collect();
        self.actions.trash.from.retain(|id, _| held.contains(id));
    }

    /// Is there something Try again of a failed move to Trash would ask for? The items it was left for, still
    /// shown. Nothing held (`None`: a board scene set the message, a protected item): as before, the selection.
    pub(super) fn trash_can_retry(&self) -> bool {
        match &self.actions.trash.again {
            Some(ids) => self.listing.items.iter().any(|item| ids.contains(&item.id)),
            None => true,
        }
    }

    /// Is there something Try again of a failed Undo would put back? For a source that answers in the call, as
    /// before, always.
    pub(super) fn undo_can_retry(&self) -> bool {
        !self.actions.source.answers_later() || !self.actions.undo.is_empty()
    }
}

#[cfg(test)]
#[path = "to_trash_tests.rs"]
mod tests;
