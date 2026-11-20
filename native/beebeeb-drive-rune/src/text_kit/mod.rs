//! Pure text building blocks for the editor: lines, highlight spans, find and indent.
//! No UI. Nothing calls it yet (ux-15 removes the allow).
#![allow(dead_code)]

pub mod find;
pub mod highlight;
pub mod indent;
pub mod lines;
