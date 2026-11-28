//! The status bar's way out of a paused app: next to the word "Paused" a "Resume" item, which asks the
//! engine to resume (the same call as the tray's and the menu's) and reads the state again.

use crate::State;
use crate::theme::tokens::{Theme, type_};
use rune::El;

use super::kit;

pub const RESUME: &str = "Resume";
pub const RESUME_KEY: &str = "status-resume";

/// The Resume item for a paused bar.
pub fn resume_item(t: &Theme) -> El {
    kit::label(RESUME_KEY, RESUME, type_::STATUS_STRONG, t.text)
        .role("button")
        .label("Resume sync")
        .focusable(true)
        .on_press(press_resume)
}

/// Resumes sync. A refusal changes nothing; the next poll shows whatever state the engine is in.
pub fn press_resume(s: &mut State) {
    let _ = s.shell.actions.resume_sync();
    s.shell.refresh();
}
