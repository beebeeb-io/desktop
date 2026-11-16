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
