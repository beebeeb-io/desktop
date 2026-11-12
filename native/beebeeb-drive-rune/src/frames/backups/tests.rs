//! Tests of Backups: every built frame's content, the controls by pointer and by keyboard,
//! names and roles, the protected root, and the one-time code staying out of the tree.

use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::source::{BackupsSource, FixtureSource, RootAction, Status};
use super::{BackupsUi, Dialog, FRAMES, Intent, Page};
use crate::State;
use crate::shell::app::{Screen, SettingsPane};
use crate::shell::registry::view;
use crate::shell::state::init;

fn scene(id: &str) -> fn(&mut State) {
    FRAMES
        .iter()
        .find(|(frame, _)| *frame == id)
        .unwrap_or_else(|| panic!("no scene {id}"))
        .1
}

fn runtime(id: &str) -> Runtime<State> {
    let mut state = init();
    scene(id)(&mut state);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (958.0, 958.0), env);
    // As the app runs: the shell's keys first, then this group's through the registry.
    rt.on_key = Some(crate::shell::host::on_key);
    rt.frame(0.0);
    rt
}

fn find(rt: &Runtime<State>, key: &str) -> Option<rune::ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn info(rt: &Runtime<State>, key: &str) -> rune::ElementInfo {
    find(rt, key).unwrap_or_else(|| panic!("no element keyed {key}"))
}

fn has(rt: &Runtime<State>, key: &str) -> bool {
    find(rt, key).is_some()
}

fn text(rt: &Runtime<State>, key: &str) -> String {
    info(rt, key).text.unwrap_or_default()
}

fn label(rt: &Runtime<State>, key: &str) -> String {
    info(rt, key).label.unwrap_or_default()
}

/// After an input: rebuild the tree, let what moves arrive and what leaves finish leaving (a
/// keyed element that changed place moves there, and a pointer finds it only once it has).
fn settle(rt: &mut Runtime<State>) {
    rt.frame(0.016);
    rt.settle_all();
    rt.frame(0.016);
}

fn press(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    settle(rt);
}

fn type_text(rt: &mut Runtime<State>, typed: &str) {
    for c in typed.chars() {
        press(rt, Key::Text(c.to_string().into()));
    }
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let r = info(rt, key).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}

fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key)
}

/// Tab until `key` has focus.
fn tab_to(rt: &mut Runtime<State>, key: &str) {
    for _ in 0..80 {
        if focused(rt).as_deref() == Some(key) {
            return;
        }
        press(rt, Key::Tab);
    }
    panic!("Tab never reached {key}; focus is on {:?}", focused(rt));
}

fn source(rt: &Runtime<State>) -> &dyn BackupsSource {
    rt.state.backups.source().expect("the fixture")
}

#[test]
fn the_six_built_frames_show_the_boards_words() {
    assert_eq!(FRAMES.len(), 6, "the Backups frames this lane built");
    let rt = runtime("DW-0737");
    assert_eq!(rt.state.shell.app.screen(), Screen::Settings(SettingsPane::Backups));
    assert_eq!(text(&rt, "set-title"), "Backups");
    assert_eq!(text(&rt, "card-0-head"), "All backed up");
    assert_eq!(text(&rt, "card-1-text"), "Couldn't back up 3 items");
    assert_eq!(
        text(&rt, "item-0-1-sub"),
        "1,820 items \u{00B7} 5.9 GB \u{00B7} Today, 09:30"
    );
    assert_eq!(text(&rt, "item-0-3-sub"), "1,106 items \u{00B7} 7.4 GB \u{00B7} off");
    assert_eq!(
        text(&rt, "item-0-6-sub"),
        "46 items \u{00B7} 640 MB \u{00B7} chosen by you \u{00B7} Today, 09:28"
    );
    assert_eq!(text(&rt, "item-0-7-title"), "Add a folder\u{2026}");
    assert_eq!(text(&rt, "item-1-2-sub"), "8,204 photos \u{00B7} Not signed in");
    assert_eq!(text(&rt, "item-2-0-value"), "On");
    assert!(text(&rt, "list-0-foot").contains("Backups \u{203A} Anna-Laptop"));

    let rt = runtime("DW-1411");
    assert_eq!(text(&rt, "card-0-head"), "Backing up Documents");
    assert_eq!(
        text(&rt, "card-0-sub"),
        "612 of 1,820 files \u{00B7} about 20 min left \u{00B7} the first backup takes longest"
    );
    // 34 % of the bar, as the board draws it.
    let (bar, fill) = (info(&rt, "card-0-bar").rect, info(&rt, "card-0-bar-fill").rect);
    assert!((fill[2] / bar[2] - 0.34).abs() < 0.001, "{fill:?} of {bar:?}");
    assert!(!has(&rt, "card-1"), "nothing failed in DW-1411");
    assert_eq!(text(&rt, "btn-0-label"), "Remove", "no Back up now while a backup runs");

    let rt = runtime("DW-1410");
    assert_eq!(text(&rt, "dlg-title"), "Stop backing up Projects?");
    assert!(text(&rt, "dlg-sub").contains("The 640 MB already backed up"));
    assert_eq!(text(&rt, "btn-2-label"), "Stop and keep the backup");
    assert_eq!(text(&rt, "btn-3-label"), "Stop and move the backup to Trash");
    assert_eq!(text(&rt, "btn-4-label"), "Cancel");

    let rt = runtime("DW-0739");
    assert_eq!(text(&rt, "set-title"), "Anna's iPad");
    assert_eq!(text(&rt, "item-0-0-value"), "4,120 photos \u{00B7} 38 GB");
    assert_eq!(text(&rt, "item-0-1-value"), "19 Oct");
    assert_eq!(text(&rt, "item-0-2-value"), "Signed in");
    assert_eq!(text(&rt, "item-2-0-title"), "Delete this backup");

    let rt = runtime("DW-0742");
    assert_eq!(text(&rt, "dlg-title"), "Delete this backup?");
    assert!(text(&rt, "dlg-text-0").starts_with("4,120 photos and 38 GB will be moved to Trash."));
    assert_eq!(rt.state.backups.code_len(), 2);

    let rt = runtime("DW-0741");
    assert_eq!(text(&rt, "set-title"), "Couldn't back up");
    assert_eq!(
        text(&rt, "item-0-2-sub"),
        "Documents \u{00B7} it kept changing while it was backing up"
    );
    assert_eq!(text(&rt, "btn-3-label"), "Try all again");
    assert_eq!(
        text(&rt, "item-1-0-sub"),
        "Desktop",
        "an exclusion says where, never an error"
    );
}

#[test]
fn the_backups_root_is_protected_and_no_frame_draws_a_control_that_changes_it() {
    for (id, _) in FRAMES {
        let rt = runtime(id);
        let root = source(&rt).root();
        assert!(root.is_protected, "{id}");
        assert!(root.allows(RootAction::Open) && !root.allows(RootAction::Delete));
        for e in rt.elements() {
            if !e.focusable || e.exiting {
                continue;
            }
            let name = e.label.clone().unwrap_or_default();
            for word in ["Rename", "Move Backups", "Share", "Delete Backups"] {
                assert!(!name.contains(word), "{id}: a control named {name:?}");
            }
        }
    }
    // The only deleting control is about ONE device's copies, behind the code.
    let mut rt = runtime("DW-0739");
    click(&mut rt, "item-2-0");
    assert_eq!(rt.state.backups.dialog, Some(Dialog::DeleteDevice("ipad".into())));
    assert_eq!(source(&rt).devices().len(), 3, "asking deletes nothing");
}

#[test]
fn a_switch_starts_and_stops_a_known_folder_by_pointer_and_by_keyboard() {
    let mut rt = runtime("DW-0737");
    let on = |rt: &Runtime<State>, id: &str| source(rt).folders().iter().any(|f| f.id == id && f.on);
    assert!(!on(&rt, "music"));
    assert_eq!(info(&rt, "item-0-3-switch").role, "switch");
    assert_eq!(label(&rt, "item-0-3-switch"), "Music");
    click(&mut rt, "item-0-3-switch");
    assert!(on(&rt, "music"));
    tab_to(&mut rt, "item-0-0-switch");
    press(&mut rt, Key::Text(" ".into()));
    assert!(!on(&rt, "desktop"), "Space on the focused switch");
    press(&mut rt, Key::Enter);
    assert!(on(&rt, "desktop"), "Enter on the focused switch");
    // A chosen folder has Remove, never a switch.
    assert!(!has(&rt, "item-0-6-switch"));
    assert_eq!(label(&rt, "btn-1"), "Remove Projects");
}

#[test]
fn back_up_now_starts_a_backup_and_the_card_shows_it_moving() {
    let mut rt = runtime("DW-0737");
    tab_to(&mut rt, "btn-0");
    press(&mut rt, Key::Enter);
    assert!(matches!(source(&rt).status(), Status::Running { .. }));
    assert_eq!(text(&rt, "card-0-head"), "Backing up Documents");
    assert_eq!(info(&rt, "card-0-bar").role, "progressbar");
    assert_eq!(label(&rt, "card-0-bar"), "Backing up Documents, 34 per cent");
}

#[test]
fn remove_asks_first_and_each_answer_does_what_it_says() {
    let mut rt = runtime("DW-0737");
    click(&mut rt, "btn-1");
    assert_eq!(rt.state.backups.dialog, Some(Dialog::StopFolder("projects".into())));
    assert_eq!(info(&rt, "dlg").role, "alertdialog");
    press(&mut rt, Key::Escape);
    assert_eq!(rt.state.backups.dialog, None, "Esc closes the alert");
    assert_eq!(source(&rt).folders().len(), 7, "and nothing changed");
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::Settings(SettingsPane::Backups),
        "Esc closed the alert, not settings"
    );

    click(&mut rt, "btn-1");
    click(&mut rt, "btn-4");
    assert_eq!(source(&rt).folders().len(), 7, "Cancel");

    click(&mut rt, "btn-1");
    click(&mut rt, "btn-2");
    assert_eq!(rt.state.backups.dialog, None);
    assert_eq!(source(&rt).folders().len(), 6, "Stop and keep the backup");
    assert!(!has(&rt, "btn-1"), "Projects left the list with its Remove");

    let mut rt = runtime("DW-1410");
    tab_to(&mut rt, "btn-3");
    press(&mut rt, Key::Enter);
    assert_eq!(
        source(&rt).folders().len(),
        6,
        "Stop and move the backup to Trash, by keyboard"
    );
}

#[test]
fn what_could_not_back_up_retries_one_or_all_and_never_an_exclusion() {
    let mut rt = runtime("DW-0737");
    click(&mut rt, "card-1");
    assert_eq!(rt.state.backups.page, Page::Failures);
    assert_eq!(label(&rt, "btn-0"), "Try again: Render final.mov");
    click(&mut rt, "btn-0");
    assert_eq!(source(&rt).not_backed_up().len(), 3);
    assert_eq!(text(&rt, "item-0-0-title"), "Invoice scan.pdf");
    assert_eq!(text(&rt, "btn-2-label"), "Try all again");
    tab_to(&mut rt, "btn-2");
    press(&mut rt, Key::Enter);
    let left = source(&rt).not_backed_up();
    assert_eq!(left.len(), 1);
    assert!(left[0].excluded);
    // Only the exclusion is left: one list, no button at all.
    assert_eq!(text(&rt, "item-0-0-title"), "Site walkthrough.mkv");
    assert!(!has(&rt, "btn-0"), "an exclusion has no Try again");
    press(&mut rt, Key::Escape);
    assert_eq!(rt.state.backups.page, Page::Section, "Esc goes back to the section");
    assert!(!has(&rt, "card-1"), "no failures, no card");
    press(&mut rt, Key::Escape);
    assert!(
        !matches!(rt.state.shell.app.screen(), Screen::Settings(_)),
        "Esc on the section leaves settings"
    );
}

#[test]
fn deleting_a_devices_backup_needs_six_digits_and_a_code_the_engine_accepts() {
    let mut rt = runtime("DW-0737");
    click(&mut rt, "item-1-1");
    assert_eq!(rt.state.backups.page, Page::Device("ipad".into()));
    click(&mut rt, "item-2-0");
    assert_eq!(info(&rt, "dlg").role, "dialog");
    assert!(!info(&rt, "btn-0").focusable, "Delete backup is off until six digits");
    click(&mut rt, "btn-0");
    assert_eq!(source(&rt).devices().len(), 3);

    type_text(&mut rt, "51x4");
    assert_eq!(rt.state.backups.code_len(), 3, "digits only");
    press(&mut rt, Key::Backspace);
    assert_eq!(rt.state.backups.code_len(), 2);
    press(&mut rt, Key::Enter);
    assert_eq!(source(&rt).devices().len(), 3, "Enter does nothing before six digits");
    type_text(&mut rt, "42639");
    assert_eq!(rt.state.backups.code_len(), 6, "six at most");
    assert!(info(&rt, "btn-0").focusable);

    // A code the engine does not accept: the boxes empty, the dialog stays, nothing is deleted.
    let mut refused = runtime("DW-0739");
    click(&mut refused, "item-2-0");
    type_text(&mut refused, "000000");
    press(&mut refused, Key::Enter);
    assert_eq!(source(&refused).devices().len(), 3);
    assert_eq!(refused.state.backups.code_len(), 0);
    assert!(refused.state.backups.dialog.is_some());

    click(&mut rt, "btn-0");
    assert_eq!(source(&rt).devices().len(), 2);
    assert_eq!(rt.state.backups.page, Page::Section);
    assert_eq!(rt.state.backups.dialog, None);
    assert_eq!(rt.state.backups.code_len(), 0, "the code is wiped after use");
    assert_eq!(text(&rt, "item-1-1-title"), "Old iPhone");
}

#[test]
fn the_code_is_in_no_name_and_no_text_of_the_element_tree() {
    let mut rt = runtime("DW-0739");
    click(&mut rt, "item-2-0");
    type_text(&mut rt, "73913");
    assert_eq!(rt.state.backups.code_len(), 5);
    assert_eq!(info(&rt, "secure-code").role, "textbox");
    assert_eq!(label(&rt, "secure-code"), "Authenticator code, 6 digits, 5 entered");
    assert!(has(&rt, "code-4-digit") && !has(&rt, "code-5-digit"));
    let dialog = info(&rt, "dlg").rect;
    for e in rt.elements() {
        // The dialog's own sentences hold the board's "4,120 photos": only what sits inside
        // the code field is checked for digits, and everything for the whole code.
        let inside_field = e
            .key
            .as_deref()
            .is_some_and(|k| k.starts_with("code-") || k == "secure-code");
        for words in [e.label.as_deref(), e.text.as_deref(), e.help.as_deref()]
            .into_iter()
            .flatten()
        {
            assert!(!words.contains("73913"), "{:?} holds the code: {words:?}", e.key);
            if inside_field && e.key.as_deref() != Some("secure-code") && e.key.as_deref() != Some("code-label") {
                assert!(
                    !words.chars().any(|c| c.is_ascii_digit()),
                    "{:?} shows a digit to the tree: {words:?}",
                    e.key
                );
            }
        }
    }
    assert!(dialog[2] > 0.0);
    // Closing the dialog wipes what was typed.
    press(&mut rt, Key::Escape);
    assert_eq!(rt.state.backups.code_len(), 0);
}

#[test]
fn rows_that_lead_elsewhere_ask_the_shell_and_carry_the_devices_id() {
    let mut rt = runtime("DW-0737");
    click(&mut rt, "item-0-7");
    click(&mut rt, "item-2-0");
    click(&mut rt, "item-1-1");
    tab_to(&mut rt, "item-1-0");
    press(&mut rt, Key::Enter);
    click(&mut rt, "item-1-1");
    assert_eq!(
        rt.state.backups.take_intents(),
        vec![
            Intent::PickFolder,
            Intent::MailBackup,
            Intent::OpenInPhotos("ipad".into()),
            Intent::OpenInFiles("ipad".into()),
        ]
    );
    // The picker's answer: Beebeeb's own folder cannot be chosen.
    assert!(!rt.state.backups.folder_picked(r"C:\Users\Anna\Beebeeb\Work"));
    assert!(rt.state.backups.folder_picked(r"C:\Users\Anna\Projects 2"));
    // The crumb goes back to the section.
    click(&mut rt, "crumb-row");
    assert_eq!(rt.state.backups.page, Page::Section);
}

#[test]
fn leaving_the_section_forgets_what_was_open_and_wipes_the_code() {
    let mut rt = runtime("DW-0742");
    assert_eq!(rt.state.backups.code_len(), 2);
    rt.update(|s, _| crate::frames::settings::go(s, "DW-0701"));
    rt.frame(0.0);
    assert_eq!(rt.state.backups.dialog, None);
    assert_eq!(rt.state.backups.page, Page::Section);
    assert_eq!(rt.state.backups.code_len(), 0);
    // The sidebar's Backups item comes back to the section's own page.
    rt.update(|s, _| crate::frames::settings::go(s, "DW-0737"));
    rt.frame(0.0);
    assert_eq!(text(&rt, "set-title"), "Backups");
    assert!(!has(&rt, "dlg"));
}

#[test]
fn a_build_with_no_source_draws_the_not_built_frame_never_sample_values() {
    let mut state = init();
    crate::frames::settings::go(&mut state, "DW-0737");
    state.backups = BackupsUi::new(None);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (958.0, 958.0), env);
    rt.frame(0.0);
    assert!(has(&rt, "not-built"));
    assert!(!has(&rt, "card-0"));
    assert!(
        super::source::first(false).is_none(),
        "a build that ships starts unwired"
    );
    assert!(super::source::first(true).is_some());
    // The fixture the scenes use is the board's.
    assert_eq!(FixtureSource::board().folders().len(), 7);
}
