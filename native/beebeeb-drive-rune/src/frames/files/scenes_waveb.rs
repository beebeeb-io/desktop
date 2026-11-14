//! Board frames of the Files window that had no scene (Wave B, batch F1): several rows selected
//! (DW-0403), a file kept on this PC while it downloads (DW-0412), Lisbon trip (DW-0420), the
//! list's own menu (DW-1103) and instant search on this PC (DW-1105). Each draws through the real
//! view code over the board's fixture source; the engine binds where it has an answer.

use super::{MenuKind, OpenMenu};
use crate::State;
use crate::ui::gate::Scene;

/// The rows of Budget 2026.xlsx and Contract draft.pdf in the board's Files.
const BUDGET: usize = 5;
const CONTRACT: usize = 6;

/// DW-0403: Budget 2026.xlsx and Contract draft.pdf picked with Ctrl+click: the bar under the toolbar,
/// the panel counting the two.
pub fn multi_select(s: &mut State) {
    s.select_only(BUDGET);
    s.toggle_row(CONTRACT);
}

/// DW-0412: Contract draft.pdf kept on this PC, 0.7 of 1.2 MB down, the message "Keeping on this PC".
pub fn keeping(s: &mut State) {
    super::keeping::scene(s);
}

/// DW-0420: Lisbon trip, one level down: back is on, the path is the title, thumbnails and the video's mark.
pub fn lisbon_trip(s: &mut State) {
    use crate::frames::global::samples::TripByDate;
    s.source = Box::new(TripByDate::new());
    (s.listing, s.list_error) = crate::load(s.source.as_ref(), None);
    s.actions = super::Actions::new(Box::new(TripByDate::files()));
    s.open_folder(Some(TripByDate::FOLDER.to_string()));
    s.clear_selection();
    // The board lists the rows oldest first under a header that reads "Name": the header is the
    // window's own sort (Name), the rows stand as the board has them (the sort of the folder above).
    s.actions.sort = super::Sort::default();
    s.global = crate::frames::global::Ui::default();
    s.global.grid.thumbs = Box::new(super::long_list::TripThumbs);
}

/// DW-1103: the menu of a right-click on the list's empty space, "Sort by" open.
pub fn background_menu(s: &mut State) {
    s.show_detail_pane = false;
    s.clear_selection();
    s.actions.menu = Some(OpenMenu {
        kind: MenuKind::Background,
        at: (560.0, 452.0),
        hot: Some(super::background_menu::SORT_BY),
    });
}

pub const SCENES: &[Scene<State>] = &[
    ("DW-0403", multi_select),
    ("DW-0412", keeping),
    ("DW-0420", lisbon_trip),
    ("DW-1103", background_menu),
    ("DW-1105", crate::frames::search::dw_1105),
];
