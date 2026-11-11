//! The board frames of Shared as states of a fresh `State` (`--scene DW-05xx`, the frame gate).

use super::source::BoardShared;
use super::{SharedUi, Tab};
use crate::State;
use crate::ui::sidebar::SidebarSection;

fn scene(s: &mut State, tab: Tab, source: BoardShared) {
    s.sidebar_section = SidebarSection::Shared;
    s.frames.shared = SharedUi {
        source: Box::new(source),
        tab,
        selected: None,
    };
}

pub fn dw_0501(s: &mut State) {
    scene(s, Tab::WithMe, BoardShared::new());
}
pub fn dw_0502(s: &mut State) {
    scene(s, Tab::ByMe, BoardShared::new());
}
pub fn dw_0503(s: &mut State) {
    scene(s, Tab::Requests, BoardShared::new());
}
pub fn dw_0508(s: &mut State) {
    scene(s, Tab::WithMe, BoardShared::empty());
}
pub fn dw_0509(s: &mut State) {
    scene(s, Tab::ByMe, BoardShared::empty());
}
pub fn dw_0510(s: &mut State) {
    scene(s, Tab::Requests, BoardShared::empty());
}
