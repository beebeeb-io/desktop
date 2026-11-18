//! The Office frames that are not drawn yet (DW-1507 to DW-1509, DW-1557 to DW-1561, DW-1563, DW-1564). One module per
//! lane, each with ITS OWN `FRAMES` table, so no two lanes share a file: `docs` (OF5), `sheets` (OF2), `slides` (OF3),
//! `dialogs` (OF4), `states` (OF6). Every row shows `placeholder::scene` until its lane draws the frame.
//!
//! DW-1559 (sheets), DW-1560 (slides), DW-1558 (dialogs) are 0.9.1 frames (the editor): their rows stay placeholders in
//! 0.9.0.
//!
//! The Office frames that ARE drawn live in `frames::viewers` (the viewer's own group).

pub mod dialogs;
pub mod docs;
pub mod edit;
pub mod open_with;
pub mod sheets;
pub mod slides;
pub mod states;
