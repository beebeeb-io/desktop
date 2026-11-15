//! Tests of the Wave B scenes of Files (DW-0403, DW-0412, DW-0420, DW-1103, DW-1105): each is
//! registered, draws its key element through the real view code, and what it shows answers to a press.
//! Helpers are `tests.rs`'s.

use super::keeping::Download;
use super::scenes_waveb::SCENES;
use super::source::FilesFixture;
use super::tests::*;
use crate::State;
use crate::init;
use rune::Runtime;

fn drawn(frame: &str) -> Runtime<State> {
    let mut state = init();
    let run = SCENES
        .iter()
        .find(|(id, _)| *id == frame)
        .unwrap_or_else(|| panic!("{frame} is a scene"))
        .1;
    run(&mut state);
    let mut rt = runtime_of(state);
    settle(&mut rt);
    rt
}

fn text(rt: &Runtime<State>, key: &str) -> Option<String> {
    find(rt, key).and_then(|e| e.text)
}

#[test]
fn the_five_scenes_are_registered_once_in_the_shell_s_table() {
    let ids: Vec<&str> = SCENES.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, ["DW-0403", "DW-0412", "DW-0420", "DW-1103", "DW-1105"]);
    // The table is joined to every other group's in `frames::scenes` by one line.
    let joined = include_str!("../mod.rs").matches("files::scenes_waveb::SCENES").count();
    assert_eq!(joined, 1);
}

#[test]
fn dw_0403_counts_the_two_rows_in_its_bar_and_its_panel_and_done_clears_them() {
    let mut rt = drawn("DW-0403");
    assert_eq!(text(&rt, "selbar-count").as_deref(), Some("2 selected"));
    assert_eq!(text(&rt, "detail-name").as_deref(), Some("2 items"));
    assert_eq!(
        text(&rt, "detail-selected-value").as_deref(),
        Some("Budget 2026.xlsx, Contract draft.pdf")
    );
    assert_eq!(text(&rt, "detail-on-this-pc-value").as_deref(), Some("1 of 2"));
    click(&mut rt, "selbar-done-label");
    assert!(find(&rt, "selbar").is_none(), "no bar once nothing is selected");
    assert!(rt.state.selected_rows().is_empty());
}

#[test]
fn one_row_selected_has_no_bar() {
    let mut state = init();
    state.select_only(6);
    let mut rt = runtime_of(state);
    settle(&mut rt);
    assert!(find(&rt, "selbar").is_none());
    assert_eq!(text(&rt, "detail-name").as_deref(), Some("Contract draft.pdf"));
}

#[test]
fn dw_0412_shows_how_far_the_copy_is_in_the_row_the_panel_and_the_status_line() {
    let rt = drawn("DW-0412");
    assert_eq!(
        text(&rt, "row-6-download-words").as_deref(),
        Some("Downloading, 0.7 of 1.2 MB")
    );
    assert_eq!(
        text(&rt, "offline-detail").as_deref(),
        Some("Downloading \u{b7} 0.7 of 1.2 MB")
    );
    assert_eq!(text(&rt, "status-text").as_deref(), Some("Syncing"));
    assert!(
        find(&rt, "free-up-btn").is_none(),
        "a copy on its way is not on this PC to free up"
    );
    assert!(find(&rt, "toast-text").is_some(), "the message is up");
}

#[test]
fn a_source_that_cannot_tell_how_far_shows_no_bar() {
    let mut state = init();
    state.select_only(6);
    let mut rt = runtime_of(state);
    settle(&mut rt);
    assert!(rt.state.download_of("contract").is_none());
    assert!(find(&rt, "row-6-download-words").is_none());
    assert!(find(&rt, "row-6-bar").is_none());
}

/// A copy that comes down for another reason (a player reading it, a conflict copy) shows its bar and does not claim a
/// keep: the panel's Available offline stays off.
#[test]
fn a_copy_that_is_not_kept_shows_its_bar_and_does_not_turn_available_offline_on() {
    let mut state = init();
    state.select_only(6);
    let mut fixture = FilesFixture::default();
    let total = state.listing.items[6].size.unwrap_or(0).max(0) as u64;
    fixture.more.downloads = vec![Download {
        id: state.listing.items[6].id.clone(),
        done: total / 2,
        total,
        kept: false,
    }];
    state.actions.source = Box::new(fixture);
    let mut rt = runtime_of(state);
    settle(&mut rt);
    assert!(find(&rt, "row-6-download-words").is_some(), "the bar is drawn");
    let kept_note = text(&rt, "offline-detail");
    assert!(
        kept_note.as_deref().is_none_or(|t| !t.contains(" of ")),
        "no download figures under Available offline: {kept_note:?}"
    );
}

#[test]
fn a_download_reads_its_figures_in_the_unit_of_the_whole() {
    let d = Download {
        id: "x".into(),
        done: 734_003,
        total: 1_258_291,
        kept: true,
    };
    assert_eq!(d.figures(), "0.7 of 1.2 MB");
    assert!((d.fraction() - 0.583).abs() < 0.001);
    let small = Download {
        id: "x".into(),
        done: 100 * 1024,
        total: 220 * 1024,
        kept: true,
    };
    assert_eq!(small.figures(), "100 of 220 KB");
}

#[test]
fn dw_0420_is_lisbon_trip_one_level_down() {
    let rt = drawn("DW-0420");
    assert_eq!(text(&rt, "crumb-here").as_deref(), Some("Lisbon trip"));
    assert_eq!(text(&rt, "item-count").as_deref(), Some("7 items"));
    assert!(rt.state.can_go_back());
    assert!(find(&rt, "row-5-icon").is_some(), "the video has its mark");
}

#[test]
fn dw_1103_opens_the_list_menu_with_sort_by_beside_it() {
    let mut rt = drawn("DW-1103");
    assert_eq!(text(&rt, "menu-item-0-label").as_deref(), Some("New folder"));
    assert_eq!(text(&rt, "menu-1-item-0-label").as_deref(), Some("Name"));
    assert_eq!(text(&rt, "menu-1-item-5-label").as_deref(), Some("Z to A"));
    // as Grid switches the folder's view and closes both menus.
    click(&mut rt, "menu-item-3");
    assert!(crate::frames::global::grid::on(&rt.state));
    assert!(rt.state.actions.menu.is_none());
}

#[test]
fn a_right_click_on_the_empty_list_opens_the_menu_and_a_choice_of_sort_by_sorts() {
    let mut state = init();
    state.show_detail_pane = false;
    let mut rt = runtime_of(state);
    settle(&mut rt);
    // A real right-click, on the list's empty space under the last row.
    rt.secondary_down(500.0, 600.0);
    rt.secondary_up(500.0, 600.0);
    settle(&mut rt);
    assert!(find(&rt, "menu-item-0").is_some(), "the menu opened");
    assert!(
        find(&rt, "menu-1").is_none(),
        "Sort by is not open until it is highlighted"
    );
    // Sort by is picked: its choices open and the menu stays.
    click(&mut rt, "menu-item-4");
    assert!(rt.state.actions.menu.is_some());
    click(&mut rt, "menu-1-item-2");
    assert_eq!(rt.state.actions.sort.by, super::SortBy::Size);
    assert!(rt.state.actions.menu.is_none());
}

#[test]
fn dw_1105_says_what_search_cannot_do_and_opens_no_panel() {
    let rt = drawn("DW-1105");
    assert_eq!(
        text(&rt, "search-note").as_deref(),
        Some(crate::frames::search::PLAIN_NOTE)
    );
    assert!(find(&rt, "detail-pane").is_none());
    assert_eq!(text(&rt, "item-count").as_deref(), Some("3 results"));
}
