//! The "local file list was rebuilt" notice (144c): the engine says it once (`take_rebuilt_notice`), the Files
//! window shows it as a calm banner with Dismiss, and once dismissed it does not come back, also not in a new window
//! over the same engine (a restart: the engine has said it already).

use super::protected::REBUILT_WORDS;
use crate::State;
use crate::frames::files::source::{
    Account, ActionError, Change, Counts, Details, FilesSource, NoEngine, StorageSplit,
};
use crate::frames::files::tests::{click, find, runtime_of, settle};
use crate::source::DriveItem;
use rune::Runtime;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

type R = Result<(), ActionError>;

/// The engine's flag: set once, taken once.
type Flag = Arc<Mutex<bool>>;

struct Says(Flag);

#[rustfmt::skip]
impl FilesSource for Says {
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
    fn take_rebuilt_notice(&mut self) -> Option<String> { std::mem::take(&mut *(self.0).lock().unwrap()).then(|| "x".to_string()) }
}

fn window(flag: &Flag) -> Runtime<State> {
    let mut state = crate::init();
    state.actions.source = Box::new(Says(flag.clone()));
    let mut rt = runtime_of(state);
    settle(&mut rt);
    rt
}

#[test]
fn rebuilt_notice_shows_once_and_dismissed_stays_gone_across_restart() {
    let flag: Flag = Arc::new(Mutex::new(true));
    let mut rt = window(&flag);
    assert!(find(&rt, "banner").is_some(), "the notice is shown");
    assert_eq!(
        REBUILT_WORDS,
        "Beebeeb rebuilt its list of your files on this PC. Nothing was lost; your files are on the server."
    );
    click(&mut rt, "banner-dismiss");
    settle(&mut rt);
    assert!(find(&rt, "banner").is_none(), "dismissed: gone");
    settle(&mut rt);
    assert!(find(&rt, "banner").is_none(), "it does not come back");
    let rt = window(&flag);
    assert!(find(&rt, "banner").is_none(), "a restart: the engine said it already");
}
