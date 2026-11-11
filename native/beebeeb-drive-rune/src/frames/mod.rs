//! The board's frames, one module per group (one lane each). A group exposes `view`, `on_key`
//! and a `FRAMES` table of its board frames (frame id, and the scene that shows it).
//!
//! A new group adds TWO lines here: its `pub mod`, and its `FRAMES` in [`scenes`]. The shell
//! names its own scenes once (`shell::SCENES`) and never changes for a new group.

pub mod files;
pub mod settings;
pub mod signin;

use std::sync::OnceLock;

use crate::State;
use crate::ui::gate::Scene;

/// Every scene the app can be rendered in (`--scene`, `scripts/compare.sh`): the shell's own,
/// then every group's board frames. Built once.
pub fn scenes(shell: &'static [Scene<State>]) -> &'static [Scene<State>] {
    static ALL: OnceLock<Vec<Scene<State>>> = OnceLock::new();
    ALL.get_or_init(|| [shell, files::SCENES, settings::FRAMES].concat())
}
