//! Trash on a source that answers late, as the engine's does (`engine_host::trash`): the states the view draws, the
//! notice, the controls that have no engine call ("not available yet", never a silent nothing), and the selection
//! that follows its ITEM when the list re-sorts under it. The source is a stub here: the engine's own is tested in
//! `engine_host::trash::tests`.

use std::cell::RefCell;
use std::rc::Rc;

use super::source::{Emptying, Loaded, Notice, Pumped, TrashSource, TrashTotals};
use super::{Intent, TrashUi};
use crate::State;
use crate::frames::shared::parts::table::{Extra, Kind, Row};
use crate::frames::shared::tests::{alt, click, info, press, press_with, runtime, settle, space};
use crate::shell::app::Section;
use rune::{Key, Runtime};

/// What the stub holds; a test changes it between polls.
#[derive(Default)]
struct Held {
    rows: Vec<Row>,
    loaded: Loaded,
    notice: Option<Notice>,
    restored: Vec<String>,
    /// How often the window told the source to forget.
    forgot: usize,
    /// How often the window told the source the section opened.
    opened: usize,
}

struct Stub(Rc<RefCell<Held>>);

impl TrashSource for Stub {
    fn items(&self) -> Vec<Row> {
        self.0.borrow().rows.clone()
    }
    fn totals(&self) -> TrashTotals {
        TrashTotals {
            items: self.0.borrow().rows.len(),
            size: "1 KB".into(),
        }
    }
    fn emptying(&self) -> Option<Emptying> {
        None
    }
    fn restore(&mut self, id: &str) -> bool {
        let mut held = self.0.borrow_mut();
        held.restored.push(id.to_string());
        let before = held.rows.len();
        held.rows.retain(|row| row.id != id);
        held.rows.len() != before
    }
    fn loaded(&self) -> Loaded {
        self.0.borrow().loaded
    }
    fn pump(&mut self) -> Pumped {
        Pumped {
            changed: true,
            busy: false,
            notice: self.0.borrow_mut().notice.take(),
        }
    }
    fn count(&self) -> Option<usize> {
        None
    }
    fn forget(&mut self) {
        let mut held = self.0.borrow_mut();
        held.forgot += 1;
        held.rows.clear();
    }
    fn opened(&mut self) {
        self.0.borrow_mut().opened += 1;
    }
    fn actions_available(&self) -> bool {
        false
    }
}

fn row(id: &str, name: &str) -> Row {
    Row {
        id: id.into(),
        name: name.into(),
        kind: Kind::Document,
        date: "2 Nov 2026".into(),
        size: "1 KB".into(),
        extra: Extra::Words(String::new()),
        pc: None,
    }
}

thread_local! {
    static HELD: RefCell<Option<Rc<RefCell<Held>>>> = const { RefCell::new(None) };
}

fn live(s: &mut State) {
    crate::frames::scene(s, Section::Trash);
    let held = HELD.with(|h| h.borrow().clone()).expect("the test's stub");
    s.frames.trash = TrashUi::on(Box::new(Stub(held)));
}

/// A Trash window on a stub holding `held`.
fn window(held: Held) -> (Runtime<State>, Rc<RefCell<Held>>) {
    let held = Rc::new(RefCell::new(held));
    HELD.with(|h| *h.borrow_mut() = Some(held.clone()));
    let mut rt = runtime(live);
    poll(&mut rt);
    (rt, held)
}

fn poll(rt: &mut Runtime<State>) {
    rt.update(|s, tx| {
        super::poll(s, tx);
    });
    settle(rt);
}

fn three() -> Vec<Row> {
    vec![row("a", "Alpha.pdf"), row("b", "Bravo.pdf"), row("c", "Charlie.pdf")]
}

#[test]
fn every_state_of_a_list_that_is_not_here_is_drawn_and_none_says_trash_is_empty() {
    for (loaded, title) in [
        (Loaded::Loading, "Loading Trash\u{2026}"),
        (Loaded::Offline, "Can\u{2019}t reach Beebeeb"),
        (Loaded::Failed, "Couldn\u{2019}t load Trash"),
        (Loaded::SessionEnded, "Your session ended"),
        (Loaded::SignedOut, "Signed out"),
    ] {
        let (rt, _) = window(Held {
            loaded,
            ..Held::default()
        });
        assert_eq!(info(&rt, "empty-title").text.as_deref(), Some(title), "{loaded:?}");
        assert!(rt.find("btn-empty-trash").is_none(), "{loaded:?}: nothing to empty");
        assert!(rt.find("Trash-badge").is_none(), "{loaded:?}: no count");
    }
    let (rt, _) = window(Held::default());
    assert_eq!(
        info(&rt, "empty-title").text.as_deref(),
        Some("Trash is empty"),
        "a list read as empty"
    );
}

#[test]
fn a_refused_restore_is_drawn() {
    let (mut rt, held) = window(Held {
        rows: three(),
        ..Held::default()
    });
    assert!(rt.find("notice").is_none());
    held.borrow_mut().notice = Some(Notice::RestoreRefused("Bravo.pdf".into()));
    poll(&mut rt);
    assert_eq!(
        info(&rt, "notice-words").text.as_deref(),
        Some("Couldn\u{2019}t restore \u{201C}Bravo.pdf\u{201D}. It\u{2019}s still in Trash.")
    );
}

#[test]
fn a_restore_that_was_not_made_is_drawn_without_a_name() {
    let (mut rt, held) = window(Held {
        rows: three(),
        ..Held::default()
    });
    held.borrow_mut().notice = Some(Notice::RestoreNotDone);
    poll(&mut rt);
    assert_eq!(
        info(&rt, "notice-words").text.as_deref(),
        Some("A restore didn\u{2019}t go through before Beebeeb was locked or signed out. The item is still in Trash.")
    );
}

#[test]
fn controls_with_no_engine_call_say_not_available_yet_and_push_nothing() {
    let (mut rt, _) = window(Held {
        rows: three(),
        ..Held::default()
    });
    let says = |rt: &Runtime<State>| info(rt, "notice-words").text;
    click(&mut rt, "btn-empty-trash");
    assert_eq!(says(&rt).as_deref(), Some("Not available yet in this version."));
    rt.state.frames.trash.notice = None;
    settle(&mut rt);

    click(&mut rt, "row-1");
    press(&mut rt, Key::Delete);
    assert_eq!(
        says(&rt).as_deref(),
        Some("Not available yet in this version."),
        "Delete permanently"
    );
    rt.state.frames.trash.notice = None;
    press_with(&mut rt, Key::Enter, alt());
    assert!(rt.find("notice").is_some(), "Details");
    rt.state.frames.trash.notice = None;
    press(&mut rt, space());
    assert!(rt.find("notice").is_some(), "Preview");
    rt.state.frames.trash.notice = None;

    // The menu's Details and Delete permanently… (items 1 and 2).
    for item in [1usize, 2] {
        rt.state.frames.trash.notice = None;
        press(&mut rt, Key::ContextMenu);
        for _ in 0..item {
            press(&mut rt, Key::Down);
        }
        press(&mut rt, Key::Enter);
        assert!(rt.find("notice").is_some(), "menu item {item}");
    }
    assert!(
        rt.state
            .frames
            .intents
            .iter()
            .all(|i| !matches!(i, Intent::Frame(_) | Intent::Do(..) | Intent::About(..))),
        "no intent that nothing drains: {:?}",
        rt.state.frames.intents
    );
}

#[test]
fn the_selection_follows_its_item_when_the_list_re_sorts_and_restore_acts_on_that_item() {
    let (mut rt, held) = window(Held {
        rows: three(),
        ..Held::default()
    });
    click(&mut rt, "row-1");
    assert_eq!(rt.state.frames.trash.selected_item.as_deref(), Some("b"));
    // A re-read puts a newer item first: Bravo is now on row 2.
    held.borrow_mut().rows.insert(0, row("n", "Newest.pdf"));
    poll(&mut rt);
    assert_eq!(
        rt.state.frames.trash.selected,
        Some(2),
        "the selection moved with Bravo"
    );
    assert!(rt.state.frames.trash.restore_selected());
    assert_eq!(
        held.borrow().restored,
        ["b"],
        "the item chosen, not whatever holds row 1"
    );

    // The selected item leaves the list (restored elsewhere): the selection ends, no other item is chosen.
    click(&mut rt, "row-0");
    let first = rt.state.frames.trash.selected_item.clone().expect("a selection");
    held.borrow_mut().rows.retain(|r| r.id != first);
    poll(&mut rt);
    assert_eq!(
        (
            rt.state.frames.trash.selected,
            rt.state.frames.trash.selected_item.clone()
        ),
        (None, None)
    );
}

/// A lock, and another account behind the gate: the window tells the source to forget on its own side (its poll), and
/// no name of before is in the element tree.
#[test]
fn a_gate_or_account_change_makes_the_window_drop_every_name() {
    use crate::shell::actions::FixtureEngine;
    use crate::shell::app::GateState;
    let engine = std::sync::Arc::new(FixtureEngine::ready());
    let shell = crate::shell::host::Shell::fixture_on(engine.clone(), crate::shell::platform::current());
    let mut state = crate::shell::state::init_with(shell);
    crate::frames::go(&mut state, Section::Trash);
    let held = Rc::new(RefCell::new(Held {
        rows: three(),
        ..Held::default()
    }));
    state.frames.trash = TrashUi::on(Box::new(Stub(held.clone())));
    let env = rune::Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, crate::view, (1278.0, 798.0), env);
    poll(&mut rt);
    let drawn = |rt: &Runtime<State>| {
        rt.elements()
            .into_iter()
            .filter(|e| !e.exiting)
            .any(|e| e.text.as_deref() == Some("Bravo.pdf") || e.label.as_deref() == Some("Bravo.pdf"))
    };
    assert!(drawn(&rt), "the list is drawn");
    assert_eq!(held.borrow().forgot, 0);

    engine.set_gate(GateState::Locked);
    rt.update(|s, _| {
        s.shell.refresh();
    });
    poll(&mut rt);
    assert_eq!(held.borrow().forgot, 1, "the lock: forgotten on the window's side");
    assert!(!drawn(&rt));

    held.borrow_mut().rows = three();
    held.borrow_mut().loaded = Loaded::Ready;
    engine.set_gate(GateState::Ready);
    engine.set_account("another-account");
    rt.update(|s, _| {
        s.shell.refresh();
    });
    poll(&mut rt);
    assert!(held.borrow().forgot >= 2, "another account: forgotten again");

    // An account change while the gate stays Ready (no lock in between): the section must re-open so the new
    // account's list is read. Without the `was_open = false` in the forget branch, "Loading" stays forever.
    let opened_before = held.borrow().opened;
    held.borrow_mut().rows = three();
    held.borrow_mut().loaded = Loaded::Ready;
    engine.set_account("a-third-account");
    rt.update(|s, _| {
        s.shell.refresh();
    });
    poll(&mut rt);
    assert!(held.borrow().forgot >= 3, "a third account: forgotten");
    assert!(
        held.borrow().opened > opened_before,
        "the section re-opens after an account change while the gate stays Ready"
    );
}
