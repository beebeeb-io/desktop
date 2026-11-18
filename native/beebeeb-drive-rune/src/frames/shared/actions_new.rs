//! Shared's New request (DW-0504), create-request and New (DW-0414).
//!
//! - `Frame("DW-0504")` opens the New request dialog; `Do("create-request")` sends the dialog's title, note and four
//!   choices to the source (`create_request`), once. The dialog closes when the source took it (its link arrives later,
//!   through `take_link`, and goes to the clipboard); when it did not, the lists say "Not available yet".
//! - `Frame("DW-0414")`, Shared's toolbar New, opens Files' New menu the way This PC does.

use rune::Tx;

use super::dialogs::Dialog;
use super::source::Notice;
use crate::State;
use crate::frames::Intent;

/// Takes `intent` when it is one of these; `false`: not taken.
pub fn take(s: &mut State, _tx: &mut Tx, intent: &Intent) -> bool {
    match intent {
        Intent::Frame("DW-0504") => {
            s.frames.shared.open_dialog(Dialog::NewRequest);
            true
        }
        Intent::Frame("DW-0414") => open_files_new(s),
        Intent::Do("create-request", _) => create(s),
        _ => false,
    }
}

/// Send the dialog's request; only while the dialog is the open one.
fn create(s: &mut State) -> bool {
    let ui = &mut s.frames.shared;
    if ui.dialog != Some(Dialog::NewRequest) {
        return true;
    }
    let form = ui.fields.request.form(&ui.fields.title.text, &ui.fields.note.text);
    if form.title.is_empty() {
        return true;
    }
    if ui.source.create_request(form) {
        ui.close_dialog();
    } else {
        ui.notice = Some(Notice::NotAvailable);
    }
    true
}

/// Shared's New: the window goes to Files with its New menu open, as This PC's does.
fn open_files_new(s: &mut State) -> bool {
    use crate::shell::app::{Event, Route, Section};
    crate::shell::host::dispatch(s, Event::Navigate(Route::section(Section::Files)));
    s.actions.menu = None;
    s.toggle_menu(crate::frames::files::MenuKind::New);
    true
}
