//! Tests: the This PC frame's icons.

use crate::theme::file_icon_colors::tint_color;
use crate::theme::file_icons::Tint;
use crate::theme::tokens;
use crate::ui::kit;

use super::super::source::{Change, Kind, Origin, What};
use super::change_row;
use chrono::NaiveDate;

/// A clip in the history draws the registry's video icon in the video tint.
#[test]
fn a_history_row_draws_its_icon_from_the_registry() {
    let t = tokens::theme(false);
    let change = Change {
        id: "1".into(),
        name: "clip.mov".into(),
        kind: Kind::Video,
        what: What::Uploaded { place: "Files".into() },
        origin: Origin::ThisPc,
        date: NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
        time: "09:39".into(),
    };
    kit::record();
    let _ = change_row(0, &change, 0, 1, &t);
    let notes = kit::take();
    let n = notes.get("change-0-tile-icon").expect("icon note");
    assert!(n.icon.is_some());
    assert!(n.color == Some(tint_color(Tint::Video, false)));
}
