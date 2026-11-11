//! Trash: the list, the empty state, selection and the intents.

use crate::frames::Intent;
use crate::frames::shared::tests::{click, info, press, runtime};
use rune::Key;

#[test]
fn trash_lists_six_rows_of_twelve_items_and_counts_them_in_the_sidebar() {
    let rt = runtime(super::scenes::dw_0747);
    assert!(rt.find("row-5").is_some() && rt.find("row-6").is_none());
    assert_eq!(
        info(&rt, "item-count").text.as_deref(),
        Some("12 items \u{00B7} 240 MB, counts toward your storage")
    );
    assert_eq!(info(&rt, "Trash-badge").text.as_deref(), Some("12"));
    let new = info(&rt, "new-btn");
    assert!(!new.focusable, "New is not available in Trash");
}

#[test]
fn empty_trash_asks_for_the_warning_and_delete_does_too() {
    let mut rt = runtime(super::scenes::dw_0747);
    let button = info(&rt, "btn-empty-trash");
    assert_eq!(
        (Some(button.role), button.label.as_deref()),
        (Some("button"), Some("Empty Trash\u{2026}"))
    );
    click(&mut rt, "btn-empty-trash");
    assert_eq!(rt.state.frames.intents, vec![Intent::Frame("DW-0749")]);
    // The list's keys, with focus nowhere (a focused button keeps the arrows for itself).
    let mut rt = runtime(super::scenes::dw_0747);
    rt.state.frames.intents.push(Intent::Frame("DW-0749"));
    press(&mut rt, Key::Delete);
    assert_eq!(
        rt.state.frames.intents.len(),
        1,
        "Delete with nothing selected does nothing"
    );
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Down);
    assert_eq!(rt.state.frames.trash.selected, Some(1));
    press(&mut rt, Key::Enter);
    assert_eq!(rt.state.frames.intents.len(), 1, "nothing in Trash opens");
    press(&mut rt, Key::Delete);
    assert_eq!(rt.state.frames.intents.len(), 2);
}

#[test]
fn restoring_takes_the_row_out_and_keeps_a_selection() {
    let mut rt = runtime(super::scenes::dw_0747);
    click(&mut rt, "row-5");
    assert_eq!(rt.state.frames.trash.selected, Some(5));
    assert!(rt.state.frames.trash.restore_selected());
    crate::frames::shared::tests::settle(&mut rt);
    assert!(rt.find("row-5").is_none() && rt.find("row-4").is_some());
    assert_eq!(rt.state.frames.trash.selected, Some(4));
    assert_eq!(rt.state.frames.trash.source.totals().items, 11);
}

#[test]
fn an_empty_trash_hides_empty_trash_and_its_sidebar_count() {
    let rt = runtime(super::scenes::dw_0117);
    assert_eq!(info(&rt, "empty-title").text.as_deref(), Some("Trash is empty"));
    assert!(rt.find("btn-empty-trash").is_none() && rt.find("Trash-badge").is_none() && rt.find("rows").is_none());
}

#[test]
fn the_trash_menu_restores_asks_for_details_and_for_the_warning() {
    let mut rt = runtime(super::scenes::dw_0748);
    let restore = info(&rt, "mi-restore");
    assert_eq!(
        (Some(restore.role), restore.label.as_deref()),
        (Some("menuitem"), Some("Restore"))
    );
    press(&mut rt, Key::Enter);
    assert!(rt.find("menu").is_none(), "the menu closes");
    assert_eq!(
        rt.state.frames.trash.source.totals().items,
        11,
        "Contract old.pdf is back"
    );
    assert_eq!(info(&rt, "row-1-name").text.as_deref(), Some("Budget 2025.xlsx"));

    let mut rt = runtime(super::scenes::dw_0748);
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Enter);
    assert_eq!(rt.state.frames.intents, vec![Intent::Frame("DW-0749")]);
    let mut rt = runtime(super::scenes::dw_0748);
    click(&mut rt, "mi-details");
    assert_eq!(rt.state.frames.intents, vec![Intent::Frame("DW-0411")]);
    let mut rt = runtime(super::scenes::dw_0748);
    press(&mut rt, Key::Escape);
    assert!(rt.find("menu").is_none() && rt.state.frames.intents.is_empty());
    assert_eq!(rt.state.frames.trash.source.totals().items, 12);
}

#[test]
fn emptying_shows_how_far_it_is_and_empty_trash_waits() {
    let mut rt = runtime(super::scenes::dw_0749b);
    assert_eq!(info(&rt, "item-count").text.as_deref(), Some("7 items left"));
    assert_eq!(info(&rt, "Trash-badge").text.as_deref(), Some("7"));
    let bar = info(&rt, "banner-bar");
    assert_eq!(
        (Some(bar.role), bar.label.as_deref()),
        (Some("progressbar"), Some("42 %"))
    );
    let (track, fill) = (bar.rect[2], info(&rt, "banner-bar-fill").rect[2]);
    assert!(
        (fill / track - 0.42).abs() < 0.001,
        "the bar is at 42 %: {fill} of {track}"
    );
    assert!(rt.find("row-2").is_some() && rt.find("row-3").is_none());
    let button = info(&rt, "btn-empty-trash");
    assert!(!button.focusable, "Empty Trash… waits while Trash empties");
    click(&mut rt, "btn-empty-trash");
    assert!(rt.state.frames.intents.is_empty());
}
