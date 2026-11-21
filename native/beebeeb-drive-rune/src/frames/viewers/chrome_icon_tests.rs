//! The viewer's title draws the file's icon in its tint, left of the name.

use rune::{Env, Runtime};

use super::super::source::{FixtureViewerSource, board};
use super::super::open_from;
use crate::theme::file_icon_colors::tint_color;
use crate::theme::file_icons::Tint;
use crate::ui::kit;
use crate::{State, init, view};

#[test]
fn the_title_of_a_deck_draws_the_powerpoint_icon_left_of_the_name() {
    let mut state = init();
    assert!(open_from(&mut state, Box::new(FixtureViewerSource::new()), board::DECK));
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), Env { system_dark: false, scale: 2.0 });
    kit::record();
    rt.frame(0.0);
    rt.frame(0.0);
    let notes = kit::take();
    let note = notes.get("viewer-title-icon").expect("no viewer-title-icon");
    assert_eq!(note.color, Some(tint_color(Tint::PowerPoint, false)));
    let at = |key: &str| {
        rt.elements().into_iter().find(|e| e.key.as_deref() == Some(key) && !e.exiting).unwrap_or_else(|| panic!("no {key}"))
    };
    assert!(at("viewer-title-icon").rect[0] < at("crumb-name").rect[0]);
}
