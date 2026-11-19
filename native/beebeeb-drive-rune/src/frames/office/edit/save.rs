//! Save and its three states for all three editors (DW-1552 to DW-1554): what the dot, Save and the status line
//! say while the revision differs from the last landed save's, a save on its way, and how its answer is told apart
//! when it comes later (a save leaves the window's thread: `Settled`, applied by `viewers::settle`).
//!
//! The revision a save was asked at is kept here ([`Sent`], in `OfficeUi.save`): what the person types while it is
//! on its way stays unsaved after it lands. The presses and the answers' wiring are `viewers/office_over.rs`'s.

use crate::frames::viewers::{Ui, office::Keeping};
use super::EditUi;

/// The toast while a save is on its way.
pub const SAVING: &str = "Saving\u{2026}";

/// The toast, and the status line, of a save kept on this PC while Beebeeb cannot be reached.
pub const QUEUED_TOAST: &str = "Saved on this PC \u{00B7} sends when Beebeeb can be reached";

/// A save that was asked and has not answered: the revision it holds, and the file as it was then.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub revision: u64,
    pub id: String,
    pub version: u32,
}

/// What the window can tell of a save on its way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seen {
    Waiting,
    Landed,
    /// Kept on this PC, not sent yet (offline): the file has not moved.
    Queued,
    Failed,
}

/// A save may begin: there is something to save, and none is on its way.
pub fn can_begin(edit: &EditUi) -> bool {
    !edit.stopped && !edit.saving && edit.unsaved()
}

/// The save of `sent` landed: the editor is saved up to the revision it held, no further.
pub fn landed(edit: &mut EditUi, sent: &Sent) {
    edit.saved_revision = sent.revision;
    edit.saving = false;
}

/// The save of `sent` is kept on this PC only: nothing is unsaved, but it is not a version yet.
pub fn queued(edit: &mut EditUi, sent: &Sent) {
    landed(edit, sent);
}

/// How the document is kept, as the status line says it after Encrypted (Word, sheet and deck alike).
pub fn kept_words(edit: &EditUi, version: u32) -> String {
    if edit.unsaved() { "unsaved changes".to_string() } else { format!("saved as version {version}") }
}

/// The edit was dropped (Discard my changes and reopen): nothing is unsaved any more.
pub fn dropped(edit: &mut EditUi) {
    edit.saved_revision = edit.revision;
    edit.saving = false;
}

/// Read the window's file and toast for what became of `sent`. A moved version (or another file: a Keep both)
/// is a landing, so is "Saved on this PC"; any other words after "Saving..." are a save that did not happen.
pub fn seen(sent: &Sent, file_id: &str, version: u32, toast: Option<&str>) -> Seen {
    if sent.id != file_id || sent.version != version {
        return Seen::Landed;
    }
    match toast {
        Some(words) if words.starts_with("Saved") => Seen::Landed,
        Some(words) if words != SAVING => Seen::Failed,
        _ => Seen::Waiting,
    }
}

/// How the open live document is kept, for the dot, Save and the status line.
pub fn keeping(ui: &Ui) -> Keeping {
    use crate::frames::viewers::office_over::Over;
    match (&ui.office.edit, &ui.office.over) {
        (None, _) => Keeping::Saved,
        (Some(_), Some(Over::Conflict(_))) => Keeping::Conflict,
        (Some(edit), _) if edit.saving => Keeping::Saving,
        (Some(edit), _) if edit.unsaved() => Keeping::Unsaved,
        _ => Keeping::Saved,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::office::edit::EditKind;

    fn sent() -> Sent {
        Sent { revision: 3, id: "f1".into(), version: 7 }
    }

    #[test]
    fn a_save_kept_on_this_pc_is_not_a_landing() {
        assert_eq!(seen(&sent(), "f1", 7, Some(QUEUED_TOAST)), Seen::Queued);
        assert_eq!(seen(&sent(), "f1", 7, Some("Saved as version 8")), Seen::Landed);
    }

    #[test]
    fn offline_the_status_says_where_the_save_is_not_a_version() {
        let mut edit = EditUi::new(EditKind::Docx);
        edit.revision = 3;
        queued(&mut edit, &sent());
        assert!(!edit.unsaved());
        assert_eq!(kept_words(&edit, 7), "Saved on this PC, uploads when online");
        // Typing on after it is unsaved again, and a later landing is a version again.
        edit.revision = 4;
        assert_eq!(kept_words(&edit, 7), "unsaved changes");
        landed(&mut edit, &Sent { revision: 4, ..sent() });
        assert_eq!(kept_words(&edit, 8), "saved as version 8");
    }
}
