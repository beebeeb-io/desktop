//! A Windows older than the helper's sandbox needs (`SandboxError::NeedsNewerWindows`) cannot show Office or media
//! files in the app. The two paths (the Office host, the media player) say it in the same words and log it ONCE per
//! app start: the reason only, never a file name or a path.

use std::sync::atomic::{AtomicBool, Ordering};

/// The one sentence both viewers draw.
pub const NEEDS_WINDOWS_UPDATE: &str = "This file can't be shown until Windows is updated.";

static LOGGED: AtomicBool = AtomicBool::new(false);

/// Logs the reason, the first time this app start. Office and media share the flag.
pub fn note_needs_windows_update() {
    note_once(&LOGGED, || {
        tracing::warn!(reason = "needs windows update", "viewer helper not started")
    });
}

/// Runs `emit` the first time `flag` is taken. `true`: it ran.
fn note_once(flag: &AtomicBool, emit: impl FnOnce()) -> bool {
    if flag.swap(true, Ordering::AcqRel) {
        return false;
    }
    emit();
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn needs_windows_update_is_logged_once_for_two_failures() {
        let flag = AtomicBool::new(false);
        let lines = Cell::new(0);
        // An Office failure, then a media failure, in one start.
        note_once(&flag, || lines.set(lines.get() + 1));
        note_once(&flag, || lines.set(lines.get() + 1));
        assert_eq!(lines.get(), 1, "one line for two failures");
    }

    #[test]
    fn the_sentence_is_the_briefs() {
        assert_eq!(
            NEEDS_WINDOWS_UPDATE,
            "This file can't be shown until Windows is updated."
        );
    }
}
