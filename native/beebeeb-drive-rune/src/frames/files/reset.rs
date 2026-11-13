//! What the Files window forgets, and when.
//!
//! Two moments, each with ONE function, both called by the shell (`shell::host`):
//!
//! * [`sync`], after every route event: the route left the Files window (Settings, a viewer,
//!   another section), so every dialog of this window closes through the wiping path. A Share
//!   dialog and its password, a Preview and its page, a typed name do not wait behind another
//!   screen to come back with it.
//! * [`end_session`], when the session's gate closes or another account is signed in (the app
//!   locked, the account signed out): the WHOLE window state of the session goes. Nothing of the
//!   earlier session is drawn, kept for Try again, or put back by Undo under the next one. That
//!   includes Search over the window.
//!
//! The password of a link (`Actions::share_password`) is a secure `rune::TextEdit`: `clear`
//! zeroizes its buffer and its history, and the edit zeroizes again when it is dropped. Both
//! happen here: the edit is cleared, then replaced, so the old one is dropped.

use crate::State;
use crate::shell::app::Section;
use crate::shell::registry::{self, FrameGroup};

/// Is the Files window what the route shows? (Backups shows its folder in it: `protected.rs`.)
fn on_files_route(s: &State) -> bool {
    matches!(
        registry::group_for(&s.shell.app),
        FrameGroup::Section(Section::Files | Section::Backups)
    )
}

/// Every dialog of the Files window closes, and what was typed in one is wiped: Share (and the
/// link's password), Details, Version history, Preview (and its page), New folder, Move to, a
/// name being typed in a row, an open menu. No source is asked anything.
pub fn close_dialogs(s: &mut State) {
    let a = &mut s.actions;
    a.dialog = None;
    a.menu = None;
    a.rename = None;
    a.name_error = None;
    a.name_edit = rune::TextEdit::default();
    super::share::wipe(s);
    // Dropped as well as cleared: the old edit zeroizes on drop.
    s.actions.share_password = rune::TextEdit::new("").secure();
}

/// After a route event: a route that is not the Files window's closes its dialogs.
pub fn sync(s: &mut State) {
    let open = s.actions.dialog.is_some() || s.actions.rename.is_some() || s.actions.menu.is_some();
    if open && !on_files_route(s) {
        close_dialogs(s);
    }
}

/// The session ended under the window (lock, sign-out, another account): everything it held of
/// that session goes. The listing itself is read again by the shell (`on_gate_change`).
pub fn end_session(s: &mut State) {
    close_dialogs(s);
    let a = &mut s.actions;
    // What Try again would repeat: a share names a file, an upload names paths on this PC.
    a.retry_share = None;
    a.retry_upload = None;
    // What Undo would put back: the earlier session's rows.
    a.undo.clear();
    a.toast = None;
    a.toast_until = None;
    a.selection.clear();
    a.anchor = None;
    a.forward.clear();
    a.made.clear();
    a.conflicts_done = 0;
    a.protected.alert = None;
    a.typed = super::long_list::Typed::default();
    s.selected = None;
    s.files.back.clear();
    // Search over the window: what was typed, what was narrowed to and the result that was
    // selected are the earlier session's. Its source stays: that is wiring, not session state.
    s.search = rune::TextEdit::new("");
    let find = &mut s.frames.find;
    find.open = false;
    find.selected = None;
    find.narrow = crate::frames::search::source::Narrow::All;
    find.pictured = false;
    // Two versions (DW-0429) is the SHELL's dialog (`shell::app`): the route machine drops it
    // on a sign-out and for another account, and keeps it across a lock ("unlock returns to the
    // same place"). It holds no secret and is not closed from here.
    forget_session_ended_rows(s);
}

/// HOOK for the session-ended window (`frames/files/session.rs`, PR 372, not merged here): the
/// rows it holds of the session that ended are dropped HERE when another session starts. Empty
/// until that PR is in; its lane fills this body and nothing else has to change.
fn forget_session_ended_rows(_s: &mut State) {}
