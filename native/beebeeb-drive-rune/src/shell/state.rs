//! The app's root state: one value the whole main window reads.
//!
//! `ui/**` components take `&State` and `Cx<State>`, so every frame group draws from this
//! struct. A frame lane that needs state of its own adds ONE field here (and its default in
//! [`init`]); the shell's own state lives in [`State::shell`].

use rune::TextEdit;

use super::host::Shell;
use crate::source::{self, BoardFixtureSource, DriveSource, Listing};
use crate::ui::sidebar::SidebarSection;

pub struct State {
    // --- Files (DW-0401): read by `ui/**` ---
    pub(crate) source: Box<dyn DriveSource>,
    pub(crate) listing: Listing,
    /// Why `listing` could not be read, when it could not: the window shows this instead of rows.
    pub(crate) list_error: Option<source::SourceError>,
    pub(crate) storage: source::StorageSummary,
    /// `false`: the source cannot know the account's storage (a `--db` source never asks the server).
    pub(crate) storage_known: bool,
    pub(crate) selected: Option<usize>,
    pub(crate) search: TextEdit,
    pub(crate) sidebar_section: SidebarSection,
    pub(crate) show_detail_pane: bool,
    /// What the Files window keeps beside the listing (`ui::FilesUi`).
    pub(crate) files: crate::ui::FilesUi,
    /// The shell: the route machine, the gate, the b, the actions (`shell/host.rs`).
    pub shell: Shell,
    // --- one line per frame group below ---
    /// What the Files frames keep: selection, menus, toasts (`frames::files`).
    pub(crate) actions: crate::frames::files::Actions,
    /// The global elements: the banner, the loading strip (`frames::global`).
    pub(crate) global: crate::frames::global::Ui,
    /// The live feedback mode (test builds only).
    #[cfg(feature = "feedback")]
    pub(crate) feedback: crate::feedback::Feedback,
    pub frames: crate::frames::Ui,
    pub settings: crate::frames::settings::SettingsUi,
    pub(crate) signin: Option<crate::frames::signin::Ui>,
}

/// The state headless renders and tests start from: the fixture engine, nothing wired.
pub fn init() -> State {
    init_with(Shell::fixture())
}

pub fn init_with(shell: Shell) -> State {
    crate::frames::files::board_fixture(init_from(shell, Box::new(BoardFixtureSource::new()), Some(6)))
}

/// The folder as the window holds it: the listing, or an empty one and the reason it could not be read.
pub fn load(source: &dyn DriveSource, folder_id: Option<&str>) -> (Listing, Option<source::SourceError>) {
    match source.list(folder_id) {
        Ok(listing) => (listing, None),
        Err(error) => {
            let root = source::Crumb {
                id: None,
                name: "Files".into(),
            };
            let unread = Listing {
                folder: root.clone(),
                path: vec![root],
                items: Vec::new(),
            };
            (unread, Some(error))
        }
    }
}

/// `selected`: the board fixture has Contract draft.pdf (index 6) selected per the board; a real
/// database starts with nothing selected.
pub fn init_from(shell: Shell, source: Box<dyn DriveSource>, selected: Option<usize>) -> State {
    let (listing, list_error) = load(source.as_ref(), None);
    let storage = source.storage();
    State {
        source,
        listing,
        list_error,
        storage_known: storage.is_some(),
        storage: storage.unwrap_or(source::StorageSummary {
            used_bytes: 0,
            quota_bytes: 0,
        }),
        selected,
        search: TextEdit::new(""),
        sidebar_section: SidebarSection::Files,
        show_detail_pane: true,
        files: crate::ui::FilesUi::default(),
        shell,
        actions: crate::frames::files::Actions::default(),
        global: crate::frames::global::Ui::default(),
        #[cfg(feature = "feedback")]
        feedback: crate::feedback::Feedback::new(),
        frames: crate::frames::Ui::default(),
        settings: Default::default(),
        signin: crate::frames::signin::start(),
    }
}
