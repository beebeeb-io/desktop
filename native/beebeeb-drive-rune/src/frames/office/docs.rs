//! Slice OF5 (Office viewer): the read-only documents viewer, DW-1507.
//! Not drawn yet: the row shows `placeholder::scene` until OF5 replaces it with the frame's own scene.

use crate::State;
use crate::frames::placeholder;

/// The group's board frames and the scene that shows each (`frames::scenes`).
pub const FRAMES: &[crate::ui::gate::Scene<State>] = &[("DW-1507", placeholder::scene)];
