//! Tests of the action dialogs (lane files-actions): Share (DW-0407), Details (DW-0411), Version
//! history (DW-0413), Preview (DW-0408, DW-0408b) and Two versions (DW-0429). Each is reached
//! by the window's own controls (a menu item, a key, a button), and each test says what was
//! asked of the source: how often, and with what. Helpers are `tests.rs`'s.

use super::action_scenes::{conflict_fixture, install, moodboard};
use super::actions_data::{Keep, LinkAccess, LinkExpiry, ShareOptions};
use super::dialogs::Dialog;
use super::share::PASSWORD_INPUT;
use super::source::{Action, ActionError, Call, FilesSource, NoEngine};
use super::tests::*;
use super::{Intent, Toast};
use crate::State;
use crate::init;
use crate::shell::app::{Dialog as ShellDialog, Event, Intent as ShellIntent, Screen, Section};
use rune::{Key, Runtime};

fn pick(rt: &mut Runtime<State>, label: &str) {
    right_click(rt, "row-6-date");
    let item = rt
        .elements()
        .into_iter()
        .find(|e| e.role == "menuitem" && e.label.as_deref() == Some(label))
        .unwrap_or_else(|| panic!("no menu item {label}"));
    click(rt, &item.key.unwrap());
}

fn double_click(rt: &mut Runtime<State>, key: &str) {
    let (x, y) = centre(rt, key);
    for _ in 0..2 {
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
    }
    settle(rt);
}

fn type_text(rt: &mut Runtime<State>, text: &str) {
    for c in text.chars() {
        press(rt, Key::Text(c.to_string()), NONE);
    }
}

fn label_of(rt: &Runtime<State>, key: &str) -> String {
    info(rt, key).label.unwrap_or_default()
}

fn text_of(rt: &Runtime<State>, key: &str) -> String {
    info(rt, key).text.unwrap_or_default()
}

fn share_of(rt: &Runtime<State>) -> Option<&super::share::Share> {
    match &rt.state.actions.dialog {
        Some(Dialog::Share(share)) => Some(share),
        _ => None,
    }
}

fn conflict_file(rt: &Runtime<State>) -> Option<Option<String>> {
    match rt.state.shell.app.dialog() {
        Some(ShellDialog::Conflict { file }) => Some(file.clone()),
        _ => None,
    }
}

/// The board's window with its two waiting files, the test keeping its handle on the calls.
fn conflicted() -> (Runtime<State>, Calls) {
    let (rt, calls, _) = conflicted_with(None);
    (rt, calls)
}

fn conflicted_with(refuse: Option<(Action, ActionError)>) -> (Runtime<State>, Calls, Fails) {
    let mut state = init();
    let fixture = conflict_fixture();
    let (calls, fails) = (fixture.calls.clone(), fixture.fail.clone());
    fails.lock().unwrap().extend(refuse);
    install(&mut state, fixture);
    let mut rt = runtime_of(state);
    click(&mut rt, "row-6-date");
    (rt, calls, fails)
}

// ---- Share (DW-0407) ----

#[test]
fn share_opens_from_the_menu_and_the_panel_as_a_named_modal_on_the_boards_defaults() {
    let (mut rt, calls, _) = spied();
    pick(&mut rt, "Share\u{2026}");
    let dialog = info(&rt, "dlg");
    assert_eq!((dialog.role, dialog.label.as_deref()), ("dialog", Some("Share")));
    assert_eq!(label_of(&rt, "item-0-0"), "Anyone with the link, Can view");
    assert_eq!(label_of(&rt, "item-0-1"), "Link expires, In 30 days");
    let switch = info(&rt, "item-0-2-switch");
    assert_eq!((switch.role, switch.label.as_deref()), ("switch", Some("Password")));
    assert!(share_of(&rt).is_some_and(|s| !s.password_on), "Password is off");
    for (key, name) in [
        ("btn-0", "Copy link"),
        ("btn-1", "Invite someone\u{2026}"),
        ("btn-2", "Cancel"),
    ] {
        let b = info(&rt, key);
        assert_eq!((b.role, b.label.as_deref(), b.focusable), ("button", Some(name), true));
    }
    assert_eq!(text_of(&rt, "dlg-file-name"), "Contract draft.pdf");
    assert_eq!(text_of(&rt, "dlg-file-sub"), "1.2 MB");
    assert!(
        find(&rt, PASSWORD_INPUT).is_none(),
        "no password field until it is asked for"
    );
    // The list behind hears no key, and Esc shares nothing.
    press(&mut rt, Key::Delete, NONE);
    assert_eq!(rt.state.listing.items.len(), 11);
    press(&mut rt, Key::Escape, NONE);
    assert!(rt.state.actions.dialog.is_none());
    // The details panel's button opens the same dialog.
    click(&mut rt, "share-box");
    assert!(share_of(&rt).is_some());
    press(&mut rt, Key::Escape, NONE);
    assert!(asked(&calls).is_empty(), "opening and closing shares nothing");
}

#[test]
fn cancel_shares_nothing_wipes_the_password_and_the_choices_start_again() {
    let (mut rt, calls, _) = spied();
    pick(&mut rt, "Share\u{2026}");
    click(&mut rt, "item-0-0");
    assert_eq!(info(&rt, "menu-0").label.as_deref(), Some("Anyone with the link"));
    click(&mut rt, "menu-0-item-1");
    assert_eq!(share_of(&rt).map(|s| s.access), Some(LinkAccess::Download));
    assert_eq!(label_of(&rt, "item-0-0"), "Anyone with the link, Can download");
    click(&mut rt, "item-0-2-switch");
    assert_eq!(
        focused(&rt).as_deref(),
        Some(PASSWORD_INPUT),
        "the field has the keyboard"
    );
    type_text(&mut rt, "hunter2");
    assert_eq!(rt.state.actions.share_password.text, "hunter2");
    click(&mut rt, "btn-2");
    assert!(rt.state.actions.dialog.is_none(), "Cancel closes");
    assert!(asked(&calls).is_empty(), "Cancel asks nothing of the source");
    assert_eq!(rt.state.actions.share_password.text, "", "the password is wiped");
    assert_eq!(rt.state.live_toast(), None);
    // Opened again: the board's defaults, not the last choices.
    pick(&mut rt, "Share\u{2026}");
    let share = share_of(&rt).unwrap();
    assert_eq!(
        (share.access, share.expires, share.password_on),
        (LinkAccess::View, LinkExpiry::Month, false)
    );
}

#[test]
fn copy_link_asks_once_with_the_choices_and_then_says_link_copied() {
    let (mut rt, calls, _) = spied();
    pick(&mut rt, "Share\u{2026}");
    click(&mut rt, "item-0-0");
    click(&mut rt, "menu-0-item-1");
    click(&mut rt, "item-0-1");
    assert_eq!(info(&rt, "menu-0").label.as_deref(), Some("Link expires"));
    click(&mut rt, "menu-0-item-1");
    assert_eq!(label_of(&rt, "item-0-1"), "Link expires, In 7 days");
    click(&mut rt, "btn-0");
    assert_eq!(
        asked(&calls),
        [Call::ShareLink(
            ids(&["contract"]),
            ShareOptions {
                access: LinkAccess::Download,
                expires: LinkExpiry::Week,
                password: None,
            }
        )]
    );
    assert!(rt.state.actions.dialog.is_none(), "the dialog closes");
    assert_eq!(rt.state.live_toast(), Some(Toast::LinkCopied));
    assert_eq!(info(&rt, "toast").label.as_deref(), Some("Link copied"));
}

#[test]
fn the_keyboard_arrives_on_a_choice_and_only_copy_link_itself_makes_the_link() {
    // On arrival the keyboard is on the first choice, never on the button that sends: Space
    // and Enter there open that choice's pop-up and make no link.
    for key in [Key::Enter, Key::Text(" ".into())] {
        let (mut rt, calls, _) = spied();
        pick(&mut rt, "Share\u{2026}");
        assert_eq!(focused(&rt).as_deref(), Some("item-0-0"), "arrival focus");
        press(&mut rt, key, NONE);
        assert!(asked(&calls).is_empty(), "a key on arrival sends nothing");
        assert!(share_of(&rt).is_some(), "the dialog is still open");
        assert_eq!(rt.state.live_toast(), None);
    }
    // Copy link by the keyboard: Tab to it, Enter.
    let (mut rt, calls, _) = spied();
    pick(&mut rt, "Share\u{2026}");
    tab_to(&mut rt, "btn-0");
    press(&mut rt, Key::Enter, NONE);
    assert_eq!(
        asked(&calls),
        [Call::ShareLink(ids(&["contract"]), ShareOptions::default())]
    );
    assert_eq!(
        ShareOptions::default().expires,
        LinkExpiry::Month,
        "30 days unless chosen"
    );
}

#[test]
fn a_link_that_asks_for_a_password_never_goes_out_without_one() {
    let (mut rt, calls, _) = spied();
    pick(&mut rt, "Share\u{2026}");
    click(&mut rt, "item-0-2-switch");
    assert!(share_of(&rt).is_some_and(|s| s.password_on));
    // Nothing typed: no link, and the dialog says what is missing.
    click(&mut rt, "btn-0");
    assert!(
        asked(&calls).is_empty(),
        "no link without the password it should ask for"
    );
    assert!(share_of(&rt).is_some_and(|s| s.password_missing));
    assert_eq!(focused(&rt).as_deref(), Some(PASSWORD_INPUT));
    type_text(&mut rt, "s3cret-words");
    // The characters are in no element: Rune draws a secure field as bullets.
    assert!(
        rt.elements()
            .iter()
            .all(|e| e.text.as_deref().is_none_or(|t| !t.contains("s3cret"))
                && e.label.as_deref().is_none_or(|l| !l.contains("s3cret"))),
        "the password is in the element tree"
    );
    press(&mut rt, Key::Enter, NONE);
    let made = asked(&calls);
    assert_eq!(made.len(), 1, "exactly one link");
    let Call::ShareLink(what, options) = &made[0] else {
        panic!("not a link: {made:?}");
    };
    assert_eq!(what, &ids(&["contract"]));
    assert_eq!(options.password.as_deref(), Some("s3cret-words"));
    assert!(!format!("{made:?}").contains("s3cret"), "a password is never printed");
    assert_eq!(rt.state.actions.share_password.text, "", "wiped once the link is made");
    assert_eq!(rt.state.live_toast(), Some(Toast::LinkCopied));
}

#[test]
fn turning_the_password_off_wipes_it_and_the_link_then_has_none() {
    let (mut rt, calls, _) = spied();
    pick(&mut rt, "Share\u{2026}");
    click(&mut rt, "item-0-2-switch");
    type_text(&mut rt, "abc");
    click(&mut rt, "item-0-2-switch");
    assert_eq!(rt.state.actions.share_password.text, "");
    assert!(find(&rt, PASSWORD_INPUT).is_none());
    click(&mut rt, "btn-0");
    assert_eq!(
        asked(&calls),
        [Call::ShareLink(ids(&["contract"]), ShareOptions::default())]
    );
}

#[test]
fn a_link_that_could_not_be_made_says_so_and_never_says_copied() {
    let (mut rt, calls, fails) = spied();
    fails.lock().unwrap().push((Action::ShareLink, ActionError::Offline));
    pick(&mut rt, "Share\u{2026}");
    click(&mut rt, "btn-0");
    assert!(asked(&calls).is_empty());
    assert_eq!(
        rt.state.live_toast(),
        Some(Toast::Failed(Action::ShareLink, ActionError::Offline))
    );
}

/// Tab until `key` holds the keyboard.
fn tab_to(rt: &mut Runtime<State>, key: &str) {
    for _ in 0..16 {
        if focused(rt).as_deref() == Some(key) {
            return;
        }
        press(rt, Key::Tab, NONE);
    }
    panic!("Tab never reached {key}; the keyboard is on {:?}", focused(rt));
}

#[test]
fn try_again_after_a_link_failed_opens_the_dialog_again_and_never_makes_a_plain_link() {
    let (mut rt, calls, fails) = spied();
    fails.lock().unwrap().push((Action::ShareLink, ActionError::Offline));
    pick(&mut rt, "Share\u{2026}");
    click(&mut rt, "item-0-0");
    click(&mut rt, "menu-0-item-1");
    click(&mut rt, "item-0-1");
    click(&mut rt, "menu-0-item-0");
    click(&mut rt, "item-0-2-switch");
    type_text(&mut rt, "s3cret-words");
    let (access, expires) = share_of(&rt).map(|s| (s.access, s.expires)).unwrap();
    assert_ne!(
        (access, expires),
        (LinkAccess::default(), LinkExpiry::default()),
        "the choices are not the defaults"
    );
    click(&mut rt, "btn-0");
    assert_eq!(
        rt.state.live_toast(),
        Some(Toast::Failed(Action::ShareLink, ActionError::Offline))
    );
    assert!(asked(&calls).is_empty(), "the refused link is no link");
    assert_eq!(rt.state.actions.share_password.text, "", "wiped with the dialog");
    // Another row is selected meanwhile: Try again is about the file the dialog was about.
    click(&mut rt, "row-2-date");
    fails.lock().unwrap().clear();

    click(&mut rt, "toast-action");
    assert!(
        asked(&calls).is_empty(),
        "Try again asks nothing by itself: no plain link"
    );
    let again = share_of(&rt).expect("the Share dialog is open again");
    assert_eq!(
        again.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
        ["contract"]
    );
    assert_eq!(
        (again.access, again.expires, again.password_on),
        (access, expires, true)
    );
    assert_eq!(rt.state.actions.share_password.text, "", "the password is typed again");
    assert!(rt.state.actions.retry_share.is_none(), "kept for one Try again only");
    // Copy link without the password still makes no link.
    click(&mut rt, "btn-0");
    assert!(asked(&calls).is_empty());
    type_text(&mut rt, "s3cret-words");
    click(&mut rt, "btn-0");
    let made = asked(&calls);
    assert_eq!(made.len(), 1, "exactly one request: {made:?}");
    let Call::ShareLink(what, options) = &made[0] else {
        panic!("not the full request: {made:?}");
    };
    assert_eq!(what, &ids(&["contract"]));
    assert_eq!((options.access, options.expires), (access, expires));
    assert_eq!(options.password.as_deref(), Some("s3cret-words"));
    assert_eq!(
        made.iter().filter(|c| matches!(c, Call::CopyLink(_))).count(),
        0,
        "0 plain links"
    );
    assert_eq!(rt.state.live_toast(), Some(Toast::LinkCopied));
}

#[test]
fn the_gates_notes_never_carry_a_passwords_characters() {
    let (mut rt, _, _) = spied();
    pick(&mut rt, "Share\u{2026}");
    click(&mut rt, "item-0-2-switch");
    type_text(&mut rt, "s3cret-words");
    // Select all of it: a selection is what the note would otherwise quote.
    rt.update(|s, _| s.actions.share_password.anchor = 0);
    settle(&mut rt);
    assert_eq!(rt.state.actions.share_password.text, "s3cret-words");
    let (from, to) = rt.state.actions.share_password.range();
    assert_ne!(from, to, "the password is selected");
    assert!(
        find(&rt, "field-0-caret-at").is_none(),
        "a secure field's selection is neither read nor measured for the gate"
    );
    crate::ui::kit::record();
    let share = share_of(&rt).unwrap();
    let _ = super::share::view(&rt.state, share, &crate::theme::tokens::theme(false));
    let notes = crate::ui::kit::take();
    assert!(notes.len() > 10, "the view was noted: {} notes", notes.len());
    let texts: Vec<&str> = notes.values().filter_map(|n| n.text.as_deref()).collect();
    assert!(
        texts.iter().all(|t| !t.contains("s3cret") && !t.contains("words")),
        "a note carries the password"
    );
    assert_eq!(
        notes.get("field-0-value").and_then(|n| n.text.as_deref()),
        Some("\u{2022}".repeat(12).as_str()),
        "the gate is told the bullets' count"
    );
}

#[test]
fn enter_on_a_focused_button_of_share_presses_that_button_not_copy_link() {
    // Cancel.
    let (mut rt, calls, _) = spied();
    pick(&mut rt, "Share\u{2026}");
    tab_to(&mut rt, "btn-2");
    press(&mut rt, Key::Enter, NONE);
    assert!(share_of(&rt).is_none(), "Enter on Cancel cancels");
    assert!(asked(&calls).is_empty(), "and makes no link");
    assert_eq!(rt.state.live_toast(), None);
    // Invite someone…
    pick(&mut rt, "Share\u{2026}");
    tab_to(&mut rt, "btn-1");
    press(&mut rt, Key::Enter, NONE);
    assert_eq!(rt.state.actions.intents.last(), Some(&Intent::Frame("DW-0514")));
    assert!(asked(&calls).is_empty(), "Enter on Invite makes no link");
    // The Password switch.
    let (mut rt, calls, _) = spied();
    pick(&mut rt, "Share\u{2026}");
    tab_to(&mut rt, "item-0-2-switch");
    press(&mut rt, Key::Enter, NONE);
    assert!(
        share_of(&rt).is_some_and(|s| s.password_on),
        "Enter on the switch turns it"
    );
    assert!(asked(&calls).is_empty());
}

#[test]
fn enter_on_a_focused_button_of_two_versions_presses_that_button_not_keep_both() {
    // Cancel: nothing is decided.
    let (mut rt, calls) = conflicted();
    press(&mut rt, Key::Enter, NONE);
    assert!(conflict_file(&rt).is_some());
    tab_to(&mut rt, "btn-3");
    press(&mut rt, Key::Enter, NONE);
    assert_eq!(conflict_file(&rt), None, "Enter on Cancel cancels");
    assert!(asked(&calls).is_empty(), "and answers nothing");
    assert_eq!(rt.state.actions.source.conflicts().len(), 2);
    // Keep this PC's: that answer, once.
    press(&mut rt, Key::Enter, NONE);
    tab_to(&mut rt, "btn-1");
    press(&mut rt, Key::Enter, NONE);
    assert_eq!(asked(&calls), [Call::KeepVersion("contract".into(), Keep::Here)]);
}

#[test]
fn invite_someone_goes_to_the_shared_sections_invite_dialog() {
    let (mut rt, calls, _) = spied();
    pick(&mut rt, "Share\u{2026}");
    click(&mut rt, "btn-1");
    assert!(rt.state.actions.dialog.is_none());
    assert_eq!(rt.state.actions.intents.last(), Some(&Intent::Frame("DW-0514")));
    assert!(
        matches!(rt.state.shell.app.screen(), Screen::Drive(route) if route.section == Section::Shared),
        "the Shared section shows"
    );
    assert!(matches!(
        &rt.state.frames.shared.dialog,
        Some(crate::frames::shared::dialogs::Dialog::Invite { share, .. }) if share == "p-contract"
    ));
    assert!(asked(&calls).is_empty(), "no link was made on the way");
}

// ---- Details (DW-0411) ----

#[test]
fn alt_enter_opens_details_with_the_boards_rows_and_done_closes_it() {
    let (mut rt, calls, _) = spied();
    press(&mut rt, Key::Enter, ALT);
    let dialog = info(&rt, "dlg");
    assert_eq!((dialog.role, dialog.label.as_deref()), ("dialog", Some("Details")));
    assert_eq!(text_of(&rt, "dlg-file-name"), "Contract draft.pdf");
    assert_eq!(text_of(&rt, "dlg-file-sub"), "PDF document");
    let rows: Vec<(String, String)> = (0..7)
        .map(|i| {
            (
                text_of(&rt, &format!("item-0-{i}-title")),
                text_of(&rt, &format!("item-0-{i}-value")),
            )
        })
        .collect();
    let want = [
        ("Size", "1.2 MB"),
        ("Changed", "Yesterday, 16:42"),
        ("Added", "28 Oct 2026"),
        ("Where", "Files"),
        ("On this PC", "No"),
        ("Encrypted", "Yes, end to end"),
        ("Version history", "3 versions"),
    ];
    assert_eq!(
        rows.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect::<Vec<_>>(),
        want
    );
    assert!(find(&rt, "item-0-7").is_none());
    let done = info(&rt, "btn-0");
    assert_eq!((done.role, done.label.as_deref()), ("button", Some("Done")));
    click(&mut rt, "btn-0");
    assert!(rt.state.actions.dialog.is_none());
    // The menu's Details opens it too; Esc closes.
    pick(&mut rt, "Details");
    assert!(matches!(rt.state.actions.dialog, Some(Dialog::Details(_))));
    press(&mut rt, Key::Escape, NONE);
    assert!(rt.state.actions.dialog.is_none());
    assert!(asked(&calls).is_empty());
}

#[test]
fn details_of_a_file_on_this_pc_says_yes_and_its_last_row_goes_to_version_history() {
    let (mut rt, _, _) = spied();
    click(&mut rt, "row-5-date");
    press(&mut rt, Key::Enter, ALT);
    let on_pc = (0..7)
        .find(|i| find(&rt, &format!("item-0-{i}-title")).is_some_and(|e| e.text.as_deref() == Some("On this PC")))
        .unwrap();
    assert_eq!(text_of(&rt, &format!("item-0-{on_pc}-value")), "Yes");
    press(&mut rt, Key::Escape, NONE);
    click(&mut rt, "row-6-date");
    press(&mut rt, Key::Enter, ALT);
    let link = info(&rt, "item-0-6");
    assert_eq!((link.role, link.focusable), ("link", true));
    click(&mut rt, "item-0-6");
    assert!(matches!(&rt.state.actions.dialog, Some(Dialog::Versions(item)) if item.id == "contract"));
    assert_eq!(info(&rt, "dlg").label.as_deref(), Some("Version history"));
}

// ---- Version history (DW-0413) ----

#[test]
fn version_history_lists_the_versions_and_only_earlier_ones_can_be_restored() {
    let (mut rt, calls, _) = spied();
    pick(&mut rt, "Version history\u{2026}");
    assert_eq!(text_of(&rt, "dlg-sub"), "Contract draft.pdf \u{00B7} 3 versions");
    let rows: Vec<(String, String)> = (0..3)
        .map(|i| {
            (
                text_of(&rt, &format!("item-0-{i}-title")),
                text_of(&rt, &format!("item-0-{i}-sub")),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (
                "Yesterday, 16:42".to_string(),
                "1.2 MB \u{00B7} Current version".to_string()
            ),
            ("Yesterday, 11:05".to_string(), "1.1 MB".to_string()),
            ("28 Oct, 14:20".to_string(), "980 KB".to_string()),
        ]
    );
    assert_eq!(text_of(&rt, "item-0-0-pill-label"), "Current");
    assert_eq!(
        [label_of(&rt, "btn-0"), label_of(&rt, "btn-1"), label_of(&rt, "btn-2")],
        ["Restore", "Restore", "Done"]
    );
    assert!(find(&rt, "btn-3").is_none(), "the current version has no Restore");
    assert!(label_of(&rt, "item-0-0").contains("Changed on Anna's iPhone"));
    // The details panel's button opens the same dialog; Done and Esc close; nothing was asked.
    click(&mut rt, "btn-2");
    assert!(rt.state.actions.dialog.is_none());
    click(&mut rt, "versions-box");
    assert!(matches!(rt.state.actions.dialog, Some(Dialog::Versions(_))));
    press(&mut rt, Key::Escape, NONE);
    assert!(rt.state.actions.dialog.is_none());
    assert!(asked(&calls).is_empty());
}

#[test]
fn restore_asks_once_and_every_version_that_was_there_is_still_there() {
    let (mut rt, calls, _) = spied();
    pick(&mut rt, "Version history\u{2026}");
    click(&mut rt, "btn-0");
    assert_eq!(
        asked(&calls),
        [Call::RestoreVersion("contract".into(), "contract-v2".into())]
    );
    assert!(
        matches!(rt.state.actions.dialog, Some(Dialog::Versions(_))),
        "the dialog stays and shows the result"
    );
    assert_eq!(text_of(&rt, "dlg-sub"), "Contract draft.pdf \u{00B7} 4 versions");
    let whens: Vec<String> = (0..4).map(|i| text_of(&rt, &format!("item-0-{i}-title"))).collect();
    assert_eq!(
        whens,
        ["Just now", "Yesterday, 16:42", "Yesterday, 11:05", "28 Oct, 14:20"]
    );
    assert_eq!(text_of(&rt, "item-0-0-sub"), "1.1 MB \u{00B7} Current version");
    assert_eq!(
        text_of(&rt, "item-0-1-sub"),
        "1.2 MB",
        "the version that was current stays, and can be gone back to"
    );
    assert_eq!(
        [label_of(&rt, "btn-0"), label_of(&rt, "btn-3")],
        ["Restore".to_string(), "Done".to_string()]
    );
    // The details panel counts the same versions.
    press(&mut rt, Key::Escape, NONE);
    assert_eq!(text_of(&rt, "detail-version-history-value"), "4 versions");
}

#[test]
fn a_restore_that_is_refused_changes_nothing_and_says_so() {
    let (mut rt, calls, fails) = spied();
    fails
        .lock()
        .unwrap()
        .push((Action::RestoreVersion, ActionError::Offline));
    pick(&mut rt, "Version history\u{2026}");
    click(&mut rt, "btn-0");
    assert!(asked(&calls).is_empty());
    assert_eq!(text_of(&rt, "dlg-sub"), "Contract draft.pdf \u{00B7} 3 versions");
    assert_eq!(
        rt.state.live_toast(),
        Some(Toast::Failed(Action::RestoreVersion, ActionError::Offline))
    );
    assert_eq!(
        Toast::failure(Action::RestoreVersion, ActionError::Offline),
        "Couldn't go back to that version. Check your connection."
    );
}

#[test]
fn a_double_click_on_a_version_previews_the_file() {
    let (mut rt, _, _) = spied();
    pick(&mut rt, "Version history\u{2026}");
    double_click(&mut rt, "item-0-1-title");
    assert!(matches!(&rt.state.actions.dialog, Some(Dialog::Preview(p)) if p.item.id == "contract"));
}

// ---- A window with nothing behind it ----

#[test]
fn a_source_that_knows_nothing_gives_no_sample_data_and_refuses_every_action() {
    let mut source = NoEngine;
    let item = moodboard();
    assert!(source.versions(&item).is_empty());
    assert!(source.conflicts().is_empty());
    assert!(source.preview(&item).is_none());
    assert_eq!(source.share_of(&item), None);
    let refused = Err(ActionError::Unavailable);
    assert_eq!(
        source.share_link(std::slice::from_ref(&item), &ShareOptions::default()),
        refused
    );
    assert_eq!(source.restore_version(&item, "v1"), refused);
    assert_eq!(source.open_with(&item), refused);
    assert_eq!(source.keep_version("moodboard", Keep::Both), refused);
}

#[test]
fn without_an_engine_version_history_lists_nothing_and_says_so() {
    let mut state = init();
    state.actions.source = Box::new(NoEngine);
    let mut rt = runtime_of(state);
    click(&mut rt, "row-6-date");
    pick(&mut rt, "Version history\u{2026}");
    assert_eq!(text_of(&rt, "dlg-sub"), "Contract draft.pdf");
    assert!(find(&rt, "item-0-0").is_none(), "no invented version");
    assert_eq!(
        text_of(&rt, "dlg-text-0"),
        "The versions of this file can't be shown right now."
    );
    assert_eq!(label_of(&rt, "btn-0"), "Done");
}

// ---- Preview (DW-0408, DW-0408b) ----

#[test]
fn space_previews_in_the_window_and_space_or_esc_closes_without_leaving_files() {
    let (mut rt, calls, _) = spied();
    let route = rt.state.shell.app.screen();
    press(&mut rt, Key::Text(" ".into()), NONE);
    assert!(matches!(&rt.state.actions.dialog, Some(Dialog::Preview(p)) if p.shown.is_some()));
    let card = info(&rt, "pv-card");
    assert_eq!(
        (card.role, card.label.as_deref()),
        ("dialog", Some("Preview of Contract draft.pdf"))
    );
    assert_eq!(text_of(&rt, "pv-name"), "Contract draft.pdf");
    assert_eq!(text_of(&rt, "pv-place"), "Page 1 of 6");
    assert_eq!(text_of(&rt, "pv-page-title"), "Service agreement");
    for (key, name) in [
        ("pv-close", "Close preview"),
        ("pv-share", "Share\u{2026}"),
        ("pv-open", "Open in its app"),
    ] {
        let b = info(&rt, key);
        assert_eq!((b.role, b.label.as_deref(), b.focusable), ("button", Some(name), true));
    }
    assert_eq!(rt.state.shell.app.screen(), route, "Preview never moves the route");
    // The list behind hears no key.
    press(&mut rt, Key::Delete, NONE);
    assert_eq!(rt.state.listing.items.len(), 11);
    press(&mut rt, Key::Text(" ".into()), NONE);
    assert!(rt.state.actions.dialog.is_none(), "Space closes");
    press(&mut rt, Key::Text(" ".into()), NONE);
    press(&mut rt, Key::Escape, NONE);
    assert!(rt.state.actions.dialog.is_none(), "Esc closes");
    pick(&mut rt, "Preview");
    click(&mut rt, "pv-close");
    assert!(rt.state.actions.dialog.is_none(), "the cross closes");
    assert!(asked(&calls).is_empty(), "previewing opens nothing in another app");
}

#[test]
fn left_and_right_preview_the_neighbouring_files_and_skip_folders() {
    let (mut rt, _, _) = spied();
    press(&mut rt, Key::Text(" ".into()), NONE);
    press(&mut rt, Key::Right, NONE);
    assert_eq!(rt.state.selected, Some(7));
    assert!(matches!(&rt.state.actions.dialog, Some(Dialog::Preview(p)) if p.item.id == "harbour"));
    press(&mut rt, Key::Left, NONE);
    press(&mut rt, Key::Left, NONE);
    assert_eq!(rt.state.selected, Some(5), "Budget 2026.xlsx");
    // Above it are folders only: Left stays.
    press(&mut rt, Key::Left, NONE);
    assert_eq!(rt.state.selected, Some(5));
    assert!(matches!(&rt.state.actions.dialog, Some(Dialog::Preview(_))));
}

#[test]
fn the_previews_open_opens_the_file_and_its_share_goes_to_the_share_dialog() {
    let (mut rt, calls, _) = spied();
    press(&mut rt, Key::Text(" ".into()), NONE);
    click(&mut rt, "pv-open");
    assert!(rt.state.actions.dialog.is_none());
    // A PDF opens in the window's viewer (`frames::viewers::open_item`): the route is on the
    // file, and it is not also handed to another app.
    assert!(
        matches!(rt.state.shell.app.screen(), Screen::Drive(route) if route.item.as_deref() == Some("contract")),
        "Open in its app shows the file"
    );
    assert!(asked(&calls).is_empty());
    rt.update(|s, _| crate::frames::viewers::close(s));
    settle(&mut rt);
    assert!(matches!(rt.state.shell.app.screen(), Screen::Drive(route) if route.item.is_none()));
    click(&mut rt, "row-6-date");
    press(&mut rt, Key::Text(" ".into()), NONE);
    click(&mut rt, "pv-share");
    assert!(share_of(&rt).is_some_and(|s| s.items[0].id == "contract"));
    assert_eq!(info(&rt, "dlg").label.as_deref(), Some("Share"));
}

#[test]
fn a_file_with_no_preview_says_so_and_offers_the_two_ways_on() {
    let (mut rt, calls, _) = spied();
    rt.update(|s, _| {
        s.listing.items.insert(9, moodboard());
        s.select_only(9);
    });
    settle(&mut rt);
    press(&mut rt, Key::Text(" ".into()), NONE);
    assert!(matches!(&rt.state.actions.dialog, Some(Dialog::Preview(p)) if p.shown.is_none()));
    assert_eq!(text_of(&rt, "pv-name"), "Moodboard.sketch");
    assert_eq!(text_of(&rt, "pv-blank-title"), "No preview for this kind of file");
    assert_eq!(
        text_of(&rt, "pv-blank-text"),
        "Moodboard.sketch \u{00B7} 18.4 MB. Open it in another app, or keep it on this PC to open it offline."
    );
    assert!(
        find(&rt, "pv-open").is_none(),
        "no Open in its app: Windows has none for it"
    );
    assert!(asked(&calls).is_empty(), "nothing is downloaded until a button is used");
    click(&mut rt, "pv-open-with");
    assert_eq!(asked(&calls), [Call::OpenWith("moodboard".into())]);
    assert!(rt.state.actions.dialog.is_none());
    // Make available offline keeps it on this PC (DW-0412) and says so.
    press(&mut rt, Key::Text(" ".into()), NONE);
    click(&mut rt, "pv-keep");
    assert_eq!(
        asked(&calls).last(),
        Some(&Call::KeepOffline(ids(&["moodboard"]), true))
    );
    assert_eq!(rt.state.live_toast(), Some(Toast::Keeping));
    assert_eq!(
        rt.state.listing.items[9].pc_status.as_deref(),
        Some("available_offline")
    );
    assert!(rt.state.actions.dialog.is_none());
}

#[test]
fn open_goes_to_the_viewer_for_a_file_it_renders_and_to_the_files_app_otherwise() {
    let (mut rt, calls, _) = spied();
    // A file of a type the window has no viewer for goes to its own app.
    click(&mut rt, "row-9-date");
    let other = rt.state.listing.items[9].id.clone();
    press(&mut rt, Key::Enter, NONE);
    assert_eq!(asked(&calls), [Call::Open(vec![other])]);
    assert!(matches!(rt.state.shell.app.screen(), Screen::Drive(route) if route.item.is_none()));
    // A file the viewer renders (its sample's notes): Open shows it in the window.
    let notes = crate::frames::viewers::source::board::NOTES;
    rt.update(|s, _| {
        s.listing.items[6].id = notes.to_string();
        s.select_only(6);
    });
    settle(&mut rt);
    press(&mut rt, Key::Enter, NONE);
    assert!(
        matches!(rt.state.shell.app.screen(), Screen::Drive(route) if route.item.as_deref() == Some(notes)),
        "the route is on the file"
    );
    assert_eq!(asked(&calls).len(), 1, "it was not also handed to another app");
}

// ---- Two versions (DW-0429) ----

#[test]
fn the_row_says_two_versions_and_opening_it_opens_the_choice_not_the_file() {
    let (mut rt, calls) = conflicted();
    assert_eq!(text_of(&rt, "row-6-date"), "Two versions");
    assert_eq!(
        text_of(&rt, "row-5-date"),
        "Today, 09:12",
        "only the file that waits says so"
    );
    press(&mut rt, Key::Enter, NONE);
    assert_eq!(conflict_file(&rt), Some(Some("contract".to_string())));
    let dialog = info(&rt, "dlg");
    assert_eq!((dialog.role, dialog.label.as_deref()), ("dialog", Some("Two versions")));
    assert_eq!(
        text_of(&rt, "dlg-sub"),
        "1 of 2 \u{00B7} Contract draft.pdf changed in two places at once"
    );
    assert_eq!(
        [
            label_of(&rt, "btn-0"),
            label_of(&rt, "btn-1"),
            label_of(&rt, "btn-2"),
            label_of(&rt, "btn-3")
        ],
        ["Keep both", "Keep this PC's", "Keep Anna's iPad's", "Cancel"]
    );
    assert_eq!(label_of(&rt, "version-here"), "This PC, Today, 09:12, 1.2 MB");
    assert_eq!(label_of(&rt, "version-there"), "Anna's iPad, Today, 08:50, 1.3 MB");
    assert!(asked(&calls).is_empty(), "nothing is opened and nothing is decided yet");
    // A double-click on the row opens the choice too.
    click(&mut rt, "btn-3");
    assert_eq!(conflict_file(&rt), None);
    double_click(&mut rt, "row-6-date");
    assert_eq!(conflict_file(&rt), Some(Some("contract".to_string())));
}

#[test]
fn cancel_and_esc_decide_nothing_and_both_versions_keep_waiting() {
    let (mut rt, calls) = conflicted();
    for leave in ["cancel", "esc"] {
        click(&mut rt, "row-6-date");
        press(&mut rt, Key::Enter, NONE);
        assert!(conflict_file(&rt).is_some());
        if leave == "cancel" {
            click(&mut rt, "btn-3");
        } else {
            press(&mut rt, Key::Escape, NONE);
        }
        assert_eq!(conflict_file(&rt), None, "{leave} closes");
        assert!(asked(&calls).is_empty(), "{leave} asks nothing of the source");
        assert_eq!(
            rt.state.actions.source.conflicts().len(),
            2,
            "{leave}: both files still wait"
        );
        assert_eq!(text_of(&rt, "row-6-date"), "Two versions");
        assert_eq!(rt.state.listing.items.len(), 11);
    }
}

#[test]
fn keep_both_is_the_default_asks_once_and_saves_the_other_version_beside_the_file() {
    let (mut rt, calls) = conflicted();
    let before = rt.state.actions.source.versions(&rt.state.listing.items[6]).len();
    press(&mut rt, Key::Enter, NONE);
    // Enter is Keep both: the default.
    press(&mut rt, Key::Enter, NONE);
    assert_eq!(asked(&calls), [Call::KeepVersion("contract".into(), Keep::Both)]);
    let names = names(&rt);
    assert_eq!(names.len(), 12);
    assert_eq!(
        [names[6].as_str(), names[7].as_str()],
        ["Contract draft.pdf", "Contract draft (Anna's iPad).pdf"],
        "the other device's version stands next to the file, its device's name added"
    );
    assert_eq!(
        rt.state.actions.source.versions(&rt.state.listing.items[6]).len(),
        before,
        "the file's own versions are untouched"
    );
    // The next file's choice follows.
    assert_eq!(conflict_file(&rt), Some(Some("doc-shot-list".to_string())));
    assert_eq!(
        text_of(&rt, "dlg-sub"),
        "2 of 2 \u{00B7} Shot list.docx changed in two places at once"
    );
    assert_eq!(label_of(&rt, "btn-2"), "Keep Anna's iPhone's");
    click(&mut rt, "btn-3");
    assert_eq!(asked(&calls).len(), 1, "Cancel on the second asks nothing");
    assert_eq!(rt.state.actions.source.conflicts().len(), 1, "and it keeps waiting");
}

#[test]
fn keeping_one_version_asks_once_and_the_other_stays_in_version_history() {
    for (button, keep, current, other) in [
        ("btn-1", Keep::Here, "Today, 09:12", "Today, 08:50"),
        ("btn-2", Keep::There, "Today, 08:50", "Today, 09:12"),
    ] {
        let (mut rt, calls) = conflicted();
        let item = rt.state.listing.items[6].clone();
        let before = rt.state.actions.source.versions(&item);
        press(&mut rt, Key::Enter, NONE);
        click(&mut rt, button);
        assert_eq!(asked(&calls), [Call::KeepVersion("contract".into(), keep)], "{button}");
        let after = rt.state.actions.source.versions(&item);
        assert_eq!(
            after.len(),
            before.len() + 2,
            "{button}: both versions are in the history"
        );
        assert_eq!((after[0].when.as_str(), after[0].current), (current, true), "{button}");
        assert_eq!((after[1].when.as_str(), after[1].current), (other, false), "{button}");
        for old in &before {
            assert!(after.iter().any(|v| v.id == old.id), "{button}: {} was lost", old.when);
        }
        assert_eq!(
            rt.state.listing.items.len(),
            11,
            "{button}: no file was added or removed"
        );
        assert_eq!(
            text_of(&rt, "row-6-date"),
            "Yesterday, 16:42",
            "{button}: it no longer waits"
        );
    }
}

#[test]
fn an_answer_that_is_refused_decides_nothing_and_says_so() {
    let (mut rt, calls, _) = conflicted_with(Some((Action::KeepVersion, ActionError::Offline)));
    press(&mut rt, Key::Enter, NONE);
    click(&mut rt, "btn-1");
    assert!(asked(&calls).is_empty());
    assert_eq!(rt.state.actions.source.conflicts().len(), 2);
    assert_eq!(conflict_file(&rt), None);
    assert_eq!(
        rt.state.live_toast(),
        Some(Toast::Failed(Action::KeepVersion, ActionError::Offline))
    );
}

#[test]
fn the_flyouts_review_opens_the_first_waiting_file() {
    let (mut rt, calls) = conflicted();
    rt.update(|s, _| {
        crate::shell::host::dispatch(s, Event::Request(ShellIntent::ReviewConflict { file: None }));
    });
    settle(&mut rt);
    assert_eq!(conflict_file(&rt), Some(None));
    assert_eq!(
        text_of(&rt, "dlg-sub"),
        "1 of 2 \u{00B7} Contract draft.pdf changed in two places at once"
    );
    click(&mut rt, "btn-2");
    assert_eq!(asked(&calls), [Call::KeepVersion("contract".into(), Keep::There)]);
}

#[test]
fn review_with_nothing_waiting_says_so_and_closes() {
    let (mut rt, calls, _) = spied();
    rt.update(|s, _| {
        crate::shell::host::dispatch(s, Event::Request(ShellIntent::ReviewConflict { file: None }));
    });
    settle(&mut rt);
    assert_eq!(text_of(&rt, "dlg-sub"), "No file is waiting for a choice.");
    assert_eq!(label_of(&rt, "btn-0"), "Close");
    click(&mut rt, "btn-0");
    assert_eq!(conflict_file(&rt), None);
    assert!(asked(&calls).is_empty());
}
