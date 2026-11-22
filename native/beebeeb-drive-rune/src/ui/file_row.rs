//! The one file row (`.mk-row`) and its column header (`.mk-thead`), for every list.
//!
//! A row is a frame: the tile and the name on the left, then the caller's cells in order. The
//! frame owns the selection fill (`t.selection_row`), the dimmed state, the role (`row`), the label
//! (the name) and the row's place in the whole list (`position_in_set`). Names longer than
//! [`NAME_MAX`] characters are cut in the middle ([`shortened`]); the pointer shows the whole name.
//!
//! Keys, for a row keyed `k`: `k`, `k-tile`, `k-icon`, `k-name`, and `name-cell`.
//! Header keys, for a column keyed `c`: `column-headers`, `col-c-cell`, `col-c`, `sort-arrow`.
// The areas move onto `row` and `header_with` in wave 3.
#![allow(dead_code)]

use crate::theme::file_icons::FileIcon;
use crate::theme::icons;
use crate::theme::tokens::{Theme, radius, type_};
use rune::{Align, Color, El, el, row as hrow};

use super::kit;

/// `.mk-row { height: 30px }`
pub const ROW_H: f32 = 30.0;
/// `.mk-thead { height: 28px }`
pub const HEADER_H: f32 = 28.0;
/// Space between columns.
pub const COL_GAP: f32 = 12.0;
/// How much a dimmed row shows (a cut item, one that cannot be opened now).
pub const DIMMED: f32 = 0.45;
/// Names up to this many characters are written whole (the board's `midCut`).
pub const NAME_MAX: usize = 58;

/// How a row looks right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowLook {
    pub selected: bool,
    pub focused: bool,
    pub dimmed: bool,
    /// (position, set size over the whole list), 1-based.
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
    /// Where the title sits in its cell (sizes right, the PC glyph centred).
    pub align: Align,
    /// What a screen reader says when the title is short for something ("PC": "On this PC").
    pub spoken: Option<&'static str>,
}

impl Column {
    /// A start-aligned column whose title is what is read.
    pub const fn new(key: &'static str, title: &'static str, width: Width) -> Column {
        Column {
            key,
            title,
            width,
            align: Align::Start,
            spoken: None,
        }
    }

    pub const fn align(mut self, align: Align) -> Column {
        self.align = align;
        self
    }

    pub const fn spoken(mut self, words: &'static str) -> Column {
        self.spoken = Some(words);
        self
    }
}

/// A name longer than [`NAME_MAX`] characters, shortened in the middle: its first 32 characters,
/// an ellipsis, its last 23, so the start, the number and the extension stay. `None`: whole.
pub fn shortened(name: &str) -> Option<String> {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= NAME_MAX {
        return None;
    }
    // ceil(58 x 0.55) and floor(58 x 0.4)
    let (head, tail) = (32, 23);
    let mut out: String = chars[..head].iter().collect();
    out.push('\u{2026}');
    out.extend(&chars[chars.len() - tail..]);
    Some(out)
}

/// A kind's tile (`.mk-ftile`): 22 pt, the kind's outlined glyph on a neutral wash.
/// Keys `<key>-tile`, `<key>-icon`.
pub fn icon_tile(key: &str, icon: FileIcon, t: &Theme) -> El {
    tile_frame(key)
        .fill(t.tile)
        .child(super::file_icon::file_icon(format!("{key}-icon"), icon, 14.0, t))
}

/// The tile's empty frame (22 pt, rounded), keyed `<key>-tile`: for a thumbnail.
pub fn tile_frame(key: &str) -> El {
    el().key(format!("{key}-tile"))
        .size(22.0, 22.0)
        .radius(6.0)
        .clip_rounded(6.0)
        .center()
}

/// The one file row: `icon` in its tile, `name` (cut in the middle when long), then `cells`.
pub fn row(key: &str, label: &str, icon: FileIcon, name: &str, cells: Vec<El>, look: RowLook, t: &Theme) -> El {
    row_with(key, label, icon_tile(key, icon, t), name, Vec::new(), cells, look, t)
}

/// [`row`] with the caller's tile (a thumbnail) and marks after the name (a lock).
#[allow(clippy::too_many_arguments)]
pub fn row_with(
    key: &str,
    label: &str,
    tile: El,
    name: &str,
    marks: Vec<El>,
    cells: Vec<El>,
    look: RowLook,
    t: &Theme,
) -> El {
    let mut name_cell = hrow()
        .key("name-cell")
        .grow(1.0)
        .min_w(0.0)
        .center_y()
        .gap(8.0)
        .child(tile)
        .child(name_label(key, name, t));
    for mark in marks {
        name_cell = name_cell.child(mark);
    }

    let lit = look.selected || look.focused;
    let mut out = hrow()
        .key(key.to_string())
        .role("row")
        .label(label)
        .h(ROW_H)
        .w_full()
        .px(14.0)
        .gap(COL_GAP)
        .center_y()
        .radius(radius::CONTROL)
        .fill(if lit { t.selection_row } else { Color::TRANSPARENT })
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .focus_selected(lit)
        .child(name_cell);
    for cell in cells {
        out = out.child(cell);
    }
    if let Some((position, size)) = look.position {
        out = out.position_in_set(position, size);
    }
    if look.dimmed {
        out = out.opacity(DIMMED);
    }
    out
}

/// A row's name: shortened in the middle when long, the whole name under the pointer.
fn name_label(key: &str, name: &str, t: &Theme) -> El {
    let short = shortened(name);
    let label = kit::label(
        format!("{key}-name"),
        short.clone().unwrap_or_else(|| name.to_string()),
        type_::LIST_ITEM,
        t.text,
    )
    .truncate_middle()
    .min_w(0.0);
    match short {
        Some(_) => label.help(name),
        None => label.help_when_truncated(name),
    }
}

/// Column headers: one `columnheader` per column; `sorted` is (column, ascending) and draws the
/// arrow beside that column's title.
pub fn header(columns: &[Column], sorted: Option<(usize, bool)>, t: &Theme) -> El {
    header_with(columns, sorted, |_, cell| cell, t)
}

/// [`header`], with `each(column index, cell)` to make a cell sort on press.
pub fn header_with(columns: &[Column], sorted: Option<(usize, bool)>, each: impl Fn(usize, El) -> El, t: &Theme) -> El {
    let mut out = hrow()
        .key("column-headers")
        .role("row")
        .h(HEADER_H)
        .w_full()
        .mt(4.0)
        .px(20.0)
        .gap(COL_GAP)
        .center_y()
        .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
        .border_layout();
    for (i, c) in columns.iter().enumerate() {
        let mut cell = hrow()
            .key(format!("col-{}-cell", c.key))
            .role("columnheader")
            .label(c.spoken.unwrap_or(c.title))
            .center_y()
            .gap(3.0)
            .child(kit::label(
                format!("col-{}", c.key),
                c.title,
                type_::COLUMN_HEADER,
                t.text_tertiary,
            ));
        cell = match c.width {
            Width::Fixed(w) => cell.w(w),
            Width::Flex(g) => cell.grow(g).min_w(0.0),
        };
        cell = match c.align {
            Align::Start => cell,
            Align::Center => cell.justify_center(),
            Align::End => cell.justify_end(),
        };
        if let Some((col, ascending)) = sorted
            && col == i
        {
            let arrow = kit::glyph("sort-arrow", &icons::CHEVRON_DOWN, 11.0, t.text_tertiary);
            // The board has one arrow glyph; descending turns it over.
            let arrow = if ascending {
                arrow
            } else {
                arrow.rotate(std::f32::consts::PI)
            };
            cell = cell.sorted(Some(ascending)).child(arrow);
        }
        out = out.child(each(i, cell));
    }
    out
}

#[cfg(test)]
#[path = "file_row_tests.rs"]
mod tests;
