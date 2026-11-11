//! Frame groups: one directory per group of board frames, drawn from a `<Group>Source`.
//!
//! Lane f-shared-search-trash: `shared` (DW-05xx), `trash` (DW-0747…, DW-0117). The checklist of
//! frames, built and not built, is `shared/FRAMES.md`.

pub mod shared;
pub mod trash;

use crate::State;
use crate::shell::app::{Event, Route, Section};
use crate::shell::registry::{Frame, FrameGroup};
use crate::ui::sidebar::SidebarSection;
use rune::{KeyEvent, Tx};

/// Somewhere a control wants to go that this lane does not draw: the shell's route machine
/// (or the lane that owns the target frame) picks it up. A control never does nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Open a board frame that is another view, dialog or menu (`DW-0749`, `DW-0504`, …).
    Frame(&'static str),
    /// Open the item with this id as Files would.
    OpenItem(String),
    /// Something the engine or the shell does with the item of this id, named by the board's
    /// words ("copy-link", "preview", "leave"): a clipboard write, a confirmation, a further menu.
    Do(&'static str, String),
}

/// What the lane's groups keep on `State` (ONE field: `State::frames`).
#[derive(Default)]
pub struct Ui {
    pub shared: shared::SharedUi,
    pub trash: trash::TrashUi,
    /// The navigation intents emitted so far, oldest first.
    pub intents: Vec<Intent>,
}

/// The registry's lines (`shell::registry::FRAMES`): Shared and Trash.
pub const SHARED: Frame = Frame {
    group: FrameGroup::Section(Section::Shared),
    view: shared::view,
    on_key: Some(shared::on_key),
};
pub const TRASH: Frame = Frame {
    group: FrameGroup::Section(Section::Trash),
    view: trash::view,
    on_key: Some(trash::on_key),
};

/// Put the window on a section, as a click in the sidebar does (the scenes and the tests).
pub fn go(s: &mut State, section: Section) {
    crate::shell::host::dispatch(s, Event::Navigate(Route::section(section)));
}

/// The keys of the section the window shows. `None`: not one of this lane's sections.
pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> Option<bool> {
    match s.sidebar_section {
        SidebarSection::Shared => Some(shared::on_key(s, ev, tx)),
        SidebarSection::Trash => Some(trash::on_key(s, ev, tx)),
        _ => None,
    }
}

/// The root key handler of this lane's windows (`El::on_key`).
pub fn on_key_root(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    on_key(s, ev, tx).unwrap_or(false)
}
