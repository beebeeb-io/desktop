//! Tests of the dialogs and the name field (lane f-files-rest): New folder (DW-0417), Move to
//! (DW-0409), Rename in the row (DW-0410, DW-0410b), with scripted keys and clicks, and what each
//! asks of the source. Helpers are `tests.rs`'s.

use super::dialogs::{Dialog, MOVE_LIST, NAME_INPUT};
use super::rename::RENAME_INPUT;
use super::source::{Action, ActionError, BoardDrive, Call, FilesFixture, NEW_FOLDER_ID};
use super::tests::*;
use super::{Intent, Toast};
use crate::State;
use crate::init;
use rune::{Key, Mods, Runtime};

/// The board's window over a fixture whose changes its folders show (as `board_fixture` wires
/// it), the test keeping its handles on what was asked and what is refused.
fn wired() -> (Runtime<State>, Calls, Fails) {
    let mut state = init();
    let fixture = FilesFixture::default();
    let (calls, fails) = (fixture.calls.clone(), fixture.fail.clone());
    state.source = Box::new(BoardDrive::sharing(&fixture.edits));
    state.actions.source = Box::new(fixture);
    let mut rt = runtime_of(state);
    click(&mut rt, "row-6-date");
    (rt, calls, fails)
}

fn type_text(rt: &mut Runtime<State>, text: &str) {
    for c in text.chars() {
        press(rt, Key::Text(c.to_string()), NONE);
    }
}

fn label_of(rt: &Runtime<State>, key: &str) -> String {
    info(rt, key).label.unwrap_or_default()
}

// ---- New folder (DW-0417) ----

#[test]
fn new_folder_is_a_modal_dialog_with_its_name_selected_and_named_controls() {
    let (mut rt, calls, _) = wired();
    press(&mut rt, Key::Command('n'), Mods { shift: true, ..CTRL });
    let dialog = info(&rt, "dlg");
    assert_eq!((dialog.role, dialog.label.as_deref()), ("dialog", Some("New folder")));
    assert_eq!(focused(&rt).as_deref(), Some(NAME_INPUT), "the field has the keyboard");
    let edit = &rt.state.actions.name_edit;
    assert_eq!((edit.text.as_str(), edit.range()), ("Untitled folder", (0, 15)));
    assert_eq!(label_of(&rt, NAME_INPUT), "Folder name");
    assert_eq!(
        info(&rt, "field-0-msg").text.as_deref(),
        Some("Goes into Files. The name is encrypted too.")
    );
    for (key, name) in [("btn-0", "Create"), ("btn-1", "Cancel")] {
        let b = info(&rt, key);
        assert_eq!((b.role, b.label.as_deref(), b.focusable), ("button", Some(name), true));
    }
    // The list under the dialog hears no key: Delete trashes nothing.
    press(&mut rt, Key::Delete, NONE);
    assert_eq!(rt.state.listing.items.len(), 11);
    // Esc closes it, and nothing was asked.
    press(&mut rt, Key::Escape, NONE);
    assert!(rt.state.actions.dialog.is_none());
    assert!(find(&rt, "dlg").is_none());
    assert!(asked(&calls).is_empty());
}

#[test]
fn typing_a_name_and_enter_makes_the_folder_and_shows_it() {
    let (mut rt, calls, _) = wired();
    press(&mut rt, Key::Command('n'), Mods { shift: true, ..CTRL });
    type_text(&mut rt, "Receipts");
    press(&mut rt, Key::Enter, NONE);
    assert_eq!(asked(&calls), [Call::NewFolder(None, "Receipts".into())]);
    assert!(rt.state.actions.dialog.is_none());
    // The new folder is shown, empty (DW-1106), and Back finds it in Files, in the folder's order.
    assert_eq!(rt.state.listing.folder.name, "Receipts");
    assert_eq!(rt.state.listing.folder.id.as_deref(), Some(NEW_FOLDER_ID));
    assert!(rt.state.listing.items.is_empty());
    assert!(find(&rt, "empty").is_some(), "Nothing here yet");
    press(&mut rt, Key::Left, ALT);
    let listed = names(&rt);
    assert_eq!(listed.len(), 12);
    assert_eq!(
        &listed[..6],
        [
            "Backups",
            "Documents",
            "Invoices 2026",
            "Lisbon trip",
            "Receipts",
            "Shoot selects"
        ]
    );
}

#[test]
fn the_create_button_and_cancel_work_with_the_pointer() {
    let (mut rt, calls, _) = wired();
    click(&mut rt, "new-btn");
    press(&mut rt, Key::Down, NONE);
    press(&mut rt, Key::Enter, NONE);
    assert!(matches!(rt.state.actions.dialog, Some(Dialog::NewFolder(_))));
    click(&mut rt, "btn-1");
    assert!(rt.state.actions.dialog.is_none(), "Cancel closes");
    assert!(asked(&calls).is_empty());
    press(&mut rt, Key::Command('n'), Mods { shift: true, ..CTRL });
    click(&mut rt, "btn-0");
    assert_eq!(asked(&calls), [Call::NewFolder(None, "Untitled folder".into())]);
    assert_eq!(rt.state.listing.folder.name, "Untitled folder");
}

#[test]
fn a_folder_name_that_cannot_be_given_keeps_the_dialog_and_says_why() {
    let (mut rt, calls, _) = wired();
    press(&mut rt, Key::Command('n'), Mods { shift: true, ..CTRL });
    type_text(&mut rt, "documents");
    press(&mut rt, Key::Enter, NONE);
    assert!(rt.state.actions.dialog.is_some());
    assert!(asked(&calls).is_empty(), "nothing was asked of the source");
    let message = info(&rt, "field-0-msg");
    assert_eq!(
        (message.role, message.text.as_deref()),
        (
            "alert",
            Some("There's already a folder called Documents here. Choose another name.")
        )
    );
    // Typing takes the error away; the note is back.
    type_text(&mut rt, " 2");
    assert_eq!(
        info(&rt, "field-0-msg").text.as_deref(),
        Some("Goes into Files. The name is encrypted too.")
    );
    press(&mut rt, Key::Enter, NONE);
    assert_eq!(asked(&calls), [Call::NewFolder(None, "documents 2".into())]);
}

#[test]
fn new_folder_does_nothing_where_nothing_can_be_added_and_says_a_failure() {
    let (mut rt, calls, fails) = wired();
    fails.lock().unwrap().push((Action::NewFolder, ActionError::Offline));
    press(&mut rt, Key::Command('n'), Mods { shift: true, ..CTRL });
    press(&mut rt, Key::Enter, NONE);
    assert!(asked(&calls).is_empty());
    assert!(rt.state.actions.dialog.is_none());
    assert_eq!(
        rt.state.live_toast(),
        Some(Toast::Failed(Action::NewFolder, ActionError::Offline))
    );
    assert_eq!(
        Toast::failure(Action::NewFolder, ActionError::Offline),
        "Couldn't make the folder. Check your connection."
    );
}

// ---- Rename in the row (DW-0410, DW-0410b) ----

#[test]
fn f2_turns_the_name_into_a_field_with_the_extension_left_out_of_the_selection() {
    let (mut rt, calls, _) = wired();
    press(&mut rt, Key::Function(2), NONE);
    assert_eq!(focused(&rt).as_deref(), Some(RENAME_INPUT));
    assert_eq!(label_of(&rt, RENAME_INPUT), "Name");
    let edit = &rt.state.actions.name_edit;
    assert_eq!((edit.text.as_str(), edit.range()), ("Contract draft.pdf", (0, 14)));
    assert!(find(&rt, "row-6-name").is_none(), "the field stands where the name was");
    // Typing replaces the selected part and keeps .pdf; Enter saves.
    type_text(&mut rt, "Signed contract");
    press(&mut rt, Key::Enter, NONE);
    assert_eq!(
        asked(&calls),
        [Call::Rename("contract".into(), "Signed contract.pdf".into())]
    );
    assert!(rt.state.actions.rename.is_none());
    let row = rt.state.selected.unwrap();
    assert_eq!(rt.state.listing.items[row].name, "Signed contract.pdf");
    assert_eq!(
        rt.state.listing.items[row].id, "contract",
        "the renamed row stays selected"
    );
    assert_eq!(
        focused(&rt).as_deref(),
        Some("rows"),
        "the keyboard is the list's again"
    );
    // The folder is still listed with the new name when it is shown again.
    rt.update(|s, _| s.open_folder(Some("f-lisbon".into())));
    settle(&mut rt);
    press(&mut rt, Key::Left, ALT);
    assert!(names(&rt).contains(&"Signed contract.pdf".to_string()));
}

#[test]
fn escape_keeps_the_old_name_and_a_click_elsewhere_saves() {
    let (mut rt, calls, _) = wired();
    press(&mut rt, Key::Function(2), NONE);
    type_text(&mut rt, "Other");
    press(&mut rt, Key::Escape, NONE);
    assert!(rt.state.actions.rename.is_none());
    assert_eq!(rt.state.listing.items[6].name, "Contract draft.pdf");
    assert!(asked(&calls).is_empty());

    press(&mut rt, Key::Function(2), NONE);
    type_text(&mut rt, "Other");
    click(&mut rt, "row-2-date");
    assert_eq!(asked(&calls), [Call::Rename("contract".into(), "Other.pdf".into())]);
    assert!(rt.state.actions.rename.is_none());
}

#[test]
fn rename_from_the_menu_and_a_name_already_used_shows_the_tip_and_keeps_the_field() {
    let (mut rt, calls, _) = wired();
    right_click(&mut rt, "row-6-date");
    let item = rt
        .elements()
        .into_iter()
        .find(|e| e.role == "menuitem" && e.label.as_deref() == Some("Rename"))
        .and_then(|e| e.key)
        .unwrap();
    click(&mut rt, &item);
    assert!(rt.state.actions.rename.is_some());
    assert_eq!(focused(&rt).as_deref(), Some(RENAME_INPUT));
    // Names compare without case (DW-0410b).
    press(&mut rt, Key::Command('a'), CTRL);
    type_text(&mut rt, "harbour.HEIC");
    press(&mut rt, Key::Enter, NONE);
    assert!(rt.state.actions.rename.is_some(), "the field stays");
    assert!(asked(&calls).is_empty());
    let tip = info(&rt, "tip");
    assert_eq!(
        (tip.role, tip.label.as_deref()),
        (
            "alert",
            Some("There's already a file called Harbour.heic here. Choose another name.")
        )
    );
    // F2 does nothing while the name is taken; Esc keeps the old name.
    press(&mut rt, Key::Function(2), NONE);
    assert!(rt.state.actions.rename.is_some());
    press(&mut rt, Key::Escape, NONE);
    assert!(rt.state.actions.rename.is_none());
    assert!(find(&rt, "tip").is_none());
    assert_eq!(rt.state.listing.items[6].name, "Contract draft.pdf");
}

#[test]
fn a_protected_folder_is_not_renamed_and_several_rows_are_not_either() {
    let (mut rt, calls, _) = wired();
    click(&mut rt, "row-0-date");
    press(&mut rt, Key::Function(2), NONE);
    assert!(rt.state.actions.rename.is_none());
    assert_eq!(rt.state.actions.intents.last(), Some(&Intent::Frame("DW-1110")));
    click(&mut rt, "row-5-date");
    click_with(&mut rt, "row-6-date", SHIFT);
    press(&mut rt, Key::Function(2), NONE);
    assert!(rt.state.actions.rename.is_none());
    assert!(asked(&calls).is_empty());
}

// ---- Move to (DW-0409) ----

fn open_move(rt: &mut Runtime<State>) {
    right_click(rt, "row-6-date");
    let item = rt
        .elements()
        .into_iter()
        .find(|e| e.role == "menuitem" && e.label.as_deref() == Some("Move\u{2026}"))
        .and_then(|e| e.key)
        .unwrap();
    click(rt, &item);
}

#[test]
fn move_offers_the_folders_without_backups_and_names_its_target() {
    let (mut rt, calls, _) = wired();
    open_move(&mut rt);
    let dialog = info(&rt, "dlg");
    assert_eq!((dialog.role, dialog.label.as_deref()), ("dialog", Some("Move to")));
    assert_eq!(info(&rt, "dlg-sub").text.as_deref(), Some("Contract draft.pdf"));
    let Some(Dialog::Move(m)) = &rt.state.actions.dialog else {
        panic!("no Move dialog")
    };
    let offered: Vec<&str> = m.choices.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(offered, ["Documents", "Invoices 2026", "Lisbon trip", "Shoot selects"]);
    assert_eq!(label_of(&rt, "item-0-0"), "Documents, 48 items");
    assert_eq!(info(&rt, "item-0-0").role, "option");
    assert_eq!(label_of(&rt, "btn-0"), "Move to Documents");
    // A click chooses; the button follows.
    click(&mut rt, "item-0-2");
    assert_eq!(label_of(&rt, "btn-0"), "Move to Lisbon trip");
    // The keyboard: the list is one stop, the arrows choose, Enter moves.
    assert_eq!(focused(&rt).as_deref(), Some(MOVE_LIST));
    press(&mut rt, Key::Up, NONE);
    assert_eq!(label_of(&rt, "btn-0"), "Move to Invoices 2026");
    press(&mut rt, Key::Enter, NONE);
    assert_eq!(
        asked(&calls),
        [Call::Move(ids(&["contract"]), Some("f-invoices".into()))]
    );
    assert!(rt.state.actions.dialog.is_none());
    assert_eq!(rt.state.listing.items.len(), 10, "the row left the folder shown");
    assert!(!names(&rt).contains(&"Contract draft.pdf".to_string()));
}

#[test]
fn move_goes_into_a_folder_and_back_up_and_cancel_moves_nothing() {
    let (mut rt, calls, _) = wired();
    open_move(&mut rt);
    click(&mut rt, "item-0-2");
    press(&mut rt, Key::Right, NONE);
    let Some(Dialog::Move(m)) = &rt.state.actions.dialog else {
        panic!("no Move dialog")
    };
    assert_eq!(m.here().name, "Lisbon trip");
    let inside: Vec<&str> = m.choices.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(inside, ["Day 1", "Day 2"]);
    assert_eq!(
        label_of(&rt, "btn-0"),
        "Move to Lisbon trip",
        "nothing chosen: the folder shown"
    );
    // The path's first folder is a way up.
    let up = info(&rt, "dlg-path-0");
    assert_eq!((up.role, up.focusable), ("link", true));
    click(&mut rt, "dlg-path-0");
    let Some(Dialog::Move(m)) = &rt.state.actions.dialog else {
        panic!("no Move dialog")
    };
    assert_eq!(
        (m.path.len(), m.picked),
        (1, Some(2)),
        "back in Files, Lisbon trip chosen"
    );
    click(&mut rt, "btn-2");
    assert!(rt.state.actions.dialog.is_none());
    assert!(asked(&calls).is_empty());
    assert_eq!(rt.state.listing.items.len(), 11);
}

#[test]
fn moving_into_lisbon_trip_shows_the_file_there_and_new_folder_in_move_is_chosen() {
    let (mut rt, calls, _) = wired();
    open_move(&mut rt);
    // New folder, from the dialog: made in the folder it shows, then chosen.
    click(&mut rt, "btn-1");
    assert!(matches!(rt.state.actions.dialog, Some(Dialog::NewFolder(_))));
    type_text(&mut rt, "Signed");
    press(&mut rt, Key::Enter, NONE);
    let Some(Dialog::Move(m)) = &rt.state.actions.dialog else {
        panic!("back in the Move dialog")
    };
    assert_eq!(m.target().name, "Signed");
    assert_eq!(label_of(&rt, "btn-0"), "Move to Signed");
    click(&mut rt, "btn-0");
    assert_eq!(
        asked(&calls),
        [
            Call::NewFolder(None, "Signed".into()),
            Call::Move(ids(&["contract"]), Some(NEW_FOLDER_ID.into()))
        ]
    );
    // The file is in the new folder when it is opened.
    rt.update(|s, _| s.open_folder(Some(NEW_FOLDER_ID.into())));
    settle(&mut rt);
    assert_eq!(names(&rt), ["Contract draft.pdf"]);
}

#[test]
fn move_refuses_a_folder_that_has_the_name_and_a_protected_selection() {
    let (mut rt, calls, _) = wired();
    // Give the file the name of one in Lisbon trip, then move it there.
    press(&mut rt, Key::Function(2), NONE);
    type_text(&mut rt, "Tickets");
    press(&mut rt, Key::Enter, NONE);
    let row = rt.state.selected.unwrap();
    let key = format!("row-{row}-date");
    right_click(&mut rt, &key);
    let item = rt
        .elements()
        .into_iter()
        .find(|e| e.role == "menuitem" && e.label.as_deref() == Some("Move\u{2026}"))
        .and_then(|e| e.key)
        .unwrap();
    click(&mut rt, &item);
    click(&mut rt, "item-0-2");
    click(&mut rt, "btn-0");
    assert!(rt.state.actions.dialog.is_some(), "the dialog stays");
    let error = info(&rt, "dlg-error");
    assert_eq!(
        (error.role, error.text.as_deref()),
        (
            "alert",
            Some("There's already a file called Tickets.pdf here. Choose another name.")
        )
    );
    assert_eq!(asked(&calls).len(), 1, "only the rename was asked");
    press(&mut rt, Key::Escape, NONE);

    // Backups does not move: the protected-folder message's frame is asked for instead.
    click(&mut rt, "row-0-date");
    rt.update(|s, tx| {
        s.begin_move(tx);
    });
    settle(&mut rt);
    assert!(rt.state.actions.dialog.is_none());
    assert_eq!(rt.state.actions.intents.last(), Some(&Intent::Frame("DW-1110")));
}

// ---- The board's scenes ----

#[test]
fn the_four_scenes_set_what_their_frames_draw() {
    for (frame, key) in [
        ("DW-0417", "field-0"),
        ("DW-0409", "item-0-3"),
        ("DW-0410", "rename-caret"),
        ("DW-0410b", "tip"),
    ] {
        let mut state = init();
        let (_, scene) = super::SCENES.iter().find(|(name, _)| *name == frame).unwrap();
        scene(&mut state);
        let rt = runtime_of(state);
        assert!(find(&rt, key).is_some(), "{frame} draws {key}");
    }
}
