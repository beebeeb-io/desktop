//! ONE function opens a file from anywhere in the app: [`open_file`]. Files, Search, Shared, This PC (kept), the
//! details pane's Open, Photos, the About page's licence link and the Office dialog's link all call it; no call site
//! decides for itself where a file goes. The order is fixed:
//!
//! 1. the app's own preview, when the kind has one (the viewer, `frames::viewers`);
//! 2. else the app the system has for that kind;
//! 3. else the system's file manager, with the file selected in its folder.
//!
//! Steps 2 and 3 are the file source's ([`FilesSource::open_outside`]): the path is resolved under the account-bound
//! handle, and a kind that is not run for safety goes to step 3. The viewer's own "Open in another app…" button is a
//! person's press and stays outside this function.

use std::path::{Path, PathBuf};

use crate::State;
use crate::frames::files::source::{ActionError, Outside};
use crate::frames::viewers::{self, kinds};
use crate::source::DriveItem;

/// What is opened.
pub enum Target<'a> {
    /// A file of the drive, by its row.
    Item(&'a DriveItem),
    /// A file that is not in the drive: the licence notice beside the exe. Only text kinds have a preview.
    Path(PathBuf),
}

/// Where the file went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opened {
    /// The app's own viewer (step 1).
    InApp,
    /// The app the system has for its kind (step 2).
    DefaultApp,
    /// The file manager, the file selected (step 3).
    Explorer,
    /// Nothing opened, and why.
    Failed(ActionError),
}

/// Open `target`: in the app when the kind has a preview, else outside it by steps 2 and 3.
pub fn open_file(s: &mut State, target: Target) -> Opened {
    match target {
        Target::Item(item) => open_item(s, item),
        Target::Path(path) => open_path(s, path),
    }
}

fn open_item(s: &mut State, item: &DriveItem) -> Opened {
    if item.is_folder {
        return Opened::Failed(ActionError::Failed);
    }
    if has_preview(s, &item.name) && viewers::open_item(s, item) {
        return Opened::InApp;
    }
    outside(s.actions.source.open_outside(item))
}

/// w31: steps 2 and 3 only, for a file the app has no preview of even though its name has a kind (HEIC, RAW): the
/// system's app, else the file manager with it selected. Photos' "Open in Photos / Preview".
pub fn open_outside_app(s: &mut State, item: &DriveItem) -> Opened {
    if item.is_folder {
        return Opened::Failed(ActionError::Failed);
    }
    outside(s.actions.source.open_outside(item))
}

fn open_path(s: &mut State, path: PathBuf) -> Opened {
    if local_has_preview(&path) && viewers::open_local(s, &path) {
        return Opened::InApp;
    }
    outside(s.actions.source.open_path_outside(&path))
}

fn outside(answer: Result<Outside, ActionError>) -> Opened {
    match answer {
        Ok(Outside::DefaultApp) => Opened::DefaultApp,
        Ok(Outside::Explorer) => Opened::Explorer,
        Err(why) => Opened::Failed(why),
    }
}

/// Whether the app shows files of this name itself: the window's viewer source says it does, and the kind is not one
/// of the Office types that have no preview (`.ppt`).
fn has_preview(s: &State, name: &str) -> bool {
    !matches!(kinds::office_route(name), Some(kinds::OfficeRoute::NoPreview(_))) && viewers::previews(s, name)
}

/// The kinds a local file (not in the drive) is shown as: text and Markdown.
pub fn local_has_preview(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "md" | "markdown" | "txt" | "log"))
}

/// A file as the lists that know only its id and name hand it over (Search's results).
pub fn item_named(id: &str, name: &str) -> DriveItem {
    DriveItem {
        id: id.to_string(),
        parent_id: None,
        name: name.to_string(),
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

/// A viewer source and a spy for the tests of every entry point: what the window's viewer shows, and what went
/// outside the app.
#[cfg(test)]
pub(crate) mod testing {
    use std::sync::{Arc, Mutex};

    use crate::State;
    use crate::frames::files::source::{Call, FilesFixture};
    use crate::frames::viewers::source::{Body, Details, OpenError, OpenFile, TextDoc, ViewerSource, plain_lines};

    /// Shows every file as text, or (`shows: false`) says it has no preview for any kind.
    pub struct Viewer {
        pub shows: bool,
    }

    impl ViewerSource for Viewer {
        fn open(&mut self, id: &str) -> Result<OpenFile, OpenError> {
            Ok(OpenFile {
                id: id.to_string(),
                path: vec!["Files".into()],
                name: "x".into(),
                facts: String::new(),
                version: 0,
                body: Body::Text(TextDoc {
                    lines: plain_lines("shown in the app"),
                    truncated: None,
                }),
                details: Details::default(),
                newer: None,
            })
        }
        fn poll(&mut self, _: &str) -> Option<Body> {
            None
        }
        fn cancel(&mut self, _: &str) {}
        fn retry(&mut self, id: &str) -> Result<OpenFile, OpenError> {
            self.open(id)
        }
        fn previews(&self, _: &str) -> bool {
            self.shows
        }
    }

    pub type Calls = Arc<Mutex<Vec<Call>>>;

    /// Wires `s` with a viewer that shows every kind (`shows`) or none, and a spy source; the spy's calls.
    pub fn wire_into(s: &mut State, shows: bool) -> Calls {
        crate::frames::viewers::wire(s, Box::new(move || Box::new(Viewer { shows })));
        let fixture = FilesFixture::default();
        let calls = fixture.calls.clone();
        s.actions.source = Box::new(fixture);
        calls
    }

    /// The app's state over a spy source and a viewer that shows every kind (`shows`) or none.
    pub fn state(shows: bool) -> (State, Calls) {
        let mut s = crate::init();
        let calls = wire_into(&mut s, shows);
        (s, calls)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::state;
    use super::*;
    use crate::frames::files::source::{Call, FilesFixture};
    use std::sync::atomic::Ordering;

    fn item(name: &str) -> DriveItem {
        item_named("f-1", name)
    }

    #[test]
    fn step_1_a_kind_with_a_preview_opens_in_the_app_and_nothing_goes_outside() {
        let (mut s, calls) = state(true);
        assert_eq!(open_file(&mut s, Target::Item(&item("a.pdf"))), Opened::InApp);
        assert!(s.viewer.ui.is_some());
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn step_1_has_no_exception_for_a_kept_file_and_office_without_a_preview_goes_on_to_step_2() {
        // `.ppt` has no preview here even from a viewer that shows everything else.
        let (mut s, calls) = state(true);
        assert_eq!(open_file(&mut s, Target::Item(&item("old.ppt"))), Opened::DefaultApp);
        assert_eq!(*calls.lock().unwrap(), [Call::OpenDefault("f-1".into())]);
    }

    #[test]
    fn step_2_a_kind_with_no_preview_goes_to_the_app_the_system_has_for_it() {
        let (mut s, calls) = state(false);
        assert_eq!(open_file(&mut s, Target::Item(&item("a.blend"))), Opened::DefaultApp);
        assert_eq!(*calls.lock().unwrap(), [Call::OpenDefault("f-1".into())]);
        assert!(s.viewer.ui.is_none(), "the viewer was not opened");
    }

    #[test]
    fn step_3_with_no_app_for_the_kind_the_file_manager_shows_the_file_selected() {
        let (mut s, _) = state(false);
        let fixture = FilesFixture::default();
        fixture.no_default_app.store(true, Ordering::SeqCst);
        let calls = fixture.calls.clone();
        s.actions.source = Box::new(fixture);
        assert_eq!(open_file(&mut s, Target::Item(&item("a.unknownkind"))), Opened::Explorer);
        assert_eq!(*calls.lock().unwrap(), [Call::OpenRevealed("f-1".into())]);
    }

    #[test]
    fn a_folder_is_not_a_file_and_a_source_that_cannot_open_outside_says_so() {
        let (mut s, calls) = state(false);
        let folder = DriveItem {
            is_folder: true,
            ..item("Folder")
        };
        assert_eq!(open_file(&mut s, Target::Item(&folder)), Opened::Failed(ActionError::Failed));
        assert!(calls.lock().unwrap().is_empty());
        let mut s = crate::init();
        crate::frames::viewers::wire(&mut s, Box::new(|| Box::new(super::testing::Viewer { shows: false })));
        s.actions.source = Box::new(crate::frames::files::source::NoEngine);
        assert_eq!(
            open_file(&mut s, Target::Item(&item("a.blend"))),
            Opened::Failed(ActionError::Unavailable)
        );
    }

    #[test]
    fn a_local_text_or_markdown_file_opens_in_the_app_and_any_other_kind_goes_outside() {
        let dir = std::env::temp_dir().join(format!("bb-open-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (md, bin) = (dir.join("notice.md"), dir.join("notice.bin"));
        std::fs::write(&md, "# Title\n\n- item\n\n```\ncode\n```\n").unwrap();
        std::fs::write(&bin, [0u8; 4]).unwrap();
        let (mut s, calls) = state(false);
        assert_eq!(open_file(&mut s, Target::Path(md)), Opened::InApp);
        let body = &s.viewer.ui.as_ref().unwrap().file.body;
        assert!(matches!(body, crate::frames::viewers::source::Body::Markdown(_)), "{body:?}");
        assert!(calls.lock().unwrap().is_empty());
        assert_eq!(open_file(&mut s, Target::Path(bin.clone())), Opened::DefaultApp);
        assert_eq!(*calls.lock().unwrap(), [Call::OpenPathDefault(bin)]);
    }
}
