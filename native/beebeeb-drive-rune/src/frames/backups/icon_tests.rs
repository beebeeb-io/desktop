use crate::theme::file_icon_colors::tint_color;
use crate::theme::file_icons::Tint;
use crate::theme::tokens;
use crate::ui::kit;

use super::file_tile;

/// A file that did not back up draws the registry's icon for its name.
#[test]
fn a_backups_file_row_draws_its_icon_from_the_registry() {
    let t = tokens::theme(false);
    kit::record();
    let _ = file_tile(0, "report.pdf", &t);
    let notes = kit::take();
    let n = notes.get("tile-0-icon").expect("icon note");
    assert!(n.icon.is_some());
    assert!(n.color == Some(tint_color(Tint::Pdf, false)));
}
