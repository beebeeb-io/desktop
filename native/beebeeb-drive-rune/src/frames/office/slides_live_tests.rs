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
