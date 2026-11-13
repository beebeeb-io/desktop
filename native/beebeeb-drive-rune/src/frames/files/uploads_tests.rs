//! Tests of the upload frames (lane files-actions): the words on Windows' picker (DW-0415,
//! DW-0416, DW-0480: Windows draws it), a drag over the list (DW-0481), uploads in the status
//! line and in their panel (DW-0426, DW-0427). Helpers are `tests.rs`'s.

use super::source::{Action, ActionError, Call};
use super::tests::*;
use super::uploads::{DropHover, PickerKind, UploadState, picker_words, upload_button};
use super::{Intent, MenuKind, Toast};
use crate::State;
use rune::{Key, Runtime};
use std::path::PathBuf;

fn pick_new(rt: &mut Runtime<State>, label: &str) {
    rt.update(|s, _| s.toggle_menu(MenuKind::New));
    settle(rt);
    let item = rt
        .elements()
        .into_iter()
        .find(|e| e.role == "menuitem" && e.label.as_deref() == Some(label))
        .unwrap_or_else(|| panic!("no menu item {label}"));
    click(rt, &item.key.unwrap());
}

fn files(names: &[&str]) -> Vec<PathBuf> {
    names.iter().map(PathBuf::from).collect()
}

fn text_of(rt: &Runtime<State>, key: &str) -> String {
    info(rt, key).text.unwrap_or_default()
}

/// The window after two files were dropped on it: the fixture's sample uploads are on.
fn uploading() -> (Runtime<State>, Calls, Fails) {
    let (mut rt, calls, fails) = spied();
    rt.update(|s, tx| {
        s.upload(None, &files(&["a.heic", "b.heic"]), tx);
    });
    settle(&mut rt);
    calls.lock().unwrap().clear();
    (rt, calls, fails)
}

// ---- Windows' picker (DW-0415, DW-0416, DW-0480): OS-drawn; these are its words ----

#[test]
fn the_pickers_words_are_the_boards() {
    let files = picker_words(PickerKind::Files, "Files", None);
    assert_eq!(
        (files.title.as_str(), files.button.as_str(), files.filter),
        ("Upload to Files", "Upload", Some("All files")),
        "DW-0415"
    );
    let folder = picker_words(PickerKind::Folder, "Files", None);
    assert_eq!(
        (folder.title.as_str(), folder.button.as_str(), folder.filter),
        ("Upload a folder to Files", "Upload", None),
        "DW-0480"
    );
    assert_eq!(upload_button(Some(3)), "Upload 3", "DW-0416: three photos chosen");
    assert_eq!(upload_button(Some(0)), "Upload");
    assert_eq!(
        picker_words(PickerKind::Files, "Lisbon trip", None).title,
        "Upload to Lisbon trip"
    );
}

#[test]
fn upload_files_and_upload_a_folder_ask_for_the_picker_and_draw_none() {
    let (mut rt, calls, _) = spied();
    for (label, frame, kind) in [
        ("Upload files\u{2026}", "DW-0415", PickerKind::Files),
        ("Upload a folder\u{2026}", "DW-0480", PickerKind::Folder),
    ] {
        pick_new(&mut rt, label);
        assert_eq!(rt.state.actions.intents.last(), Some(&Intent::Frame(frame)));
        let ask = rt
            .state
            .take_picker()
            .unwrap_or_else(|| panic!("{label} asked for no picker"));
        assert_eq!((ask.kind, ask.into.clone()), (kind, None));
        assert!(rt.state.take_picker().is_none(), "it is taken once");
        assert!(
            find(&rt, "dlg").is_none(),
            "{label}: Beebeeb draws no picker of its own"
        );
    }
    // Ctrl+O is Upload files.
    press(&mut rt, Key::Command('o'), CTRL);
    assert_eq!(
        rt.state.take_picker().map(|a| a.title),
        Some("Upload to Files".to_string())
    );
    assert!(asked(&calls).is_empty(), "asking for the picker uploads nothing");
}

#[test]
fn what_was_picked_uploads_into_the_folder_asked_for_and_cancel_does_nothing() {
    let (mut rt, calls, _) = spied();
    pick_new(&mut rt, "Upload files\u{2026}");
    let ask = rt.state.take_picker().unwrap();
    // Cancel, Esc: the picker comes back with nothing.
    for nothing in [None, Some(Vec::new())] {
        let ask = ask.clone();
        rt.update(move |s, tx| {
            assert!(!s.picked(&ask, nothing.clone(), tx));
        });
    }
    assert!(asked(&calls).is_empty());
    assert!(rt.state.actions.source.uploads().is_none(), "nothing uploads");
    let chosen = files(&["Meeting notes.pdf", "Quote 2026.xlsx", "Studio plan.pdf"]);
    let picked = chosen.clone();
    rt.update(move |s, tx| {
        assert!(s.picked(&ask, Some(picked.clone()), tx));
    });
    settle(&mut rt);
    assert_eq!(asked(&calls), [Call::Upload(None, chosen)]);
    // And uploads then show in the status line (DW-0426).
    assert_eq!(text_of(&rt, "status-text"), "Uploading 3 of 5 \u{00B7} 42 MB left");
}

#[test]
fn a_folder_that_takes_nothing_asks_for_no_picker() {
    let (mut rt, _, _) = spied();
    rt.update(|s, _| s.open_folder(Some(super::source::BACKUPS_ID.to_string())));
    settle(&mut rt);
    rt.update(|s, _| s.ask_picker(PickerKind::Files));
    assert!(rt.state.take_picker().is_none());
}

// ---- A drag from File Explorer (DW-0481) ----

#[test]
fn a_drag_over_a_folder_lights_its_row_and_says_where_a_drop_goes() {
    let (mut rt, calls, _) = spied();
    rt.update(|s, _| s.drop_over(1, true));
    settle(&mut rt);
    assert_eq!(rt.state.drop_row(), Some(1));
    let tip = info(&rt, "tip");
    assert_eq!(
        (tip.role, tip.label.as_deref()),
        ("status", Some("Upload to Documents"))
    );
    assert_eq!(text_of(&rt, "tip-text"), "Upload to Documents");
    assert!(
        find(&rt, "drag-count").is_none(),
        "no count unless the system gives one"
    );
    // It left the row: nothing stays lit.
    rt.update(|s, _| s.drop_over(1, false));
    settle(&mut rt);
    assert_eq!(rt.state.drop_row(), None);
    assert!(find(&rt, "tip").is_none());
    // With the system's count the badge says it (the board's "3").
    rt.update(|s, _| {
        s.actions.drop_hover = Some(DropHover {
            row: None,
            count: Some(3),
        });
        s.drop_over(2, true);
    });
    settle(&mut rt);
    assert_eq!(text_of(&rt, "drag-count"), "3");
    assert_eq!(text_of(&rt, "tip-text"), "Upload to Invoices 2026");
    assert!(asked(&calls).is_empty(), "hovering uploads nothing");
}

#[test]
fn backups_and_files_do_not_light_up_and_a_drop_ends_the_hover() {
    let (mut rt, calls, _) = spied();
    // Backups takes nothing; a file is not a folder.
    for row in [0, 6] {
        rt.update(move |s, _| s.drop_over(row, true));
        settle(&mut rt);
        assert_eq!(rt.state.drop_row(), None, "row {row}");
        assert!(find(&rt, "tip").is_none());
    }
    rt.update(|s, tx| {
        s.drop_over(1, true);
        assert!(s.upload(Some("f-documents".into()), &files(&["a.pdf"]), tx));
    });
    settle(&mut rt);
    assert_eq!(rt.state.drop_row(), None, "the drop ends the hover");
    assert_eq!(asked(&calls).len(), 1);
}

// ---- Uploads (DW-0426, DW-0427) ----

#[test]
fn nothing_uploading_leaves_the_status_line_as_it_was() {
    let (mut rt, _, _) = spied();
    assert_eq!(text_of(&rt, "status-text"), "Up to date");
    click(&mut rt, "status-bar");
    assert!(!rt.state.actions.uploads_open, "no Uploads panel without uploads");
    assert_eq!(rt.state.actions.intents.last(), Some(&Intent::Frame("DW-1201")));
}

#[test]
fn the_status_line_says_what_uploads_and_opens_uploads() {
    let (mut rt, calls, _) = uploading();
    assert_eq!(text_of(&rt, "status-text"), "Uploading 3 of 5 \u{00B7} 42 MB left");
    let line = info(&rt, "status-bar");
    assert_eq!(line.role, "button");
    assert!(line.label.unwrap().starts_with("Uploading 3 of 5"));
    click(&mut rt, "status-bar");
    let panel = info(&rt, "uploads");
    assert_eq!((panel.role, panel.label.as_deref()), ("dialog", Some("Uploads")));
    assert_eq!(
        text_of(&rt, "uploads-figures"),
        "3 of 5 \u{00B7} 42 MB left \u{00B7} Continues in the background"
    );
    let groups: Vec<String> = (0..3).map(|g| text_of(&rt, &format!("uploads-group-{g}"))).collect();
    assert_eq!(groups, ["NEEDS ATTENTION", "UPLOADING", "WAITING"]);
    let rows: Vec<(String, String)> = (0..4)
        .map(|i| {
            (
                text_of(&rt, &format!("upload-{i}-name")),
                text_of(&rt, &format!("upload-{i}-sub")),
            )
        })
        .collect();
    assert_eq!(
        rows.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect::<Vec<_>>(),
        [
            ("Scan 12 Oct.pdf", "Stopped after 5 tries. Not enough space."),
            ("Miradouro.heic", "2.1 of 3.6 MB"),
            ("Tram 28.mov", "Continuing, 34 of 82 MB"),
            ("Ponte.heic", "3.2 MB \u{00B7} To Lisbon trip"),
        ]
    );
    assert_eq!(info(&rt, "upload-1-bar").role, "progressbar");
    assert!(find(&rt, "upload-3-bar").is_none(), "what waits has no bar");
    // Esc closes it; nothing was asked.
    press(&mut rt, Key::Escape, NONE);
    assert!(!rt.state.actions.uploads_open);
    assert!(find(&rt, "uploads").is_none());
    assert!(asked(&calls).is_empty());
}

#[test]
fn try_again_pause_all_and_cancel_upload_each_ask_once() {
    let (mut rt, calls, _) = uploading();
    click(&mut rt, "status-bar");
    let retry = info(&rt, "upload-0-retry");
    assert_eq!((retry.role, retry.label.as_deref()), ("button", Some("Try again")));
    click(&mut rt, "upload-0-retry");
    assert_eq!(asked(&calls), [Call::RetryUpload("up-scan".into())]);
    assert!(find(&rt, "upload-0-retry").is_none(), "it waits its turn again");
    // Pause all: once, and the b is told (DW-1204); the button then offers the way back.
    assert_eq!(info(&rt, "uploads-pause").label.as_deref(), Some("Pause all"));
    click(&mut rt, "uploads-pause");
    assert_eq!(asked(&calls).last(), Some(&Call::PauseUploads(true)));
    assert_eq!(rt.state.actions.intents.last(), Some(&Intent::Frame("DW-1204")));
    assert_eq!(info(&rt, "uploads-pause").label.as_deref(), Some("Resume all"));
    assert!(text_of(&rt, "status-text").starts_with("Paused"));
    click(&mut rt, "uploads-pause");
    assert_eq!(asked(&calls).last(), Some(&Call::PauseUploads(false)));
    // Right-click a row: Cancel upload.
    right_click(&mut rt, "upload-3-name");
    assert_eq!(info(&rt, "menu-9-item-0").label.as_deref(), Some("Cancel upload"));
    click(&mut rt, "menu-9-item-0");
    assert_eq!(asked(&calls).last(), Some(&Call::CancelUpload("up-ponte".into())));
    assert_eq!(asked(&calls).len(), 4, "one call per press");
    let left = rt.state.actions.source.uploads().unwrap();
    assert!(left.of("up-ponte").is_none());
    assert_eq!(left.items.len(), 3);
}

#[test]
fn a_refused_pause_says_so_and_changes_nothing() {
    let (mut rt, calls, fails) = uploading();
    fails.lock().unwrap().push((Action::PauseUploads, ActionError::Offline));
    click(&mut rt, "status-bar");
    click(&mut rt, "uploads-pause");
    assert!(asked(&calls).is_empty());
    assert!(!rt.state.actions.source.uploads().unwrap().paused);
    assert_eq!(
        rt.state.live_toast(),
        Some(Toast::Failed(Action::PauseUploads, ActionError::Offline))
    );
}

/// A refusal in the panel says so under its own label and offers NO Try again: the message's
/// Try again would repeat nothing (or, for the panel's own Try again, upload an earlier drop).
#[test]
fn a_refusal_in_the_uploads_panel_offers_no_try_again_that_does_nothing() {
    type Press = fn(&mut State, &mut rune::Tx);
    let cases: [(Action, Action, Press); 3] = [
        (Action::PauseUploads, Action::PauseUploads, |s, tx| s.pause_uploads(tx)),
        (Action::CancelUpload, Action::CancelUpload, |s, tx| {
            s.cancel_one_upload("up-ponte", tx)
        }),
        (Action::RetryUpload, Action::RetryUpload, |s, tx| {
            s.retry_one_upload("up-scan", tx)
        }),
    ];
    for (refuse, label, press) in cases {
        let (mut rt, calls, fails) = uploading();
        // An earlier drop that failed is still remembered: the panel's refusal must not reach it.
        rt.update(|s, _| s.actions.retry_upload = Some((None, files(&["earlier.pdf"]))));
        fails.lock().unwrap().push((refuse, ActionError::Offline));
        let before = rt.state.actions.source.uploads().unwrap();
        rt.update(press);
        settle(&mut rt);
        assert_eq!(
            rt.state.live_toast(),
            Some(Toast::Failed(label, ActionError::Offline)),
            "{refuse:?}"
        );
        assert!(find(&rt, "toast-action").is_none(), "{refuse:?}: a Try again is drawn");
        // And asked for directly, it repeats nothing.
        rt.update(|s, tx| s.retry(tx));
        settle(&mut rt);
        assert!(asked(&calls).is_empty(), "{refuse:?}: {:?}", asked(&calls));
        let after = rt.state.actions.source.uploads().unwrap();
        assert_eq!(
            (after.items.len(), after.paused),
            (before.items.len(), before.paused),
            "{refuse:?}: nothing changed"
        );
    }
    // A drop that failed still offers Try again: that one repeats the upload.
    let (mut rt, _, fails) = spied();
    fails.lock().unwrap().push((Action::Upload, ActionError::Offline));
    rt.update(|s, tx| {
        s.upload(None, &files(&["a.heic"]), tx);
    });
    settle(&mut rt);
    assert_eq!(
        rt.state.live_toast(),
        Some(Toast::Failed(Action::Upload, ActionError::Offline))
    );
    assert!(find(&rt, "toast-action").is_some(), "a failed drop keeps its Try again");
}

/// Cancel upload takes the upload's row out of the folder; what was selected stays selected BY
/// ITEM, also the rows under the one that went.
#[test]
fn cancel_upload_keeps_the_selection_on_the_same_items() {
    let (mut rt, _, _) = uploading();
    // The upload's row stands above the selected ones.
    rt.update(|s, _| {
        let mut arriving = s.listing.items[9].clone();
        arriving.id = "up-ponte".into();
        arriving.name = "Ponte.heic".into();
        s.listing.items.insert(2, arriving);
        s.selected = Some(7);
        s.actions.selection = [5, 7].into_iter().collect();
        s.actions.anchor = Some(5);
    });
    let id_at = |rt: &Runtime<State>, row: usize| rt.state.listing.items[row].id.clone();
    let (five, seven) = (id_at(&rt, 5), id_at(&rt, 7));
    let rows = rt.state.listing.items.len();
    rt.update(|s, tx| s.cancel_one_upload("up-ponte", tx));
    settle(&mut rt);
    assert_eq!(rt.state.listing.items.len(), rows - 1, "the upload's row went");
    assert_eq!(
        rt.state.selected.map(|i| id_at(&rt, i)),
        Some(seven.clone()),
        "the lead row"
    );
    assert_eq!(
        rt.state.actions.anchor.map(|i| id_at(&rt, i)),
        Some(five.clone()),
        "the anchor"
    );
    let selected: Vec<String> = rt.state.actions.selection.iter().map(|&i| id_at(&rt, i)).collect();
    assert_eq!(selected, [five, seven], "the same two items");
    // The cancelled row itself selected: it leaves the selection, the other stays.
    let (mut rt, _, _) = uploading();
    rt.update(|s, _| {
        let mut arriving = s.listing.items[9].clone();
        arriving.id = "up-ponte".into();
        s.listing.items.insert(2, arriving);
        s.selected = Some(2);
        s.actions.selection = [2, 6].into_iter().collect();
    });
    let six = id_at(&rt, 6);
    rt.update(|s, tx| s.cancel_one_upload("up-ponte", tx));
    settle(&mut rt);
    assert_eq!(rt.state.selected, None, "the row that went is not selected");
    let selected: Vec<String> = rt.state.actions.selection.iter().map(|&i| id_at(&rt, i)).collect();
    assert_eq!(selected, [six]);
}

#[test]
fn an_upload_shows_in_its_row_with_its_bar_or_that_it_waits() {
    let mut state = crate::init();
    super::action_scenes::uploading(&mut state);
    let rt = runtime_of(state);
    assert_eq!(rt.state.listing.folder.name, "Lisbon trip");
    let row_of = |name: &str| rt.state.listing.items.iter().position(|i| i.name == name).unwrap();
    let (moving, waiting) = (row_of("Miradouro.heic"), row_of("Ponte.heic"));
    assert_eq!(
        text_of(&rt, &format!("row-{moving}-upload-words")),
        "Uploading, 2.1 of 3.6 MB"
    );
    assert_eq!(info(&rt, &format!("row-{moving}-bar")).role, "progressbar");
    assert!(
        find(&rt, &format!("row-{moving}-date")).is_none(),
        "the bar stands where the date was"
    );
    assert_eq!(text_of(&rt, &format!("row-{waiting}-date")), "Just now");
    assert!(find(&rt, &format!("row-{waiting}-bar")).is_none());
    let uploads = rt.state.actions.source.uploads().unwrap();
    assert!(matches!(uploads.of("up-ponte").unwrap().state, UploadState::Waiting(_)));
}
