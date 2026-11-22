//! The scene a board frame shows before its lane draws it: the Files window, as a window with nothing
//! selected and no pane open shows it. It claims nothing about the frame it stands in for.
//!
//! The progress counter keys on this function's name: a `FRAMES` row that names `placeholder::scene` is a
//! frame that is NOT built. A lane that draws its frame replaces the row's scene with its own.

#[cfg(test)]
use crate::State;
#[cfg(test)]
use crate::shell::app::Section;

/// The Files window, nothing selected, no pane open: a neutral state, never a frame's content.
#[cfg(test)]
pub fn scene(s: &mut State) {
    super::go(s, Section::Files);
    s.clear_selection();
    s.show_detail_pane = false;
}
