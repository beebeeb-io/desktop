//! The one status bar: its keys, height and dot colours, `dot_for`, `frame`, and the delegates
//! (`status_line`, `status_offline`) drawing the same tree as `bar`.

use rune::{Color, Cx, El, ElementInfo, Env, Runtime, el};

use super::*;
use crate::theme::tokens::theme;
use crate::ui::{status_line, status_offline};

struct Case {
    tree: fn(&Theme) -> El,
    dark: bool,
}

fn view(s: &Case, _: &mut Cx<Case>) -> El {
    rune::col().w_full().h_full().child((s.tree)(&theme(s.dark)))
}

fn render(tree: fn(&Theme) -> El, dark: bool) -> Vec<ElementInfo> {
    let env = Env {
        system_dark: dark,
        scale: 2.0,
    };
    let mut rt = Runtime::new(Case { tree, dark }, view, (900.0, 200.0), env);
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
    rt.elements().into_iter().filter(|e| !e.exiting).collect()
}

fn find<'a>(els: &'a [ElementInfo], key: &str) -> Option<&'a ElementInfo> {
    els.iter().find(|e| e.key.as_deref() == Some(key))
}

fn get<'a>(els: &'a [ElementInfo], key: &str) -> &'a ElementInfo {
    find(els, key).unwrap_or_else(|| panic!("no element keyed {key}"))
}

/// What a tree draws, element by element: keys, roles, words, boxes and colours.
fn drawn(els: &[ElementInfo]) -> Vec<String> {
    els.iter()
        .map(|e| {
            format!(
                "{:?} {} {:?} {:?} {:?} {} {:?} {:?}",
                e.key, e.role, e.text, e.label, e.rect, e.radius, e.fill, e.text_color
            )
        })
        .collect()
}

type Make = fn(&Theme) -> El;
const DOTS: [(Dot, Make); 5] = [
    (Dot::Green, |t| {
        bar(Some(Dot::Green), Some("Up to date"), "4 on this PC", t)
    }),
    (Dot::Busy, |t| {
        bar(Some(Dot::Busy), Some("Up to date"), "4 on this PC", t)
    }),
    (Dot::Idle, |t| {
        bar(Some(Dot::Idle), Some("Up to date"), "4 on this PC", t)
    }),
    (Dot::Warn, |t| {
        bar(Some(Dot::Warn), Some("Up to date"), "4 on this PC", t)
    }),
    (Dot::Red, |t| bar(Some(Dot::Red), Some("Up to date"), "4 on this PC", t)),
];

fn expected(dot: Dot, t: &Theme) -> Color {
    match dot {
        Dot::Green => t.green,
        Dot::Busy => t.text_tertiary,
        Dot::Idle => t.text_quaternary,
        // `frames/viewers/style.rs` `warn` (private there).
        Dot::Warn => rune::color::hex(if t.dark { "#F67222" } else { "#C94B00" }).unwrap(),
        Dot::Red => t.red,
    }
}

#[test]
fn bar_has_the_keys_role_height_and_dot_colour_in_both_themes() {
    for dark in [false, true] {
        let t = theme(dark);
        for (dot, tree) in DOTS {
            let els = render(tree, dark);
            for key in [
                "status-bar",
                "status-left",
                "status-dot",
                "status-text",
                "status-spacer",
                "status-details",
            ] {
                get(&els, key);
            }
            let b = get(&els, "status-bar");
            assert_eq!(b.role, "status");
            assert_eq!(b.rect[3], 28.0, "height");
            assert_eq!(b.fill, t.status_bar_bg);
            assert_eq!(get(&els, "status-dot").fill, expected(dot, &t), "{dot:?} dark={dark}");
            assert_eq!(get(&els, "status-text").text.as_deref(), Some("Up to date"));
            assert_eq!(get(&els, "status-details").text.as_deref(), Some("4 on this PC"));
        }
    }
}

#[test]
fn no_state_draws_no_dot_and_no_word() {
    let els = render(|t| bar(Some(Dot::Green), None, "Nothing syncs here", t), false);
    assert!(find(&els, "status-dot").is_none());
    assert!(find(&els, "status-text").is_none());
    get(&els, "status-left");
    assert_eq!(get(&els, "status-details").text.as_deref(), Some("Nothing syncs here"));
}

#[test]
fn dot_for_follows_the_sync_word() {
    assert_eq!(dot_for(UP_TO_DATE), Dot::Green);
    assert_eq!(dot_for(SYNCING), Dot::Busy);
    assert_eq!(dot_for(PAUSED), Dot::Idle);
    assert_eq!(status_line::UP_TO_DATE, UP_TO_DATE);
}

#[test]
fn frame_keeps_the_callers_children_in_order() {
    let els = render(
        |t| {
            frame(
                vec![
                    el().key("c-a").size(10.0, 10.0),
                    el().key("c-b").size(10.0, 10.0),
                    el().key("c-c").size(10.0, 10.0),
                ],
                t,
            )
        },
        false,
    );
    let t = theme(false);
    let b = get(&els, "status-bar");
    assert_eq!(b.role, "status");
    assert_eq!(b.rect[3], 28.0);
    assert_eq!(b.fill, t.status_bar_bg);
    let xs: Vec<f32> = ["c-a", "c-b", "c-c"].iter().map(|k| get(&els, k).rect[0]).collect();
    assert!(xs[0] < xs[1] && xs[1] < xs[2], "{xs:?}");
    // The same bar as `bar` draws: box, fill and the hairline's layout room (children sit below it).
    let full = render(|t| bar(Some(Dot::Green), Some("x"), "y", t), false);
    let fb = get(&full, "status-bar");
    assert_eq!((b.rect, b.fill, b.radius), (fb.rect, fb.fill, fb.radius));
    let centre = |e: &ElementInfo| e.rect[1] + e.rect[3] / 2.0;
    assert!((centre(get(&els, "c-a")) - centre(get(&full, "status-left"))).abs() < 0.01);
}

#[test]
fn the_delegates_draw_bars_tree() {
    for dark in [false, true] {
        assert_eq!(
            drawn(&render(|t| status_line::status_line(Some(SYNCING), "2 left", t), dark)),
            drawn(&render(|t| bar(Some(Dot::Busy), Some(SYNCING), "2 left", t), dark)),
        );
        assert_eq!(
            drawn(&render(|t| status_line::status_line(None, "2 left", t), dark)),
            drawn(&render(|t| bar(None, None, "2 left", t), dark)),
        );
        assert_eq!(
            drawn(&render(
                |t| status_offline::status_offline("Last seen 5 min ago", t),
                dark
            )),
            drawn(&render(
                |t| bar(Some(Dot::Idle), Some("Offline"), "Last seen 5 min ago", t),
                dark
            )),
        );
        assert_eq!(
            drawn(&render(status_bar, dark)),
            drawn(&render(
                |t| bar(
                    Some(Dot::Green),
                    Some(UP_TO_DATE),
                    "4 on this PC \u{00B7} 3 available offline \u{00B7} 4 online only",
                    t
                ),
                dark
            )),
        );
    }
}
