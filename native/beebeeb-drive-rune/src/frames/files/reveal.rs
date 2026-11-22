//! Show one item in Files, selected: what Insights' largest file, Shared and Search ask for.
//!
//! [`reveal`] finds the item (`FilesSource::item_of`), puts Files on its folder and selects it. A folder whose listing
//! does not hold the item yet (the first sync is still bringing it) keeps the item as the one to select: the read after
//! the next finished check (`reread::take_reread`) selects it and says where to scroll. An item the account does not
//! know says so once, in a toast.

use super::Toast;
use super::reread::LIST_TOP;
use super::source::{Action, ActionError};
use crate::State;
use crate::shell::app::{Event, Route, Screen, Section};
use crate::ui::file_list::ROW_H;
use rune::Tx;

/// Files on the folder of the item with this id, the item selected and in view. `false`: the item is not known (a
/// toast says so) or the session has ended.
pub fn reveal(s: &mut State, tx: &mut Tx, id: &str) -> bool {
    let Some(item) = s.actions.source.item_of(id) else {
        s.show_toast(Toast::Failed(Action::Open, ActionError::Failed), tx);
        return false;
    };
    let files_shown = matches!(s.shell.app.screen(), Screen::Drive(route) if route.section == Section::Files);
    if !files_shown {
        crate::shell::host::dispatch(s, Event::Navigate(Route::section(Section::Files)));
    }
    if s.listing.folder.id != item.parent_id {
        if s.refused_without_session() {
            return false;
        }
        s.open_folder(item.parent_id.clone());
    }
    s.actions.reveal = Some(id.to_string());
    if let Some(y) = s.apply_reveal() {
        tx.scroll_to_y("rows", y);
    }
    true
}

impl State {
    /// Select the item to reveal if the listing holds it now, and clear the wish. Returns the scroll that puts its row
    /// in view (list view only).
    pub(super) fn apply_reveal(&mut self) -> Option<f32> {
        let id = self.actions.reveal.clone()?;
        let row = self.listing.items.iter().position(|item| item.id == id)?;
        self.actions.reveal = None;
        self.select_ids(&[id]);
        if crate::frames::global::grid::on(self) {
            return None;
        }
        let top = LIST_TOP + row as f32 * ROW_H;
        let (shown, scrolled) = (self.actions.window.1.max(ROW_H), self.actions.list_scroll.get());
        // Already in view: leave the list where it is.
        (top < scrolled || top + ROW_H > scrolled + shown * 0.6).then(|| (top - ROW_H).max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::super::actions_data::ShareOptions;
    use super::super::source::{Account, Change, Counts, Details, FilesSource, NoEngine, StorageSplit};
    use super::super::tests::runtime_of;
    use super::*;
    use crate::source::{Crumb, DriveItem, DriveSource, Listing, SourceError, StorageSummary};
    use rune::Runtime;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    type R = Result<(), ActionError>;
    type Rows = Arc<Mutex<Vec<DriveItem>>>;

    fn item(id: &str, parent: Option<&str>, is_folder: bool) -> DriveItem {
        DriveItem {
            id: id.into(),
            parent_id: parent.map(Into::into),
            name: format!("{id}.txt"),
            is_folder,
            size: None,
            child_count: None,
            modified_at: String::new(),
            mime: None,
            has_thumbnail: false,
            shared_type: None,
            shared_badges: Vec::new(),
            pc_status: None,
            is_protected: false,
        }
    }

    /// The folder "f-deep" holds whatever `rows` holds now; the root holds a sibling.
    struct Drive(Rows);

    impl DriveSource for Drive {
        fn list(&self, folder: Option<&str>) -> Result<Listing, SourceError> {
            let (crumb, items) = match folder {
                Some("f-deep") => (
                    Crumb {
                        id: Some("f-deep".into()),
                        name: "Deep".into(),
                    },
                    self.0.lock().unwrap().clone(),
                ),
                None => (
                    Crumb {
                        id: None,
                        name: "Files".into(),
                    },
                    vec![item("near", None, false)],
                ),
                _ => return Err(SourceError::NotFound),
            };
            Ok(Listing {
                folder: crumb.clone(),
                path: vec![crumb],
                items,
            })
        }
        fn storage(&self) -> Option<StorageSummary> {
            None
        }
    }

    struct Source;

    #[rustfmt::skip]
    impl FilesSource for Source {
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
        fn share_link(&mut self, _: &[DriveItem], _: &ShareOptions) -> R { Ok(()) }
        fn open(&mut self, _: &[DriveItem]) -> R { Ok(()) }
        fn show_in_file_explorer(&mut self, _: &[DriveItem]) -> R { Ok(()) }
        fn upload(&mut self, _: Option<&str>, _: &[PathBuf]) -> R { Ok(()) }
        fn new_folder(&mut self, _: Option<&str>, _: &str) -> R { Ok(()) }
        fn rename(&mut self, _: &DriveItem, _: &str) -> R { Ok(()) }
        fn move_to(&mut self, _: &[DriveItem], _: Option<&str>) -> R { Ok(()) }
        fn free_up_space(&mut self, _: &[DriveItem]) -> R { Ok(()) }
        fn open_in_file_explorer(&mut self, _: Option<&str>) -> R { Ok(()) }
        fn changes(&mut self) -> Vec<Change> { Vec::new() }
        fn item_of(&self, id: &str) -> Option<DriveItem> { (id == "target").then(|| item("target", Some("f-deep"), false)) }
    }

    fn window(rows: Rows) -> Runtime<State> {
        let mut state = crate::init();
        state.source = Box::new(Drive(rows));
        state.actions.source = Box::new(Source);
        runtime_of(state)
    }

    fn selected_ids(s: &State) -> Vec<String> {
        s.actions
            .selection
            .iter()
            .map(|&r| s.listing.items[r].id.clone())
            .collect()
    }

    #[test]
    fn an_item_in_another_folder_is_selected_there_once_its_listing_holds_it() {
        let rows: Rows = Arc::default();
        let mut rt = window(rows.clone());
        rt.update(|s, tx| {
            assert!(s.listing.folder.id.is_none());
            assert!(reveal(s, tx, "target"));
            // Landed on the folder; the item is not in the listing yet: nothing selected, the wish kept.
            assert_eq!(s.listing.folder.id.as_deref(), Some("f-deep"));
            assert!(s.actions.selection.is_empty());
            assert_eq!(s.actions.reveal.as_deref(), Some("target"));
        });
        // The listing arrives later: the read after a check.
        *rows.lock().unwrap() = vec![
            item("other", Some("f-deep"), false),
            item("target", Some("f-deep"), false),
        ];
        rt.update(|s, _| {
            let (read, error) = crate::load(s.source.as_ref(), Some("f-deep"));
            s.take_reread(read, error);
            assert_eq!(selected_ids(s), vec!["target".to_string()]);
            assert_eq!(s.selected, Some(1));
            assert!(s.actions.reveal.is_none());
        });
    }

    #[test]
    fn an_item_already_listed_is_selected_at_once() {
        let rows: Rows = Arc::new(Mutex::new(vec![item("target", Some("f-deep"), false)]));
        let mut rt = window(rows);
        rt.update(|s, tx| {
            assert!(reveal(s, tx, "target"));
            assert_eq!(selected_ids(s), vec!["target".to_string()]);
            assert!(s.actions.reveal.is_none());
        });
    }

    #[test]
    fn an_id_that_is_gone_says_so_once_and_changes_nothing() {
        let mut rt = window(Arc::default());
        rt.update(|s, tx| {
            assert!(!reveal(s, tx, "nobody"));
            assert_eq!(s.live_toast(), Some(Toast::Failed(Action::Open, ActionError::Failed)));
            assert!(s.listing.folder.id.is_none());
            assert!(s.actions.reveal.is_none());
        });
    }

    #[test]
    fn leaving_the_folder_drops_the_wish() {
        let mut rt = window(Arc::default());
        rt.update(|s, tx| {
            assert!(reveal(s, tx, "target"));
            s.open_folder(None);
            assert!(s.actions.reveal.is_none());
        });
    }
}
