//! The board frames of Trash as states of a fresh `State` (`--scene DW-0747`, the frame gate).

use super::TrashUi;
use super::source::BoardTrash;
use crate::State;
use crate::ui::sidebar::SidebarSection;

pub fn dw_0747(s: &mut State) {
    s.sidebar_section = SidebarSection::Trash;
    s.frames.trash = TrashUi::default();
}

pub fn dw_0117(s: &mut State) {
    s.sidebar_section = SidebarSection::Trash;
    s.frames.trash = TrashUi {
        source: Box::new(BoardTrash::empty()),
        selected: None,
    };
}
