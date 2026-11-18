//! The live editor's frame (W2 of the editor epic): the state the editor keeps while a document is open for
//! editing, the poll that drains the source's answers, and `chrome`, the ribbon and status line around a live body.
//! Every later task owns one file below and hangs its tests under that file with `#[path]`; nobody edits this
//! file again.

use beebeeb_office_protocol::office::{EditKind, Edited, Marks, PageRect};
use rune::{Cx, El, Tx, col};

use crate::State;
use crate::frames::viewers::{self, Ui, office::Keeping, source::EditAnswer};
use crate::theme::tokens;

pub mod clipboard;
pub mod ime;
pub mod new_document;
pub mod palette;
pub mod recovery;
pub mod save;
pub mod sheet;
pub mod sheet_ribbon;
pub mod slide;
pub mod slide_ribbon;
pub mod word;
pub mod word_insert;
pub mod word_ribbon;

/// Keys of the editor's focusable surfaces start with this; `has_keyboard` reads it from `Tx::focused()`.
pub const FOCUS_PREFIX: &str = "edit-";

/// What the editor keeps while a document is open for editing (`OfficeUi.edit`).
#[derive(Debug, Clone)]
pub struct EditUi {
    pub kind: EditKind,
    /// The revision the helper last answered.
    pub revision: u64,
    /// The revision of the last save that landed. `revision != saved_revision`: unsaved.
    pub saved_revision: u64,
    pub caret: Option<PageRect>,
    pub selection: Vec<PageRect>,
    pub marks: Option<Marks>,
    pub can_undo: bool,
    pub can_redo: bool,
    /// A save is on its way (typing during it stays unsaved).
    pub saving: bool,
    /// An editor surface has the keyboard.
    pub has_keyboard: bool,
    /// The input method's text not yet committed.
    pub preedit: Option<String>,
    /// The last copy's text and html, until the clipboard code took it.
    pub clip: Option<(String, Option<String>)>,
    /// The slide layouts' names, once asked.
    pub layouts: Vec<String>,
    /// The editor stopped: the body is no longer live.
    pub stopped: bool,
    /// A recovery was restored, saved at this time.
    pub restored: Option<String>,
}

impl EditUi {
    pub fn new(kind: EditKind) -> Self {
        EditUi {
            kind,
            revision: 0,
            saved_revision: 0,
            caret: None,
            selection: Vec::new(),
            marks: None,
            can_undo: false,
            can_redo: false,
            saving: false,
            has_keyboard: false,
            preedit: None,
            clip: None,
            layouts: Vec::new(),
            stopped: false,
            restored: None,
        }
    }

    pub fn unsaved(&self) -> bool {
        self.revision != self.saved_revision
    }

    /// Take what an `Edited` says. Pictures and cells are the viewer's (`viewer_office`), not kept here.
    pub fn apply(&mut self, e: &Edited) {
        self.revision = e.revision;
        self.caret = e.caret;
        self.selection = e.selection.clone();
        self.marks = Some(e.marks);
        self.can_undo = e.can_undo;
        self.can_redo = e.can_redo;
    }
}

/// Drain the source's editor answers into `EditUi`, and note whether an editor surface has the keyboard.
/// `true`: something changed, the view must run again.
pub fn poll(s: &mut State, tx: &mut Tx) -> bool {
    let focused = tx.focused().is_some_and(|key| key.starts_with(FOCUS_PREFIX));
    let Some(ui) = viewers::ui(s) else { return false };
    let Some(mut edit) = ui.office.edit.take() else { return false };
    let mut changed = edit.has_keyboard != focused;
    edit.has_keyboard = focused;
    loop {
        let (source, id) = ui.edit_source();
        let id = id.to_string();
        let Some(answer) = source.poll_edit(&id) else { break };
        changed = true;
        match answer {
            EditAnswer::Edited(e) => edit.apply(&e),
            EditAnswer::Clip { text, html } => edit.clip = Some((text, html)),
            EditAnswer::Layouts(names) => edit.layouts = names,
            EditAnswer::Stopped => edit.stopped = true,
            EditAnswer::Restored { at } => edit.restored = Some(at),
            EditAnswer::CopyTooLarge => ui.toast = Some(crate::frames::viewers::source::COPY_TOO_LARGE.to_string()),
        }
    }
    ui.office.edit = Some(edit);
    clipboard::flush_clip(s, tx);
    changed
}

/// A live page, grid or slide with the ribbon above it and the status line under it.
pub fn chrome(_s: &State, cx: &mut Cx<State>, ui: &Ui, kind: EditKind, body: El) -> El {
    let t = tokens::theme(cx.system_dark());
    let unsaved = ui.office.edit.as_ref().is_some_and(EditUi::unsaved);
    let keeping = if unsaved { Keeping::Unsaved } else { Keeping::Saved };
    // The tools of the kind's ribbon come from its own file once that task lands; until then the shared ribbon.
    let ribbon = match kind {
        EditKind::Docx => word_ribbon::ribbon(ui, unsaved, &t),
        EditKind::Xlsx => sheet_ribbon::ribbon(ui, unsaved, &t),
        EditKind::Pptx => slide_ribbon::ribbon(ui, unsaved, &t),
    };
    col()
        .key("edit-chrome")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .child(ribbon)
        .child(body)
        .child(viewers::office::status_live(ui, keeping, &t))
}
