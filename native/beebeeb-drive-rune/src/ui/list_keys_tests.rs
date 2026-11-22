//! Tests: list keyboard handling.

use rune::{Key, KeyEvent, Mods};

use super::*;

const NAMES: [&str; 8] = ["Alpha", "apple", "Banana", "berry", "Cherry", "alder", "Date", "date palm"];

fn keys(multi: bool) -> ListKeys {
    ListKeys { len: NAMES.len(), page: 3, multi }
}

fn at(focus: usize) -> ListCursor {
    ListCursor { focus: Some(focus), anchor: Some(focus), ..ListCursor::default() }
}

fn ev(key: Key, mods: Mods) -> KeyEvent {
    KeyEvent { key, mods }
}

const NONE: Mods = Mods { shift: false, cmd: false, alt: false, ctrl: false, logo: false };
const SHIFT: Mods = Mods { shift: true, ..NONE };
const CTRL: Mods = Mods { ctrl: true, cmd: true, ..NONE };
const ALT: Mods = Mods { alt: true, ..NONE };

fn press(key: Key, mods: Mods, k: &ListKeys, c: &mut ListCursor) -> ListAction {
    on_list_key(&ev(key, mods), k, c, 0, |i| NAMES[i].to_string())
}

fn typed(s: &str, now: u64, k: &ListKeys, c: &mut ListCursor) -> ListAction {
    on_list_key(&ev(Key::Text(s.into()), NONE), k, c, now, |i| NAMES[i].to_string())
}

fn mv(to: usize, select: Select) -> ListAction {
    ListAction::Move { to, select }
}

#[test]
fn up_down_move_one_row_and_stop_at_the_ends() {
    let k = keys(true);
    let mut c = at(3);
    assert_eq!(press(Key::Down, NONE, &k, &mut c), mv(4, Select::Only));
    assert_eq!((c.focus, c.anchor), (Some(4), Some(4)));
    assert_eq!(press(Key::Up, NONE, &k, &mut c), mv(3, Select::Only));
    let mut c = at(0);
    assert_eq!(press(Key::Up, NONE, &k, &mut c), mv(0, Select::Only));
    let mut c = at(7);
    assert_eq!(press(Key::Down, NONE, &k, &mut c), mv(7, Select::Only));
}

#[test]
fn no_focus_starts_at_the_top_and_end_at_the_bottom() {
    let k = keys(false);
    let mut c = ListCursor::default();
    assert_eq!(press(Key::Down, NONE, &k, &mut c), mv(0, Select::Only));
    let mut c = ListCursor::default();
    assert_eq!(press(Key::End, NONE, &k, &mut c), mv(7, Select::Only));
}

#[test]
fn home_end_go_to_the_ends() {
    let k = keys(false);
    let mut c = at(4);
    assert_eq!(press(Key::Home, NONE, &k, &mut c), mv(0, Select::Only));
    assert_eq!(press(Key::End, NONE, &k, &mut c), mv(7, Select::Only));
}

#[test]
fn page_keys_move_a_page_and_stop_at_the_ends() {
    let k = keys(false);
    let mut c = at(1);
    assert_eq!(press(Key::PageDown, NONE, &k, &mut c), mv(4, Select::Only));
    assert_eq!(press(Key::PageDown, NONE, &k, &mut c), mv(7, Select::Only));
    assert_eq!(press(Key::PageDown, NONE, &k, &mut c), mv(7, Select::Only));
    assert_eq!(press(Key::PageUp, NONE, &k, &mut c), mv(4, Select::Only));
    assert_eq!(press(Key::PageUp, NONE, &k, &mut c), mv(1, Select::Only));
    assert_eq!(press(Key::PageUp, NONE, &k, &mut c), mv(0, Select::Only));
}

#[test]
fn page_zero_still_moves_one_row() {
    let k = ListKeys { len: 5, page: 0, multi: false };
    let mut c = at(2);
    assert_eq!(press(Key::PageDown, NONE, &k, &mut c), mv(3, Select::Only));
}

#[test]
fn shift_move_extends_from_the_anchor() {
    let k = keys(true);
    let mut c = at(2);
    assert_eq!(press(Key::Down, SHIFT, &k, &mut c), mv(3, Select::Extend));
    assert_eq!(press(Key::Down, SHIFT, &k, &mut c), mv(4, Select::Extend));
    assert_eq!(c.anchor, Some(2));
    assert_eq!(extend_range(&c, 4), 2..=4);
    assert_eq!(press(Key::Home, SHIFT, &k, &mut c), mv(0, Select::Extend));
    assert_eq!(extend_range(&c, 0), 0..=2);
    assert_eq!(press(Key::PageDown, SHIFT, &k, &mut c), mv(3, Select::Extend));
    assert_eq!(press(Key::End, SHIFT, &k, &mut c), mv(7, Select::Extend));
    assert_eq!(extend_range(&c, 7), 2..=7);
}

#[test]
fn shift_move_without_anchor_starts_at_the_focus() {
    let k = keys(true);
    let mut c = ListCursor { focus: Some(5), anchor: None, ..ListCursor::default() };
    assert_eq!(press(Key::Up, SHIFT, &k, &mut c), mv(4, Select::Extend));
    assert_eq!(c.anchor, Some(5));
}

#[test]
fn ctrl_move_moves_focus_only_and_keeps_the_anchor() {
    let k = keys(true);
    let mut c = at(2);
    assert_eq!(press(Key::Down, CTRL, &k, &mut c), mv(3, Select::Keep));
    assert_eq!(press(Key::End, CTRL, &k, &mut c), mv(7, Select::Keep));
    assert_eq!((c.focus, c.anchor), (Some(7), Some(2)));
}

#[test]
fn single_select_shift_and_ctrl_move_are_plain_moves() {
    let k = keys(false);
    let mut c = at(2);
    assert_eq!(press(Key::Down, SHIFT, &k, &mut c), mv(3, Select::Only));
    assert_eq!(press(Key::Down, CTRL, &k, &mut c), mv(4, Select::Only));
    assert_eq!(c.anchor, Some(4));
    assert_eq!(press(Key::Text(" ".into()), CTRL, &k, &mut c), mv(4, Select::Only));
}

#[test]
fn space_selects_and_ctrl_space_toggles() {
    let k = keys(true);
    let mut c = ListCursor { focus: Some(3), anchor: Some(1), ..ListCursor::default() };
    assert_eq!(press(Key::Text(" ".into()), CTRL, &k, &mut c), ListAction::Toggle(3));
    assert_eq!(c.anchor, Some(3));
    assert_eq!(press(Key::Text(" ".into()), NONE, &k, &mut c), mv(3, Select::Only));
    let mut c = ListCursor { focus: Some(5), anchor: Some(2), ..ListCursor::default() };
    assert_eq!(press(Key::Text(" ".into()), SHIFT, &k, &mut c), mv(5, Select::Extend));
    assert_eq!(extend_range(&c, 5), 2..=5);
    let mut c = ListCursor::default();
    assert_eq!(press(Key::Text(" ".into()), NONE, &k, &mut c), ListAction::None);
}

#[test]
fn command_keys_act_on_the_focused_row() {
    let k = keys(true);
    let mut c = at(6);
    assert_eq!(press(Key::Enter, NONE, &k, &mut c), ListAction::Open(6));
    assert_eq!(press(Key::Function(2), NONE, &k, &mut c), ListAction::Rename(6));
    assert_eq!(press(Key::Delete, NONE, &k, &mut c), ListAction::Delete);
    assert_eq!(press(Key::ContextMenu, NONE, &k, &mut c), ListAction::Menu(6));
    assert_eq!(press(Key::Function(10), SHIFT, &k, &mut c), ListAction::Menu(6));
    assert_eq!(press(Key::Function(10), NONE, &k, &mut c), ListAction::None);
    assert_eq!(press(Key::Up, ALT, &k, &mut c), ListAction::Parent);
    assert_eq!(press(Key::Left, ALT, &k, &mut c), ListAction::None);
    assert_eq!(press(Key::Escape, NONE, &k, &mut c), ListAction::None);
    assert_eq!(c.focus, Some(6));
}

#[test]
fn command_keys_without_focus_do_nothing_but_parent_and_delete() {
    let k = keys(true);
    let mut c = ListCursor::default();
    assert_eq!(press(Key::Enter, NONE, &k, &mut c), ListAction::None);
    assert_eq!(press(Key::Function(2), NONE, &k, &mut c), ListAction::None);
    assert_eq!(press(Key::ContextMenu, NONE, &k, &mut c), ListAction::None);
    assert_eq!(press(Key::Up, ALT, &k, &mut c), ListAction::Parent);
    assert_eq!(press(Key::Delete, NONE, &k, &mut c), ListAction::Delete);
}

#[test]
fn empty_list_moves_nowhere() {
    let k = ListKeys { len: 0, page: 10, multi: true };
    let mut c = at(3);
    assert_eq!(press(Key::Down, NONE, &k, &mut c), ListAction::None);
    assert_eq!(c.focus, None);
    assert_eq!(typed("a", 0, &k, &mut c), ListAction::None);
    assert_eq!(press(Key::Up, ALT, &k, &mut c), ListAction::Parent);
}

#[test]
fn a_list_that_shrank_clamps_the_cursor() {
    let k = ListKeys { len: 3, page: 10, multi: true };
    let mut c = at(7);
    assert_eq!(press(Key::Up, NONE, &k, &mut c), mv(1, Select::Only));
}

#[test]
fn type_ahead_matches_a_case_insensitive_prefix() {
    let k = keys(false);
    let mut c = at(0);
    assert_eq!(typed("B", 0, &k, &mut c), mv(2, Select::Only));
    assert_eq!(typed("e", 100, &k, &mut c), mv(3, Select::Only));
    assert_eq!(c.focus, Some(3));
}

#[test]
fn a_longer_prefix_stays_on_the_row_that_still_matches() {
    let k = keys(false);
    let mut c = at(7);
    assert_eq!(typed("d", 0, &k, &mut c), mv(6, Select::Only));
    assert_eq!(typed("a", 200, &k, &mut c), mv(6, Select::Only));
    assert_eq!(typed("t", 400, &k, &mut c), mv(6, Select::Only));
    assert_eq!(typed("e", 600, &k, &mut c), mv(6, Select::Only));
    assert_eq!(typed(" ", 800, &k, &mut c), mv(7, Select::Only));
}

#[test]
fn a_repeated_letter_cycles_and_wraps() {
    let k = keys(false);
    let mut c = at(2);
    assert_eq!(typed("a", 0, &k, &mut c), mv(5, Select::Only));
    assert_eq!(typed("a", 300, &k, &mut c), mv(0, Select::Only));
    assert_eq!(typed("a", 600, &k, &mut c), mv(1, Select::Only));
    assert_eq!(typed("A", 900, &k, &mut c), mv(5, Select::Only));
}

#[test]
fn the_buffer_resets_after_a_second_without_a_key() {
    let k = keys(false);
    let mut c = at(0);
    assert_eq!(typed("b", 0, &k, &mut c), mv(2, Select::Only));
    // 999 ms later the buffer is "be": berry.
    assert_eq!(typed("e", 999, &k, &mut c), mv(3, Select::Only));
    // 1000 ms after that it starts over: "c".
    assert_eq!(typed("c", 1999, &k, &mut c), mv(4, Select::Only));
    let mut c = at(0);
    typed("b", 0, &k, &mut c);
    // Exactly 1000 ms: new buffer "a", cycling from Banana to alder.
    assert_eq!(typed("a", 1000, &k, &mut c), mv(5, Select::Only));
}

#[test]
fn no_match_leaves_focus_where_it_is() {
    let k = keys(false);
    let mut c = at(4);
    assert_eq!(typed("z", 0, &k, &mut c), ListAction::None);
    assert_eq!(c.focus, Some(4));
    assert_eq!(typed("c", 100, &k, &mut c), ListAction::None);
    assert_eq!(c.focus, Some(4));
}

#[test]
fn space_outside_a_live_buffer_selects_and_another_key_clears_it() {
    let k = keys(false);
    let mut c = at(0);
    typed("d", 0, &k, &mut c);
    press(Key::Down, NONE, &k, &mut c);
    // The buffer was cleared by Down, so Space selects instead of typing.
    assert_eq!(on_list_key(&ev(Key::Text(" ".into()), NONE), &k, &mut c, 100, |i| NAMES[i].into()), mv(7, Select::Only));
}

#[test]
fn ctrl_letters_are_not_type_ahead() {
    let k = keys(true);
    let mut c = at(0);
    assert_eq!(press(Key::Text("b".into()), CTRL, &k, &mut c), ListAction::None);
    assert_eq!(press(Key::Command('a'), CTRL, &k, &mut c), ListAction::None);
    assert_eq!(c.focus, Some(0));
}
