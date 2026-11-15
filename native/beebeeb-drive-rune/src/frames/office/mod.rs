//! The Office frames that are not drawn yet (DW-1508, DW-1509, DW-1557 to DW-1561). One module per lane, each
//! with ITS OWN `FRAMES` table, so no two lanes share a file: `sheets` (OF2), `slides` (OF3), `dialogs` (OF4).
//! Every row shows `placeholder::scene` until its lane draws the frame.
//!
//! The Office frames that ARE drawn live in `frames::viewers` (the viewer's own group).

pub mod dialogs;
pub mod sheets;
pub mod slides;
