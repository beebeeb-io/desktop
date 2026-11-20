//! Tests of the live deck (V3-5): the filmstrip asks the thumbnails in view, Present draws the slide's pixels.

use super::*;
use crate::frames::office::docs::live_tests::{find, live_runtime};

fn deck(count: u32) -> Body {
    Body::Deck(DeckBody::new(DeckView::of(count)))
}

#[test]
fn office_slides_thumbnails_ask_their_slides() {
    let (_rt, log) = live_runtime(deck(60), 1.0, None);
    let thumbs: Vec<u32> = log
        .pictures
        .borrow()
        .iter()
        .filter_map(|k| match k {
            OfficePicture::Thumb { slide, .. } => Some(*slide),
            _ => None,
        })
        .collect();
    assert!(!thumbs.is_empty(), "the visible thumbnails are asked");
    assert!(thumbs.len() < 30, "not every slide: {} asked", thumbs.len());
    assert!(!thumbs.contains(&59), "the last slide is far off screen");
    assert!(thumbs.contains(&0));
}

#[test]
fn office_slides_the_stage_asks_the_slide_on_it() {
    let (_rt, log) = live_runtime(deck(5), 1.0, None);
    assert!(log.pictures.borrow().iter().any(|k| matches!(
        k,
        OfficePicture::Page {
            page: 0,
            zoom_pct: 100,
            dpi: 96
        }
    )));
}

#[test]
fn office_slides_present_draws_the_slide_pixels() {
    let mut body = DeckBody::new(DeckView::of(5));
    body.presenting = true;
    let (rt, log) = live_runtime(Body::Deck(body), 1.0, Some((1600, 900)));
    assert!(find(&rt, "pres-stage").is_some(), "Present is on");
    assert!(find(&rt, "pres-picture").is_some(), "the slide's own pixels are drawn");
    assert!(find(&rt, "pres-points").is_none(), "never the fixture's words");
    assert!(
        log.pictures.borrow().iter().any(|k| matches!(
            k,
            OfficePicture::Page {
                page: 0,
                zoom_pct: 199,
                ..
            }
        )),
        "asked at the zoom that fits the screen: {:?}",
        log.pictures.borrow()
    );
}

#[test]
fn office_slides_present_zoom_fits_the_screen() {
    assert_eq!(present_zoom((1920.0, 1080.0)), 300);
    assert_eq!(present_zoom((1278.0, 798.0)), 199);
    assert_eq!(present_zoom((640.0, 1000.0)), 100, "the narrower side decides");
}

#[test]
fn office_slides_quiet_line_when_a_slide_could_not_draw_everything() {
    let mut view = DeckView::of(3);
    view.quiet = true;
    let (rt, _) = live_runtime(Body::Deck(DeckBody::new(view)), 1.0, None);
    assert!(
        rt.elements()
            .into_iter()
            .any(|e| e.text.as_deref() == Some(states::QUIET_LINE) && !e.exiting)
    );
}

#[test]
fn office_slides_presenting_follows_the_deck() {
    let mut body = DeckBody::new(DeckView::of(3));
    let (mut rt, _) = live_runtime(Body::Deck(body.clone()), 1.0, None);
    assert!(!presenting(&rt.state));
    body.presenting = true;
    viewers::ui(&mut rt.state).expect("a file").file.body = Body::Deck(body);
    assert!(presenting(&rt.state));
}

/// The wheel over the slide, at the middle of `key`: `dy` points, `precise` for a touchpad.
fn wheel_over(rt: &mut rune::Runtime<State>, key: &str, dy: f32, precise: bool) {
    let r = find(rt, key).unwrap_or_else(|| panic!("{key} is drawn")).rect;
    rt.wheel_event(
        rune::WheelEvent::new(0.0, dy)
            .at(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
            .precise(precise),
    );
    rt.frame(0.0);
    rt.frame(0.0);
}

fn current(rt: &mut rune::Runtime<State>) -> u32 {
    viewers::deck_mut(&mut rt.state).expect("a deck").view.current
}

#[test]
fn office_slides_the_wheel_steps_one_slide_per_notch() {
    let (mut rt, _log) = live_runtime(deck(5), 1.0, None);
    assert_eq!(current(&mut rt), 1);
    wheel_over(&mut rt, "stage", -100.0, false);
    assert_eq!(current(&mut rt), 2, "a notch toward you: the next slide");
    wheel_over(&mut rt, "stage", -100.0, false);
    assert_eq!(current(&mut rt), 3);
    wheel_over(&mut rt, "stage", 100.0, false);
    assert_eq!(current(&mut rt), 2, "away from you: the previous slide");
    // A touchpad: one slide per 120 points of travel, not one per delta.
    wheel_over(&mut rt, "stage", -50.0, true);
    wheel_over(&mut rt, "stage", -50.0, true);
    assert_eq!(current(&mut rt), 2, "100 points: not yet");
    wheel_over(&mut rt, "stage", -50.0, true);
    assert_eq!(current(&mut rt), 3, "150 points: one slide");
    // A smooth wheel sends a notch in parts: four of 10 points are one slide, not four.
    for _ in 0..4 {
        wheel_over(&mut rt, "stage", -10.0, false);
    }
    assert_eq!(current(&mut rt), 4, "one notch in parts: one slide");
    // Never past the last.
    for _ in 0..5 {
        wheel_over(&mut rt, "stage", -100.0, false);
    }
    assert_eq!(current(&mut rt), 5);
}

#[test]
fn office_slides_the_wheel_steps_in_present() {
    let (mut rt, _log) = live_runtime(deck(5), 1.0, None);
    rt.update(|s, _| viewers::deck_mut(s).expect("a deck").present());
    rt.frame(0.0);
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(1.0);
    wheel_over(&mut rt, "pres-stage", -100.0, false);
    assert_eq!(current(&mut rt), 2);
}

// ---------------------------------------------------------------------------
// The filmstrip scrolls (P3-8): a deck longer than one screen
// ---------------------------------------------------------------------------

use crate::frames::office::docs::live_tests::Log;

fn thumbs_asked(log: &Log) -> Vec<u32> {
    log.pictures
        .borrow()
        .iter()
        .filter_map(|k| match k {
            OfficePicture::Thumb { slide, .. } => Some(*slide),
            _ => None,
        })
        .collect()
}

/// `count` presses of `key` on the deck, then the window draws what it asks.
fn press_deck(rt: &mut rune::Runtime<State>, log: &Log, key: Key, count: u32) {
    log.pictures.borrow_mut().clear();
    rt.update(|s, _| {
        let deck = viewers::deck_mut(s).expect("a deck");
        for _ in 0..count {
            assert!(deck.on_key(&key, &Mods::default()));
        }
    });
    rt.frame(0.0);
    rt.frame(0.0);
}

fn inside(rt: &rune::Runtime<State>, row: &str, strip: &str) -> bool {
    let (r, s) = (
        find(rt, row).unwrap_or_else(|| panic!("{row} is drawn")).rect,
        find(rt, strip).expect("the strip is drawn").rect,
    );
    r[1] >= s[1] - 0.5 && r[1] + r[3] <= s[1] + s[3] + 0.5
}

#[test]
fn office_slides_filmstrip_asks_only_the_slides_in_view() {
    let (_rt, log) = live_runtime(deck(200), 1.0, None);
    let asked = thumbs_asked(&log);
    assert!(asked.contains(&0), "the first slide is in view");
    assert!(
        asked.iter().all(|&n| n < 20),
        "slides far down are not asked: {asked:?}"
    );
}

#[test]
fn office_slides_filmstrip_keeps_the_current_slide_in_view_after_page_down_x100() {
    let (mut rt, log) = live_runtime(deck(200), 1.0, None);
    press_deck(&mut rt, &log, Key::PageDown, 100);
    assert_eq!(current(&mut rt), 101);
    assert!(find(&rt, "fs-100").is_some(), "slide 101's line is drawn");
    assert!(inside(&rt, "fs-100", "fs-list"), "and it is inside the strip");
    assert!(find(&rt, "fs-0").is_none(), "slide 1 has scrolled away");
}

#[test]
fn office_slides_filmstrip_reaches_slide_150_and_asks_it_once() {
    let (mut rt, log) = live_runtime(deck(200), 1.0, None);
    press_deck(&mut rt, &log, Key::PageDown, 149);
    assert_eq!(current(&mut rt), 150);
    assert!(inside(&rt, "fs-149", "fs-list"));
    let asked = thumbs_asked(&log);
    assert_eq!(asked.iter().filter(|&&n| n == 149).count(), 1, "asked once: {asked:?}");
    assert!(
        asked.iter().all(|&n| (100..200).contains(&n)),
        "none out of view: {asked:?}"
    );
}

#[test]
fn office_slides_filmstrip_wheel_scrolls_the_strip_not_the_slide() {
    let (mut rt, log) = live_runtime(deck(200), 1.0, None);
    log.pictures.borrow_mut().clear();
    wheel_over(&mut rt, "fs-list", -400.0, false);
    assert_eq!(current(&mut rt), 1, "the wheel over the strip leaves the slide");
    assert!(find(&rt, "fs-0").is_none(), "the first line has scrolled away");
    let asked = thumbs_asked(&log);
    assert!(
        !asked.is_empty() && asked.iter().all(|&n| n > 0),
        "the new ones are asked: {asked:?}"
    );
    wheel_over(&mut rt, "fs-list", 100_000.0, false);
    assert!(find(&rt, "fs-0").is_some(), "and back to the top it stops");
}

/// A deck whose slides are `page_pt` points.
fn deck_of_size(page_pt: (u32, u32)) -> DeckBody {
    let mut view = DeckView::of(3);
    view.page_pt = page_pt;
    DeckBody::new(view)
}

#[test]
fn office_slides_a_4_3_slide_keeps_its_proportions_on_the_stage() {
    // 720 x 540 points (PowerPoint's 4:3): on the board's 640 x 360 it is 480 x 360, never stretched to 16:9.
    let (rt, _) = live_runtime(Body::Deck(deck_of_size((720, 540))), 1.0, Some((960, 720)));
    let slide = find(&rt, "slide").expect("the slide on the stage");
    assert!(
        (slide.rect[2] - 480.0).abs() < 0.6 && (slide.rect[3] - 360.0).abs() < 0.6,
        "4:3: {:?}",
        slide.rect
    );
    // The same at a display scale of 150 %: layout points do not change with it.
    let (rt, _) = live_runtime(Body::Deck(deck_of_size((720, 540))), 1.5, Some((1440, 1080)));
    let slide = find(&rt, "slide").expect("the slide on the stage");
    assert!((slide.rect[2] / slide.rect[3] - 4.0 / 3.0).abs() < 0.01, "{:?}", slide.rect);
    // A 16:9 slide is the board's.
    let (rt, _) = live_runtime(Body::Deck(deck_of_size(PAGE_PT)), 1.0, Some((1280, 720)));
    let slide = find(&rt, "slide").expect("the slide on the stage");
    assert!((slide.rect[2] - 640.0).abs() < 0.6 && (slide.rect[3] - 360.0).abs() < 0.6);
}

#[test]
fn office_slides_present_keeps_a_4_3_slide_in_proportion() {
    let mut body = deck_of_size((720, 540));
    body.presenting = true;
    let (rt, _) = live_runtime(Body::Deck(body), 1.0, Some((1433, 1075)));
    let picture = find(&rt, "pres-picture").expect("the slide's pixels");
    assert!(
        (picture.rect[2] / picture.rect[3] - 4.0 / 3.0).abs() < 0.01,
        "{:?}",
        picture.rect
    );
}

#[test]
fn office_slides_box_fits_the_board_in_the_slides_proportions() {
    assert_eq!(slide_box((960, 540), 100), (640.0, 360.0));
    assert_eq!(slide_box((720, 540), 100), (480.0, 360.0));
    assert_eq!(slide_box((720, 540), 50), (240.0, 180.0));
    // A portrait slide: the height decides.
    assert_eq!(slide_box((540, 960), 100), (202.5, 360.0));
}
