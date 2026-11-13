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

#[cfg(test)]
mod tests {
    use super::super::source::{Account, Counts, Details, FilesSource, NoEngine, StorageSplit};
    use super::super::tests::runtime_of;
    use super::*;
    use crate::source::DriveItem;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    type R = Result<(), ActionError>;

    /// A source that answers later, as the engine's does: every action is accepted, and what it did is what
    /// the test puts in `later`. An upload is refused when `left` says what was not copied.
    #[derive(Default)]
    struct Later {
        later: Arc<Mutex<Vec<Change>>>,
        left: Option<Vec<PathBuf>>,
        /// How many items Free up space was asked for.
        asked: Arc<Mutex<usize>>,
    }

    #[rustfmt::skip]
    impl FilesSource for Later {
        fn counts(&self) -> Option<Counts> { None }
        fn account(&self) -> Option<Account> { None }
        fn storage_split(&self) -> Option<StorageSplit> { None }
        fn sync_state(&self) -> Option<String> { None }
        fn details(&self, item: &DriveItem, folder: &str) -> Details { NoEngine.details(item, folder) }
        fn changed_label(&self, _: &DriveItem) -> Option<String> { None }
        fn can_add(&self, _: Option<&str>) -> bool { true }
        fn sort_of(&self, _: Option<&str>) -> Option<super::super::Sort> { None }
        fn set_sort(&mut self, _: Option<&str>, _: super::super::Sort) {}
        fn trash(&mut self, _: &[DriveItem]) -> R { Ok(()) }
        fn restore(&mut self, _: &[DriveItem]) -> R { Ok(()) }
        fn duplicate(&mut self, _: &[DriveItem], _: &[&str]) -> R { Ok(()) }
        fn keep_offline(&mut self, _: &[DriveItem], _: bool) -> R { Ok(()) }
        fn copy_link(&mut self, _: &[DriveItem]) -> R { Ok(()) }
        fn open(&mut self, _: &[DriveItem]) -> R { Ok(()) }
        fn show_in_file_explorer(&mut self, _: &[DriveItem]) -> R { Ok(()) }
        fn upload(&mut self, _: Option<&str>, _: &[PathBuf]) -> R {
            if self.left.is_some() { Err(ActionError::Failed) } else { Ok(()) }
        }
        fn upload_left(&mut self) -> Option<Vec<PathBuf>> { self.left.take() }
        fn new_folder(&mut self, _: Option<&str>, _: &str) -> R { Ok(()) }
        fn rename(&mut self, _: &DriveItem, _: &str) -> R { Ok(()) }
        fn move_to(&mut self, _: &[DriveItem], _: Option<&str>) -> R { Ok(()) }
        fn free_up_space(&mut self, items: &[DriveItem]) -> R { *self.asked.lock().unwrap() += items.len(); Ok(()) }
        fn open_in_file_explorer(&mut self, _: Option<&str>) -> R { Ok(()) }
        fn changes(&mut self) -> Vec<Change> { std::mem::take(&mut self.later.lock().unwrap()) }
        fn answers_later(&self) -> bool { true }
    }

    #[test]
    fn nothing_of_a_change_is_drawn_before_the_source_said_what_it_did_and_then_per_item() {
        let later = Arc::new(Mutex::new(Vec::new()));
        let asked = Arc::new(Mutex::new(0));
        let mut state = crate::init();
        state.actions.source = Box::new(Later {
            later: later.clone(),
            left: None,
            asked: asked.clone(),
        });
        let mut rt = runtime_of(state);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let out = seen.clone();
        rt.update(move |s, tx| {
            // Two rows that can change (not Backups).
            let rows: Vec<usize> = (0..s.listing.items.len())
                .filter(|&row| !s.listing.items[row].is_protected && !s.listing.items[row].is_folder)
                .take(2)
                .collect();
            assert_eq!(rows.len(), 2);
            s.listing.items.swap(0, rows[0]);
            s.listing.items.swap(1, rows[1]);
            s.selected = Some(0);
            for row in 0..2 {
                s.listing.items[row].pc_status = Some("on_this_pc".into());
                s.actions.selection.insert(row);
            }
            assert!(s.free_up_space(tx));
            *out.lock().unwrap() = vec![s.listing.items[0].id.clone(), s.listing.items[1].id.clone()];
        });
        let ids = seen.lock().unwrap().clone();
        assert_eq!(*asked.lock().unwrap(), 2, "the source was not asked for the two rows");
        rt.update(|s, _| {
            assert!(
                !super::super::protected::poll(s),
                "a frame with no answer changed something"
            );
            assert_eq!(
                s.listing.items[0].pc_status.as_deref(),
                Some("on_this_pc"),
                "drawn before it was done"
            );
            assert_eq!(s.actions.toast, None, "said before it was done");
        });
        // The engine freed the first and left the second.
        later.lock().unwrap().push(Change::Local {
            action: Action::FreeUpSpace,
            done: vec![ids[0].clone()],
            failed: 1,
        });
        rt.update(|s, _| {
            assert!(super::super::protected::poll(s));
            assert_eq!(s.listing.items[0].pc_status.as_deref(), Some("online_only"));
            assert_eq!(
                s.listing.items[1].pc_status.as_deref(),
                Some("on_this_pc"),
                "the item left alone was drawn as freed"
            );
            assert_eq!(
                s.actions.toast,
                Some(Toast::Failed(Action::FreeUpSpace, ActionError::NotAll))
            );
        });
        // All of it done: the plain message.
        later.lock().unwrap().push(Change::Local {
            action: Action::FreeUpSpace,
            done: vec![ids[1].clone()],
            failed: 0,
        });
        rt.update(|s, _| {
            assert!(super::super::protected::poll(s));
            assert_eq!(s.listing.items[1].pc_status.as_deref(), Some("online_only"));
            assert_eq!(s.actions.toast, Some(Toast::FreedUp));
        });
    }

    #[test]
    fn try_again_after_a_drop_that_went_in_partly_holds_only_what_was_not_copied() {
        let files: Vec<PathBuf> = ["one.pdf", "two.pdf", "three.pdf"].iter().map(PathBuf::from).collect();
        let mut state = crate::init();
        state.actions.source = Box::new(Later {
            later: Arc::default(),
            left: Some(files[1..].to_vec()),
            asked: Arc::default(),
        });
        let mut rt = runtime_of(state);
        rt.update(|s, tx| {
            assert!(!s.upload(None, &files, tx));
            assert_eq!(
                s.actions.retry_upload,
                Some((None, files[1..].to_vec())),
                "Try again would copy a file twice"
            );
            // Try again sends those alone, and it goes in.
            s.retry(tx);
            assert_eq!(s.actions.retry_upload, None);
        });
    }
}
