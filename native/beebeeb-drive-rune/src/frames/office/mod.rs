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
