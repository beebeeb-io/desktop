//! The grid of a folder (`.mk-gridview`): one tile per item, its name under it (at most two
//! lines) and one line of figures under the name. A tile is plain until its thumbnail arrives
//! (DW-0119: "No spinners"); a folder or a document shows its glyph; a video carries its length.
//!
//! Keys: `grid` (the padded area), `rows` (the tiles' box: the grid's one Tab stop, as the list's),
//! `grid-<i>`, `grid-<i>-thumb`, `-icon`, `-badge`, `-badge-icon`, `-badge-text`, `-name`, `-sub`.

use crate::theme::tokens::Theme;
use rune::tok::Family;
use rune::{Align, Color, El, Icon, TextStyle, col, el, row};

use super::{file_list, global_icons, kit};

/// `.mk-gcell`: 128 x 150 (`grid-template-columns: repeat(auto-fill, 128px)`, `grid-auto-rows: 150px`).
pub const CELL_W: f32 = 128.0;
pub const CELL_H: f32 = 150.0;
/// `.mk-gridview`: `gap: 10px 14px`, `padding: 16px 18px`.
pub const ROW_GAP: f32 = 10.0;
pub const COL_GAP: f32 = 14.0;
pub const PAD_Y: f32 = 16.0;
pub const PAD_X: f32 = 18.0;
/// `.mk-gname`: 12/500 on a 15 line, centred, two lines at most.
pub const NAME: TextStyle = TextStyle::new(12.0, 500.0).lh(15.0);
/// `.mk-gsub`: 10.5/400 in the machine face.
pub const SUB: TextStyle = TextStyle::new(10.5, 400.0).family(Family::Mono).lh(13.5);
/// `.mk-gbadge`: 10/600 in the machine face.
pub const BADGE: TextStyle = TextStyle::new(10.0, 600.0).family(Family::Mono).lh(13.0);

/// What the tile shows.
#[derive(Clone, Copy)]
pub enum Thumb {
    /// A kind's glyph, `size` points across (a folder 44, a document 34).
    Glyph(&'static Icon, f32),
    /// A picture or a video whose thumbnail has not arrived: a plain tile.
    Plain,
    /// The thumbnail arrived.
    Picture,
}

/// One item of the grid.
pub struct Tile<'a> {
    pub name: &'a str,
    /// The line under the name: a size, or how many items a folder holds.
    pub sub: &'a str,
    pub thumb: Thumb,
    /// A video's length ("0:41").
    pub length: Option<&'a str>,
    pub selected: bool,
}

/// How many tiles stand side by side in a grid `width` points wide (padding included).
pub fn columns(width: f32) -> usize {
    let inner = width - 2.0 * PAD_X;
    (((inner + COL_GAP) / (CELL_W + COL_GAP)).floor() as usize).max(1)
}

/// One tile (`.mk-gcell`). The caller adds what a press does.
pub fn tile(i: usize, item: &Tile, t: &Theme) -> El {
    let k = format!("grid-{i}");
    let mut thumb = el()
        .key(format!("{k}-thumb"))
        .a11y_hidden()
        .size(104.0, 96.0)
        .radius(9.0)
        .clip_rounded(9.0)
        .center();
    thumb = match item.thumb {
        Thumb::Glyph(ic, size) => {
            thumb
                .fill(t.text.alpha(0.06))
                .child(kit::glyph(format!("{k}-icon"), ic, size, t.text_tertiary))
        }
        Thumb::Plain => thumb.fill(t.text.alpha(0.08)),
        Thumb::Picture => file_list::thumbnail(thumb),
    };
    if let Some(length) = item.length {
        thumb = thumb.child(
            row()
                .key(format!("{k}-badge"))
                .pin(None, None, Some(5.0), Some(5.0))
                .h(18.0)
                .px(5.0)
                .gap(3.0)
                .radius(6.0)
                .items_center()
                .fill(Color::BLACK.alpha(0.55))
                .child(kit::glyph(
                    format!("{k}-badge-icon"),
                    &global_icons::PLAY,
                    9.0,
                    Color::WHITE,
                ))
                .child(kit::label(format!("{k}-badge-text"), length, BADGE, Color::WHITE)),
        );
    }
    col()
        .key(k.clone())
        .size(CELL_W, CELL_H)
        .padding(6.0)
        .gap(6.0)
        .radius(10.0)
        .items_center()
        .fill(if item.selected {
            t.selection_row
        } else {
            Color::TRANSPARENT
        })
        .child(thumb)
        .child(
            kit::label(format!("{k}-name"), item.name, NAME, t.text)
                .max_lines(2)
                .w_full()
                .align(Align::Center),
        )
        .child(
            // `.mk-gsub { margin-top: -4px }`: 2 under the name.
            kit::label(
                format!("{k}-sub"),
                item.sub,
                SUB,
                if item.selected {
                    t.text_secondary
                } else {
                    t.text_tertiary
                },
            )
            .mt(-4.0),
        )
}

/// The grid: `tiles` in rows of as many as fit, scrolling when they do not fit the height.
/// `rows` is the box the caller makes focusable (the grid's one Tab stop).
pub fn grid(rows: El, tiles: Vec<El>) -> El {
    // A column, so the tiles' box is as high as its rows and no higher (`align-content: start`):
    // stretched over the grid's height, wrapped rows would spread apart.
    col()
        .key("grid")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .h_full()
        .py(PAD_Y)
        .px(PAD_X)
        .scroll(rune::ScrollSpec { stick_bottom: false })
        .child(
            rows.key("rows")
                .w_full()
                .flex_wrap()
                .items_start()
                .gaps(ROW_GAP, COL_GAP)
                .children(tiles),
        )
}
