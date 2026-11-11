//! Backups on Windows (the board's DW-0737, DW-1411, DW-0738, DW-1410, DW-0739, DW-0742,
//! DW-0741): a section of the settings view. NO frame of the group is drawn yet
//! (`frames/this_pc/FRAMES.md` is the checklist); this module holds what the frames will read:
//! [`source::BackupsSource`] and the board's sample content, with the rule the owner set for the
//! Backups root (`is_protected`: it cannot be renamed, moved, deleted or shared from the app).
//!
//! Until a view reads it the code has no caller outside its tests.
#![cfg_attr(not(test), allow(dead_code))]

pub mod source;
