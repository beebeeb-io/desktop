//! Tests of the workbook's scroll (`sheets_scroll`) and its tabs behind "more" (`sheets_tabs`): the wheel, a sideways
//! delta (Shift+wheel as the platform delivers it) and the scroll bars move `top_left` by the quantum; the keys move the
//! viewports with it (one truth); a selection is never changed by a scroll; tabs past the bar's width go behind "more".

use rune::{Key, KeyEvent, Mods, Runtime, ScrollBars};

use super::super::{SheetView, sheets_fixture};
use super::*;
use crate::frames::office::docs::live_tests::{find, live_runtime};

/// A big workbook: 400 rows by 30 columns, B2:B9 selected.
fn big() -> SheetView {
    let mut view = sheets_fixture::board();
    view.cells.clear();
    view.col_widths = vec![80.0; 30];
    view.row_heights = vec![24.0; 400];
    view.extent = (400, 30);
    view
}

fn sheet(rt: &mut Runtime<State>) -> SheetView {
    match &viewers::ui(&mut rt.state).expect("a file").file.body {
        Body::Sheet(v) => v.clone(),
        _ => panic!("not a sheet"),
    }
}

fn rect(rt: &Runtime<State>, key: &str) -> [f32; 4] {
    find(rt, key).unwrap_or_else(|| panic!("{key} is drawn")).rect
}

/// A few frames with time going by (a scroll bar and a menu fade in; at no time they stay clear and take no press).
fn settle(rt: &mut Runtime<State>) {
    for _ in 0..20 {
        rt.frame(1.0 / 60.0);
    }
}

/// The middle of the grid.
fn middle(rt: &Runtime<State>) -> (f32, f32) {
    let g = rect(rt, "sh-grid");
    (g[0] + g[2] / 2.0, g[1] + g[3] / 2.0)
}

/// How far each viewport is scrolled, from where its spacer stands (the instrument the state is checked against).
fn offsets(rt: &Runtime<State>) -> (f32, f32) {
    (
        rect(rt, DOWN)[1] - rect(rt, "sh-scroll-v-body")[1],
        rect(rt, ACROSS)[0] - rect(rt, "sh-scroll-x-body")[0],
    )
}

fn press(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    settle(rt);
}

#[test]
fn office_sheets_the_wheel_and_shift_wheel_move_the_top_left() {
    let (mut rt, _log) = live_runtime(Body::Sheet(big()), 1.0, None);
    let selection = sheet(&mut rt).selection;
    let (x, y) = middle(&rt);
    rt.wheel(x, y, -100.0);
    settle(&mut rt);
    assert_eq!(
        sheet(&mut rt).top_left,
        Cell { row: 5, col: 0 },
        "100 points down: 5 rows of {ROW_Q}"
    );
    // Shift+wheel: the platform layer turns a Shift'd wheel sideways before the toolkit sees it.
    rt.wheel_xy(x, y, -128.0, 0.0);
    settle(&mut rt);
    let view = sheet(&mut rt);
    assert_eq!(
        view.top_left,
        Cell { row: 5, col: 2 },
        "128 points across: 2 columns of {COL_Q}"
    );
    assert_eq!(view.selection, selection, "a scroll never changes the selection");
    assert!(
        find(&rt, "sh-line-5").is_some(),
        "the grid is drawn from the new top row"
    );
}

#[test]
fn office_sheets_keys_move_the_viewports_too() {
    let (mut rt, _log) = live_runtime(Body::Sheet(big()), 1.0, None);
    rt.on_key = Some(crate::shell::host::on_key);
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Right);
    assert_eq!(sheet(&mut rt).top_left, Cell { row: 1, col: 1 });
    let (down, across) = offsets(&rt);
    assert!(
        (down - ROW_Q).abs() < 0.6 && (across - COL_Q).abs() < 0.6,
        "{down}, {across}"
    );
    // The next wheel turn starts where the keys left it.
    let (x, y) = middle(&rt);
    rt.wheel(x, y, -100.0);
    settle(&mut rt);
    assert_eq!(sheet(&mut rt).top_left.row, 6);
    // End reaches the last row, and the viewport can stand there (it does not snap back).
    press(&mut rt, Key::End);
    settle(&mut rt);
    assert_eq!(sheet(&mut rt).top_left.row, 399);
    assert!((offsets(&rt).0 - 399.0 * ROW_Q).abs() < 0.6, "{}", offsets(&rt).0);
}

#[test]
fn office_sheets_the_scroll_bars_move_the_top_left() {
    let (mut rt, _log) = live_runtime(Body::Sheet(big()), 1.0, None);
    rt.set_system_scroll_bars(ScrollBars::Always);
    rt.update(|_, _| {});
    settle(&mut rt);
    let thumbs: Vec<[f32; 4]> = rt
        .elements()
        .into_iter()
        .filter(|e| e.key.as_deref() == Some("thumb") && !e.exiting)
        .map(|e| e.rect)
        .collect();
    let tall = thumbs
        .iter()
        .find(|r| r[3] > r[2])
        .copied()
        .expect("the bar down the grid");
    let wide = thumbs.iter().find(|r| r[2] > r[3]).copied().expect("the bar across it");
    let drag = |rt: &mut Runtime<State>, from: (f32, f32), by: (f32, f32)| {
        rt.pointer_move(from.0, from.1);
        rt.pointer_down(from.0, from.1);
        rt.pointer_move(from.0 + by.0 / 2.0, from.1 + by.1 / 2.0);
        rt.frame(1.0 / 60.0);
        rt.pointer_move(from.0 + by.0, from.1 + by.1);
        rt.pointer_up(from.0 + by.0, from.1 + by.1);
        settle(rt);
    };
    drag(&mut rt, (tall[0] + tall[2] / 2.0, tall[1] + tall[3] / 2.0), (0.0, 60.0));
    let (down, _) = offsets(&rt);
    let row = sheet(&mut rt).top_left.row;
    assert!(
        row > 0,
        "the bar down moved the rows: {down}; thumb {tall:?}, under it {:?}",
        rt.element_at(tall[0] + tall[2] / 2.0, tall[1] + tall[3] / 2.0)
    );
    assert_eq!(row, (down / ROW_Q).round() as u32);
    drag(&mut rt, (wide[0] + wide[2] / 2.0, wide[1] + wide[3] / 2.0), (60.0, 0.0));
    let (_, across) = offsets(&rt);
    let col = sheet(&mut rt).top_left.col;
    assert!(col > 0, "the bar across moved the columns: {across}");
    assert_eq!(col, (across / COL_Q).round() as u32);
}

#[test]
fn office_sheets_tabs_past_the_width_go_behind_more() {
    let mut view = big();
    view.tabs = (1..=30).map(|i| format!("Quarterly figures {i:02}")).collect();
    view.more.unnamed = 3;
    let (mut rt, _log) = live_runtime(Body::Sheet(view), 1.0, None);
    settle(&mut rt);
    assert!(find(&rt, "sh-tab-0").is_some(), "the first tabs fit");
    assert!(find(&rt, "sh-tab-29").is_none(), "the last does not");
    let drawn = (0..30).filter(|i| find(&rt, &format!("sh-tab-{i}")).is_some()).count();
    let more = find(&rt, "sh-tab-more-t")
        .and_then(|e| e.text)
        .expect("the more control");
    assert_eq!(
        more,
        format!("{} more", 30 - drawn + 3),
        "the hidden tabs and the unnamed sheets"
    );
    // Every drawn tab is inside the bar.
    let bar = rect(&rt, "sh-tabs");
    let control = rect(&rt, "sh-tab-more");
    assert!(
        control[0] + control[2] <= bar[0] + bar[2] + 0.5,
        "{control:?} in {bar:?}"
    );
    // The menu: the hidden tabs, and the sheets not listed.
    let c = (control[0] + control[2] / 2.0, control[1] + control[3] / 2.0);
    rt.pointer_down(c.0, c.1);
    rt.pointer_up(c.0, c.1);
    settle(&mut rt);
    assert!(find(&rt, "sh-more-menu").is_some(), "the menu opens");
    assert_eq!(
        find(&rt, "sh-more-unnamed-t").and_then(|e| e.text),
        Some("3 more sheets not listed here".to_string())
    );
    let item = rect(&rt, "sh-more-29");
    let (ix, iy) = (item[0] + 4.0, item[1] + item[3] / 2.0);
    let under = rt.element_at(ix, iy);
    rt.pointer_down(ix, iy);
    rt.pointer_up(ix, iy);
    settle(&mut rt);
    let view = sheet(&mut rt);
    assert_eq!(
        (view.active, view.sheet, view.more.open),
        (29, 29, false),
        "item {item:?}, menu {:?}, under it {under:?}",
        find(&rt, "sh-more-menu").map(|e| e.rect)
    );
    assert!(find(&rt, "sh-tab-29").is_some(), "the open sheet's tab is in the bar");
    assert!(find(&rt, "sh-more-menu").is_none());
    assert_eq!(
        find(&rt, "st-0").and_then(|e| e.text),
        Some("Sheet 30 of 33".to_string())
    );
}

#[test]
fn office_sheets_three_tabs_that_fit_have_no_more() {
    let (rt, _log) = live_runtime(Body::Sheet(big()), 1.0, None);
    assert!(find(&rt, "sh-tab-2").is_some() && find(&rt, "sh-tab-more").is_none());
}

#[test]
fn office_sheets_shown_tabs_keep_the_open_one() {
    let mut view = big();
    view.tabs = (0..10).map(|i| format!("Sheet {i}")).collect();
    view.active = 9;
    let (shown, hidden) = super::super::sheets_tabs::shown_tabs(&view, 300.0);
    assert!(shown.contains(&9), "{shown:?}");
    assert!(!hidden.is_empty() && !hidden.contains(&9), "{hidden:?}");
    assert_eq!(shown.len() + hidden.len(), 10);
}

#[test]
fn office_sheets_esc_closes_the_more_menu_and_keeps_the_file() {
    let mut view = big();
    view.tabs = (1..=30).map(|i| format!("Quarterly figures {i:02}")).collect();
    let (mut rt, _log) = live_runtime(Body::Sheet(view), 1.0, None);
    rt.on_key = Some(crate::shell::host::on_key);
    settle(&mut rt);
    let control = rect(&rt, "sh-tab-more");
    rt.pointer_down(control[0] + 4.0, control[1] + control[3] / 2.0);
    rt.pointer_up(control[0] + 4.0, control[1] + control[3] / 2.0);
    settle(&mut rt);
    assert!(sheet(&mut rt).more.open, "the menu is open");
    press(&mut rt, Key::Escape);
    assert!(
        viewers::ui(&mut rt.state).is_some(),
        "Esc closed the menu, not the file"
    );
    assert!(!sheet(&mut rt).more.open && find(&rt, "sh-more-menu").is_none());
}
