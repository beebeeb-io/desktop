//! What accepted actions changed (`FilesSource::changes`), taken into the rows: a copy next to its original, a
//! folder that was made, and what this PC holds of an item once the engine said so.

use super::local::Late;
use super::source::{Action, ActionError, Change};
use crate::State;

impl State {
    /// Take what accepted actions changed (`FilesSource::changes`) into the rows: a copy stands
    /// next to its original, selected. Returns whether a copy arrived, and what failed after all.
    /// What a change of what this PC holds did is not in the answer: it is said at the next frame
    /// ([`State::late_changes`]), whoever drained it.
    pub fn take_changes(&mut self) -> (bool, Vec<(Action, ActionError)>) {
        let mut copies = Vec::new();
        let mut failures = Vec::new();
        for change in self.actions.source.changes() {
            match change {
                Change::Copied { after, item } => {
                    // A copy made in a folder that is no longer shown is there when it is shown again.
                    if let Some(at) = self.listing.items.iter().position(|i| i.id == after) {
                        copies.push(item.id.clone());
                        self.listing.items.insert(at + 1, *item);
                    }
                }
                Change::Made { item } => {
                    // A folder made in the folder shown stands in its rows, in the folder's order.
                    if item.parent_id == self.listing.folder.id {
                        self.listing.items.push((*item).clone());
                        self.apply_sort();
                    }
                    self.actions.made.push(*item);
                }
                Change::Failed(action, why) => failures.push((action, why)),
                // A cancelled upload: its row leaves the folder now, and the message says where the copy is.
                Change::Cancelled { id } => {
                    self.listing.items.retain(|item| item.id != id);
                    self.selected = self.selected.filter(|&i| i < self.listing.items.len());
                    self.actions.selection.retain(|&i| i < self.listing.items.len());
                    self.set_toast(super::Toast::UploadCancelled);
                }
                // A move to Trash or its Undo, answered later: drawn now, per item (`to_trash.rs`).
                Change::Trash { action, items } => self.trash_answered(action, items),
                Change::Local { action, items } => {
                    // The row reads what the engine says of the item now, changed or left: by id, never by
                    // position (the rows may have changed since it was asked), never from the action.
                    for outcome in &items {
                        let Some(now) = &outcome.now else { continue };
                        for row in self.listing.items.iter_mut().filter(|row| row.id == outcome.id) {
                            row.pc_status = now.clone();
                        }
                    }
                    let left: Vec<&super::source::LocalOutcome> = items.iter().filter(|o| o.left.is_some()).collect();
                    let late = match left.first().and_then(|first| first.left) {
                        None => Late::Done(action),
                        Some(first) => Late::Left {
                            action,
                            why: if left.len() < items.len() {
                                ActionError::NotAll
                            } else if left.iter().any(|o| o.left == Some(ActionError::Protected)) {
                                ActionError::Protected
                            } else {
                                first
                            },
                            ids: left.iter().map(|o| o.id.clone()).collect(),
                            // Try again is for what asking again can change: not a protected item, not a
                            // folder that holds something kept.
                            again: left
                                .iter()
                                .filter(|o| !matches!(o.left, Some(ActionError::Protected | ActionError::HoldsKept)))
                                .map(|o| o.id.clone())
                                .collect(),
                        },
                    };
                    self.actions.local.unsaid.push(late);
                }
            }
        }
        if !copies.is_empty() {
            self.select_ids(&copies);
        }
        (!copies.is_empty(), failures)
    }

    /// Every frame (`protected::poll`): for a source that answers later (`FilesSource::answers_later`), what
    /// the engine did to the copies on this PC is drawn now, per item, and said. Never before the engine did it.
    /// `true`: a row or a message changed.
    pub(super) fn late_changes(&mut self) -> bool {
        let mut changed = false;
        if self.actions.source.answers_later() {
            let waiting = self.actions.local.unsaid.len();
            let (copied, failures) = self.take_changes();
            changed = copied || self.actions.local.unsaid.len() > waiting;
            // Another action's late failure is said the same way; it names no items.
            for (action, why) in failures {
                self.actions.local.unsaid.push(Late::Left {
                    action,
                    why,
                    ids: Vec::new(),
                    again: Vec::new(),
                });
            }
        }
        self.say_late() || changed
    }
}

#[cfg(test)]
#[path = "local_tests.rs"]
mod tests;
