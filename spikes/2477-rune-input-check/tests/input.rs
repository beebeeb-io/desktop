//! The probe driven through Rune's own headless `Runtime`, the way Rune's tests drive it: synthetic keys,
//! input-method events, pointer events and an in-memory clipboard. Nothing here calls a probe handler directly:
//! an assertion can only hold if Rune delivered the event to the app-drawn surface.
//!
//! This proves Rune's dispatch above the window system. It proves nothing about Win32, a real input method or
//! the OS clipboard: those are the real-window legs (`win/run.ps1`).

use rune::clipboard::{MemoryClipboard, memory};
use rune::{ClipboardContent, Env, Ime, Key, KeyEvent, Mods, Runtime};
use rune_input_check::{self as probe, CELL, LINE, PAD, Probe, WINDOW};

fn runtime_at(scale: f32) -> (Runtime<Probe>, MemoryClipboard) {
    let mut rt = probe::app(probe::spec()).headless_runtime(WINDOW, Env::new(false, scale));
    let clipboard = memory();
    rt.use_clipboard(&clipboard);
    rt.frame(0.0);
    (rt, clipboard)
}

fn runtime() -> (Runtime<Probe>, MemoryClipboard) {
    runtime_at(1.0)
}

/// Frames enough for queued input, the next frame's `poll` and layout callbacks to run.
fn settle(rt: &mut Runtime<Probe>) {
    for _ in 0..4 {
        rt.frame(1.0 / 60.0);
    }
}

fn rect(rt: &Runtime<Probe>, key: &str) -> Option<[f32; 4]> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .map(|e| e.rect)
}

fn focused(rt: &Runtime<Probe>, key: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.key.as_deref() == Some(key) && e.focused)
}

/// The window point in front of `cell` on the canvas's line.
fn cell_point(rt: &Runtime<Probe>, cell: usize) -> (f32, f32) {
    let canvas = rect(rt, "canvas").expect("canvas");
    (canvas[0] + PAD + cell as f32 * CELL, canvas[1] + PAD + LINE / 2.0)
}

fn click(rt: &mut Runtime<Probe>, x: f32, y: f32) {
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}

fn click_cell(rt: &mut Runtime<Probe>, cell: usize) {
    let (x, y) = cell_point(rt, cell);
    click(rt, x, y);
}

fn click_key(rt: &mut Runtime<Probe>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("{key}"));
    click(rt, x, y);
}

fn press(rt: &mut Runtime<Probe>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    settle(rt);
}

fn chord(rt: &mut Runtime<Probe>, c: char) {
    rt.key(KeyEvent {
        key: Key::Command(c),
        mods: Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        },
    });
    settle(rt);
}

fn type_text(rt: &mut Runtime<Probe>, s: &str) {
    for ch in s.chars() {
        press(rt, Key::Text(ch.to_string()));
    }
}

fn ime(rt: &mut Runtime<Probe>, ev: Ime) {
    rt.ime(ev);
    settle(rt);
}

fn lines<'a>(rt: &'a Runtime<Probe>, prefix: &str) -> Vec<&'a str> {
    rt.state
        .log
        .iter()
        .map(String::as_str)
        .filter(|l| l.starts_with(prefix))
        .collect()
}

/// The caret rectangle the probe draws, from its own model: what the candidate window must sit at.
fn caret_rect(rt: &Runtime<Probe>) -> [f32; 4] {
    let (x, y) = cell_point(rt, rt.state.caret_cell());
    [x, y - LINE / 2.0, 2.0, LINE]
}

// 1. Key presses and committed text.

#[test]
fn keys_and_typed_text_reach_the_canvas_exactly_once() {
    let (mut rt, _) = runtime();
    click_cell(&mut rt, 0);
    type_text(&mut rt, "ab");
    press(&mut rt, Key::Left);
    type_text(&mut rt, "c");
    assert_eq!(
        rt.state.text(),
        "acb",
        "typed text is inserted at the caret, each character once"
    );
    press(&mut rt, Key::Backspace);
    press(&mut rt, Key::End);
    press(&mut rt, Key::Delete);
    assert_eq!((rt.state.text(), rt.state.caret), ("ab".to_string(), 2));
    assert_eq!(
        lines(&rt, "key ").len(),
        7,
        "one handler call per key: {:?}",
        rt.state.log
    );
    assert_eq!(
        lines(&rt, "ime ").len(),
        0,
        "plain typing is keys, never an input-method commit"
    );
    assert_eq!(
        rt.state.field.text, "",
        "the Rune text field beside the canvas heard none of it"
    );
}

#[test]
fn shift_and_arrows_select_and_typing_replaces_the_selection() {
    let (mut rt, _) = runtime();
    click_cell(&mut rt, 0);
    type_text(&mut rt, "hello");
    rt.key(KeyEvent {
        key: Key::Home,
        mods: Mods {
            shift: true,
            ..Mods::default()
        },
    });
    settle(&mut rt);
    assert_eq!(rt.state.selected(), "hello", "the key's modifiers arrive with it");
    let canvas = rect(&rt, "canvas").unwrap();
    assert_eq!(
        rect(&rt, "selection"),
        Some([canvas[0] + PAD, canvas[1] + PAD, 5.0 * CELL, LINE])
    );
    type_text(&mut rt, "x");
    assert_eq!(rt.state.text(), "x");
    assert_eq!(rect(&rt, "selection"), None);
}

// 2. Composition: start, update with the pre-edit string and its caret, commit, cancel.

#[test]
fn a_composition_starts_updates_with_its_cursor_and_commits_into_the_text() {
    let (mut rt, _) = runtime();
    click_cell(&mut rt, 0);
    type_text(&mut rt, "x");
    ime(&mut rt, Ime::Enabled);
    ime(&mut rt, Ime::Preedit("か".into(), Some((3, 3))));
    assert_eq!(
        (rt.state.text(), rt.state.preedit.as_str()),
        ("x".to_string(), "か"),
        "the pre-edit is not text yet"
    );
    let canvas = rect(&rt, "canvas").unwrap();
    assert_eq!(
        rect(&rt, "preedit"),
        Some([canvas[0] + PAD + CELL, canvas[1] + PAD + LINE - 2.0, CELL, 2.0]),
        "underlined at the caret"
    );
    assert_eq!(
        rt.state.caret_cell(),
        2,
        "the input method's cursor is after its one character"
    );

    // An update: two characters, the input method's cursor moved to the front (bytes, not characters).
    ime(&mut rt, Ime::Preedit("かな".into(), Some((0, 0))));
    assert_eq!(
        (
            rt.state.preedit.as_str(),
            rt.state.preedit_cursor,
            rt.state.caret_cell()
        ),
        ("かな", Some((0, 0)), 1)
    );
    assert_eq!(rect(&rt, "preedit").map(|r| r[2]), Some(2.0 * CELL));
    ime(&mut rt, Ime::Preedit("かな".into(), Some((3, 6))));
    assert_eq!(rt.state.caret_cell(), 2, "a byte offset of 3 is one character in");

    // Commit, as winit's Windows backend sends it: the pre-edit emptied, then the result.
    rt.ime(Ime::Preedit(String::new(), None));
    ime(&mut rt, Ime::Commit("仮名".into()));
    assert_eq!(
        (rt.state.text(), rt.state.preedit.as_str(), rt.state.caret),
        ("x仮名".to_string(), "", 3)
    );
    assert_eq!(rect(&rt, "preedit"), None);
    assert_eq!(
        lines(&rt, "ime "),
        [
            "ime Enabled",
            "ime Preedit(\"か\", Some((3, 3)))",
            "ime Preedit(\"かな\", Some((0, 0)))",
            "ime Preedit(\"かな\", Some((3, 6)))",
            "ime Preedit(\"\", None)",
            "ime Commit(\"仮名\")",
        ]
    );
    assert_eq!(rt.state.field.text, "");
}

#[test]
fn a_cancelled_composition_leaves_the_text_alone_and_return_is_the_input_methods_meanwhile() {
    let (mut rt, _) = runtime();
    click_cell(&mut rt, 0);
    type_text(&mut rt, "x");
    ime(&mut rt, Ime::Preedit("か".into(), None));
    assert_eq!(
        rt.state.caret_cell(),
        2,
        "no cursor range: the caret goes after the pre-edit"
    );
    let keys = lines(&rt, "key ").len();
    // Return and Escape while composing belong to the input method: the canvas's `on_key` must not hear them.
    press(&mut rt, Key::Enter);
    press(&mut rt, Key::Escape);
    assert_eq!(lines(&rt, "key ").len(), keys, "{:?}", rt.state.log);
    // Cancel: an empty pre-edit and no commit.
    ime(&mut rt, Ime::Preedit(String::new(), None));
    assert_eq!(
        (rt.state.text(), rt.state.preedit.as_str(), rt.state.caret_cell()),
        ("x".to_string(), "", 1)
    );
    assert_eq!(rect(&rt, "preedit"), None);
    press(&mut rt, Key::Enter);
    assert_eq!(
        lines(&rt, "key ").len(),
        keys + 1,
        "after the composition ended, Return is a key again"
    );
}

#[test]
fn a_composition_replaces_the_selection_and_losing_the_window_ends_it() {
    let (mut rt, _) = runtime();
    click_cell(&mut rt, 0);
    type_text(&mut rt, "abc");
    chord(&mut rt, 'a');
    ime(&mut rt, Ime::Preedit("に".into(), Some((3, 3))));
    assert_eq!((rt.state.text(), rt.state.preedit.as_str()), (String::new(), "に"));
    // What the window does when it is deactivated.
    rt.cancel_composition();
    settle(&mut rt);
    assert_eq!((rt.state.text(), rt.state.preedit.as_str()), (String::new(), ""));
    assert_eq!(lines(&rt, "ime ").last(), Some(&"ime Preedit(\"\", None)"));
}

// 3. The candidate-window position: Rune's `ime_area` is what the window hands the platform.

#[test]
fn the_input_method_area_is_the_drawn_caret_and_follows_it() {
    let (mut rt, _) = runtime();
    assert_eq!(rt.ime_area, None, "no caret drawn, no input method");
    click_cell(&mut rt, 0);
    assert_eq!(
        rt.ime_area,
        Some(caret_rect(&rt)),
        "on at the caret once the canvas has the keyboard"
    );
    type_text(&mut rt, "abc");
    let after_typing = rt.ime_area.expect("ime area");
    assert_eq!(after_typing, caret_rect(&rt));
    assert_eq!(after_typing[0], cell_point(&rt, 3).0, "three cells along");
    press(&mut rt, Key::Home);
    assert_eq!(rt.ime_area.map(|a| a[0]), Some(cell_point(&rt, 0).0));
    // During a composition it follows the input method's own cursor.
    ime(&mut rt, Ime::Preedit("かな".into(), Some((3, 3))));
    assert_eq!(rt.ime_area.map(|a| a[0]), Some(cell_point(&rt, 1).0));
    assert_eq!(
        rt.state.caret_drawn, rt.ime_area,
        "`on_drawn_layout` reports the same rectangle to the app"
    );
}

#[test]
fn the_input_method_area_is_in_points_at_any_scale() {
    let (mut one, _) = runtime_at(1.0);
    let (mut two, _) = runtime_at(2.0);
    for rt in [&mut one, &mut two] {
        click_cell(rt, 0);
        type_text(rt, "ab");
    }
    assert_eq!(one.ime_area, Some(caret_rect(&one)));
    assert_eq!(
        two.ime_area, one.ime_area,
        "the same rectangle in points on a 200% display"
    );
}

#[test]
fn a_focused_text_field_takes_the_input_method_from_the_canvas() {
    let (mut rt, _) = runtime();
    click_cell(&mut rt, 0);
    let canvas_area = rt.ime_area.expect("the canvas's caret");
    click_key(&mut rt, "field");
    let field_area = rt.ime_area.expect("the field's caret");
    assert!(
        field_area[1] < canvas_area[1],
        "the field is above the canvas: {field_area:?} {canvas_area:?}"
    );
    ime(&mut rt, Ime::Commit("猫".into()));
    assert_eq!((rt.state.field.text.as_str(), rt.state.text()), ("猫", String::new()));
    assert_eq!(
        lines(&rt, "ime Commit").len(),
        0,
        "the app's hook does not hear a field's commit"
    );
}

// 4. Clipboard.

#[test]
fn copy_writes_text_and_html_and_paste_reads_what_another_app_put_there() {
    let (mut rt, clipboard) = runtime();
    click_cell(&mut rt, 0);
    type_text(&mut rt, "hello");
    chord(&mut rt, 'a');
    chord(&mut rt, 'c');
    assert_eq!(clipboard.text().as_deref().map(String::as_str), Some("hello"));
    assert_eq!(
        clipboard.content().and_then(|c| c.html().map(str::to_owned)).as_deref(),
        Some("<b>hello</b>"),
        "the HTML form is written beside the text"
    );
    // Another app changes the clipboard; the canvas pastes it at the caret.
    press(&mut rt, Key::End);
    clipboard.set_text(" world");
    chord(&mut rt, 'v');
    assert_eq!(rt.state.text(), "hello world");
    assert_eq!(
        lines(&rt, "paste "),
        ["paste text=\" world\" html=false rtf=false image=false files=0"]
    );
}

#[test]
fn cut_removes_the_selection_and_a_rich_paste_carries_its_other_forms() {
    let (mut rt, clipboard) = runtime();
    click_cell(&mut rt, 0);
    type_text(&mut rt, "abc");
    chord(&mut rt, 'a');
    chord(&mut rt, 'x');
    assert_eq!(
        (rt.state.text(), clipboard.text().as_deref().map(String::as_str)),
        (String::new(), Some("abc"))
    );
    clipboard.set_content(ClipboardContent::new("R").with_html("<i>R</i>").with_rtf("{\\rtf1 R}"));
    chord(&mut rt, 'v');
    assert_eq!(rt.state.text(), "R");
    assert_eq!(
        lines(&rt, "paste "),
        ["paste text=\"R\" html=true rtf=true image=false files=0"]
    );
}

// 5. Focus and the pointer.

#[test]
fn the_canvas_takes_the_keyboard_on_a_press_and_loses_it_to_tab_and_to_a_click_elsewhere() {
    let (mut rt, _) = runtime();
    assert!(!rt.state.has_keyboard && rect(&rt, "caret").is_none());
    type_text(&mut rt, "z");
    assert_eq!(rt.state.text(), "", "without focus the canvas hears no key");

    click_cell(&mut rt, 0);
    assert!(focused(&rt, "canvas") && rt.state.has_keyboard && rect(&rt, "caret").is_some());
    type_text(&mut rt, "a");

    press(&mut rt, Key::Tab);
    assert!(
        !focused(&rt, "canvas") && !rt.state.has_keyboard && rect(&rt, "caret").is_none(),
        "Tab moved on"
    );
    // Tab goes round to the Rune text field, whose own caret has the input method now.
    assert!(focused(&rt, "field"));
    assert!(
        rt.ime_area.is_some_and(|a| a[1] < rect(&rt, "canvas").unwrap()[1]),
        "{:?}",
        rt.ime_area
    );
    type_text(&mut rt, "z");
    assert_eq!((rt.state.text(), rt.state.field.text.as_str()), ("a".to_string(), "z"));

    click_cell(&mut rt, 1);
    assert!(rt.state.has_keyboard);
    click_key(&mut rt, "other");
    assert!(
        focused(&rt, "other") && !rt.state.has_keyboard && rect(&rt, "caret").is_none(),
        "another control took focus"
    );
    assert_eq!(rt.ime_area, None, "no caret anywhere: the input method is off");
    assert_eq!(
        lines(&rt, "focus "),
        [
            "focus canvas=true",
            "focus canvas=false",
            "focus canvas=true",
            "focus canvas=false"
        ]
    );
}

#[test]
fn the_window_losing_and_gaining_focus_is_heard() {
    let (mut rt, _) = runtime();
    rt.window_focus(true);
    settle(&mut rt);
    rt.window_focus(false);
    settle(&mut rt);
    assert_eq!(lines(&rt, "window "), ["window focus=true", "window focus=false"]);
    assert!(!rt.state.window_focused);
}

#[test]
fn a_press_places_the_caret_and_a_drag_selects_in_the_canvas_own_coordinates() {
    let (mut rt, _) = runtime();
    click_cell(&mut rt, 0);
    type_text(&mut rt, "hello");
    click_cell(&mut rt, 2);
    assert_eq!(
        (rt.state.caret, rt.state.anchor),
        (2, 2),
        "the hit-test got the canvas's own x"
    );

    rt.state.log.clear();
    let (from, to) = (cell_point(&rt, 1), cell_point(&rt, 4));
    rt.pointer_move(from.0, from.1);
    rt.pointer_down(from.0, from.1);
    rt.pointer_move((from.0 + to.0) / 2.0, to.1);
    rt.pointer_move(to.0, to.1);
    rt.pointer_up(to.0, to.1);
    settle(&mut rt);
    assert_eq!((rt.state.selection(), rt.state.selected()), ((1, 4), "ell".to_string()));
    let pointer = lines(&rt, "pointer ");
    let y = PAD + LINE / 2.0;
    assert_eq!(
        pointer.first(),
        Some(&format!("pointer Down x={:.0} y={y:.0} clicks=1 shift=false", PAD + CELL).as_str())
    );
    assert_eq!(
        pointer.last(),
        Some(&format!("pointer Up x={:.0} y={y:.0} clicks=1 shift=false", PAD + 4.0 * CELL).as_str())
    );
    assert!(
        pointer.iter().filter(|l| l.starts_with("pointer Move")).count() >= 2,
        "{pointer:?}"
    );
}

#[test]
fn a_second_press_in_place_is_a_double_click() {
    let (mut rt, _) = runtime();
    click_cell(&mut rt, 0);
    type_text(&mut rt, "hello");
    let (x, y) = cell_point(&rt, 2);
    rt.pointer_move(x, y);
    for _ in 0..2 {
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
    }
    settle(&mut rt);
    assert_eq!(rt.state.selected(), "hello", "{:?}", rt.state.log);
    assert!(lines(&rt, "pointer Down").iter().any(|l| l.contains("clicks=2")));
}
