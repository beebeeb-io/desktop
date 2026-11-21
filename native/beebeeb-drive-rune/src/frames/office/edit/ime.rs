//! The input method's text into the editor (input check §3, §6).
//!
//! Composition stays in this process: a `Preedit` only fills `EditUi.preedit`; nothing crosses to the helper until
//! `Commit`, so a keystroke of pre-edit costs no round trip. An empty `Commit` (a cancelled composition) inserts
//! nothing, `Preedit("")` ends the composition, and the pre-edit's cursor range is in BYTES of its string.
#![allow(dead_code)]

use std::cell::Cell;

use beebeeb_office_protocol::check::MAX_OP_TEXT_BYTES;
use beebeeb_office_protocol::office::EditOp;
use rune::{Color, El, Ime, TextStyle, Tx, col, el, row, text};
use zeroize::Zeroize;

use super::EditUi;
use crate::State;
use crate::frames::viewers::{self, Ui};

thread_local! {
    /// The byte range of the pre-edit's cursor or converted clause. `EditUi` has room for the text only (its file
    /// is frozen), so the range lives beside it; it is read only while `EditUi.preedit` holds a text, and every
    /// path that empties the text empties the range.
    static RANGE: Cell<Option<(usize, usize)>> = const { Cell::new(None) };
}

/// The marked range of the pre-edit now on screen, in bytes of `EditUi.preedit`. `None` when it is a bare caret
/// or there is no pre-edit.
pub fn preedit_range(edit: &EditUi) -> Option<(usize, usize)> {
    let text = edit.preedit.as_deref().filter(|t| !t.is_empty())?;
    RANGE.get().filter(|&(a, b)| a < b && b <= text.len() && text.is_char_boundary(a) && text.is_char_boundary(b))
}

/// End the composition, wiping its text.
fn end_preedit(edit: &mut EditUi) {
    if let Some(mut old) = edit.preedit.take() {
        old.zeroize();
    }
    RANGE.set(None);
}

/// What the input method says: the pre-edit, a commit, or that it turned on or off.
pub fn on_ime(s: &mut State, tx: &mut Tx, ime: Ime) {
    // A caret that glides takes the candidate window with it (input check limit 2).
    tx.instant();
    let Some(ui) = viewers::ui(s) else { return };
    let Some(edit) = ui.office.edit.as_mut() else { return };
    if edit.stopped {
        end_preedit(edit);
        return;
    }
    match &ime {
        Ime::Preedit(pre, range) => {
            if pre.is_empty() {
                end_preedit(edit);
            } else {
                if let Some(mut old) = edit.preedit.replace(pre.clone()) {
                    old.zeroize();
                }
                RANGE.set(*range);
            }
        }
        Ime::Commit(committed) => {
            end_preedit(edit);
            if !committed.is_empty() {
                insert(ui, committed);
            }
        }
        // A composition cannot outlive the input method being off.
        Ime::Disabled => end_preedit(edit),
        Ime::Enabled => {}
    }
}

/// Send `text` as `InsertText`, in pieces the protocol takes whole (a dictation can be long).
fn insert(ui: &mut Ui, text: &str) {
    let (source, id) = ui.edit_source();
    let id = id.to_string();
    let mut rest = text;
    while !rest.is_empty() {
        let mut end = rest.len().min(MAX_OP_TEXT_BYTES);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        let (piece, tail) = rest.split_at(end);
        if !source.edit(&id, EditOp::InsertText(piece.to_string())) {
            return;
        }
        rest = tail;
    }
}

/// The element that tells the system where the caret is. `None`: nothing to say (no editor, it has not the
/// keyboard, it stopped, or there is no caret). The element is the caret's size in page points and draws nothing;
/// the surface places it where it draws the caret (and scales it), so the candidate window follows the caret.
pub fn caret_el(s: &State) -> Option<El> {
    let edit = s.viewer.ui.as_ref()?.office.edit.as_ref()?;
    if !edit.has_keyboard || edit.stopped {
        return None;
    }
    let rect = edit.caret?.rect;
    Some(el().key("office-caret").size(rect.w.max(1.0), rect.h.max(1.0)).ime_cursor())
}

/// The pre-edit drawn as a short line of text at `size` px in `ink`, every character underlined and the marked
/// range (the clause being converted) in bold with a thicker line. The surface puts it at the caret. `None`: no
/// composition is open.
pub fn preedit_el(s: &State, size: f32, ink: Color) -> Option<El> {
    let edit = s.viewer.ui.as_ref()?.office.edit.as_ref()?;
    if edit.stopped {
        return None;
    }
    let pre = edit.preedit.as_deref().filter(|t| !t.is_empty())?;
    let (a, b) = preedit_range(edit).unwrap_or((0, 0));
    let piece = |part: &str, marked: bool| {
        col()
            .child(
                text(part)
                    .font(TextStyle::new(size, if marked { 700.0 } else { 400.0 }))
                    .color(ink),
            )
            .child(el().w_full().h(if marked { 2.0 } else { 1.0 }).fill(ink))
    };
    let mut line = row().key("office-preedit");
    if a == b {
        line = line.child(piece(pre, false));
    } else {
        for (part, marked) in [(&pre[..a], false), (&pre[a..b], true), (&pre[b..], false)] {
            if !part.is_empty() {
                line = line.child(piece(part, marked));
            }
        }
    }
    Some(line)
}

#[cfg(test)]
#[path = "ime_tests.rs"]
mod tests;
