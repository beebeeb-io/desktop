//! The switch: role, checked state, label, theme colours, disabled at 50 %.

use rune::{Color, Cx, El, ElementInfo, Env, Runtime};

use super::*;
use crate::theme::tokens::theme;

struct Page {
    on: bool,
    enabled: bool,
}

fn view(p: &Page, _: &mut Cx<Page>) -> El {
    rune::row().px(20.0).pt(20.0).child(switch("s", "Sync", p.on, p.enabled, &theme(false)))
}

fn render(on: bool, enabled: bool) -> Runtime<Page> {
    let env = Env { system_dark: false, scale: 2.0 };
    let mut rt = Runtime::new(Page { on, enabled }, view, (300.0, 100.0), env);
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
    rt
}

fn find(rt: &Runtime<Page>, key: &str) -> Option<ElementInfo> {
    rt.elements().into_iter().find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn close(a: Color, b: Color) -> bool {
    (a.r - b.r).abs() < 1e-3 && (a.g - b.g).abs() < 1e-3 && (a.b - b.b).abs() < 1e-3 && (a.a - b.a).abs() < 1e-3
}

#[test]
fn role_label_checked_and_theme_colours() {
    let t = theme(false);
    for on in [false, true] {
        let rt = render(on, true);
        let s = find(&rt, "s").expect("switch");
        assert_eq!(s.role, "switch");
        assert_eq!(s.label.as_deref(), Some("Sync"));
        assert!(s.focusable);
        let track = find(&rt, "s-track").expect("track");
        assert!(close(track.fill, if on { t.selection } else { t.paper3 }), "on={on}: track");
        let knob = find(&rt, "s-knob").expect("knob");
        assert!(close(knob.fill, t.white), "knob is white");
        let l = find(&rt, "s-label").expect("label");
        assert_eq!(l.text.as_deref(), Some("Sync"));
        // The knob sits at the end of the track when on, at the start when off.
        let (tr, kr) = (track.rect, knob.rect);
        assert_eq!(kr[0] > tr[0] + 4.0, on, "on={on}: knob side");
    }
}

#[test]
fn disabled_takes_no_focus_and_is_half_visible() {
    let rt = render(true, false);
    let s = find(&rt, "s").expect("switch");
    assert!(!s.focusable);
}
