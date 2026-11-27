//! Copy, cut and paste keys (input check §3 point 4, §4 limits 5 and 6).
//!
//! Ctrl+C asks the editor for the selection (`copy_selection`); the answer, text and HTML, arrives through the
//! source's poll into `EditUi.clip` and [`flush_clip`] writes it with `Tx::copy_rich`. Ctrl+X asks the same and then
//! sends `Delete` once the copy is on the clipboard. Ctrl+V reads the clipboard and sends `Paste`: plain text only in 0.9.0, even when the clipboard
//! also holds HTML.
#![allow(dead_code)]

use std::cell::Cell;

use beebeeb_office_protocol::check::MAX_OP_TEXT_BYTES;
use beebeeb_office_protocol::office::{EditKind, EditOp};
use rune::{ClipboardContent, KeyEvent, Tx};
use zeroize::Zeroize;

use crate::State;
use crate::frames::viewers::{self, Ui};

/// What the person is told when a paste is over what one edit can carry.
pub const PASTE_TOO_LARGE: &str = "That is too much to paste at once. Paste it in smaller pieces.";

thread_local! {
    /// A cut whose copy answer has not come yet. The `Delete` goes from [`flush_clip`], and only when the answer
    /// put text on the clipboard.
    static CUT_PENDING: Cell<bool> = const { Cell::new(false) };
}

/// The copy was refused (too large): the cut that waited on it removes nothing.
pub fn cancel_cut() {
    CUT_PENDING.with(|c| c.set(false));
}

/// A clipboard key. `false`: not taken. Called first by each surface's `on_key`.
pub fn on_clip_key(s: &mut State, tx: &mut Tx, ev: &KeyEvent) -> bool {
    if ev.mods.alt {
        return false;
    }
    let chord = ['c', 'x', 'v'].into_iter().find(|&c| ev.is_command(c));
    // Shift stays free for other chords, except Ctrl+Shift+V (paste as plain text, which is all paste is here).
    let Some(chord) = chord.filter(|&c| c == 'v' || !ev.mods.shift) else {
        return false;
    };
    let Some(ui) = viewers::ui(s) else { return false };
    let Some(edit) = ui.office.edit.as_ref().filter(|e| !e.stopped) else {
        return false;
    };
    let (kind, has_selection) = (edit.kind, !edit.selection.is_empty());
    tx.instant();
    match chord {
        'c' => {
            cancel_cut();
            ask_copy(ui);
        }
        'x' => {
            // Never delete what was not asked for: a cut whose copy was not taken removes nothing. In a document an
            // empty selection is a caret, and Delete would remove the next character; cells and shapes have no
            // selection rectangles to look at.
            // The `Delete` waits for the answer: `flush_clip` sends it once the text is on the clipboard.
            let asked = ask_copy(ui) && (kind != EditKind::Docx || has_selection);
            CUT_PENDING.with(|c| c.set(asked));
        }
        _ => tx.read_clipboard(paste),
    }
    true
}

fn ask_copy(ui: &mut Ui) -> bool {
    let (source, id) = ui.edit_source();
    let id = id.to_string();
    source.copy_selection(&id)
}

/// The editor's answer to a copy goes onto the clipboard: text, and HTML beside it when there is some. An empty
/// answer (nothing was selected) leaves the clipboard alone. `true`: the clipboard was written.
///
/// Call this from the poll, after `edit::poll` drained the source: the answer comes later than the key.
pub fn flush_clip(s: &mut State, tx: &mut Tx) -> bool {
    let Some(edit) = viewers::ui(s).and_then(|ui| ui.office.edit.as_mut()) else {
        return false;
    };
    let Some((mut text, html)) = edit.clip.take() else {
        return false;
    };
    let mut html = html.filter(|h| !h.is_empty());
    let written = !text.is_empty();
    if written {
        let mut content = ClipboardContent::new(text.as_str());
        if let Some(html) = &html {
            content = content.with_html(html.as_str());
        }
        tx.copy_rich(content);
    }
    text.zeroize();
    html.zeroize();
    // An empty answer means nothing was taken, so a waiting cut is dropped, not run.
    if CUT_PENDING.with(|c| c.replace(false))
        && written
        && let Some(ui) = viewers::ui(s)
    {
        let (source, id) = ui.edit_source();
        let id = id.to_string();
        source.edit(&id, EditOp::Delete { back: false, word: false });
    }
    written
}

/// What `Tx::read_clipboard` found: its plain text becomes a `Paste`.
fn paste(s: &mut State, content: &ClipboardContent, tx: &mut Tx) {
    tx.instant();
    if content.text().is_empty() {
        return;
    }
    // One paragraph break is one `\n` (Windows copies `\r\n`, old Mac apps `\r`).
    let text = content.text().replace("\r\n", "\n").replace('\r', "\n");
    let Some(ui) = viewers::ui(s) else { return };
    if ui.office.edit.as_ref().is_none_or(|e| e.stopped) {
        return;
    }
    if text.len() > MAX_OP_TEXT_BYTES {
        ui.toast = Some(PASTE_TOO_LARGE.to_string());
        return;
    }
    let (source, id) = ui.edit_source();
    let id = id.to_string();
    source.edit(&id, EditOp::Paste { text });
}
