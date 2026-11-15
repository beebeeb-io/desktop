//! Slice OF4 (plan40 batch 1): the Office engine's dialogs, DW-1557 and DW-1558.
//! Not drawn yet: each row shows `placeholder::scene` until OF4 replaces it with the frame's own scene.

use crate::State;
use crate::frames::placeholder;

/// The group's board frames and the scene that shows each (`frames::scenes`).
pub const FRAMES: &[crate::ui::gate::Scene<State>] =
    &[("DW-1557", placeholder::scene), ("DW-1558", placeholder::scene)];
