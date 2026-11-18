//! Sort, the toolbar's search and the status line of Shared and Trash (w18-shared-trash-chrome).

use super::actions_sort::{By, SectionSort};
use super::parts::table::{Extra, Kind, Row};
use super::tests::{click, info, runtime, settle};
use crate::frames::Intent;

fn row(name: &str, size_words: &str, bytes: Option<u64>, at: Option<i64>) -> Row {
    Row {
        id: name.into(),
        name: name.into(),
        kind: Kind::Document,
        date: String::new(),
        size: size_words.into(),
        extra: Extra::Words(String::new()),
        pc: None,
        bytes,
        at,
    }
}

fn names(rows: &[Row]) -> Vec<&str> {
    rows.iter().map(|r| r.name.as_str()).collect()
}

/// The drawn words say "9 KB" is the biggest (its first digit), the bytes say otherwise.
fn samples() -> Vec<Row> {
    vec![
        row("a-small", "9 KB", Some(9_000), Some(10)),
        row("b-big", "1.2 GB", Some(1_200_000_000), Some(30)),
        row("c-mid", "40 MB", Some(40_000_000), Some(20)),
        row("d-folder", "", None, None),
    ]
}

#[test]
fn sort_by_size_orders_by_bytes_not_by_the_drawn_words() {
    let mut sort = SectionSort::default();
    sort.pick(2);
    assert_eq!(sort.by, By::Size);
    assert_eq!(names(&sort.arrange(samples())), ["b-big", "c-mid", "a-small", "d-folder"]);
    sort.pick(2);
    assert_eq!(names(&sort.arrange(samples())), ["d-folder", "a-small", "c-mid", "b-big"]);
}

#[test]
fn date_is_newest_first_by_default_and_name_ignores_case() {
    let sort = SectionSort::default();
    assert_eq!(names(&sort.arrange(samples())), ["b-big", "c-mid", "a-small", "d-folder"]);
    let mut sort = SectionSort::default();
    sort.pick(0);
    let mut rows = samples();
    rows[0].name = "B-upper".into();
    assert_eq!(names(&sort.arrange(rows)), ["b-big", "B-upper", "c-mid", "d-folder"]);
}

#[test]
fn the_search_filter_lists_only_rows_whose_name_holds_it() {
    let sort = SectionSort {
        filter: "  BIG ".into(),
        ..SectionSort::default()
    };
    assert_eq!(names(&sort.arrange(samples())), ["b-big"]);
}

#[test]
fn shared_sort_menu_sorts_the_rows_drawn_and_search_filters_them() {
    let mut rt = runtime(super::scenes::dw_0508);
    let drawn = |rt: &rune::Runtime<crate::State>| -> Vec<String> { (0..8).filter_map(|i| row_name(rt, i)).collect() };
    let before = drawn(&rt);
    assert!(before.len() > 2, "{before:?}");
    click(&mut rt, "sort-btn");
    click(&mut rt, "mi-name");
    let mut want = before.clone();
    want.sort_by_key(|n| n.to_lowercase());
    assert_eq!(drawn(&rt), want, "Sort by Name");
    // Enter in the search field filters the list, as the intent router hands it to the section.
    let first = want[0].clone();
    rt.update(|s, tx| {
        let intent = Intent::Do("search", first.to_lowercase());
        assert!(crate::frames::shared::actions::take(s, tx, &intent));
    });
    settle(&mut rt);
    assert!(drawn(&rt).iter().all(|n| n.to_lowercase().contains(&first.to_lowercase())));
}

fn row_name(rt: &rune::Runtime<crate::State>, i: usize) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(&format!("row-{i}-name")) && !e.exiting)
        .and_then(|e| e.text)
}

#[test]
fn trash_sort_menu_and_filter_work_on_its_rows() {
    let mut rt = runtime(crate::frames::trash::scenes::dw_0747);
    let before = rt.state.frames.trash.rows();
    click(&mut rt, "sort-btn");
    click(&mut rt, "mi-name");
    let mut want: Vec<String> = before.iter().map(|r| r.name.clone()).collect();
    want.sort_by_key(|n| n.to_lowercase());
    let got: Vec<String> = rt.state.frames.trash.rows().into_iter().map(|r| r.name).collect();
    assert_eq!(got, want);
    assert_eq!(row_name(&rt, 0).as_deref(), Some(want[0].as_str()));
    rt.update(|s, tx| {
        let intent = Intent::Do("search", want[0].clone());
        assert!(crate::frames::trash::actions::take(s, tx, &intent));
    });
    assert!(rt.state.frames.trash.rows().len() < before.len());
}

#[test]
fn shared_and_trash_say_the_sync_words_files_says() {
    for scene in [
        super::scenes::dw_0508 as fn(&mut crate::State),
        crate::frames::trash::scenes::dw_0747,
    ] {
        let mut rt = runtime(scene);
        rt.state.frames.pc = Box::new(crate::frames::UnwiredPc);
        settle(&mut rt);
        let words = rt.state.actions.source.sync_state().expect("the board's Files source has sync words");
        assert_eq!(info(&rt, "status-text").text.as_deref(), Some(words.as_str()));
    }
}
