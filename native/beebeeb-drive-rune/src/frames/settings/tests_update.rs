//! The update's button as drawn (DW-0781's banner, DW-0789b's dialog) at each step of the one press: its words, what
//! a press asks for, and that it waits while the press is worked on. The board draws the version on offer only; the
//! other steps are the board sample with `BoardSettings::with_update_press`.

use super::source::{BoardSettings, Press, UPDATE_CAPTION};
use super::{Dialog, Intent};
use crate::{init, view, State};
use rune::{Env, Runtime};

/// `frame` drawn on `source`, with the app's own keys and poll.
fn on(frame: &str, source: BoardSettings) -> Runtime<State> {
    let mut state = init();
    assert!(super::scene(frame, &mut state), "{frame} is a built frame");
    state.settings.source = Box::new(source);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (958.0, 638.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.poll = Some(crate::shell::host::poll);
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.016);
    rt
}

fn text(rt: &Runtime<State>, key: &str) -> String {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .and_then(|e| e.text)
        .unwrap_or_else(|| panic!("no text keyed {key}"))
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let r = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
        .rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.settle_all();
    rt.frame(0.016);
}

/// General with a newer version on offer: ONE press, the board's "Restart to update", the caption, and a click asks
/// for it (nothing else). After a step of it failed: "Try again", the failure's words in the caption's place, and a
/// click asks for the same press.
#[test]
fn general_offers_the_one_press_and_a_click_asks_for_it() {
    let mut rt = on("DW-0781", BoardSettings::new().with_update_press(Press::Restart, None));
    assert_eq!(text(&rt, "banner-0-title"), "Version 0.9.2 is available");
    assert_eq!(text(&rt, "btn-0-label"), "Restart to update");
    assert_eq!(text(&rt, "banner-0-text"), UPDATE_CAPTION);
    click(&mut rt, "btn-0");
    assert_eq!(rt.state.settings.intents, vec![Intent::RestartToUpdate]);

    let failed = "The installer did not start. Nothing was installed.";
    let mut rt = on(
        "DW-0781",
        BoardSettings::new().with_update_press(Press::Retry, Some(failed)),
    );
    assert_eq!(text(&rt, "btn-0-label"), "Try again");
    assert_eq!(text(&rt, "banner-0-text"), failed);
    click(&mut rt, "btn-0");
    assert_eq!(rt.state.settings.intents, vec![Intent::RestartToUpdate]);
}

/// While a press is worked on the banner says what is happening in the caption's place, and the button takes no
/// press.
#[test]
fn a_press_being_worked_on_says_what_happens_and_takes_no_press() {
    let says = "Downloading version 0.9.2: 40 %";
    let mut rt = on(
        "DW-0781",
        BoardSettings::new().with_update_press(Press::Wait, Some(says)),
    );
    assert_eq!(text(&rt, "banner-0-text"), says);
    click(&mut rt, "btn-0");
    assert!(rt.state.settings.intents.is_empty(), "{:?}", rt.state.settings.intents);
}

/// The notes' button follows the step too, and a press closes the dialog: Restart to update, or Try again after a
/// step failed; while the press is worked on it takes no press and the dialog stays.
#[test]
fn the_notes_button_follows_the_step_and_closes_the_dialog() {
    // While the press has words (what failed, its step, why it waits) the caption says them, as General does.
    let failed = "The installer did not start. Nothing was installed.";
    let rt = on(
        "DW-0789b",
        BoardSettings::new().with_update_press(Press::Retry, Some(failed)),
    );
    assert_eq!(text(&rt, "dlg-text-0"), failed);
    assert_eq!(text(&rt, "btn-2-label"), "Try again");

    let mut rt = on("DW-0789b", BoardSettings::new().with_update_press(Press::Retry, None));
    assert_eq!(rt.state.settings.dialog, Some(Dialog::ReleaseNotes));
    assert_eq!(text(&rt, "dlg-text-0"), UPDATE_CAPTION);
    assert_eq!(text(&rt, "btn-2-label"), "Try again");
    click(&mut rt, "btn-2");
    assert_eq!(rt.state.settings.dialog, None);
    assert_eq!(rt.state.settings.intents, vec![Intent::RestartToUpdate]);

    let mut rt = on("DW-0789b", BoardSettings::new());
    assert_eq!(text(&rt, "btn-2-label"), "Restart to update");
    click(&mut rt, "btn-2");
    assert_eq!(rt.state.settings.dialog, None);
    assert_eq!(rt.state.settings.intents, vec![Intent::RestartToUpdate]);

    let mut rt = on("DW-0789b", BoardSettings::new().with_update_press(Press::Wait, None));
    click(&mut rt, "btn-2");
    assert_eq!(rt.state.settings.dialog, Some(Dialog::ReleaseNotes), "it stays");
    assert!(rt.state.settings.intents.is_empty());
}

/// About drawn on `source` (DW-0751), tall enough for the update rows.
fn about_on(source: BoardSettings) -> Runtime<State> {
    let mut state = init();
    state.settings.source = Box::new(source);
    super::open_on(&mut state, "DW-0751");
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (958.0, 738.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.poll = Some(crate::shell::host::poll);
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.016);
    rt
}

fn element(rt: &Runtime<State>, key: &str) -> Option<rune::ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

/// Review of #441: where the release notes do not open (the app with no newer version on offer), About draws no row
/// that opens nothing: no "Release notes" row, and the older channel's row says what this PC runs and links nothing.
/// Where they open (the board sample), both rows are drawn as the board draws them.
#[test]
fn about_draws_no_row_to_release_notes_that_do_not_open() {
    let rt = about_on(BoardSettings::new().with_notes_closed());
    assert_eq!(text(&rt, "item-1-1-title"), "Release channel");
    assert!(element(&rt, "item-1-2").is_none(), "no Release notes row");
    let rt = about_on(BoardSettings::new());
    assert_eq!(text(&rt, "item-1-2-title"), "Release notes", "drawn where they open");

    let mut rt = about_on(BoardSettings::new().with_older_channel().with_notes_closed());
    assert_eq!(text(&rt, "item-1-2-sub"), beebeeb_engine::os_words::for_this_os("This PC runs 0.9.1, from the Beta channel.").as_str());
    assert_ne!(element(&rt, "item-1-2").expect("the older channel's row").role, "link");
    assert!(element(&rt, "item-1-3").is_none(), "no Release notes row");
    click(&mut rt, "item-1-2");
    assert_eq!(rt.state.settings.dialog, None);
    assert!(rt.state.settings.intents.is_empty(), "{:?}", rt.state.settings.intents);
}

/// A test or local build has no updater: About still draws the Updates section, disabled, with the one plain line.
/// Check now takes no press and the Release channel row opens no menu.
#[test]
fn about_draws_the_updates_section_disabled_without_an_updater() {
    let mut rt = about_on(BoardSettings::new().with_no_updater().with_notes_closed());
    assert_eq!(text(&rt, "item-1-0-title"), "Check for updates");
    assert_eq!(text(&rt, "item-1-0-sub"), "Unavailable in this test build");
    assert_eq!(text(&rt, "item-1-1-sub"), "Unavailable in this test build");
    assert!(
        rt.elements()
            .iter()
            .any(|e| e.text.as_deref() == Some(super::about::NO_UPDATER_LINE)),
        "the one line says why"
    );
    click(&mut rt, "btn-1");
    click(&mut rt, "item-1-1");
    assert!(rt.state.settings.intents.is_empty());
    assert!(rt.state.settings.menu.is_none());
}
