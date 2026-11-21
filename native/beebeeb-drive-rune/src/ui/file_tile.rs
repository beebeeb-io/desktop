//! The one file tile (stub: filled by drive-complete-016).
#![allow(dead_code, unused_variables)]

use super::file_row::RowLook;
use super::grid_tiles::Thumb;
use crate::theme::file_icons::FileIcon;
use crate::theme::tokens::Theme;
use rune::{El, el};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileSize {
    Small,
    Medium,
    Large,
}

pub enum TilePicture<'a> {
    Icon(FileIcon),
    Thumb(&'a Thumb),
}

#[allow(clippy::too_many_arguments)]
pub fn tile(
    key: &str,
    label: &str,
    picture: TilePicture,
    name: &str,
    badge: Option<El>,
    look: RowLook,
    size: TileSize,
    t: &Theme,
) -> El {
    el()
}
