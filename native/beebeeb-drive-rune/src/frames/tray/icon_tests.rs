use crate::theme::file_icon_colors::tint_color;
use crate::theme::file_icons::Tint;
use crate::theme::tokens;
use crate::ui::kit;

use super::super::source::Tile;
use super::tile;

/// A recent image without a thumbnail draws the registry's image icon.
#[test]
fn a_tray_tile_draws_its_icon_from_the_registry() {
    let t = tokens::theme(false);
    kit::record();
    let _ = tile("row-0", "photo.heic", Tile::Image, &t);
    let notes = kit::take();
    let n = notes.get("row-0-icon").expect("icon note");
    assert!(n.icon.is_some());
    assert!(n.color == Some(tint_color(Tint::Image, false)));
}
