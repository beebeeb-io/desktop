//! The source is told which folder the window shows, once per folder, and again after the session changed
//! (`protected::poll`, the top of it): every way a folder comes to be shown ends in that one call. A source of the
//! test's own counts what it is told. Every id and name is the board's.

use super::poll;
use crate::State;
use crate::frames::files::source::{
    Account, ActionError, Change, Counts, Details, FilesSource, NoEngine, StorageSplit,
};
use crate::frames::files::tests::{runtime_of, settle};
use crate::source::DriveItem;
use rune::Runtime;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

type R = Result<(), ActionError>;

/// What the source was told, in order: the folder shown (`None`: the top of Files).
#[derive(Default, Clone)]
struct Told(Arc<Mutex<Vec<Option<String>>>>);

struct Counting(Told);

#[rustfmt::skip]
impl FilesSource for Counting {
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
    fn open(&mut self, _: &[DriveItem]) -> R { Ok(()) }
    fn show_in_file_explorer(&mut self, _: &[DriveItem]) -> R { Ok(()) }
    fn upload(&mut self, _: Option<&str>, _: &[PathBuf]) -> R { Ok(()) }
    fn new_folder(&mut self, _: Option<&str>, _: &str) -> R { Ok(()) }
    fn rename(&mut self, _: &DriveItem, _: &str) -> R { Ok(()) }
    fn move_to(&mut self, _: &[DriveItem], _: Option<&str>) -> R { Ok(()) }
    fn free_up_space(&mut self, _: &[DriveItem]) -> R { Ok(()) }
    fn open_in_file_explorer(&mut self, _: Option<&str>) -> R { Ok(()) }
    fn changes(&mut self) -> Vec<Change> { Vec::new() }
    fn wake_after(&mut self, _: Duration) {}
    fn folder_shown(&mut self, folder: Option<&str>) { (self.0).0.lock().unwrap().push(folder.map(str::to_string)); }
}

fn window() -> (Runtime<State>, Told) {
    let told = Told::default();
    let mut state = crate::init();
    state.actions.source = Box::new(Counting(told.clone()));
    (runtime_of(state), told)
}

fn frame(rt: &mut Runtime<State>) {
    rt.update(|s, _| {
        poll(s);
    });
    settle(rt);
}

fn told(told: &Told) -> Vec<Option<String>> {
    std::mem::take(&mut told.0.lock().unwrap())
}

#[test]
fn folder_shown_is_told_once_per_folder_and_again_after_a_session_change() {
    let (mut rt, log) = window();
    frame(&mut rt);
    let top = rt.state.listing.folder.id.clone();
    assert_eq!(told(&log), [top], "the folder shown at the first frame");
    frame(&mut rt);
    frame(&mut rt);
    assert_eq!(told(&log), [], "frames of the same folder tell nothing");
    // Another folder of the listing is opened: told once.
    let folder = rt
        .state
        .listing
        .items
        .iter()
        .find(|item| item.is_folder)
        .expect("the board lists a folder")
        .id
        .clone();
    rt.update(|s, _| s.open_folder(Some(folder.clone())));
    frame(&mut rt);
    frame(&mut rt);
    assert_eq!(told(&log), [Some(folder.clone())]);
    // The session moved on (a reset counts one): the same folder is told again.
    rt.update(|s, _| s.actions.session_no += 1);
    frame(&mut rt);
    frame(&mut rt);
    assert_eq!(told(&log), [Some(folder)]);
}
