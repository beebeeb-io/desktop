//! The pill: one tone table, with or without a dot, role status.

use rune::{Color, Cx, El, ElementInfo, Env, Runtime};

use super::*;
use crate::theme::tokens::theme;

const TONES: [Tone; 5] = [Tone::Neutral, Tone::Good, Tone::Warn, Tone::Bad, Tone::Accent];

struct Page {
    tone: Tone,
    dot: bool,
}

fn view(p: &Page, _: &mut Cx<Page>) -> El {
    rune::row().px(20.0).pt(20.0).child(pill("p", "Synced", p.tone, p.dot, &theme(false)))
}

fn render(tone: Tone, dot: bool) -> Runtime<Page> {
    let env = Env { system_dark: false, scale: 2.0 };
    let mut rt = Runtime::new(Page { tone, dot }, view, (300.0, 100.0), env);
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
fn each_tone_has_its_wash_ink_and_status_role() {
    let t = theme(false);
    for tone in TONES {
        let (fill, ink) = colors(tone, &t);
        let rt = render(tone, false);
        let p = find(&rt, "p").expect("pill");
        assert_eq!(p.role, "status", "{tone:?}");
        assert_eq!(p.label.as_deref(), Some("Synced"));
        assert!(close(p.fill, fill), "{tone:?}: fill");
        let l = find(&rt, "p-label").expect("label");
        assert_eq!(l.text.as_deref(), Some("Synced"));
        assert!(close(l.text_color.expect("ink"), ink), "{tone:?}: ink");
        assert!(find(&rt, "p-dot").is_none(), "{tone:?}: no dot asked");
    }
}

#[test]
fn the_dot_is_drawn_in_the_ink_only_when_asked() {
    let t = theme(false);
    for tone in TONES {
        let rt = render(tone, true);
        let d = find(&rt, "p-dot").expect("dot");
        assert!(close(d.fill, colors(tone, &t).1), "{tone:?}: dot colour");
    }
}
