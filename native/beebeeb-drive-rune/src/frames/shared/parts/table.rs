//! A list with named columns (`.mk-listwrap` > `.mk-thead` + `.mk-rows`): Name, a date, a
//! size, one column in words or with a person, and the PC state where the section has it.

use super::style;
use crate::theme::icons;
use crate::theme::tokens::{Theme, radius, type_};
use crate::ui::file_list::{PcState, pc_glyph};
use crate::ui::kit;
use rune::{Align, Color, El, Icon, col, el, row};

const GAP: f32 = 12.0;
const DATE_W: f32 = 132.0;
const SIZE_W: f32 = 84.0;

/// What a row's tile shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    Document,
    Sheet,
    /// A photo: its thumbnail (the board's sample gradient until the engine hands over pixels).
    Photo,
    /// A photo with the board's other sample (`.mk-ph5`, a harbour at dusk).
    PhotoDusk,
}

impl Kind {
    pub fn icon(self) -> &'static Icon {
        match self {
            Kind::Folder => &icons::FILES,
            Kind::Sheet => &icons::DOC_SPREADSHEET,
            Kind::Document | Kind::Photo | Kind::PhotoDusk => &icons::DOC_GENERIC,
        }
    }
}

/// The fourth column's cell.
#[derive(Clone, Debug, PartialEq)]
pub enum Extra {
    Words(String),
    Person { initials: String, name: String, hue: char },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub date: String,
    pub size: String,
    pub extra: Extra,
    pub pc: Option<PcState>,
}

/// The column set of one section.
#[derive(Clone, Copy)]
pub struct Columns {
    /// Header words: name, date, size, extra, PC ("" where the section has no PC column).
    pub names: [&'static str; 5],
    pub extra_w: f32,
    pub pc_w: f32,
}

/// The board's sample photo for a list tile (`.mk-ph4`).
fn photo(tile: El) -> El {
    let stop = |r: u8, g: u8, b: u8| rune::color::hex(&format!("#{r:02x}{g:02x}{b:02x}")).unwrap_or(Color::BLACK);
    tile.linear_gradient(
        200.0_f32.to_radians(),
        [
            (0.0, stop(199, 214, 176)),
            (0.6, stop(108, 141, 98)),
            (1.0, stop(47, 70, 52)),
        ],
    )
}

/// A tile `d` points across (`.mk-ftile`). Keys: `<key>-tile` and `<icon_key>`.
pub fn tile(key: &str, icon_key: String, kind: Kind, d: f32, r: f32, glyph: f32, t: &Theme) -> El {
    let tile = el()
        .key(format!("{key}-tile"))
        .size(d, d)
        .radius(r)
        .clip_rounded(r)
        .center();
    match kind {
        Kind::Photo => return photo(tile),
        Kind::PhotoDusk => return crate::ui::file_list::thumbnail(tile),
        _ => {}
    }
    tile.fill(t.tile)
        .child(kit::glyph(icon_key, kind.icon(), glyph, t.text_secondary))
}

/// The column headers (`.mk-thead`); the date column carries the sort arrow (newest first).
pub fn header(c: &Columns, t: &Theme) -> El {
    let head = |key: &'static str, words: &'static str| kit::label(key, words, type_::COLUMN_HEADER, t.text_tertiary);
    let mut pc = row().key("col-pc-cell").role("columnheader").w(c.pc_w).center();
    if !c.names[4].is_empty() {
        pc = pc.label("On this PC").child(head("col-pc", c.names[4]));
    }
    row()
        .key("column-headers")
        .role("row")
        .h(28.0)
        .w_full()
        .mt(4.0)
        .px(20.0)
        .gap(GAP)
        .center_y()
        .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
        .border_layout()
        .child(
            row()
                .key("col-name-cell")
                .role("columnheader")
                .grow(1.0)
                .min_w(0.0)
                .center_y()
                .child(head("col-name", c.names[0])),
        )
        .child(
            row()
                .key("col-date-cell")
                .role("columnheader")
                .sorted(Some(false))
                .w(DATE_W)
                .center_y()
                .gap(3.0)
                .child(head("col-date", c.names[1]))
                .child(kit::glyph("sort-arrow", &icons::CHEVRON_DOWN, 11.0, t.text_tertiary)),
        )
        .child(
            row()
                .key("col-size-cell")
                .role("columnheader")
                .w(SIZE_W)
                .center_y()
                .justify_end()
                .child(head("col-size", c.names[2])),
        )
        .child(
            row()
                .key("col-shared-cell")
                .role("columnheader")
                .w(c.extra_w)
                .center_y()
                .child(head("col-shared", c.names[3])),
        )
        .child(pc)
}

/// One row (`.mk-row`). Keys: `row-<i>` and `row-<i>-tile|icon|name|date|size|dash|from-avatar|from|pc`.
pub fn table_row(i: usize, r: &Row, selected: bool, c: &Columns, t: &Theme) -> El {
    let k = format!("row-{i}");
    let meta = if selected { t.text_secondary } else { t.text_tertiary };
    let name = row()
        .key("name-cell")
        .grow(1.0)
        .min_w(0.0)
        .center_y()
        .gap(8.0)
        .child(tile(&k, format!("{k}-icon"), r.kind, 22.0, 6.0, 14.0, t))
        .child(
            kit::label(format!("{k}-name"), r.name.clone(), type_::LIST_ITEM, t.text)
                .truncate()
                .min_w(0.0),
        );
    let extra = match &r.extra {
        Extra::Words(words) => row().key("extra-cell").w(c.extra_w).min_w(0.0).center_y().child(
            kit::label(format!("{k}-dash"), words.clone(), style::CELL, meta)
                .truncate()
                .min_w(0.0),
        ),
        Extra::Person { initials, name, hue } => row()
            .key("extra-cell")
            .w(c.extra_w)
            .min_w(0.0)
            .center_y()
            .gap(6.0)
            .child(kit::avatar(
                format!("{k}-from-avatar"),
                initials,
                20.0,
                super::hue(*hue),
                t.bg,
                t,
            ))
            .child(
                kit::label(format!("{k}-from"), name.clone(), style::CELL, meta)
                    .truncate()
                    .min_w(0.0),
            ),
    };
    let mut pc = el().key("pc-cell").w(c.pc_w).center();
    if let Some(state) = r.pc {
        pc = pc.child(pc_glyph(format!("{k}-pc"), state, t.bg, t));
    }
    row()
        .key(k.clone())
        .role("listitem")
        .label(&r.name)
        .h(30.0)
        .w_full()
        .px(14.0)
        .gap(GAP)
        .center_y()
        .radius(radius::CONTROL)
        .fill(if selected { t.selection_row } else { Color::TRANSPARENT })
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .focus_selected(selected)
        .child(name)
        .child(
            kit::label(format!("{k}-date"), r.date.clone(), style::CELL_MONO, meta)
                .w(DATE_W)
                .truncate(),
        )
        .child(
            kit::label(format!("{k}-size"), r.size.clone(), style::CELL_MONO, meta)
                .w(SIZE_W)
                .align(Align::End),
        )
        .child(extra)
        .child(pc)
}

/// The rows' well (`.mk-rows`); the caller adds `table_row`s with their handlers.
pub fn rows(name: &str, rows: Vec<El>) -> El {
    col()
        .key("rows")
        .role("list")
        .label(name)
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .py(4.0)
        .px(6.0)
        .scroll(rune::ScrollSpec { stick_bottom: false })
        .children(rows)
}
