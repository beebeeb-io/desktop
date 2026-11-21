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
    /// How often a finished check asked the source to read again, and what that read brings: these rows go in at the
    /// top, and these ids leave.
    refreshed: usize,
    arrives: Vec<Row>,
    leaves: Vec<String>,
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
            purge: None,
            wake_in: None,
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
    fn refresh(&mut self) {
        let mut held = self.0.borrow_mut();
        held.refreshed += 1;
        let arrived = std::mem::take(&mut held.arrives);
        held.rows.splice(0..0, arrived);
        let gone = std::mem::take(&mut held.leaves);
        held.rows.retain(|row| !gone.contains(&row.id));
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
        bytes: None,
        at: None,
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
    for (loaded, key, title) in [
        (Loaded::Loading, "loading-words", "Loading Trash\u{2026} Getting the list from Beebeeb."),
        (Loaded::Offline, "trash-failed-title", "Can\u{2019}t reach Beebeeb"),
        (Loaded::Failed, "trash-failed-title", "Couldn\u{2019}t load Trash"),
        (Loaded::SessionEnded, "trash-failed-title", "Your session ended"),
        (Loaded::SignedOut, "trash-failed-title", "Signed out"),
    ] {
        let (rt, _) = window(Held {
            loaded,
            ..Held::default()
        });
        assert_eq!(info(&rt, key).text.as_deref(), Some(title), "{loaded:?}");
        // Only a failure that trying again can mend offers the button.
        let retry = matches!(loaded, Loaded::Offline | Loaded::Failed);
        assert_eq!(rt.find("trash-failed-retry").is_some(), retry, "{loaded:?}");
        assert!(rt.find("btn-empty-trash").is_none(), "{loaded:?}: nothing to empty");
        assert!(rt.find("Trash-badge").is_none(), "{loaded:?}: no count");
    }
    let (rt, _) = window(Held::default());
    assert_eq!(
        info(&rt, "trash-empty-title").text.as_deref(),
        Some("Trash is empty"),
        "a list read as empty"
    );
}

#[test]
fn try_again_on_a_failed_trash_reads_again_and_a_failed_refresh_offers_it_too() {
    let (mut rt, held) = window(Held {
        loaded: Loaded::Failed,
        ..Held::default()
    });
    let before = held.borrow().opened;
    click(&mut rt, "trash-failed-retry");
    assert_eq!(held.borrow().opened, before + 1, "Try again asks the source to read");

    let (mut rt, held) = window(Held {
        rows: three(),
        notice: Some(Notice::RefreshFailed),
        ..Held::default()
    });
    assert_eq!(
        info(&rt, "trash-failed-title").text.as_deref(),
        Some("Couldn\u{2019}t refresh Trash")
    );
    assert!(rt.find("row-0").is_some(), "the list stays under it");
    let before = held.borrow().opened;
    click(&mut rt, "trash-failed-retry");
    assert_eq!(held.borrow().opened, before + 1);
    assert!(rt.find("trash-failed-retry").is_none(), "the failure goes once it is tried again");
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
fn a_restore_that_was_not_made_is_drawn_with_its_name() {
    let (mut rt, held) = window(Held {
        rows: three(),
        ..Held::default()
    });
    held.borrow_mut().notice = Some(Notice::RestoreNotDone(vec!["Bravo.pdf".into()]));
    poll(&mut rt);
    assert_eq!(
        info(&rt, "notice-words").text.as_deref(),
        Some(
            "Couldn\u{2019}t restore \u{201C}Bravo.pdf\u{201D} before Beebeeb was locked or signed out. \
             It\u{2019}s still in Trash."
        )
    );
    // Two restores cut by one lock: one notice, the first named and the other counted.
    held.borrow_mut().notice = Some(Notice::RestoreNotDone(vec!["Bravo.pdf".into(), "Charlie.pdf".into()]));
    poll(&mut rt);
    assert_eq!(
        info(&rt, "notice-words").text.as_deref(),
        Some(
            "Couldn\u{2019}t restore \u{201C}Bravo.pdf\u{201D} and 1 other item before Beebeeb was locked or \
             signed out. They\u{2019}re still in Trash."
        )
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

/// A finished check reads Trash again: what was deleted on another device shows, what left is gone, with no click; the
/// selection stays with its item.
#[test]
fn a_finished_check_shows_arrivals_and_removals_and_the_selection_stays_with_its_item() {
    let (mut rt, held) = window(Held {
        rows: three(),
        ..Held::default()
    });
    click(&mut rt, "row-1");
    assert_eq!(rt.state.frames.trash.selected_item.as_deref(), Some("b"));
    {
        let mut held = held.borrow_mut();
        held.arrives = vec![row("n1", "New one.pdf"), row("n2", "New two.pdf")];
        held.leaves = vec!["a".into()];
    }
    super::after_a_check(&mut rt.state);
    poll(&mut rt);
    let ids: Vec<String> = rt.state.frames.trash.source.items().into_iter().map(|r| r.id).collect();
    assert_eq!(ids, ["n1", "n2", "b", "c"], "arrivals show, the removed one is gone");
    assert_eq!(rt.state.frames.trash.selected_item.as_deref(), Some("b"));
    assert_eq!(
        rt.state.frames.trash.selected,
        Some(2),
        "Bravo's row moved, the selection with it"
    );
    assert_eq!(held.borrow().refreshed, 1);

    // The selected item itself leaves on a later check: the selection ends, no other item is chosen.
    held.borrow_mut().leaves = vec!["b".into()];
    super::after_a_check(&mut rt.state);
    poll(&mut rt);
    assert_eq!(
        (
            rt.state.frames.trash.selected,
            rt.state.frames.trash.selected_item.clone()
        ),
        (None, None)
    );
}

/// While an item's menu is open the read waits; it runs on the first poll after the menu closed.
#[test]
fn a_finished_check_waits_while_the_row_menu_is_open_and_reads_once_it_closed() {
    use crate::frames::shared::parts::menu::Open;
    let (mut rt, held) = window(Held {
        rows: three(),
        ..Held::default()
    });
    click(&mut rt, "row-1");
    rt.update(|s, _| s.frames.trash.menu = Some(Open::at(10.0, 10.0)));
    held.borrow_mut().arrives = vec![row("n1", "New one.pdf")];
    super::after_a_check(&mut rt.state);
    poll(&mut rt);
    assert_eq!(held.borrow().refreshed, 0, "nothing is read under an open menu");
    assert_eq!(rt.state.frames.trash.source.items().len(), 3);

    rt.update(|s, _| s.frames.trash.menu = None);
    poll(&mut rt);
    assert_eq!(held.borrow().refreshed, 1, "read once the menu closed");
    assert_eq!(rt.state.frames.trash.source.items().len(), 4);
    assert_eq!(rt.state.frames.trash.selected_item.as_deref(), Some("b"));
}
