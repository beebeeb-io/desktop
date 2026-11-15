//! Slice OF6 (Office viewer): the viewer's states, DW-1563 (opening) and DW-1564 (some content isn't shown).
//! Not drawn yet: each row shows `placeholder::scene` until OF6 replaces it with the frame's own scene.

use crate::State;
use crate::frames::placeholder;

/// The group's board frames and the scene that shows each (`frames::scenes`).
pub const FRAMES: &[crate::ui::gate::Scene<State>] =
    &[("DW-1563", placeholder::scene), ("DW-1564", placeholder::scene)];
