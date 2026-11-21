//! The one dialog: its keys, roles, surfaces, paddings, foot widths, Enter and Esc.

use rune::{Color, Cx, El, Env, Key, KeyEvent, Mods, Runtime, col};

use super::*;
use crate::theme::tokens::{Theme, theme};
use rune::el;

/// A window that draws one dialog and records what its buttons and its Esc did.
struct W {
    build: fn(&Theme) -> El,
    pressed: Vec<usize>,
    dismissed: u32,
}

fn view(s: &W, _: &mut Cx<W>) -> El {
    col().key("root").w_full().h_full().child((s.build)(&theme(false)))
}

fn window(build: fn(&Theme) -> El) -> Runtime<W> {
    let w = W {
        build,
        pressed: Vec::new(),
        dismissed: 0,
    };
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(w, view, (1000.0, 800.0), env);
    settle(&mut rt);
    rt
}

fn settle(rt: &mut Runtime<W>) {
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
}

fn press(rt: &mut Runtime<W>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    settle(rt);
}

fn info(rt: &Runtime<W>, key: &str) -> rune::ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
}

fn has(rt: &Runtime<W>, key: &str) -> bool {
    rt.elements()
        .iter()
        .any(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

/// A foot button that records its press.
fn btn(i: usize, label: &str, t: &Theme) -> El {
    row()
        .key(format!("b{i}"))
        .role("button")
        .label(label)
        .focusable(true)
        .h(28.0)
        .px(12.0)
        .center()
        .child(kit::label(format!("b{i}-label"), label, SUB, t.text))
        .on_press(move |s: &mut W| s.pressed.push(i))
}

fn para(t: &Theme) -> El {
    kit::label("para", "Some words in the body.", SUB, t.text_secondary)
        .wrap()
        .w_full()
}

fn titled(t: &Theme) -> El {
    dialog(
        Dialog {
            head: Head::Title {
                title: "Rename",
                sub: Some(Sub::Wrap("Give it a new name.")),
            },
            body: vec![para(t)],
            foot: Foot::Row(vec![btn(0, "Rename", t), btn(1, "Cancel", t)]),
            ..Dialog::new("dlg", "Rename", 380.0)
        },
        t,
    )
}

#[test]
fn a_card_with_title_sub_and_a_row_foot_has_its_keys_role_and_radius() {
    let rt = window(titled);
    for key in [
        "dlg",
        "dlg-clip",
        "dlg-head",
        "dlg-title",
        "dlg-sub",
        "dlg-body",
        "dlg-foot",
    ] {
        assert!(has(&rt, key), "{key}");
    }
    let dlg = info(&rt, "dlg");
    assert_eq!(
        (dlg.role, dlg.label.as_deref(), dlg.radius),
        ("dialog", Some("Rename"), RADIUS)
    );
    assert_eq!(dlg.rect[2], 380.0);
    assert_eq!(info(&rt, "dlg-title").level, Some(2));
    // Head paddings: pt 20, px 24.
    let title = info(&rt, "dlg-title").rect;
    assert_eq!((title[0] - dlg.rect[0], title[1] - dlg.rect[1]), (24.0, 20.0));
    // Row: at the trailing edge, the band's 24 pt in.
    let last = info(&rt, "b1").rect;
    assert!((dlg.rect[0] + dlg.rect[2] - 24.0 - (last[0] + last[2])).abs() < 0.01);
}

#[test]
fn the_key_and_the_prefix_name_the_parts() {
    let rt = window(|t| {
        dialog(
            Dialog {
                head: Head::Title {
                    title: "T",
                    sub: Some(Sub::Truncate("s")),
                },
                body: vec![para(t)],
                foot: Foot::Row(vec![btn(0, "OK", t)]),
                ..Dialog::new("ph-dlg", "T", 380.0)
            },
            t,
        )
    });
    for key in [
        "ph-dlg",
        "ph-dlg-clip",
        "ph-dlg-head",
        "ph-dlg-title",
        "ph-dlg-sub",
        "ph-dlg-body",
        "ph-dlg-foot",
    ] {
        assert!(has(&rt, key), "{key}");
    }
    let rt = window(|t| {
        dialog(
            Dialog {
                prefix: "sc",
                head: Head::Title {
                    title: "T",
                    sub: Some(Sub::Wrap("s")),
                },
                foot: Foot::Row(vec![btn(0, "OK", t)]),
                ..Dialog::new("sc-dlg", "T", 380.0)
            },
            t,
        )
    });
    for key in ["sc-dlg", "sc-clip", "sc-head", "sc-title", "sc-sub", "sc-foot"] {
        assert!(has(&rt, key), "{key}");
    }
}

#[test]
fn an_alert_is_an_alertdialog_with_its_words_and_an_empty_body_has_no_box() {
    let rt = window(|t| {
        dialog(
            Dialog {
                alert: true,
                head: Head::Alert {
                    tile: el().key("tile").size(36.0, 36.0),
                    title: "Delete this?",
                    sub: "It goes to the trash.",
                },
                foot: Foot::Even(vec![btn(0, "Keep", t), btn(1, "Delete", t)]),
                ..Dialog::new("dlg", "Delete", 420.0)
            },
            t,
        )
    });
    assert_eq!(info(&rt, "dlg").role, "alertdialog");
    assert!(has(&rt, "dlg-words") && has(&rt, "dlg-title") && has(&rt, "dlg-sub"));
    assert!(!has(&rt, "dlg-body"), "no body, no p-body");
    let (dlg, tile) = (info(&rt, "dlg").rect, info(&rt, "tile").rect);
    assert_eq!((tile[0] - dlg[0], tile[1] - dlg[1]), (24.0, 20.0));
}

#[test]
fn body_top_puts_a_body_first_dialog_s_words_22_points_down() {
    let rt = window(|t| {
        dialog(
            Dialog {
                body: vec![para(t)],
                body_top: Some(22.0),
                foot: Foot::Row(vec![btn(0, "OK", t)]),
                ..Dialog::new("dlg", "Body first", 380.0)
            },
            t,
        )
    });
    assert!(!has(&rt, "dlg-head"));
    assert_eq!(info(&rt, "para").rect[1] - info(&rt, "dlg").rect[1], 22.0);
}

fn even(n: usize) -> fn(&Theme) -> El {
    fn two(t: &Theme) -> El {
        dialog(
            Dialog {
                foot: Foot::Even(vec![btn(0, "Keep", t), btn(1, "Delete everything", t)]),
                ..Dialog::new("dlg", "E", 380.0)
            },
            t,
        )
    }
    fn three(t: &Theme) -> El {
        dialog(
            Dialog {
                foot: Foot::Even(vec![
                    btn(0, "A", t),
                    btn(1, "Bee bee", t),
                    btn(2, "Three words here", t),
                ]),
                ..Dialog::new("dlg", "E", 380.0)
            },
            t,
        )
    }
    if n == 2 { two } else { three }
}

#[test]
fn even_gives_equal_widths_across_the_foot() {
    for n in [2usize, 3] {
        let rt = window(even(n));
        let want = (380.0 - 48.0 - 8.0 * (n as f32 - 1.0)) / n as f32;
        let dlg = info(&rt, "dlg").rect;
        for i in 0..n {
            assert!(
                (info(&rt, &format!("b{i}")).rect[2] - want).abs() < 0.01,
                "{n} buttons, b{i}"
            );
        }
        assert!((info(&rt, "b0").rect[0] - dlg[0] - 24.0).abs() < 0.01, "{n}");
    }
}

#[test]
fn stack_gives_full_width_buttons() {
    let rt = window(|t| {
        dialog(
            Dialog {
                foot: Foot::Stack(vec![btn(0, "Keep", t), btn(1, "Leave", t)]),
                ..Dialog::new("dlg", "S", 420.0)
            },
            t,
        )
    });
    let b0 = info(&rt, "b0").rect;
    let b1 = info(&rt, "b1").rect;
    assert_eq!((b0[2], b1[2]), (420.0 - 48.0, 420.0 - 48.0));
    assert!(b1[1] > b0[1] + b0[3] - 0.01, "one under the other");
}

#[test]
fn a_panel_in_an_auto_height_parent_has_its_content_height() {
    let rt = window(|t| {
        dialog(
            Dialog {
                surface: Surface::Panel,
                body: vec![el().key("blk").w(100.0).h(90.0)],
                ..Dialog::new("dlg", "P", 360.0)
            },
            t,
        )
    });
    assert_eq!(info(&rt, "dlg").rect[3], 90.0);
    assert_eq!(info(&rt, "dlg-clip").rect[3], 90.0);
}

#[test]
fn sheet_and_panel_have_their_corners_and_add_no_padding() {
    fn part(surface: Surface, t: &Theme) -> El {
        dialog(
            Dialog {
                surface,
                head: Head::Title { title: "T", sub: None },
                body: vec![para(t)],
                foot: Foot::Row(vec![btn(0, "OK", t)]),
                ..Dialog::new("dlg", "P", 360.0)
            },
            t,
        )
    }
    for (surface, build, radius) in [
        (
            Surface::Panel,
            (|t: &Theme| part(Surface::Panel, t)) as fn(&Theme) -> El,
            PANEL_RADIUS,
        ),
        (Surface::Sheet, |t: &Theme| part(Surface::Sheet, t), SHEET_CORNERS[0]),
    ] {
        let rt = window(build);
        let dlg = info(&rt, "dlg");
        assert_eq!(dlg.radius, radius, "{surface:?}");
        let title = info(&rt, "dlg-title").rect;
        assert_eq!(
            (title[0], title[1]),
            (dlg.rect[0], dlg.rect[1]),
            "{surface:?}: no head padding"
        );
        let foot = info(&rt, "dlg-foot");
        assert_eq!(foot.fill, Color::TRANSPARENT, "{surface:?}: no band");
        let b0 = info(&rt, "b0").rect;
        assert!(
            (dlg.rect[0] + dlg.rect[2] - (b0[0] + b0[2])).abs() < 0.01,
            "{surface:?}: no foot padding"
        );
        assert_eq!(info(&rt, "dlg-clip").rect[3], dlg.rect[3], "{surface:?}");
    }
}

fn entered(enter: Option<usize>) -> fn(&Theme) -> El {
    fn one(t: &Theme) -> El {
        dialog(
            Dialog {
                foot: Foot::Row(vec![btn(0, "Save", t), btn(1, "Cancel", t)]),
                enter: Some(1),
                ..Dialog::new("dlg", "E", 380.0)
            },
            t,
        )
    }
    fn none(t: &Theme) -> El {
        dialog(
            Dialog {
                foot: Foot::Row(vec![btn(0, "Save", t), btn(1, "Cancel", t)]),
                ..Dialog::new("dlg", "E", 380.0)
            },
            t,
        )
    }
    if enter.is_some() { one } else { none }
}

#[test]
fn enter_presses_the_named_foot_button_and_nothing_without_one() {
    let mut rt = window(entered(Some(1)));
    press(&mut rt, Key::Enter);
    assert_eq!(rt.state.pressed, vec![1]);
    let mut rt = window(entered(None));
    press(&mut rt, Key::Enter);
    assert!(rt.state.pressed.is_empty());
}

#[test]
fn esc_in_a_modal_dismisses_it() {
    let mut rt = window(|t| {
        let card = dialog(
            Dialog {
                foot: Foot::Row(vec![btn(0, "Save", t), btn(1, "Cancel", t)]),
                enter: Some(0),
                ..Dialog::new("dlg", "M", 380.0)
            },
            t,
        );
        modal(32.0, card, |s: &mut W| s.dismissed += 1, t)
    });
    assert!(has(&rt, "scrim"));
    press(&mut rt, Key::Escape);
    assert_eq!(rt.state.dismissed, 1);
    press(&mut rt, Key::Enter);
    assert_eq!(
        rt.state.pressed,
        vec![0],
        "Enter inside the modal presses its default button"
    );
}

/// `card(..)` as it was before the one dialog: the boxes the delegate must keep.
fn old_card(name: &str, width: f32, title: &str, sub: Option<Sub>, body: Vec<El>, foot: El, t: &Theme) -> El {
    let mut head = col().key("dlg-head").pt(20.0).px(24.0).pb(4.0).gap(3.0).child(
        kit::label("dlg-title", title, DIALOG_TITLE, t.text)
            .role("heading")
            .level(2)
            .w_full(),
    );
    match sub {
        Some(Sub::Wrap(words)) => head = head.child(kit::label("dlg-sub", words, SUB, t.text_tertiary).wrap()),
        Some(Sub::Truncate(words)) => head = head.child(kit::label("dlg-sub", words, SUB, t.text_tertiary).truncate()),
        None => {}
    }
    let mut b = col().key("dlg-body").pt(10.0).px(24.0).pb(18.0).gap(12.0).min_h(0.0);
    for child in body {
        b = b.child(child);
    }
    let foot = foot
        .key("dlg-foot")
        .gap(8.0)
        .py(16.0)
        .px(24.0)
        .fill(t.surface)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout();
    col()
        .key("dlg")
        .role("dialog")
        .label(name)
        .pixel_snap(false)
        .w(width)
        .radius(RADIUS)
        .fill(t.card)
        .box_shadow(kit::drop(24.0, 60.0, Color::BLACK.alpha(0.28)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(
            col()
                .key("dlg-clip")
                .w_full()
                .clip_rounded(RADIUS)
                .child(head)
                .child(b)
                .child(foot),
        )
}

fn boxes(rt: &Runtime<W>) -> Vec<(Option<String>, &'static str, [f32; 4], f32, Color)> {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .map(|e| (e.key, e.role, e.rect, e.radius, e.fill))
        .collect()
}

#[test]
fn card_keeps_its_keys_and_boxes() {
    fn foot(t: &Theme) -> El {
        row()
            .center_y()
            .justify_end()
            .child(btn(0, "Save", t))
            .child(btn(1, "Cancel", t))
    }
    let cases: [(fn(&Theme) -> El, fn(&Theme) -> El); 3] = [
        (
            |t| {
                scrim(
                    0.0,
                    card(
                        "N",
                        380.0,
                        "Title",
                        Some(Sub::Wrap(
                            "A line that is long enough to wrap onto a second line in the card.",
                        )),
                        vec![para(t)],
                        foot(t),
                        t,
                    ),
                    t,
                )
            },
            |t| {
                scrim(
                    0.0,
                    old_card(
                        "N",
                        380.0,
                        "Title",
                        Some(Sub::Wrap(
                            "A line that is long enough to wrap onto a second line in the card.",
                        )),
                        vec![para(t)],
                        foot(t),
                        t,
                    ),
                    t,
                )
            },
        ),
        (
            |t| {
                scrim(
                    32.0,
                    card(
                        "N",
                        420.0,
                        "Title",
                        Some(Sub::Truncate("name.txt")),
                        vec![],
                        col().w_full().child(btn(0, "A", t).w_full()),
                        t,
                    ),
                    t,
                )
            },
            |t| {
                scrim(
                    32.0,
                    old_card(
                        "N",
                        420.0,
                        "Title",
                        Some(Sub::Truncate("name.txt")),
                        vec![],
                        col().w_full().child(btn(0, "A", t).w_full()),
                        t,
                    ),
                    t,
                )
            },
        ),
        (
            |t| card("N", 380.0, "Title", None, vec![para(t), para(t)], foot(t), t),
            |t| old_card("N", 380.0, "Title", None, vec![para(t), para(t)], foot(t), t),
        ),
    ];
    for (i, (new, old)) in cases.into_iter().enumerate() {
        let (a, b) = (window(new), window(old));
        assert!(has(&a, "dlg-body"), "case {i}: card keeps its body box, also empty");
        assert_eq!(boxes(&a), boxes(&b), "case {i}");
    }
}
