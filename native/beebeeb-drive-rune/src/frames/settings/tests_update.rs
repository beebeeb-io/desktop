//! The update's button as drawn (DW-0781's banner, DW-0789b's dialog) at each step of the spec's flow: its words, what
//! a press asks for, and that it waits while a press is worked on. The board draws the verified step only; the other
//! steps are the board sample with `BoardSettings::with_update_press`.

use super::source::{BoardSettings, Press, UPDATE_CAPTION};
use super::{Dialog, Intent};
use crate::{State, init, view};
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

/// General before the download: "Download update", the caption, and a click asks for the download (nothing else).
#[test]
fn general_offers_the_download_first_and_a_click_asks_for_it() {
    let mut rt = on("DW-0781", BoardSettings::new().with_update_press(Press::Download, None));
    assert_eq!(text(&rt, "banner-0-title"), "Version 0.9.2 is available");
    assert_eq!(text(&rt, "btn-0-label"), "Download update");
    assert_eq!(text(&rt, "banner-0-text"), UPDATE_CAPTION);
    click(&mut rt, "btn-0");
    assert_eq!(rt.state.settings.intents, vec![Intent::DownloadUpdate]);
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

/// The notes' button follows the step too, and a press closes the dialog: Download update, then Restart to update;
/// while one is worked on it takes no press and the dialog stays.
#[test]
fn the_notes_button_follows_the_step_and_closes_the_dialog() {
    let mut rt = on(
        "DW-0789b",
        BoardSettings::new().with_update_press(Press::Download, None),
    );
    assert_eq!(rt.state.settings.dialog, Some(Dialog::ReleaseNotes));
    assert_eq!(text(&rt, "dlg-text-0"), UPDATE_CAPTION);
    assert_eq!(text(&rt, "btn-2-label"), "Download update");
    click(&mut rt, "btn-2");
    assert_eq!(rt.state.settings.dialog, None);
    assert_eq!(rt.state.settings.intents, vec![Intent::DownloadUpdate]);

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
