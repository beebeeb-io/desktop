//! DW-1562 (decision D-OV-14): the board's right-click on "Service agreement.docx" in Files, with Open With's apps
//! beside the file menu. Drawn by the Files window's own code (`frames::files::open_with_menu`) over the board's
//! fixture: this sets the state only.
//!
//! The file menu offers Open With only for a type the Office viewer ships (`kinds::office_route` → `Shows`): the scene
//! turns Word's switch on for itself (`open_with_menu::Ui::board_office`), as the frame is about Open With (ruling
//! R-OW2 c). The app's own switches are not touched.

use crate::State;
use crate::frames::files::open_with_menu::OPEN_WITH;
use crate::frames::files::{MenuKind, OpenMenu};
use crate::frames::viewers::kinds::OfficeShips;
use crate::source::DriveItem;

/// The row the board adds to Files: "Service agreement.docx", "Today, 08:40", "38 KB", on this PC (downloaded).
pub const AGREEMENT_ID: &str = "agreement";
/// Its place in the board's list: after the five folders and Budget 2026.xlsx (12 items).
pub const AGREEMENT_ROW: usize = 6;

/// Word's switch on, every other type's off.
pub const WORD_ON: OfficeShips = OfficeShips {
    docx: true,
    doc: false,
    xlsx: false,
    xls: false,
    pptx: false,
};

/// The board's status line: it counts the rows without the Word file (ruling R-OW2 b: the scene matches the board).
pub const BOARD_STATUS: &str = concat!("4 on ", beebeeb_engine::device_word!(), " \u{00B7} 3 available offline \u{00B7} 4 online only");

/// The board's row for the Word file (the fixture's "today" is Thursday 5 Nov 2026).
pub fn agreement() -> DriveItem {
    DriveItem {
        id: AGREEMENT_ID.into(),
        parent_id: None,
        name: "Service agreement.docx".into(),
        is_folder: false,
        size: Some(38 * 1024),
        child_count: None,
        modified_at: "2026-11-05T08:40:00Z".into(),
        mime: Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document".into()),
        has_thumbnail: false,
        shared_type: None,
        shared_badges: vec![],
        pc_status: Some("on_this_pc".into()),
        is_protected: false,
    }
}

/// DW-1562: the row selected, the file menu open at it (as DW-0402's), "Open With" highlighted, its apps beside it.
pub fn scene_open_with(s: &mut State) {
    if !s.listing.items.iter().any(|item| item.id == AGREEMENT_ID) {
        let at = AGREEMENT_ROW.min(s.listing.items.len());
        s.listing.items.insert(at, agreement());
    }
    s.select_only(AGREEMENT_ROW);
    // The board draws DW-1562 without the details panel (`details: null`).
    s.show_detail_pane = false;
    s.actions.open_with.board_office = Some(WORD_ON);
    s.global.board_status_words = Some(BOARD_STATUS.to_string());
    s.actions.menu = Some(OpenMenu {
        kind: MenuKind::File,
        at: (430.0, 316.0),
        hot: Some(OPEN_WITH),
    });
}
