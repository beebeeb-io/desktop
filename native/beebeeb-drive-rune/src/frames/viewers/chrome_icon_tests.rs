//! The viewer's title draws the file's icon in its tint, left of the name. The title is built directly: a deck
//! opens in the office window, which draws its own bar.

use super::super::source::{FixtureViewerSource, ViewerSource, board};
use super::{Head, title};
use crate::theme::file_icon_colors::tint_color;
use crate::theme::file_icons::Tint;
use crate::theme::tokens::theme;
use crate::ui::kit;

#[test]
fn the_title_of_a_deck_draws_the_powerpoint_icon_left_of_the_name() {
    let file = FixtureViewerSource::new().open(board::DECK).expect("the board's deck");
    kit::record();
    let el = title(&file, &Head::default(), &theme(false));
    let notes = kit::take();
    let note = notes.get("viewer-title-icon").expect("no viewer-title-icon");
    assert_eq!(note.color, Some(tint_color(Tint::PowerPoint, false)));
    assert_eq!(el.children[0].key.as_deref(), Some("viewer-title-icon"));
    assert_eq!(el.children[1].key.as_deref(), Some("file-info"));
}
