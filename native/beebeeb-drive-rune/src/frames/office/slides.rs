//! Slice OF3 (plan40 batch 1): the presentation frames of the Office engine, DW-1509, DW-1560 and DW-1561.
//! Not drawn yet: each row shows `placeholder::scene` until OF3 replaces it with the frame's own scene.

use crate::State;
use crate::frames::placeholder;

/// The group's board frames and the scene that shows each (`frames::scenes`).
pub const FRAMES: &[crate::ui::gate::Scene<State>] = &[
    ("DW-1509", placeholder::scene),
    ("DW-1560", placeholder::scene),
    ("DW-1561", placeholder::scene),
];
