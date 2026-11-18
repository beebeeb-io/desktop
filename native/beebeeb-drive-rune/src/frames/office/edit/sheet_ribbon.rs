//! The sheet ribbon's tools (a later task fills this). Until then the shared ribbon of `viewers/office.rs`.
#![allow(dead_code)]

use rune::El;

use crate::frames::viewers::{self, Ui};
use crate::theme::tokens::Theme;

pub fn ribbon(ui: &Ui, unsaved: bool, t: &Theme) -> El {
    viewers::office::ribbon(ui, unsaved, t)
}
