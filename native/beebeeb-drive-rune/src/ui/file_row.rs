//! The one file row and its column header (stub: filled by drive-complete-015).
#![allow(dead_code, unused_variables)]

use crate::theme::file_icons::FileIcon;
use crate::theme::tokens::Theme;
use rune::{El, el};

/// How a row looks right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowLook {
    pub selected: bool,
    pub focused: bool,
    pub dimmed: bool,
    /// (position, set size over the whole list).
    pub position: Option<(usize, usize)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Width {
    Fixed(f32),
    Flex(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Column {
    pub key: &'static str,
    pub title: &'static str,
    pub width: Width,
}

pub fn row(key: &str, label: &str, icon: FileIcon, name: &str, cells: Vec<El>, look: RowLook, t: &Theme) -> El {
    el()
}

pub fn header(columns: &[Column], sorted: Option<(usize, bool)>, t: &Theme) -> El {
    el()
}
