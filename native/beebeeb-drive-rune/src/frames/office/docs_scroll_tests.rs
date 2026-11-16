//! Tests of the document's continuous scroll (`docs_scroll`): the wheel and the scroll bar move the stage's viewport and
//! the page in view follows it; a scroll asks the pages that come into view and no others; the keys and the outline
//! still go to a page; a picture that comes never moves the pages.

use rune::{Key, KeyEvent, Mods, Runtime, ScrollBars};

use super::super::live_tests::{asked_pages_of, doc_of, find, go_to, live_runtime};
use super::*;
use crate::frames::viewers::kinds::OfficePicture;
use crate::frames::viewers::source::Body;

fn doc(rt: &mut Runtime<State>) -> DocBody {
    viewers::doc_mut(&mut rt.state).expect("a document").clone()
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

/// The wheel over the middle of the stage, `dy` points (negative scrolls down, as the toolkit's wheel does).
fn wheel(rt: &mut Runtime<State>, dy: f32) {
    let stage = rect(rt, STAGE);
    rt.wheel(stage[0] + stage[2] / 2.0, stage[1] + stage[3] / 2.0, dy);
    settle(rt);
}

/// Where the stage is scrolled to, measured from page 1's place (the instrument the state is checked against).
fn measured(rt: &Runtime<State>) -> f32 {
    rect(rt, STAGE)[1] + PAD_TOP - rect(rt, "page")[1]
}

/// A4 at 100 %: 520 x 735.9 points, 16 between pages.
fn a4_step() -> f32 {
    520.0 * 842.0 / 595.0 + PAGE_GAP
}

#[test]
fn office_docs_wheel_moves_the_stage_and_the_page_in_view() {
    let (mut rt, _log) = live_runtime(doc_of(60, 100, 1), 1.0, None);
    assert!(measured(&rt).abs() < 0.6, "the stage opens at its top");
    wheel(&mut rt, -500.0);
    assert!((measured(&rt) - 500.0).abs() < 0.6, "moved 500: {}", measured(&rt));
    let d = doc(&mut rt);
    assert!((d.scroll_y - 500.0).abs() < 0.6, "the state follows: {}", d.scroll_y);
    assert_eq!(d.page, 1, "page 2 starts at {}", PAD_TOP + a4_step());
    wheel(&mut rt, -400.0);
    let d = doc(&mut rt);
    assert!((d.scroll_y - 900.0).abs() < 0.6, "{}", d.scroll_y);
    assert_eq!(d.page, 2);
    assert_eq!(
        find(&rt, "vbar-page").and_then(|e| e.text),
        Some("Page 2 of 60".to_string())
    );
    // Back up: the wheel the other way.
    wheel(&mut rt, 900.0);
    assert_eq!(doc(&mut rt).page, 1);
    assert!(measured(&rt).abs() < 0.6);
}

#[test]
fn office_docs_a_small_scroll_runs_no_view() {
    let (mut rt, _log) = live_runtime(doc_of(60, 100, 1), 1.0, None);
    wheel(&mut rt, -10.0);
    assert!((measured(&rt) - 10.0).abs() < 0.6, "the viewport moved");
    assert_eq!(
        doc(&mut rt).scroll_y,
        0.0,
        "under a step the state is left alone (no view runs)"
    );
}

#[test]
fn office_docs_a_scroll_asks_the_pages_that_come_into_view_and_no_others() {
    let (mut rt, log) = live_runtime(doc_of(60, 100, 1), 1.0, Some((100, 140)));
    let first = asked_pages_of(&log);
    assert_eq!(
        first,
        vec![0, 1, 2],
        "at the top: the pages on screen and one screen below"
    );
    log.pictures.borrow_mut().clear();
    // Scroll so page 31 (zero-based 30) stands at the top (two points into it).
    wheel(&mut rt, -(30.0 * a4_step() + 2.0));
    assert_eq!(doc(&mut rt).page, 31);
    let asked = asked_pages_of(&log);
    assert!(asked.contains(&30), "the page that came into view is asked: {asked:?}");
    assert!(
        asked.iter().all(|p| (28..=32).contains(p)),
        "only the pages near the screen: {asked:?}"
    );
}

#[test]
fn office_docs_the_scroll_bar_drags_the_stage() {
    let (mut rt, _log) = live_runtime(doc_of(60, 100, 1), 1.0, None);
    rt.set_system_scroll_bars(ScrollBars::Always);
    rt.update(|_, _| {});
    settle(&mut rt);
    let stage = rect(&rt, STAGE);
    let thumb = rt
        .elements()
        .into_iter()
        .filter(|e| e.key.as_deref() == Some("thumb") && !e.exiting)
        .map(|e| e.rect)
        .find(|r| r[0] >= stage[0] && r[0] + r[2] <= stage[0] + stage[2] + 0.5)
        .expect("the stage's scroll bar has a thumb");
    let (x, y) = (thumb[0] + thumb[2] / 2.0, thumb[1] + thumb[3] / 2.0);
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_move(x, y + 50.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(x, y + 100.0);
    rt.pointer_up(x, y + 100.0);
    settle(&mut rt);
    let at = measured(&rt);
    assert!(
        at > 1000.0,
        "the drag moved the stage: {at}; thumb {thumb:?}, under it {:?}",
        rt.element_at(x, y)
    );
    let d = doc(&mut rt);
    assert!(
        (d.scroll_y - at).abs() < 0.6,
        "the state follows the bar: {} vs {at}",
        d.scroll_y
    );
    assert_eq!(d.page, page_at(&vec![a4_step() - PAGE_GAP; 60], at));
}

#[test]
fn office_docs_keys_and_the_outline_still_go_to_a_page() {
    let (mut rt, _log) = live_runtime(doc_of(60, 100, 1), 1.0, None);
    rt.on_key = Some(crate::shell::host::on_key);
    let at_top = |rt: &Runtime<State>, n: u32| {
        let (stage, page) = (rect(rt, STAGE), rect(rt, &page_key(n)));
        (page[1] - stage[1] - PAD_TOP).abs() < 1.0
    };
    rt.key(KeyEvent {
        key: Key::PageDown,
        mods: Mods::default(),
    });
    settle(&mut rt);
    assert_eq!(doc(&mut rt).page, 2);
    assert!(at_top(&rt, 2), "Page Down shows page 2 from its top");
    // After a wheel, Page Down goes on from the page the wheel reached.
    wheel(&mut rt, -(8.0 * a4_step() + 2.0));
    assert_eq!(doc(&mut rt).page, 10);
    rt.key(KeyEvent {
        key: Key::PageDown,
        mods: Mods::default(),
    });
    settle(&mut rt);
    assert_eq!(doc(&mut rt).page, 11);
    assert!(at_top(&rt, 11));
    // A heading in the outline.
    let (i, want) = doc(&mut rt)
        .view
        .outline
        .iter()
        .enumerate()
        .map(|(i, (_, _, p))| (i, *p))
        .find(|(_, p)| *p != 11)
        .expect("a heading on another page");
    let row = rect(&rt, &format!("ol-{i}-row"));
    rt.pointer_down(row[0] + 4.0, row[1] + row[3] / 2.0);
    rt.pointer_up(row[0] + 4.0, row[1] + row[3] / 2.0);
    settle(&mut rt);
    assert_eq!(doc(&mut rt).page, want);
    assert!(at_top(&rt, want), "the heading's page is shown from its top");
}

#[test]
fn office_docs_a_picture_that_comes_does_not_move_the_pages() {
    // Landscape pages: their box is shorter than the board's least page height, which a picture used to stretch.
    let mut body = doc_of(6, 100, 1);
    if let Body::Doc(d) = &mut body {
        d.view.page_sizes_pt = vec![(842.0, 595.0); 6];
    }
    let (mut rt, log) = live_runtime(body, 1.0, None);
    let before = rect(&rt, "page-3");
    log.answer.set(Some((100, 140)));
    super::super::live_tests::rebuild(&mut rt, &log, |_| {});
    assert!(find(&rt, "page-picture-1").is_some(), "the picture came");
    assert_eq!(rect(&rt, "page-3"), before, "the pages stay where they were");
}

#[test]
fn office_docs_a_carried_page_draws_its_band_from_its_own_top() {
    // A body that came in with the page carried over and no offset yet: the band is the page's.
    let mut body = doc_of(60, 100, 1);
    if let Body::Doc(d) = &mut body {
        d.page = 31;
    }
    if let Body::Doc(d) = &body {
        assert!((d.view_y() - (d.page_top(31) - PAD_TOP)).abs() < 0.01);
    }
}

#[test]
fn office_docs_go_to_puts_the_page_at_the_top() {
    let (mut rt, log) = live_runtime(doc_of(60, 100, 1), 1.0, None);
    go_to(&mut rt, &log, 20);
    settle(&mut rt);
    assert_eq!(doc(&mut rt).page, 20);
    assert!((measured(&rt) - 19.0 * a4_step()).abs() < 1.0, "{}", measured(&rt));
}

#[test]
fn office_docs_a_tall_page_brings_the_next_one_as_its_foot_comes() {
    // 595 x 1000 pt pages at 200 %: 1040 x 1748 points, taller than two screens of a short window. Scrolled 1700 points,
    // page 1's foot is near the top and page 2 fills the screen: page 2 is asked; page 1's first tile row, far above, is not.
    let mut body = doc_of(4, 200, 1);
    if let Body::Doc(d) = &mut body {
        d.view.page_sizes_pt = vec![(595.0, 1000.0); 4];
    }
    let (mut rt, log) = live_runtime(body, 1.0, None);
    log.pictures.borrow_mut().clear();
    wheel(&mut rt, -1700.0);
    assert!((doc(&mut rt).scroll_y - 1700.0).abs() < 0.6);
    let keys = log.pictures.borrow().clone();
    assert!(asked_pages_of(&log).contains(&1), "page 2 is asked: {keys:?}");
    assert!(
        !keys
            .iter()
            .any(|k| matches!(k, OfficePicture::Tile { page: 0, row: 0, .. })),
        "page 1's top row scrolled far off: {keys:?}"
    );
}
