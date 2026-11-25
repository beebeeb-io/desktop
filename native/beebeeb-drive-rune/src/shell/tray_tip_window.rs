//! The first-run tray tip (DW-0380) as a Rune window of its own: a small borderless, always-on-top
//! card by the notification area (on the Mac: under the menu-bar b), placed with the flyout's code ([`flyout::position`] on the same
//! [`flyout::Anchor`]). The shell opens it when the guide reaches its first step and hides it when the
//! guide leaves it (`host::Shell::show_tray_tip`). "Got it" and Esc dismiss it: [`TipState::dismissed`]
//! is set and the main window is told (`host::on_tray_tip_dismissed`), which hides this window and
//! moves the guide on to its first tip.
//!
//! The card's words are the guide's (`guide::TRAY_TITLE`, `guide::TRAY_TEXT`).

use rune::{AppHandle, Cx, El, Key, KeyEvent, Tx, col, el, row};

use super::flyout::{self, Anchor};
use super::surfaces::TRAY_TIP_KEY;
use crate::State;
use crate::frames::signin::setup::guide::NEXT_KEY;
#[cfg(not(target_os = "macos"))]
use crate::frames::signin::setup::guide::{TRAY_TEXT, TRAY_TITLE};
use crate::theme::tokens::{self, type_};
use crate::ui::button::{ButtonState, Size, Variant, button, keys};
use crate::ui::kit;

/// The Mac's words: the status item lives in the menu bar, not a notification area.
#[cfg(target_os = "macos")]
pub const TRAY_TITLE: &str = "Beebeeb lives in the menu bar";
#[cfg(target_os = "macos")]
pub const TRAY_TEXT: &str = "Look for the b at the top right of your screen. If your menu bar is crowded, the b may be hidden behind the notch: hold \u{2318} and drag other icons away to make room. You can also allow Beebeeb in System Settings \u{203A} Menu Bar.";

/// The window's size in points: the card (300 wide) with its padding.
#[cfg(not(target_os = "macos"))]
pub const SIZE: (f32, f32) = (332.0, 172.0);
#[cfg(target_os = "macos")]
pub const SIZE: (f32, f32) = (332.0, 236.0);
/// Where the tip's window key is read from (`surfaces::TRAY_TIP_KEY`).
pub const KEY: &str = TRAY_TIP_KEY;

#[derive(Default, Clone)]
pub struct TipState {
    /// "Got it" or Esc was used: the guide goes on to its first tip. The shell sets it back when it
    /// opens the tip again ("Show the guide again").
    pub dismissed: bool,
    /// The main window, told of a dismiss. None in tests and headless renders.
    pub main: Option<AppHandle<State>>,
}

pub fn init() -> TipState {
    TipState::default()
}

/// The tip of the running app: a dismiss reaches the main window through `main`.
pub fn with_main(main: AppHandle<State>) -> TipState {
    TipState {
        dismissed: false,
        main: Some(main),
    }
}

/// The tip's top-left on the Mac, physical pixels: under the status item (`macos_anchor::place_hint`).
#[cfg(target_os = "macos")]
pub fn position(a: &Anchor) -> (i32, i32) {
    use super::flyout::macos_anchor as m;
    let f = |r: flyout::Rect| m::Frame::new(r.x as f64, r.y as f64, r.w as f64, r.h as f64);
    let display = m::Display { frame: f(a.monitor), visible: f(a.work), primary: true };
    let scale = if a.scale.is_finite() && a.scale > 0.0 { a.scale } else { 1.0 };
    match m::place_hint(a.icon.map(f), &[display], f64::from(SIZE.0), f64::from(SIZE.1)) {
        Some(p) => m::to_physical(p.x, p.y, scale),
        None => m::to_physical(f64::from(a.work.x), f64::from(a.work.y), scale),
    }
}

/// The tip's top-left, next to the b: the flyout's rule with the tip's size.
#[cfg(not(target_os = "macos"))]
pub fn position(a: &Anchor) -> (i32, i32) {
    let scale = if a.scale.is_finite() && a.scale > 0.0 { a.scale } else { 1.0 };
    let (w, h) = (
        (f64::from(SIZE.0) * scale).round() as i32,
        (f64::from(SIZE.1) * scale).round() as i32,
    );
    let margin = (f64::from(flyout::MARGIN) * scale).round() as i32;
    let edge = flyout::taskbar_edge(a);
    let x = if edge == flyout::TaskbarEdge::Left {
        a.work.x + margin
    } else {
        a.work.x + a.work.w - margin - w
    };
    let y = if edge == flyout::TaskbarEdge::Top {
        a.work.y + margin
    } else {
        a.work.y + a.work.h - margin - h
    };
    (x.max(a.work.x), y.max(a.work.y))
}

/// Once per showing: the main window hides this one and moves the guide on (`host::on_tray_tip_dismissed`). The
/// window is never closed from here: closing it would end it, not hide it.
fn dismiss(s: &mut TipState, _tx: &mut Tx) {
    if s.dismissed {
        return;
    }
    s.dismissed = true;
    if let Some(main) = &s.main {
        main.update(crate::shell::host::on_tray_tip_dismissed);
    }
}

pub fn view(_s: &TipState, cx: &mut Cx<TipState>) -> El {
    crate::ui::focus::install(cx);
    let t = tokens::theme(cx.system_dark());
    col()
        .key("tray-tip")
        .role("group")
        .label(TRAY_TITLE)
        .w_full()
        .h_full()
        .gap(8.0)
        .padding(16.0)
        .fill(t.surface)
        .child(kit::label("coach-title", TRAY_TITLE, type_::HEAD, t.text))
        .child(
            kit::label("coach-text", TRAY_TEXT, type_::BODY, t.text_tertiary)
                .wrap()
                .w_full(),
        )
        .child(
            row().justify_end().child(
                button(
                    keys(NEXT_KEY),
                    "Got it",
                    None,
                    Variant::Primary,
                    Size::Sm,
                    ButtonState::Enabled,
                    &t,
                )
                .on_press_tx(dismiss),
            ),
        )
        .child(el())
}

/// Esc dismisses it, as "Got it" does.
pub fn on_key(s: &mut TipState, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if ev.key == Key::Escape {
        dismiss(s, tx);
        return true;
    }
    false
}

#[cfg(test)]
mod tray_tip_tests {
    use super::*;
    use flyout::{Anchor, Rect};

    fn key(key: Key) -> KeyEvent {
        KeyEvent { key, mods: rune::Mods::default() }
    }

    fn anchor(work: (i32, i32, i32, i32)) -> Anchor {
        let monitor = Rect { x: 0, y: 0, w: 1920, h: 1080 };
        Anchor {
            monitor,
            work: Rect { x: work.0, y: work.1, w: work.2, h: work.3 },
            scale: 1.0,
            icon: None,
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn hint_macos_words_have_no_windows_words() {
        for t in [TRAY_TITLE, TRAY_TEXT] {
            let l = t.to_lowercase();
            for bad in ["windows", "taskbar", "notification area"] {
                assert!(!l.contains(bad), "{t}");
            }
        }
        assert!(TRAY_TEXT.contains("menu bar") || TRAY_TITLE.contains("menu bar"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn hint_macos_sits_under_the_status_item_inside_the_visible_frame() {
        let mut a = anchor((0, 38, 1512, 944));
        a.monitor = Rect { x: 0, y: 0, w: 1512, h: 982 };
        a.icon = Some(Rect { x: 1300, y: 0, w: 24, h: 24 });
        let (x, y) = position(&a);
        let (w, h) = (SIZE.0 as i32, SIZE.1 as i32);
        assert!(y >= 24, "below the icon: {y}");
        assert!(x <= 1312 && 1312 <= x + w, "under the icon: {x}");
        assert!(x >= 0 && x + w <= 1512 && y >= 38 && y + h <= 38 + 944);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn hint_macos_icon_at_the_right_edge_is_clamped_and_never_bottom_right() {
        let mut a = anchor((0, 38, 1512, 944));
        a.monitor = Rect { x: 0, y: 0, w: 1512, h: 982 };
        a.icon = Some(Rect { x: 1490, y: 0, w: 20, h: 24 });
        let (x, y) = position(&a);
        assert_eq!(x, 1512 - 8 - SIZE.0 as i32);
        assert!(y < 100);
        a.icon = None;
        assert!(position(&a).1 < 100);
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn tray_tip_sits_bottom_right_above_a_bottom_taskbar() {
        let (x, y) = position(&anchor((0, 0, 1920, 1040)));
        assert_eq!((x, y), (1920 - 12 - 332, 1040 - 12 - 172));
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn tray_tip_goes_top_right_under_a_top_taskbar() {
        let (x, y) = position(&anchor((0, 40, 1920, 1040)));
        assert_eq!((x, y), (1920 - 12 - 332, 40 + 12));
    }

    #[test]
    fn tray_tip_esc_dismisses_and_other_keys_do_not() {
        let mut s = init();
        assert!(!s.dismissed);
        let mut tx = Tx::default();
        assert!(!on_key(&mut s, &key(Key::Enter), &mut tx));
        assert!(!s.dismissed);
        assert!(on_key(&mut s, &key(Key::Escape), &mut tx));
        assert!(s.dismissed);
    }
}
