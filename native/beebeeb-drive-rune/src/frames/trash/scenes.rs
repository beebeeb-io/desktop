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
        menu: None,
    };
}

/// The menu on Contract old.pdf, Restore lit.
pub fn dw_0748(s: &mut State) {
    dw_0747(s);
    s.frames.trash.selected = Some(1);
    s.frames.trash.menu = Some(super::Open {
        at: (420.0, 166.0),
        hot: 0,
    });
}

/// Emptying: 5 of 12 deleted for good.
pub fn dw_0749b(s: &mut State) {
    s.sidebar_section = SidebarSection::Trash;
    s.frames.trash = TrashUi {
        source: Box::new(BoardTrash::emptying()),
        selected: None,
        menu: None,
    };
}
