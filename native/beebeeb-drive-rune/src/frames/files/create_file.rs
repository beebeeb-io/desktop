//! Create file (DW-1522): what the New file dialog asked for ([`viewers::take_asked`]), made by the source in the
//! folder the dialog was opened on. The dialog checked the name against the rows it showed; the engine checks it
//! again against the folder itself (a cloud-only name, an 8.3 short name) and refuses rather than overwrite. What
//! it refuses is said as every other failed action is (DW-0109), with Try again opening the dialog again.

use rune::Tx;

use super::source::Action;
use crate::State;
use crate::frames::viewers::{self, Intent as ViewerIntent};

/// The shell's poll: every Create file asked since the last poll, oldest first. Each is made, or said to have
/// failed; none waits for a later poll. `true`: something was asked.
pub fn poll(s: &mut State, tx: &mut Tx) -> bool {
    let asked = viewers::take_asked(s);
    let any = !asked.is_empty();
    for intent in asked {
        match intent {
            ViewerIntent::CreateFile { folder, name } => s.create_file(folder.as_deref(), &name, tx),
            // New file's Continue on an office type: New document (DW-1558) on that app, for the folder shown.
            ViewerIntent::NewDocument(kind) => {
                let folder = s.listing.folder.id.clone();
                crate::frames::office::edit::new_document::open(s, tx, Some(kind), folder);
            }
            // The dialog asks for nothing else.
            _ => {}
        }
    }
    any
}

impl State {
    /// Make the file. Not when the session gate closed, or moved to another session, since the press: what was
    /// asked under one session is never made under another (it is dropped, not kept for later).
    fn create_file(&mut self, folder: Option<&str>, name: &str, tx: &mut Tx) {
        if self.refused_without_session() {
            return;
        }
        match self.actions.source.new_file(folder, name) {
            // The engine sees the new file in the sync folder and lists and uploads it as any other.
            Ok(()) => self.forget_undo(),
            Err(why) => self.failed(Action::NewFile, why, tx),
        }
    }
}

#[cfg(test)]
#[path = "create_file_tests.rs"]
mod tests;
