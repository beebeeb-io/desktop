//! The one keyboard focus ring: 2 pt wide, 2 pt off the border box, in `accent_deep`
//! ([`RING_WIDTH`], [`RING_OFFSET`]). `ui/button.rs` and the other controls that name a ring
//! use [`apply`]; [`install`] sets it as the view's default so a focusable control that names
//! none (a row, a link, a card, a viewer control) draws the same ring, not AppKit's.
//!
//! The window's panes (the sidebar, the toolbar, the list, the details pane) are F6 stops:
//! each region wraps itself in [`pane`], and F6 / Shift+F6 move focus to the next / previous
//! one in the order the view built them, wrapping at the ends. A pane's own handler moves on
//! while focus is inside it; [`cycle`] (one arm in `shell/host.rs`) moves on from nowhere or
//! from a control outside every pane.

use std::cell::RefCell;
use std::rc::Rc;

use rune::{Color, Cx, El, Key, KeyEvent, Tx};

use crate::theme::tokens;

pub const RING_WIDTH: f32 = 2.0;
pub const RING_OFFSET: f32 = 2.0;

/// The ring on one element.
pub fn apply(el: El, color: Color) -> El {
    el.focus_ring(RING_WIDTH, RING_OFFSET, color)
}

/// The view's default ring. Call it at the top of every window view, on every run.
pub fn install<S>(cx: &mut Cx<S>) {
    cx.focus_ring(RING_WIDTH, RING_OFFSET, tokens::theme(cx.system_dark()).accent_deep);
    // This run's panes start empty; the last run's list stays with the panes it built.
    PANES.with(|p| *p.borrow_mut() = Rc::default());
}

/// The panes of one view run, in the order it built them.
type Panes = Rc<RefCell<Vec<&'static str>>>;

thread_local! {
    /// The list the current view run adds to: [`install`] starts a new one.
    static PANES: RefCell<Panes> = RefCell::default();
    /// The last view run's list that held a pane, for [`cycle`] (focus outside every pane).
    static LAST: RefCell<Panes> = RefCell::default();
}

/// F6 or Shift+F6 with no other modifier: `Some(back)`.
pub fn f6(ev: &KeyEvent) -> Option<bool> {
    let m = ev.mods;
    (ev.key == Key::Function(6) && !m.alt && !m.cmd).then_some(m.shift)
}

/// Marks `el` as an F6 stop keyed `key`. F6 / Shift+F6 while focus is inside it focus the
/// next / previous pane's first Tab stop, with the ring.
pub fn pane(el: El, key: &'static str) -> El {
    let panes = PANES.with(|p| p.borrow().clone());
    {
        let mut list = panes.borrow_mut();
        if !list.contains(&key) {
            list.push(key);
        }
    }
    LAST.with(|l| *l.borrow_mut() = panes.clone());
    el.key(key).on_key(move |_: &mut crate::State, ev: &KeyEvent, tx: &mut Tx| {
        let Some(back) = f6(ev) else { return false };
        let list = panes.borrow();
        if let Some(to) = step(&list, Some(key), back) {
            tx.focus(to, true);
        }
        true
    })
}

/// The pane after (`back`: before) `from`, wrapping; from none, the first (`back`: the last).
fn step(list: &[&'static str], from: Option<&str>, back: bool) -> Option<&'static str> {
    let n = list.len();
    if n == 0 {
        return None;
    }
    let i = match from.and_then(|f| list.iter().position(|k| *k == f)) {
        Some(i) if back => (i + n - 1) % n,
        Some(i) => (i + 1) % n,
        None if back => n - 1,
        None => 0,
    };
    Some(list[i])
}

/// F6 from outside every pane: the first pane (Shift+F6: the last). False for any other key,
/// or when the window has no pane.
pub fn cycle(ev: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(back) = f6(ev) else { return false };
    let to = LAST.with(|l| step(&l.borrow().borrow(), None, back));
    match to {
        Some(to) => {
            tx.focus(to, true);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Env, Mods, Runtime, col, el, row};

    #[test]
    fn the_cycle_wraps_both_ways_and_starts_at_an_end() {
        let list = ["a", "b", "c"];
        assert_eq!(step(&list, Some("a"), false), Some("b"));
        assert_eq!(step(&list, Some("c"), false), Some("a"));
        assert_eq!(step(&list, Some("a"), true), Some("c"));
        assert_eq!(step(&list, None, false), Some("a"));
        assert_eq!(step(&list, None, true), Some("c"));
        assert_eq!(step(&[], None, false), None);
    }

    /// The Files window's frame: sidebar left, then toolbar over the list, details right,
    /// a status bar that is no pane. Each pane holds two Tab stops.
    fn files(_: &crate::State, cx: &mut Cx<crate::State>) -> El {
        install(cx);
        let stops = |k: &str| {
            col()
                .child(el().key(format!("{k}-1")).size(40.0, 20.0).focusable(true))
                .child(el().key(format!("{k}-2")).size(40.0, 20.0).focusable(true))
        };
        let window = col()
            .w_full()
            .h_full()
            .child(
                row()
                    .grow(1.0)
                    .child(pane(stops("side"), "sidebar"))
                    .child(
                        col()
                            .grow(1.0)
                            .child(pane(stops("tool"), "toolbar"))
                            .child(pane(stops("row"), "file-list")),
                    )
                    .child(pane(stops("info"), "detail-pane")),
            )
            .child(el().key("status").size(40.0, 20.0).focusable(true));
        window.on_key(|_: &mut crate::State, ev: &KeyEvent, tx: &mut Tx| cycle(ev, tx))
    }

    fn focused(rt: &Runtime<crate::State>) -> Option<String> {
        rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key)
    }

    fn key(rt: &mut Runtime<crate::State>, key: Key, shift: bool) {
        rt.key(KeyEvent { key, mods: Mods { shift, ..Mods::default() } });
        rt.frame(0.0);
        rt.settle_all();
        rt.frame(0.0);
    }

    #[test]
    fn f6_walks_the_files_window_panes_in_order_and_shift_f6_walks_back() {
        let env = Env { system_dark: false, scale: 2.0 };
        let mut rt = Runtime::new(crate::init(), files, (800.0, 600.0), env);
        // Nothing focused: the app's key handler hears F6 (the shell's arm in `shell/host.rs`).
        rt.on_key = Some(|_, ev, tx| cycle(ev, tx));
        rt.frame(0.0);
        for want in ["side-1", "tool-1", "row-1", "info-1", "side-1"] {
            key(&mut rt, Key::Function(6), false);
            assert_eq!(focused(&rt).as_deref(), Some(want), "F6");
        }
        for want in ["info-1", "row-1", "tool-1", "side-1", "info-1"] {
            key(&mut rt, Key::Function(6), true);
            assert_eq!(focused(&rt).as_deref(), Some(want), "Shift+F6");
        }
        // From a control inside a pane (Tab moved there), F6 goes on from that pane.
        key(&mut rt, Key::Function(6), false);
        key(&mut rt, Key::Tab, false);
        assert_eq!(focused(&rt).as_deref(), Some("side-2"));
        key(&mut rt, Key::Function(6), false);
        assert_eq!(focused(&rt).as_deref(), Some("tool-1"));
        // From a control in no pane, F6 starts at the first.
        rt.update(|_, tx| tx.focus("status", true));
        rt.frame(0.0);
        key(&mut rt, Key::Function(6), false);
        assert_eq!(focused(&rt).as_deref(), Some("side-1"));
        assert!(rt.elements().into_iter().any(|e| e.focused && e.focus_visible), "with the ring");
    }

    fn view(_: &(), cx: &mut Cx<()>) -> El {
        install(cx);
        // A control that names no ring, and one that names the shared one.
        col()
            .child(el().key("bare").size(40.0, 20.0).focusable(true))
            .child(apply(el().key("named").size(40.0, 20.0).focusable(true), tokens::theme(false).accent_deep))
    }

    #[test]
    fn a_control_with_no_ring_of_its_own_shows_focus_from_the_keyboard_as_a_named_one_does() {
        let env = Env { system_dark: false, scale: 2.0 };
        let mut rt = Runtime::new((), view, (200.0, 100.0), env);
        for key in ["bare", "named"] {
            rt.update(move |_, tx| tx.focus(key, true));
            rt.frame(0.0);
            rt.settle_all();
            rt.frame(0.0);
            let e = rt.elements().into_iter().find(|e| e.key.as_deref() == Some(key)).expect(key);
            assert!(e.focused && e.focus_visible, "{key}: the ring shows");
        }
    }
}
