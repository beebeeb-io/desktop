//! The app's root state: one value the whole main window reads.
//!
//! `ui/**` components take `&State` and `Cx<State>`, so every frame group draws from this
//! struct. A frame lane that needs state of its own adds ONE field here (and its default in
//! [`init`]); the shell's own state lives in [`State::shell`].

use rune::TextEdit;

use crate::source::{self, BoardFixtureSource, DriveSource, Listing};
use crate::ui::sidebar::SidebarSection;

pub struct State {
    // --- Files (DW-0401): read by `ui/**` ---
    pub(crate) source: Box<dyn DriveSource>,
    pub(crate) listing: Listing,
    pub(crate) storage: source::StorageSummary,
    pub(crate) selected: Option<usize>,
    pub(crate) search: TextEdit,
    pub(crate) sidebar_section: SidebarSection,
    pub(crate) show_detail_pane: bool,
    // --- one line per frame group below ---
}

pub fn init() -> State {
    let source = Box::new(BoardFixtureSource::new());
    let listing = source.list(None);
    let storage = source.storage();
    State {
        source,
        listing,
        storage,
        selected: Some(6), // Contract draft.pdf (index 6) selected per the board
        search: TextEdit::new(""),
        sidebar_section: SidebarSection::Files,
        show_detail_pane: true,
    }
}
