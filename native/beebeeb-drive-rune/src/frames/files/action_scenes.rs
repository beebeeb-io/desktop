//! The board frames of the action dialogs and the preview: what each sets on the board's
//! sample window (`SCENES` in `mod.rs` lists them). Every scene goes the way the app's own
//! controls go: it selects the row and calls what the menu item, the key or the button calls.

use super::actions_data::More;
use super::source::{BoardDrive, FilesFixture};
use crate::State;
use crate::shell::app::{Dialog, Event};
use crate::source::DriveItem;

/// The row of Contract draft.pdf in the board's Files.
const CONTRACT: usize = 6;

/// DW-0407: Share, for Contract draft.pdf, on its defaults.
pub fn share(s: &mut State) {
    s.select_only(CONTRACT);
    s.begin_share();
}

/// DW-0411: Details of Contract draft.pdf.
pub fn details(s: &mut State) {
    s.select_only(CONTRACT);
    s.begin_details();
}

/// DW-0413: Version history of Contract draft.pdf.
pub fn versions(s: &mut State) {
    s.select_only(CONTRACT);
    s.begin_versions();
}

/// DW-0408: Preview of Contract draft.pdf, its first page.
pub fn preview(s: &mut State) {
    s.select_only(CONTRACT);
    s.preview();
}

/// The board's file of a kind Windows cannot preview (DW-0408b): Moodboard.sketch, online only.
pub fn moodboard() -> DriveItem {
    DriveItem {
        id: "moodboard".to_string(),
        name: "Moodboard.sketch".to_string(),
        size: Some(19_293_798),
        modified_at: "2026-10-30T10:00:00Z".to_string(),
        mime: Some("application/octet-stream".to_string()),
        pc_status: Some("online_only".to_string()),
        ..super::actions_data::blank_item()
    }
}

/// DW-0408b: Preview of Moodboard.sketch, which has none. The board draws it without the
/// details panel, the file as the tenth row.
pub fn no_preview(s: &mut State) {
    s.show_detail_pane = false;
    s.listing.items.insert(9, moodboard());
    s.select_only(9);
    s.preview();
}

/// The board's window with its two waiting files (DW-0429), behind the same folders.
pub fn with_conflicts(s: &mut State) {
    let mut fixture = FilesFixture::default();
    fixture.more.conflicts = More::board_conflicts();
    s.source = Box::new(BoardDrive::sharing(&fixture.edits));
    (s.listing, s.list_error) = crate::load(s.source.as_ref(), None);
    s.actions.source = Box::new(fixture);
}

/// DW-0429: Two versions of Contract draft.pdf, 1 of 2, over the list without the details panel.
pub fn two_versions(s: &mut State) {
    with_conflicts(s);
    s.show_detail_pane = false;
    s.select_only(CONTRACT);
    s.actions.conflicts_done = 0;
    crate::shell::host::dispatch(s, Event::OpenDialog(Dialog::Conflict { file: None }));
}
