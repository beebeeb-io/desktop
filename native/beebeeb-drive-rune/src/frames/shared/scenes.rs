//! The board frames of Shared as states of a fresh `State` (`--scene DW-05xx`, the frame gate).

use super::parts::menu::Open;
use super::source::BoardShared;
use super::{SharedUi, Tab};
use crate::State;
use crate::shell::app::Section;

fn scene(s: &mut State, tab: Tab, source: BoardShared) {
    crate::frames::go(s, Section::Shared);
    s.frames.shared = SharedUi {
        source: Box::new(source),
        tab,
        selected: None,
        menu: None,
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

fn menu(s: &mut State, selected: usize, at: (f32, f32), hot: usize) {
    s.frames.shared.selected = Some(selected);
    s.frames.shared.menu = Some(Open { at, hot });
}

/// The menu on Brand review.pdf, Leave… lit.
pub fn dw_0505(s: &mut State) {
    dw_0501(s);
    menu(s, 1, (430.0, 262.0), 5);
}
/// The menu on the link to Budget 2026.xlsx, Copy link lit.
pub fn dw_0506(s: &mut State) {
    dw_0502(s);
    menu(s, 3, (560.0, 332.0), 0);
}
/// The menu on the request Shoot selects, View files lit.
pub fn dw_0507(s: &mut State) {
    dw_0503(s);
    menu(s, 1, (520.0, 222.0), 1);
}
