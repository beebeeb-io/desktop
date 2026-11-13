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
/// that session goes. The rows themselves are read again by the shell (`on_gate_change`, or
/// [`read_the_root_again`] where the gate did not move).
///
/// `Actions` is taken apart field by field, with no `..`: a field added to it does not compile
/// until it is named here, as something that goes or as something that stays, with the reason.
pub fn end_session(s: &mut State, account_left: bool) {
    close_dialogs(s);
    let super::Actions {
        // Stays: the wiring, not the session's state.
        source,
        // Closed by `close_dialogs` above.
        menu: _,
        dialog: _,
        rename: _,
        name_edit: _,
        name_error: _,
        share_password: _,
        toast,
        toast_until,
        selection,
        anchor,
        sort,
        undo,
        // Stays: the ids of board frames that were asked for, no name of a file.
        intents: _,
        retry_upload,
        retry_share,
        // Stay: the window's size, the two values only a board scene sets, whose disk this is.
        window: _,
        board_crop_top: _,
        board_first_row: _,
        host: _,
        forward,
        made,
        conflicts_done,
        protected,
        typed,
        picker,
        drop_hover,
        uploads_open,
        upload_menu,
        session_no,
        // Dropped by `forget_session_ended_rows` below (PR 372's held rows and its count).
        session: _,
        local,
        first_load_strip,
    } = &mut s.actions;
    // Another session of this window: an answer asked for under the one that ended is dropped.
    *session_no += 1;
    // The folder has not loaded under the next session yet.
    *first_load_strip = false;
    // The account left (a sign-out, another account): what it was uploading is cancelled and
    // forgotten by the source, so the next account sees no status line, panel or row of it. A
    // LOCK is the same account coming back: its uploads go on (the panel says "Continues in the
    // background"); only the window's own state of them goes.
    // Asked only when there is something to forget.
    if account_left && source.uploads().is_some() {
        source.forget_uploads();
    }
    // What Try again would repeat: a share names a file, an upload names paths on this PC.
    *retry_share = None;
    *retry_upload = None;
    // The ids a change of what this PC holds was left for, and an answer of it not said yet.
    *local = super::local::Ui::default();
    // What Undo would put back: the earlier session's rows.
    undo.clear();
    *toast = None;
    *toast_until = None;
    selection.clear();
    *anchor = None;
    forward.clear();
    made.clear();
    *conflicts_done = 0;
    // The open message goes; the section last seen stays (it is the route's, not the session's).
    protected.alert = None;
    *typed = super::long_list::Typed::default();
    // Uploading (`uploads.rs`): a picker that was asked for, a drag over the list, the open panel.
    *picker = None;
    *drop_hover = None;
    *uploads_open = false;
    *upload_menu = None;
    // The root's own sort, not the one of the folder the earlier session was in.
    *sort = source.sort_of(None).unwrap_or_default();
    s.selected = None;
    s.files.back.clear();
    // Search over the window: what was typed, what was narrowed to and the result that was
    // selected are the earlier session's. Its source stays: that is wiring, not session state.
    close_search(s);
    // Two versions (DW-0429) is the SHELL's dialog (`shell::app`): the route machine drops it
    // on a sign-out and for another account, and keeps it across a lock ("unlock returns to the
    // same place"). It holds no secret and is not closed from here.
    forget_session_ended_rows(s);
}

/// The shell's session gate closed under the window (`shell::session_gate`), in any way: also
/// the session that ended while the gate stayed Ready, where the rows stay (`session.rs`). What
/// could still act goes: every dialog through the wiping path, a name being typed in a row, an
/// open menu, a picker that was asked for, and Search over the window. No source is asked.
pub fn session_closed(s: &mut State) {
    close_dialogs(s);
    s.actions.picker = None;
    s.actions.drop_hover = None;
    s.actions.upload_menu = None;
    close_search(s);
}

/// Search over the window ends: what was typed, what was narrowed to, the selected result.
fn close_search(s: &mut State) {
    s.search = rune::TextEdit::new("");
    let find = &mut s.frames.find;
    find.open = false;
    find.selected = None;
    find.narrow = crate::frames::search::source::Narrow::All;
    find.pictured = false;
}

/// The session-ended window (`frames/files/session.rs`, PR 372) holds rows it took out of the
/// listing while `KEEP_THE_LIST` was false. A sign-out or account switch drops them so they
/// cannot replace a new listing when the next session starts. The refusal count resets too.
fn forget_session_ended_rows(s: &mut State) {
    s.actions.session = super::session::Ui::default();
}

/// The rows of a session that ended under a gate that did not move (another account while it
/// stayed Ready, or a change the poll did not see happen): dropped first, then the root is read
/// from the source as it is now.
pub fn read_the_root_again(s: &mut State) {
    s.listing.items.clear();
    s.selected = None;
    s.files.back.clear();
    s.actions.forward.clear();
    (s.listing, s.list_error) = crate::load(s.source.as_ref(), None);
    s.actions.sort = s.actions.source.sort_of(None).unwrap_or_default();
    s.apply_sort();
}

/// A message whose time is over goes from the state too, and what its Try again held goes with
/// it: the share's items and choices, the upload's paths, the ids a change of what this PC holds was left
/// for. `true`: one went.
pub fn poll(s: &mut State) -> bool {
    let over = s
        .actions
        .toast_until
        .is_some_and(|until| std::time::Instant::now() >= until);
    if !over {
        return false;
    }
    s.actions.toast = None;
    s.actions.toast_until = None;
    s.actions.retry_share = None;
    s.actions.retry_upload = None;
    s.actions.local.retry = None;
    true
}
