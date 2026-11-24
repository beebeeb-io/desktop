//! The largest file in Storage insights opens Files on its folder with that file selected (audit S4).

use crate::State;
use crate::frames::files::Toast;
use crate::frames::files::actions_data::ShareOptions;
use crate::frames::files::source::ActionError;
use crate::frames::files::source::{Account, Change, Counts, Details, FilesSource, NoEngine, StorageSplit};
use crate::source::{Crumb, DriveItem, DriveSource, Listing, SourceError, StorageSummary};
use rune::{Env, Runtime};
use std::path::PathBuf;

type R = Result<(), ActionError>;

fn item(id: &str, parent: Option<&str>) -> DriveItem {
    DriveItem {
        id: id.into(),
        parent_id: parent.map(Into::into),
        name: format!("{id}.zip"),
        is_folder: false,
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

struct Drive;
impl DriveSource for Drive {
    fn list(&self, folder: Option<&str>) -> Result<Listing, SourceError> {
        let (crumb, items) = match folder {
            Some("f-backups") => (
                Crumb {
                    id: Some("f-backups".into()),
                    name: "Backups".into(),
                },
                vec![item("other", Some("f-backups")), item("f-big", Some("f-backups"))],
            ),
            None => (
                Crumb {
                    id: None,
                    name: "Files".into(),
                },
                vec![item("near", None)],
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
    fn sort_of(&self, _: Option<&str>) -> Option<crate::frames::files::Sort> { None }
    fn set_sort(&mut self, _: Option<&str>, _: crate::frames::files::Sort) {}
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
    fn item_of(&self, id: &str) -> Option<DriveItem> { (id == "f-big").then(|| item("f-big", Some("f-backups"))) }
}

#[test]
fn showing_a_largest_file_lands_on_its_folder_with_it_selected() {
    let mut state = crate::init();
    state.source = Box::new(Drive);
    state.actions.source = Box::new(Source);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, crate::view, (1278.0, 798.0), env);
    rt.frame(0.0);
    rt.update(|s: &mut State, tx| {
        s.settings.ask(super::Intent::ShowInFiles("f-big".into()));
        assert!(crate::shell::intents::drain_settings_intents(s, tx));
        assert_eq!(s.listing.folder.id.as_deref(), Some("f-backups"));
        let picked: Vec<&str> = s
            .actions
            .selection
            .iter()
            .map(|&r| s.listing.items[r].id.as_str())
            .collect();
        assert_eq!(picked, ["f-big"]);
        assert_eq!(s.live_toast(), None::<Toast>);
    });
}
