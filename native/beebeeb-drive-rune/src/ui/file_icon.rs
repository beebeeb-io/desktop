//! A file or folder icon from the registry: its art in its tint.
#![allow(dead_code)]

use crate::theme::file_icon_art::art;
use crate::theme::file_icon_colors::tint_color;
use crate::theme::file_icons::FileIcon;
use crate::theme::tokens::Theme;
use crate::ui::kit;
use rune::El;

pub fn file_icon(key: impl Into<String>, fi: FileIcon, size: f32, t: &Theme) -> El {
    kit::glyph(key, art(fi.id), size, tint_color(fi.tint, t.dark))
}
