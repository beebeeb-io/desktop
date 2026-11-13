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
    install(s, conflict_fixture());
}

/// The board's sample source with its two waiting files.
pub fn conflict_fixture() -> FilesFixture {
    let mut fixture = FilesFixture::default();
    fixture.more.conflicts = More::board_conflicts();
    fixture
}

/// Put `fixture` behind the window, with the folders that show its changes.
pub fn install(s: &mut State, fixture: FilesFixture) {
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

/// DW-0481: three files dragged from File Explorer, over Documents.
pub fn drag_over_documents(s: &mut State) {
    s.clear_selection();
    s.show_detail_pane = false;
    s.actions.drop_hover = Some(super::uploads::DropHover {
        row: None,
        count: Some(3),
    });
    s.drop_over(1, true);
}

/// A row of Lisbon trip that is on its way up (the board's two: one moving, one waiting).
fn arriving(id: &str, name: &str, bytes: i64) -> DriveItem {
    DriveItem {
        id: id.to_string(),
        parent_id: Some("f-lisbon".to_string()),
        name: name.to_string(),
        size: Some(bytes),
        modified_at: "2026-11-05T09:30:00Z".to_string(),
        mime: Some("image/heic".to_string()),
        has_thumbnail: true,
        // Not here yet, not in Beebeeb yet: its PC column says uploading or waiting.
        pc_status: None,
        ..super::actions_data::blank_item()
    }
}

/// DW-0426: Lisbon trip while three of five upload: the files dropped there stand in its rows,
/// one with its bar, one waiting, and the status line says how far it is.
pub fn uploading(s: &mut State) {
    // Nothing is selected, so no panel shows (its toggle stays on, as the board draws it).
    s.clear_selection();
    s.open_folder(Some("f-lisbon".to_string()));
    let mut tx = rune::Tx::default();
    let files = [
        std::path::PathBuf::from("Miradouro.heic"),
        std::path::PathBuf::from("Ponte.heic"),
    ];
    if s.upload(Some("f-lisbon".to_string()), &files, &mut tx) {
        s.listing
            .items
            .push(arriving("up-miradouro", "Miradouro.heic", 3_774_874));
        s.listing.items.push(arriving("up-ponte", "Ponte.heic", 3_355_443));
    }
}

/// DW-0427: Uploads, opened from the status line.
pub fn uploads_panel(s: &mut State) {
    uploading(s);
    s.status_pressed();
}
