//! Slice OF2 (plan40 batch 1): the spreadsheet frames of the Office engine, DW-1508 and DW-1559.
//! Not drawn yet: each row shows `placeholder::scene` until OF2 replaces it with the frame's own scene.

use crate::State;
use crate::frames::placeholder;

/// The group's board frames and the scene that shows each (`frames::scenes`).
pub const FRAMES: &[crate::ui::gate::Scene<State>] =
    &[("DW-1508", placeholder::scene), ("DW-1559", placeholder::scene)];
