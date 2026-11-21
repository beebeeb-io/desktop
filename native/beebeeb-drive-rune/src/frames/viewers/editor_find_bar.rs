//! Find and replace in the editor (stub; ux-14 fills it).
#![allow(dead_code)] // the seam: ux-14 calls open, close and step and reads FindState.

use rune::{El, Span};

use super::EditorView;
use crate::theme::tokens::Theme;

/// The find bar, in the window's banner slot while find is open. Stub: none.
pub fn bar(_view: &EditorView, _t: &Theme) -> Option<El> {
    None
}

/// Open find (or find and replace); a selection within one line seeds the needle.
pub fn open(view: &mut EditorView, edit: &mut rune::TextEdit, replace: bool) {
    let _ = edit;
    view.find.open = true;
    view.find.replace_open = replace;
}

/// Close find and give the focus back to the editor.
pub fn close(view: &mut EditorView, edit: &mut rune::TextEdit) {
    view.find.open = false;
    view.find.replace_open = false;
    edit.focused = true;
}

/// Select the next (or previous) match. Stub: nothing.
pub fn step(_view: &mut EditorView, _edit: &mut rune::TextEdit, _backwards: bool) {}

/// The matches as background spans over `text`. Stub: none.
pub fn match_spans(_view: &EditorView, _text: &str, _t: &Theme) -> Vec<Span> {
    Vec::new()
}
