//! Files' Share and Version history dialogs for an item that is not Files' selection (what Search, Photos and the
//! viewer ask): the dialog opens, Copy link asks `share_link` for that item only, and versions are asked for it.

use super::super::dialogs::Dialog;
use super::super::source::{Account, Action, ActionError, Change, Counts, Details, FilesSource, NoEngine, StorageSplit};
use super::super::tests::runtime_of;
use crate::State;
use crate::frames::files::actions_data::ShareOptions;
use crate::source::DriveItem;
use rune::Runtime;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

type R = Result<(), ActionError>;

#[derive(Default, Clone)]
struct Spy {
    shared: Arc<Mutex<Vec<Vec<String>>>>,
    versions: Arc<Mutex<Vec<String>>>,
}

struct Source(Spy);

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
    fn share_link(&mut self, items: &[DriveItem], _: &ShareOptions) -> R {
        self.0.shared.lock().unwrap().push(items.iter().map(|i| i.id.clone()).collect());
        Ok(())
    }
    fn open(&mut self, _: &[DriveItem]) -> R { Ok(()) }
    fn show_in_file_explorer(&mut self, _: &[DriveItem]) -> R { Ok(()) }
    fn upload(&mut self, _: Option<&str>, _: &[PathBuf]) -> R { Ok(()) }
    fn new_folder(&mut self, _: Option<&str>, _: &str) -> R { Ok(()) }
    fn rename(&mut self, _: &DriveItem, _: &str) -> R { Ok(()) }
    fn move_to(&mut self, _: &[DriveItem], _: Option<&str>) -> R { Ok(()) }
    fn free_up_space(&mut self, _: &[DriveItem]) -> R { Ok(()) }
    fn open_in_file_explorer(&mut self, _: Option<&str>) -> R { Ok(()) }
    fn changes(&mut self) -> Vec<Change> { Vec::new() }
    fn ask_versions(&mut self, item: &DriveItem) { self.0.versions.lock().unwrap().push(item.id.clone()); }
    fn item_of(&self, id: &str) -> Option<DriveItem> { (id == "elsewhere").then(|| outside("elsewhere", false)) }
}

fn outside(id: &str, is_folder: bool) -> DriveItem {
    DriveItem {
        id: id.to_string(),
        parent_id: Some("a-folder-not-listed".into()),
        name: "Invented elsewhere.txt".into(),
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

fn window() -> (Runtime<State>, Spy) {
    let spy = Spy::default();
    let mut state = crate::init();
    state.actions.source = Box::new(Source(spy.clone()));
    (runtime_of(state), spy)
}

#[test]
fn share_for_opens_for_an_item_outside_the_listing_and_copy_link_asks_for_that_item_only() {
    let (mut rt, spy) = window();
    let item = outside("elsewhere", false);
    assert!(rt.state.listing.items.iter().all(|i| i.id != item.id));
    rt.update(|s, tx| {
        assert!(s.begin_share_for(vec![item.clone()]));
        assert!(matches!(&s.actions.dialog, Some(Dialog::Share(share)) if share.items.len() == 1 && share.items[0].id == "elsewhere"));
        s.share_copy_link(tx);
    });
    assert_eq!(*spy.shared.lock().unwrap(), vec![vec!["elsewhere".to_string()]]);
}

#[test]
fn share_for_refuses_nothing_and_protected_items() {
    let (mut rt, _) = window();
    rt.update(|s, _| {
        assert!(!s.begin_share_for(Vec::new()));
        let mut protected = outside("p", true);
        protected.is_protected = true;
        assert!(!s.begin_share_for(vec![protected]));
        assert!(s.actions.dialog.is_none());
    });
}

#[test]
fn share_for_starts_with_no_password_and_an_empty_field() {
    let (mut rt, _) = window();
    rt.update(|s, _| {
        s.actions.share_password = rune::TextEdit::new("left over").secure();
        assert!(s.begin_share_for(vec![outside("elsewhere", false)]));
        assert!(s.actions.share_password.text.is_empty());
        assert!(!super::shows_secret(s));
    });
}

#[test]
fn versions_for_opens_for_an_item_outside_the_listing_and_asks_its_versions() {
    let (mut rt, spy) = window();
    rt.update(|s, _| {
        assert!(s.begin_versions_for(outside("elsewhere", false)));
        assert!(matches!(&s.actions.dialog, Some(Dialog::Versions(item)) if item.id == "elsewhere"));
    });
    assert_eq!(*spy.versions.lock().unwrap(), vec!["elsewhere".to_string()]);
}

#[test]
fn versions_for_refuses_a_folder() {
    let (mut rt, spy) = window();
    rt.update(|s, _| {
        assert!(!s.begin_versions_for(outside("a-folder", true)));
        assert!(s.actions.dialog.is_none());
    });
    assert!(spy.versions.lock().unwrap().is_empty());
}

#[test]
fn item_of_defaults_to_none() {
    assert!(NoEngine.item_of("anything").is_none());
    let _ = Action::Open;
}
