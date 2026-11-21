//! The one button, rendered in a runtime: each variant and size, its keys, its states, and the
//! width it hugs.

use rune::{Color, Cx, El, ElementInfo, Env, Key, KeyEvent, Mods, Runtime, col};

use super::*;
use crate::theme::icons;
use crate::theme::tokens::theme;

const VARIANTS: [Variant; 4] = [Variant::Primary, Variant::Secondary, Variant::Ghost, Variant::Danger];
const SIZES: [(Size, f32); 3] = [(Size::Sm, 22.0), (Size::Md, 28.0), (Size::Lg, 32.0)];

/// One button on a page, and how often it was pressed.
struct Page {
    keys: ButtonKeys,
    label: &'static str,
    icon: bool,
    variant: Variant,
    size: Size,
    state: ButtonState,
    pressed: u32,
}

impl Page {
    fn new(variant: Variant, size: Size, state: ButtonState) -> Self {
        Page {
            keys: keys("b"),
            label: "Add files",
            icon: false,
            variant,
            size,
            state,
            pressed: 0,
        }
    }
}

fn view(p: &Page, _: &mut Cx<Page>) -> El {
    let t = theme(false);
    let ic = p.icon.then_some(&icons::PLUS);
    col().key("page").px(20.0).pt(20.0).child(
        button(p.keys.clone(), p.label, ic, p.variant, p.size, p.state, &t).on_press(|p: &mut Page| p.pressed += 1),
    )
}

fn settle(rt: &mut Runtime<Page>) {
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
}

fn render(page: Page) -> Runtime<Page> {
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(page, view, (400.0, 200.0), env);
    settle(&mut rt);
    rt
}

fn find(rt: &Runtime<Page>, key: &str) -> Option<ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn info(rt: &Runtime<Page>, key: &str) -> ElementInfo {
    find(rt, key).unwrap_or_else(|| panic!("no element keyed {key}"))
}

fn click(rt: &mut Runtime<Page>, key: &str) {
    let r = info(rt, key).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}

/// Focus on the button by key, then Space.
fn space_on(rt: &mut Runtime<Page>, key: &'static str) {
    rt.update(move |_, tx| tx.focus(key, true));
    settle(rt);
    rt.key(KeyEvent {
        key: Key::Text(" ".into()),
        mods: Mods::default(),
    });
    settle(rt);
}

fn close(a: Color, b: Color) -> bool {
    (a.r - b.r).abs() < 1e-3 && (a.g - b.g).abs() < 1e-3 && (a.b - b.b).abs() < 1e-3 && (a.a - b.a).abs() < 1e-3
}

#[test]
fn each_variant_and_size_has_its_height_radius_fill_ink_keys_and_role() {
    let t = theme(false);
    for variant in VARIANTS {
        let (fill, ink) = match variant {
            Variant::Primary => (t.accent, t.accent_text_on),
            Variant::Secondary => (t.paper, t.text),
            Variant::Ghost => (Color::TRANSPARENT, t.text_secondary),
            Variant::Danger => (tokens::red_fill(), t.white),
        };
        for (size, h) in SIZES {
            let mut page = Page::new(variant, size, ButtonState::Enabled);
            page.icon = true;
            let rt = render(page);
            let what = format!("{variant:?} {size:?}");
            let b = info(&rt, "b");
            assert_eq!(b.rect[3], h, "{what}: height");
            assert_eq!(b.radius, 6.0, "{what}: radius");
            assert!(close(b.fill, fill), "{what}: fill {:?}, want {fill:?}", b.fill);
            assert_eq!(b.role, "button", "{what}: role");
            assert_eq!(b.label.as_deref(), Some("Add files"), "{what}: label");
            assert!(b.focusable, "{what}: an enabled button takes focus");
            let label = info(&rt, "b-label");
            assert_eq!(label.text.as_deref(), Some("Add files"), "{what}: words");
            let got = label.text_color.expect("the words have a colour");
            assert!(close(got, ink), "{what}: ink {got:?}, want {ink:?}");
            info(&rt, "b-icon");
        }
    }
}

#[test]
fn custom_keys_are_exactly_the_keys_drawn() {
    let mut page = Page::new(Variant::Primary, Size::Md, ButtonState::Enabled);
    page.icon = true;
    page.keys = ButtonKeys {
        root: "go".into(),
        icon: "go-glyph".into(),
        label: "go-words".into(),
    };
    let rt = render(page);
    for key in ["go", "go-glyph", "go-words"] {
        info(&rt, key);
    }
    for key in ["b", "b-icon", "b-label", "go-icon", "go-label"] {
        assert!(find(&rt, key).is_none(), "{key} is drawn");
    }
}

#[test]
fn dim_is_half_strength_and_still_presses() {
    let mut rt = render(Page::new(Variant::Primary, Size::Md, ButtonState::Dim));
    let b = info(&rt, "b");
    assert!((b.opacity - 0.5).abs() < 1e-3, "opacity {}", b.opacity);
    assert!(b.focusable);
    click(&mut rt, "b");
    assert_eq!(rt.state.pressed, 1, "a click presses a dimmed button");
    space_on(&mut rt, "b");
    assert_eq!(rt.state.pressed, 2, "Space presses a dimmed button");
}

#[test]
fn busy_and_disabled_take_no_focus_and_no_press() {
    for (state, opacity) in [(ButtonState::Busy, 1.0), (ButtonState::Disabled, 0.5)] {
        let mut rt = render(Page::new(Variant::Primary, Size::Md, state));
        let b = info(&rt, "b");
        assert!(!b.focusable, "{state:?} takes focus");
        assert!((b.opacity - opacity).abs() < 1e-3, "{state:?}: opacity {}", b.opacity);
        click(&mut rt, "b");
        space_on(&mut rt, "b");
        assert_eq!(rt.state.pressed, 0, "{state:?} ran its press");
    }
}

#[test]
fn natural_width_is_the_laid_out_width() {
    for variant in VARIANTS {
        for (size, _) in SIZES {
            for icon in [false, true] {
                for label in ["Add files", "Keep editing", "OK"] {
                    let mut page = Page::new(variant, size, ButtonState::Enabled);
                    page.icon = icon;
                    page.label = label;
                    let rt = render(page);
                    let got = info(&rt, "b").rect[2];
                    let want = natural_width(label, icon, variant, size);
                    assert!(
                        (got - want).abs() < 0.01,
                        "{variant:?} {size:?} icon {icon} {label:?}: laid out {got}, natural {want}"
                    );
                }
            }
        }
    }
}
