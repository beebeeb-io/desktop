//! Settings on the keyboard layer: F6 between the sidebar and the page, a roving list's keys
//! (type-to-jump, Page Up and Page Down), the Menu key on a pop-up row, and where focus goes
//! when the row that opened a dialog is gone.

use super::{MenuKind, PAGE_PANE, SIDEBAR_PANE, sync};
use crate::shell::host;
use crate::{State, init, view};
use rune::{ElementInfo, Env, Key, KeyEvent, Mods, Runtime};

fn runtime(frame: &str) -> Runtime<State> {
    let mut state = init();
    assert!(super::scene(frame, &mut state), "{frame} is a built frame");
    let env = Env { system_dark: false, scale: 2.0 };
    let mut rt = Runtime::new(state, view, (958.0, 638.0), env);
    rt.on_key = Some(host::on_key);
    rt.poll = Some(host::poll);
    rt.frame(0.0);
    rt
}

fn settle(rt: &mut Runtime<State>) {
    rt.frame(0.016);
    rt.settle_all();
    rt.frame(0.016);
}

fn press(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    settle(rt);
}

fn key(rt: &mut Runtime<State>, k: Key) {
    press(rt, k, Mods::default());
}

fn find(rt: &Runtime<State>, key: &str) -> Option<ElementInfo> {
    rt.elements().into_iter().find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn focused(rt: &Runtime<State>) -> Option<ElementInfo> {
    rt.elements().into_iter().find(|e| e.focused && !e.exiting)
}

fn focused_key(rt: &Runtime<State>) -> Option<String> {
    focused(rt).and_then(|e| e.key)
}

fn focus(rt: &mut Runtime<State>, key: &'static str) {
    rt.update(move |_, tx| tx.focus(key, true));
    settle(rt);
    assert_eq!(focused_key(rt).as_deref(), Some(key), "{key} takes focus");
}

/// Whether the focused element lies inside the element keyed `outer`.
fn focus_inside(rt: &Runtime<State>, outer: &str) -> bool {
    let (Some(f), Some(o)) = (focused(rt), find(rt, outer)) else { return false };
    let (a, b) = (f.rect, o.rect);
    a[0] >= b[0] - 0.5 && a[1] >= b[1] - 0.5 && a[0] + a[2] <= b[0] + b[2] + 0.5 && a[1] + a[3] <= b[1] + b[3] + 0.5
}

#[test]
fn f6_moves_between_the_sidebar_and_the_page_and_back() {
    let mut rt = runtime("DW-0701");
    key(&mut rt, Key::Function(6));
    assert!(focus_inside(&rt, SIDEBAR_PANE), "F6 from nothing: the sidebar, {:?}", focused_key(&rt));
    key(&mut rt, Key::Function(6));
    assert!(focus_inside(&rt, PAGE_PANE), "F6: the page, {:?}", focused_key(&rt));
    key(&mut rt, Key::Function(6));
    assert!(focus_inside(&rt, SIDEBAR_PANE), "F6 wraps to the sidebar, {:?}", focused_key(&rt));
    press(&mut rt, Key::Function(6), Mods { shift: true, ..Mods::default() });
    assert!(focus_inside(&rt, PAGE_PANE), "Shift+F6: back to the page, {:?}", focused_key(&rt));
}

#[test]
fn a_roving_list_is_one_tab_stop_and_jumps_by_its_rows_first_letters() {
    let mut rt = runtime("DW-0701");
    // Help: Show the guide again, Help and report a problem, Diagnostics.
    focus(&mut rt, "item-1-0");
    key(&mut rt, Key::Text("d".into()));
    assert_eq!(focused_key(&rt).as_deref(), Some("item-1-2"), "typing d picks Diagnostics");
    key(&mut rt, Key::PageUp);
    assert_eq!(focused_key(&rt).as_deref(), Some("item-1-0"), "Page Up: the first row");
    key(&mut rt, Key::PageDown);
    assert_eq!(focused_key(&rt).as_deref(), Some("item-1-2"), "Page Down: the last row");
    key(&mut rt, Key::Tab);
    let now = focused_key(&rt).unwrap_or_default();
    assert!(!now.starts_with("item-1-"), "Tab leaves the list in one step, not to {now}");
}

#[test]
fn the_menu_key_and_shift_f10_open_a_pop_up_rows_menu() {
    let mut rt = runtime("DW-0701");
    focus(&mut rt, "item-0-0");
    key(&mut rt, Key::ContextMenu);
    assert_eq!(rt.state.settings.menu.map(|m| m.kind), Some(MenuKind::Theme), "Menu key");
    key(&mut rt, Key::Escape);
    assert!(rt.state.settings.menu.is_none());
    focus(&mut rt, "item-0-0");
    press(&mut rt, Key::Function(10), Mods { shift: true, ..Mods::default() });
    assert_eq!(rt.state.settings.menu.map(|m| m.kind), Some(MenuKind::Theme), "Shift+F10");
}

#[test]
fn turning_off_file_explorer_puts_focus_on_the_list_above_the_row_it_removed() {
    let mut rt = runtime("DW-0746");
    focus(&mut rt, sync::TURN_OFF_ROW);
    key(&mut rt, Key::Text(" ".into()));
    assert!(rt.state.settings.dialog.is_some(), "the row opens the question");
    let turn_off = rt
        .elements()
        .into_iter()
        .find(|e| e.role == "button" && e.label.as_deref() == Some("Turn off") && !e.exiting)
        .and_then(|e| e.key)
        .expect("a Turn off button");
    let r = find(&rt, &turn_off).expect("drawn").rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(&mut rt);
    assert!(rt.state.settings.dialog.is_none(), "the question closed");
    assert!(find(&rt, sync::TURN_OFF_ROW).is_none(), "the row that opened it is gone");
    assert!(
        focus_inside(&rt, sync::SPACE_LIST),
        "focus lands in the list above, not nowhere: {:?}",
        focused_key(&rt)
    );
}

/// The roving group a key's control belongs to: a sidebar item (`<name>-row`), or a numbered row
/// (`item-<L>-<I>`, `device-<I>`) whose group is the key without its number.
fn group(key: &str) -> Option<String> {
    if key.ends_with("-row") {
        return Some("-row".into());
    }
    let (head, last) = key.rsplit_once('-')?;
    last.parse::<usize>().is_ok().then(|| head.to_string())
}

/// Tab to the control keyed `target`; inside its roving group (one Tab stop), the arrows take
/// it the rest of the way. Panics when focus never gets there.
pub(super) fn tab_to(rt: &mut Runtime<State>, target: &str) {
    // Tab alone first: a plain list's every control is a stop (and an arrow on a pop-up row
    // would open its menu).
    for _ in 0..120 {
        if focused_key(rt).as_deref() == Some(target) {
            return;
        }
        key(rt, Key::Tab);
    }
    let want = group(target);
    for _ in 0..160 {
        if focused_key(rt).as_deref() == Some(target) {
            return;
        }
        if want.is_some() && focused_key(rt).and_then(|k| group(&k)) == want {
            for arrow in [Key::Down, Key::Up] {
                for _ in 0..30 {
                    if focused_key(rt).as_deref() == Some(target) {
                        return;
                    }
                    key(rt, arrow.clone());
                }
            }
        }
        key(rt, Key::Tab);
    }
    panic!("Tab never reaches {target}");
}
