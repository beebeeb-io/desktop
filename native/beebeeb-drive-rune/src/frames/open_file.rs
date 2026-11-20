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
