//! The Office frames (DW-1507 to DW-1509, DW-1557 to DW-1561, DW-1563, DW-1564). One module per lane, each with ITS OWN
//! `FRAMES` table, so no two lanes share a file: `docs` (OF5), `sheets` (OF2), `slides` (OF3), `dialogs` (OF4), `states`
//! (OF6). Every row is drawn now; `edit` holds the live editor's files (DW-1508e and DW-1559 in `edit::sheet_ribbon`,
//! DW-1560 in `edit::slide_ribbon`, DW-1558 in `edit::new_document`). The editor is in 0.9.0 (the owner's 2026-11-18
//! ruling reversed decision 2539, which had put it in 0.9.1).
//!
//! The Office frames that ARE drawn live in `frames::viewers` (the viewer's own group).

pub mod dialogs;
pub mod docs;
pub mod edit;
pub mod open_with;
pub mod sheets;
pub mod slides;
pub mod states;

use rune::{El, col, row};

use crate::ui::focus;

/// The F6 stops of an office window (`ui::focus::pane`), in the order a window builds them after the sidebar: the
/// window's toolbar, the ribbon of a file being edited, a panel beside the file (the outline, the filmstrip), the file
/// itself, the details panel, the status line. F6 goes to the next one's first Tab stop, Shift+F6 to the previous one's.
pub const PANE_TOOLBAR: &str = "office-pane-toolbar";
pub const PANE_RIBBON: &str = "office-pane-ribbon";
pub const PANE_SIDE: &str = "office-pane-side";
pub const PANE_DOCUMENT: &str = "office-pane-document";
pub const PANE_DETAILS: &str = "office-pane-details";
pub const PANE_STATUS: &str = "office-pane-status";

/// This run's F6 order, as the window reads: the toolbar, the ribbon, the panel left of the file, the file, the details
/// panel, the status line (`ui::focus::pane` keeps the order in which a run first names its keys, and a window builds
/// its parts in another one). A window calls it after it built its sidebar (the first stop) and before any other pane,
/// naming only the panes it then builds: F6 to a pane that is not drawn would go nowhere.
pub(crate) fn order(ribbon: bool, side: bool, document: bool, details: bool) {
    let panes = [
        (PANE_TOOLBAR, true),
        (PANE_RIBBON, ribbon),
        (PANE_SIDE, side),
        (PANE_DOCUMENT, document),
        (PANE_DETAILS, details),
        (PANE_STATUS, true),
    ];
    for (key, drawn) in panes {
        if drawn {
            drop(focus::pane(rune::el(), key));
        }
    }
}

/// A full-width band of the window (the toolbar, the ribbon, the status line) as the F6 stop `key`.
pub(crate) fn band(band: El, key: &'static str) -> El {
    focus::pane(col().w_full().child(band), key)
}

/// The file's own part of the row under the ribbon, as an F6 stop: it takes the room the panels beside it leave.
/// `own`: nothing in it takes the keyboard (a page or a slide that is only read), so the pane itself is the stop, named
/// `own`: F6 lands on the file with the ring around it, and the window's keys scroll or page it from there.
pub(crate) fn document(parts: impl IntoIterator<Item = El>, own: Option<&'static str>) -> El {
    let mut file = row().grow(1.0).min_w(0.0).h_full().children(parts);
    if let Some(name) = own {
        file = file.role("group").label(name).focusable(true);
    }
    focus::pane(file, PANE_DOCUMENT)
}

/// A panel beside the file (as wide as it draws itself) as the F6 stop `key`.
pub(crate) fn panel(panel: El, key: &'static str) -> El {
    focus::pane(row().h_full().child(panel), key)
}
