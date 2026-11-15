//! The Preview panel over a source that answers later (as the engine's does): the panel says it is getting the
//! preview, then shows the page or says why there is none, in the board's frame and keys; closing the panel, stepping
//! to the next file and a reset each tell the source to stop and wipe. Every id and name is the board's.

use super::poll;
use crate::State;
use crate::frames::files::actions_data::{Page, PreviewOf, Previewed};
use crate::frames::files::dialogs::Dialog;
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

/// What the test keeps of the source: what the panel asked, how often it was told to stop, and what the preview
/// stands at now.
#[derive(Clone)]
struct Spy {
    stands: Arc<Mutex<Previewed>>,
    asked: Arc<Mutex<Vec<String>>>,
    cancels: Arc<Mutex<usize>>,
}

struct Later(Spy);

#[rustfmt::skip]
impl FilesSource for Later {
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
    fn preview(&mut self, item: &DriveItem) -> Option<PreviewOf> {
        self.0.asked.lock().unwrap().push(item.id.clone());
        None
    }
    fn preview_later(&mut self, _: &DriveItem) -> Option<Previewed> { Some(self.0.stands.lock().unwrap().clone()) }
    fn cancel_preview(&mut self) { *self.0.cancels.lock().unwrap() += 1; }
}

fn window() -> (Runtime<State>, Spy) {
    let spy = Spy {
        stands: Arc::new(Mutex::new(Previewed::Loading)),
        asked: Arc::default(),
        cancels: Arc::default(),
    };
    let mut state = crate::init();
    state.actions.source = Box::new(Later(spy.clone()));
    // Two plain files first.
    let free: Vec<usize> = (0..state.listing.items.len())
        .filter(|&row| !state.listing.items[row].is_protected && !state.listing.items[row].is_folder)
        .take(2)
        .collect();
    assert_eq!(free.len(), 2, "the board has too few plain files for this test");
    for (to, from) in free.into_iter().enumerate() {
        state.listing.items.swap(to, from);
    }
    (runtime_of(state), spy)
}

fn frame(rt: &mut Runtime<State>) -> bool {
    let mut changed = false;
    rt.update(|s, _| changed = poll(s));
    settle(rt);
    changed
}

fn drawn(rt: &Runtime<State>) -> String {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .flat_map(|e| [e.text, e.label].into_iter().flatten())
        .collect::<Vec<_>>()
        .join("\n")
}

fn open(rt: &mut Runtime<State>, at: usize) -> DriveItem {
    let item = rt.state.listing.items[at].clone();
    rt.update(|s, _| s.open_preview(item.clone()));
    settle(rt);
    item
}

fn page(title: &str) -> PreviewOf {
    PreviewOf {
        place: "The first lines".into(),
        page: Page {
            title: title.into(),
            byline: "Text".into(),
            sections: vec![(String::new(), "an invented first line".into())],
        },
    }
}

#[test]
fn the_panel_says_loading_then_the_page_and_says_a_failure() {
    let (mut rt, spy) = window();
    open(&mut rt, 0);
    assert!(drawn(&rt).contains("Getting the preview\u{2026}"), "{}", drawn(&rt));
    assert!(!frame(&mut rt), "a frame with no answer changed something");
    // The page lands: the next frame draws it, and nothing says loading any more.
    *spy.stands.lock().unwrap() = Previewed::Shown(page("An invented note"));
    assert!(frame(&mut rt));
    let now = drawn(&rt);
    assert!(
        now.contains("an invented first line") && now.contains("The first lines"),
        "{now}"
    );
    assert!(!now.contains("Getting the preview"), "{now}");
    // The words of every state that is not a page.
    for (stands, words) in [
        (
            Previewed::NotYet,
            "Preview isn't available for this kind of file in this version yet",
        ),
        (Previewed::TooLarge, "Too large to preview here"),
        (Previewed::Failed(ActionError::Offline), "Preview needs a connection"),
        (Previewed::Failed(ActionError::Failed), "Couldn't show a preview"),
    ] {
        *spy.stands.lock().unwrap() = Previewed::Loading;
        open(&mut rt, 0);
        *spy.stands.lock().unwrap() = stands;
        assert!(frame(&mut rt));
        let now = drawn(&rt);
        assert!(now.contains(words), "{words:?} not drawn:\n{now}");
        assert!(
            now.contains("Open in another app\u{2026}"),
            "the two ways on stay:\n{now}"
        );
        assert!(!now.contains("Getting the preview"), "{now}");
    }
}

#[test]
fn closing_the_panel_stepping_and_a_reset_tell_the_source_to_stop() {
    let (mut rt, spy) = window();
    let first = open(&mut rt, 0);
    assert_eq!(
        *spy.cancels.lock().unwrap(),
        1,
        "opening stops what an earlier panel read"
    );
    rt.update(|s, tx| s.close_dialog(tx));
    assert_eq!(*spy.cancels.lock().unwrap(), 2, "closing stops the read");
    assert!(rt.state.actions.dialog.is_none());
    // Stepping to the next file: the first read stops and the second file is asked for, in turn.
    open(&mut rt, 0);
    let before = *spy.cancels.lock().unwrap();
    rt.update(|s, _| assert!(s.preview_step(1)));
    assert_eq!(*spy.cancels.lock().unwrap(), before + 1);
    let asked = spy.asked.lock().unwrap().clone();
    assert_ne!(asked.last().unwrap(), &first.id, "the second file was asked for");
    assert!(matches!(&rt.state.actions.dialog, Some(Dialog::Preview(p)) if p.item.id != first.id));
    // A reset (the session ended) closes it and stops the read.
    let before = *spy.cancels.lock().unwrap();
    rt.update(|s, _| crate::frames::files::reset::close_dialogs(s));
    assert_eq!(*spy.cancels.lock().unwrap(), before + 1);
}
