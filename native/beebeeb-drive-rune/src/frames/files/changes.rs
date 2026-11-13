//! What accepted actions changed (`FilesSource::changes`), taken into the rows: a copy next to its original, a
//! folder that was made, and what this PC holds of an item once the engine said so.

use super::Toast;
use super::source::{Action, ActionError, Change};
use crate::State;

impl State {
    /// Take what accepted actions changed (`FilesSource::changes`) into the rows: a copy stands
    /// next to its original, selected. Returns whether a copy arrived, and what failed after all.
    pub fn take_changes(&mut self) -> (bool, Vec<(Action, ActionError)>) {
        let (copied, failures, _) = self.drain_changes();
        (copied, failures)
    }

    /// Every frame (`protected::poll`), for a source that answers later (`FilesSource::answers_later`): what
    /// the engine did to the copies on this PC is drawn now, per item, and said. Never before the engine did it.
    /// The message is set without a wake: the source wakes the window again when its time is over.
    pub(super) fn late_changes(&mut self) -> bool {
        if !self.actions.source.answers_later() {
            return false;
        }
        let (copied, failures, local) = self.drain_changes();
        let toast = match (failures.last(), local) {
            (Some(&(action, why)), _) => Some(Toast::Failed(action, why)),
            (None, Some(Action::KeepOffline)) => Some(Toast::Keeping),
            (None, Some(Action::FreeUpSpace)) => Some(Toast::FreedUp),
            _ => None,
        };
        let said = toast.is_some();
        if let Some(toast) = toast {
            self.actions.toast = Some(toast);
            self.actions.toast_until = Some(std::time::Instant::now() + toast.stays());
        }
        copied || said || local.is_some()
    }

    /// [`State::take_changes`], and the last action that changed what this PC holds of some rows.
    fn drain_changes(&mut self) -> (bool, Vec<(Action, ActionError)>, Option<Action>) {
        let mut copies = Vec::new();
        let mut failures = Vec::new();
        let mut local = None;
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
                Change::Local { action, done, failed } => {
                    // By id, never by position: the rows may have changed since it was asked.
                    let status = if action == Action::KeepOffline {
                        "available_offline"
                    } else {
                        "online_only"
                    };
                    for item in self.listing.items.iter_mut().filter(|item| done.contains(&item.id)) {
                        item.pc_status = Some(status.into());
                    }
                    local = Some(action);
                    if failed > 0 {
                        failures.push((action, ActionError::NotAll));
                    }
                }
            }
        }
        if !copies.is_empty() {
            self.select_ids(&copies);
        }
        (!copies.is_empty(), failures, local)
    }
}
