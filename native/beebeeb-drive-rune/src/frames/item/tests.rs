//! An item's Details and read-only Preview over Trash, Shared and Search: the intents taken by the shell's poll, the
//! panel that offers nothing but Close, Space that opens and closes it, and the dialog that goes (with its page and
//! its read) when its section, its session or its row goes.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use rune::{Env, Key, Runtime};

use super::source::{BoardCounts, BoardPreview};
use super::{Origin, tag};
use crate::State;
use crate::frames::Intent;
use crate::frames::files::actions_data::{Page, PreviewOf, Previewed};
use crate::frames::shared::dialogs::Dialog;
use crate::frames::shared::tests::{focused, info, press, runtime, settle, space};
use crate::shell::actions::FixtureEngine;
use crate::shell::app::{GateState, Section};
use crate::shell::host::Shell;
use crate::shell::platform::fake::FakePlatform;
use crate::shell::registry::{FrameGroup, group_for};

/// A window on `scene` whose shell poll runs, and drains the frames' queue as the running app's does.
pub(crate) fn polled(scene: fn(&mut State)) -> Runtime<State> {
    crate::shell::intents::poll_drains_for_this_test();
    let mut rt = runtime(scene);
    rt.poll = Some(crate::shell::host::poll);
    settle(&mut rt);
    rt
}

/// A Preview source answering `answer`, put in the window; the handle reads what was asked of it.
pub(crate) fn previews(rt: &mut Runtime<State>, answer: Previewed) -> Rc<RefCell<BoardCounts>> {
    let (source, counts) = BoardPreview::new(answer);
    rt.update(move |s, _| s.frames.item_preview = Box::new(source));
    counts
}

/// The text of the element keyed `key`, when one is drawn.
fn text_of(rt: &Runtime<State>, key: &str) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .and_then(|e| e.text)
}

/// The facts (title, value) the item dialog's list `n` draws.
pub(crate) fn facts(rt: &Runtime<State>, n: usize) -> Vec<(String, String)> {
    (0..)
        .map_while(|i| {
            let title = text_of(rt, &format!("item-{n}-{i}-title"))?;
            let value = text_of(rt, &format!("item-{n}-{i}-value"));
            Some((title, value.unwrap_or_default()))
        })
        .collect()
}

/// The render scene of this name (`scenes::RENDER_SCENES`).
fn scene(name: &str) -> fn(&mut State) {
    super::scenes::RENDER_SCENES
        .iter()
        .find(|(listed, _)| *listed == name)
        .map(|(_, scene)| *scene)
        .unwrap_or_else(|| panic!("no render scene {name}"))
}

fn page(words: &str) -> Previewed {
    Previewed::Shown(PreviewOf {
        place: "The whole file".into(),
        page: Page {
            title: "Invented.txt".into(),
            byline: "Text".into(),
            sections: vec![(String::new(), words.into())],
        },
    })
}

/// Trash with row `i` selected, as the arrows leave it (focus on the row).
fn trash_on_row(i: usize, answer: Previewed) -> (Runtime<State>, Rc<RefCell<BoardCounts>>) {
    let mut rt = polled(crate::frames::trash::scenes::dw_0747);
    let counts = previews(&mut rt, answer);
    for _ in 0..=i {
        press(&mut rt, Key::Down);
    }
    assert_eq!(rt.state.frames.trash.selected, Some(i));
    (rt, counts)
}

const NO_CONTROL: [&str; 5] = ["pv-share", "pv-open", "pv-open-with", "pv-keep", "pv-blank-actions"];

#[test]
fn frame_intent_tags_carry_no_payload() {
    assert_eq!(tag(&Intent::Do("search", "secret words".into())), "Do search");
    let about = tag(&Intent::About("DW-0402", "id-123".into()));
    assert!(!about.contains("id-123"), "{about}");
    assert_eq!(about, "About DW-0402");
    assert_eq!(tag(&Intent::OpenItem("id-456".into())), "OpenItem");
    assert_eq!(tag(&Intent::Frame("DW-0504")), "Frame DW-0504");
    assert_eq!(
        tag(&Intent::Do("create-request", "a typed title".into())),
        "Do create-request"
    );
}

#[test]
fn trash_preview_panel_is_read_only() {
    for (name, shown) in [("trash-preview-text", true), ("trash-preview-notyet", false)] {
        let rt = polled(scene(name));
        assert!(
            matches!(rt.state.frames.shared.dialog, Some(Dialog::Preview { .. })),
            "{name}: the panel is open"
        );
        assert!(rt.find("pv-close").is_some() && rt.find("pv-name").is_some(), "{name}");
        for key in NO_CONTROL {
            assert!(rt.find(key).is_none(), "{name}: {key} is drawn on a read-only panel");
        }
        assert_eq!(rt.find("pv-page").is_some(), shown, "{name}");
        if !shown {
            let words = info(&rt, "pv-blank-text").text.unwrap_or_default();
            assert_eq!(words, "Contract old.pdf \u{00B7} 1.1 MB. Restore it to open it.");
            assert_eq!(
                info(&rt, "pv-blank-title").text.as_deref(),
                Some("Preview isn't available for this kind of file in this version yet")
            );
        }
    }
}

#[test]
fn trash_preview_never_opens_the_viewer() {
    let (mut rt, counts) = trash_on_row(1, Previewed::NotYet);
    let screen = rt.state.shell.app.screen();
    press(&mut rt, space());
    assert_eq!(
        rt.state.frames.shared.dialog,
        Some(Dialog::Preview {
            origin: Origin::Trash,
            id: "t-contract-old".into()
        })
    );
    assert_eq!(rt.state.shell.app.screen(), screen, "the route is still Trash");
    assert_eq!(group_for(&rt.state.shell.app), FrameGroup::Section(Section::Trash));
    assert!(rt.state.viewer.ui.is_none(), "no file is open in the viewer");
    assert_eq!(
        counts.borrow().last_from,
        Some(super::source::ReadFrom::Beebeeb),
        "read from Beebeeb only"
    );
}

#[test]
fn space_opens_and_space_closes_the_trash_preview() {
    // Through the root key path (`dialogs::TRASH`'s keys), as a person presses.
    let (mut rt, counts) = trash_on_row(1, Previewed::NotYet);
    press(&mut rt, space());
    assert!(matches!(rt.state.frames.shared.dialog, Some(Dialog::Preview { .. })));
    assert_eq!(
        focused(&rt).as_deref(),
        Some("pv-close"),
        "the next Space lands in the panel"
    );
    assert_eq!((counts.borrow().starts, counts.borrow().cancels), (1, 0));
    press(&mut rt, space());
    assert_eq!(rt.state.frames.shared.dialog, None, "Space closes it again");
    assert_eq!(counts.borrow().cancels, 1, "its read stopped");
    assert_eq!(rt.state.frames.shared.fields.preview.state(), None);

    // A folder's Space shows that it has no preview, and nothing is read.
    let (mut rt, counts) = trash_on_row(0, Previewed::NotYet);
    press(&mut rt, space());
    assert!(matches!(rt.state.frames.shared.dialog, Some(Dialog::Preview { .. })));
    assert_eq!(
        rt.state.frames.shared.fields.preview.state(),
        Some(&Previewed::NoPreview)
    );
    assert_eq!(counts.borrow().starts, 0, "nothing is read for a folder");
    assert_eq!(
        info(&rt, "pv-blank-title").text.as_deref(),
        Some("No preview for this kind of file")
    );
}

#[test]
fn a_preview_that_answers_later_is_shown_when_it_answers() {
    let (mut rt, counts) = trash_on_row(1, Previewed::Loading);
    press(&mut rt, space());
    assert!(rt.state.frames.shared.fields.preview.is_loading());
    assert!(info(&rt, "pv-blank-title").text.as_deref() == Some("Getting the preview\u{2026}"));
    counts.borrow_mut().answer = page("invented first lines");
    settle(&mut rt);
    assert!(rt.find("pv-page").is_some(), "the page is drawn once it answered");
    // Esc closes it, and the page goes with it.
    press(&mut rt, Key::Escape);
    assert_eq!(rt.state.frames.shared.dialog, None);
    assert_eq!(rt.state.frames.shared.fields.preview.state(), None);
    assert!(rt.find("pv-page").is_none());
}

/// The app as it runs on the fixture engine (its gate can be moved), on Trash with row 1 selected.
fn app_on_trash(answer: Previewed) -> (Runtime<State>, Arc<FixtureEngine>, Rc<RefCell<BoardCounts>>) {
    crate::shell::intents::poll_drains_for_this_test();
    let engine = Arc::new(FixtureEngine::ready());
    let state = crate::shell::state::init_with(Shell::fixture_on(engine.clone(), Arc::new(FakePlatform::default())));
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, crate::view, (1278.0, 798.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.poll = Some(crate::shell::host::poll);
    rt.update(|s, _| crate::frames::go(s, Section::Trash));
    settle(&mut rt);
    let counts = previews(&mut rt, answer);
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Down);
    (rt, engine, counts)
}

#[test]
fn item_dialog_goes_on_lock_section_change_and_gone_row() {
    // A lock: ONE real poll of the shell closes it (the session-end block, after the poll's refresh), the page and
    // the read go, and the frame drawn after it has no page.
    let (mut rt, engine, counts) = app_on_trash(page("invented words of a trashed note"));
    press(&mut rt, space());
    assert!(rt.find("pv-page").is_some(), "the page is drawn");
    let cancels = counts.borrow().cancels;
    engine.set_gate(GateState::Locked);
    rt.update(|s, tx| {
        crate::shell::host::poll(s, 0.0, tx);
        assert_eq!(s.frames.shared.dialog, None, "closed by the poll that saw the lock");
        assert_eq!(s.frames.shared.fields.preview.state(), None, "the page went with it");
    });
    assert_eq!(counts.borrow().cancels, cancels + 1, "its read stopped");
    rt.frame(0.0);
    assert!(
        rt.find("pv-page").is_none(),
        "nothing of the page is drawn after the lock"
    );

    // The section left (a click in the sidebar, any route): it closes, and its read stops.
    let (mut rt, _engine, counts) = app_on_trash(Previewed::Loading);
    press(&mut rt, space());
    assert!(matches!(rt.state.frames.shared.dialog, Some(Dialog::Preview { .. })));
    rt.update(|s, _| crate::frames::go(s, Section::Files));
    settle(&mut rt);
    assert_eq!(rt.state.frames.shared.dialog, None);
    assert_eq!(rt.state.frames.shared.fields.preview.state(), None);
    assert_eq!(counts.borrow().cancels, 1);

    // The row gone from a list that is ready (restored elsewhere): it closes.
    let (mut rt, _engine, counts) = app_on_trash(Previewed::Loading);
    press(&mut rt, space());
    assert!(matches!(rt.state.frames.shared.dialog, Some(Dialog::Preview { .. })));
    rt.update(|s, _| {
        assert!(s.frames.trash.source.restore("t-contract-old"));
    });
    settle(&mut rt);
    assert_eq!(rt.state.frames.shared.dialog, None);
    assert_eq!(rt.state.frames.shared.fields.preview.state(), None);
    assert_eq!(counts.borrow().cancels, 1);
}

#[test]
fn an_id_not_in_the_section_shown_opens_nothing() {
    let mut rt = polled(crate::frames::trash::scenes::dw_0747);
    rt.update(|s, _| {
        // Shared's id while Trash shows; an id no list has.
        s.frames.intents.push(Intent::About("DW-0411", "s-brand-review".into()));
        s.frames.intents.push(Intent::Do("preview", "no-such-id".into()));
    });
    settle(&mut rt);
    assert!(rt.state.frames.intents.is_empty());
    assert_eq!(rt.state.frames.shared.dialog, None);
}

#[test]
fn nothing_of_the_page_or_the_account_is_in_a_debug_print() {
    let rt = polled(scene("trash-preview-text"));
    let printed = format!(
        "{:?} {:?}",
        rt.state.frames.shared.fields, rt.state.frames.shared.dialog
    );
    assert!(!printed.contains("Train at"), "{printed}");
    assert!(!printed.contains('@'), "no account in it: {printed}");
    let ask = super::source::PreviewAsk {
        id: "t-1".into(),
        name: "Invented secret name.txt".into(),
        from: super::source::ReadFrom::Beebeeb,
    };
    assert!(!format!("{ask:?}").contains("secret"));
}

/// Trash rows and how far the list got, changed by a test between polls.
struct Moving(
    Rc<
        RefCell<(
            Vec<crate::frames::shared::parts::table::Row>,
            crate::frames::trash::source::Loaded,
        )>,
    >,
);

impl crate::frames::trash::source::TrashSource for Moving {
    fn items(&self) -> Vec<crate::frames::shared::parts::table::Row> {
        self.0.borrow().0.clone()
    }
    fn totals(&self) -> crate::frames::trash::source::TrashTotals {
        crate::frames::trash::source::TrashTotals {
            items: self.0.borrow().0.len(),
            size: "1 KB".into(),
        }
    }
    fn emptying(&self) -> Option<crate::frames::trash::source::Emptying> {
        None
    }
    fn restore(&mut self, _id: &str) -> bool {
        false
    }
    fn loaded(&self) -> crate::frames::trash::source::Loaded {
        self.0.borrow().1
    }
}

fn trash_row(id: &str, name: &str) -> crate::frames::shared::parts::table::Row {
    crate::frames::shared::parts::table::Row {
        id: id.into(),
        name: name.into(),
        kind: crate::frames::shared::parts::table::Kind::Document,
        date: "2 Nov 2026".into(),
        size: "1 KB".into(),
        extra: crate::frames::shared::parts::table::Extra::Words(String::new()),
        pc: None,
        bytes: None,
        at: None,
    }
}

/// A row that left a list still on its way does not close the dialog (R2(c)), and nothing of it is drawn then; the
/// dialog that is not drawn never keeps the keys: the next key closes it and reaches the list.
#[test]
fn an_item_dialog_that_is_not_drawn_never_keeps_the_keys() {
    use crate::frames::trash::source::Loaded;
    let held = Rc::new(RefCell::new((
        vec![trash_row("a", "Alpha.txt"), trash_row("b", "Bravo.txt")],
        Loaded::Ready,
    )));
    let mut rt = polled(crate::frames::trash::scenes::dw_0747);
    let source = Moving(held.clone());
    rt.update(move |s, _| s.frames.trash = crate::frames::trash::TrashUi::on(Box::new(source)));
    settle(&mut rt);
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Down);
    crate::frames::shared::tests::press_with(&mut rt, Key::Enter, crate::frames::shared::tests::alt());
    assert!(matches!(
        rt.state.frames.shared.dialog,
        Some(Dialog::Details { ref id, .. }) if id == "b"
    ));
    assert!(rt.find("dialog-details").is_some());
    // The list re-reads and its first answer is on its way without the row: the dialog stays, undrawn.
    *held.borrow_mut() = (vec![trash_row("a", "Alpha.txt")], Loaded::Loading);
    settle(&mut rt);
    assert!(
        rt.state.frames.shared.dialog.is_some(),
        "a list on its way closes nothing"
    );
    assert!(rt.find("dialog-details").is_none(), "nothing of the row is drawn");
    // The next key closes it and reaches the list.
    press(&mut rt, Key::Up);
    assert_eq!(rt.state.frames.shared.dialog, None);
    assert_eq!(rt.state.frames.trash.selected, Some(0), "the key reached the list");
}

/// Another account under a gate that stays Ready: ONE poll of the shell closes the Preview (the session-end block's
/// first call), and its page and read go.
#[test]
fn an_account_switch_closes_the_item_dialog_in_the_poll_that_sees_it() {
    let (mut rt, engine, counts) = app_on_trash(page("invented words of a trashed note"));
    press(&mut rt, space());
    assert!(rt.find("pv-page").is_some());
    let cancels = counts.borrow().cancels;
    engine.set_account("someone-else@example.eu");
    rt.update(|s, tx| {
        crate::shell::host::poll(s, 0.0, tx);
        assert_eq!(
            s.frames.shared.dialog, None,
            "closed by the poll that saw the other account"
        );
        assert_eq!(s.frames.shared.fields.preview.state(), None);
    });
    assert_eq!(counts.borrow().cancels, cancels + 1);
}

/// The server ended the session while the gate stays Ready (the shell's session gate closes): ONE poll closes the
/// Preview (the session-end block's second call), and its page and read go.
#[test]
fn a_session_the_server_ended_closes_the_item_dialog_in_the_poll_that_sees_it() {
    use crate::frames::global::source::board;
    use crate::frames::global::{GlobalFixture, Ui};
    let (mut rt, _engine, counts) = app_on_trash(page("invented words of a trashed note"));
    press(&mut rt, space());
    assert!(rt.find("pv-page").is_some());
    let cancels = counts.borrow().cancels;
    rt.update(|s, tx| {
        s.global = Ui::new(Box::new(GlobalFixture::of(board::signed_out())));
        crate::shell::host::poll(s, 0.0, tx);
        assert_eq!(s.shell.app.gate(), GateState::Ready, "the gate stayed Ready");
        assert_eq!(
            s.frames.shared.dialog, None,
            "closed by the poll that saw the session end"
        );
        assert_eq!(s.frames.shared.fields.preview.state(), None);
    });
    assert_eq!(counts.borrow().cancels, cancels + 1);
}
