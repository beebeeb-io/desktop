//! The one file tile: an item's picture (its kind's icon, or its thumbnail) at one of three sizes.
//!
//! - `Small` (28 pt): the square alone, beside a name the caller draws (uploads, history, tray, backups).
//! - `Medium`: the grid's cell (`.mk-gcell`, 128 x 150): a 104 x 96 picture and the name under it.
//! - `Large` (40 pt): the square at the top of a dialog (share, facts).
//!
//! Every size is labelled with the name. The grid's cell is an `option` of its list (selection and
//! position in set from `RowLook`); the squares are an `image`.
//!
//! Keys: `<key>` (the tile), `<key>-thumb` (Medium's picture), `<key>-icon`, `<key>-name` (Medium).
//! A badge is the caller's element, with its own keys; the tile pins it to the picture's corner.

use super::file_row::RowLook;
use super::grid_tiles::{self, Thumb};
use super::{file_icon, file_list, kit};
use crate::theme::file_icons::FileIcon;
use crate::theme::tokens::Theme;
use rune::{Align, Color, El, col, el};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileSize {
    Small,
    Medium,
    Large,
}

impl TileSize {
    /// The square's side (Small, Large), or the picture's width (Medium).
    pub fn side(self) -> f32 {
        match self {
            TileSize::Small => 28.0,
            TileSize::Medium => 104.0,
            TileSize::Large => 40.0,
        }
    }

    /// The kind's icon across: a document's in the grid, about 0.6 of a square.
    pub fn glyph(self) -> f32 {
        match self {
            TileSize::Small => 16.0,
            TileSize::Medium => 34.0,
            TileSize::Large => 25.0,
        }
    }
}

pub enum TilePicture<'a> {
    Icon(FileIcon),
    Thumb(&'a Thumb),
}

/// One tile. The caller adds what a press does.
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
    tile_with(key, label, picture, None, name, None, badge, look, size, t)
}

/// `tile`, with the arrived thumbnail's pixels and Medium's line under the name (the grid's figures).
#[allow(clippy::too_many_arguments)]
pub(super) fn tile_with(
    key: &str,
    label: &str,
    picture: TilePicture,
    pixels: Option<&rune::Pixels>,
    name: &str,
    sub: Option<El>,
    badge: Option<El>,
    look: RowLook,
    size: TileSize,
    t: &Theme,
) -> El {
    let mut out = match size {
        TileSize::Medium => {
            let mut thumb = el()
                .key(format!("{key}-thumb"))
                .a11y_hidden()
                .size(104.0, 96.0)
                .radius(9.0)
                .clip_rounded(9.0)
                .center();
            thumb = fill(thumb, key, &picture, pixels, size, 0.06, t);
            if let Some(b) = badge {
                thumb = thumb.child(b.pin(None, None, Some(5.0), Some(5.0)));
            }
            let mut cell = col()
                .key(key.to_string())
                .size(grid_tiles::CELL_W, grid_tiles::CELL_H)
                .padding(6.0)
                .gap(6.0)
                .radius(10.0)
                .items_center()
                .fill(if look.selected {
                    t.selection_row
                } else {
                    Color::TRANSPARENT
                })
                .child(thumb)
                .child(
                    kit::label(format!("{key}-name"), name, grid_tiles::NAME, t.text)
                        .max_lines(2)
                        .w_full()
                        .align(Align::Center),
                );
            if let Some(sub) = sub {
                cell = cell.child(sub);
            }
            cell.role("option").focus_selected(look.selected)
        }
        TileSize::Small | TileSize::Large => {
            let side = size.side();
            let corner = if size == TileSize::Small {
                8.0
            } else {
                (side * 0.24).round()
            };
            let mut square = el()
                .key(key.to_string())
                .size(side, side)
                .radius(corner)
                .clip_rounded(corner)
                .center();
            square = fill(square, key, &picture, pixels, size, 0.07, t);
            if let Some(b) = badge {
                square = square.child(b.pin(None, None, Some(2.0), Some(2.0)));
            }
            square.role("image")
        }
    };
    out = out.label(label);
    if let Some((position, count)) = look.position {
        out = out.position_in_set(position + 1, count);
    }
    if look.dimmed {
        out = out.opacity(0.5);
    }
    out
}

/// The picture's fill: the kind's icon on a faint tile, a plain tile until a thumbnail arrives, or the thumbnail.
#[allow(clippy::too_many_arguments)]
fn fill(
    base: El,
    key: &str,
    picture: &TilePicture,
    pixels: Option<&rune::Pixels>,
    size: TileSize,
    faint: f32,
    t: &Theme,
) -> El {
    let icon = |base: El, fi: FileIcon, glyph: f32| {
        base.fill(t.text.alpha(faint))
            .child(file_icon::file_icon(format!("{key}-icon"), fi, glyph, t))
    };
    match picture {
        TilePicture::Icon(fi) => icon(base, *fi, size.glyph()),
        // The grid's own glyph size (a folder 44, a document 34); a square keeps its own.
        TilePicture::Thumb(Thumb::Type(fi, glyph)) => {
            icon(base, *fi, if size == TileSize::Medium { *glyph } else { size.glyph() })
        }
        TilePicture::Thumb(Thumb::Plain) => base.fill(t.text.alpha(0.08)),
        TilePicture::Thumb(Thumb::Picture) => match pixels {
            // The real picture, covering the tile (the toolkit crops what does not fit).
            Some(p) => base.pixels_fit(p, rune::SurfaceFit::Fill),
            None => file_list::thumbnail(base),
        },
        TilePicture::Thumb(Thumb::Sample(n)) => grid_tiles::sample(base, *n),
    }
}

#[cfg(test)]
#[path = "file_tile_tests.rs"]
mod tests;
