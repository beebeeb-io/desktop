//! Tests of the protected folders (DW-0422 to DW-0425, DW-1110): the ways into Backups, and that
//! the WINDOW refuses to move, rename or delete there in every path, also over a source that
//! would accept it. Helpers are `tests.rs`'s.

use super::protected::Protection;
use super::protected_fixture::{LAPTOP_ID, MAIL_ID};
use super::source::{Action, ActionError, BACKUPS_ID, BoardDrive, FilesFixture};
use super::tests::*;
use super::{Intent, Toast};
use crate::State;
use crate::init;
use crate::shell::app::{Screen, Section, SettingsPane};
use crate::shell::host;
use crate::ui::sidebar::SidebarSection;
use rune::{Env, Key, Mods, Runtime};

const OF_WHAT_IS_INSIDE: &str =
    "Mail Backup keeps its files here, so they can't be moved, renamed or deleted. Turn it on or off in Settings.";
const OF_THE_FOLDER: &str = "Mail Backup keeps its files here, so it can't be moved, renamed or deleted. You can turn the service on or off in Settings.";

/// The app as it runs: the shell's view, keys and poll over the board's window. `lenient`: the
/// source accepts every change, so only the window can refuse.
fn app(lenient: bool) -> (Runtime<State>, Calls) {
    let mut state = init();
    let mut fixture = FilesFixture::default();
    fixture.lenient = lenient;
    let calls = fixture.calls.clone();
    state.source = Box::new(BoardDrive::sharing(&fixture.edits));
    state.actions.source = Box::new(fixture);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, crate::view, (1278.0, 798.0), env);
    rt.on_key = Some(host::on_key);
    rt.poll = Some(host::poll);
    rt.frame(0.0);
    settle(&mut rt);
    (rt, calls)
}

/// In Backups, by the sidebar.
fn in_backups(lenient: bool) -> (Runtime<State>, Calls) {
    let (mut rt, calls) = app(lenient);
    click(&mut rt, "Backups-row");
    settle(&mut rt);
    (rt, calls)
}

/// In the folder the mail service manages, by the app's own controls.
fn in_mail_backup(lenient: bool) -> (Runtime<State>, Calls) {
    let (mut rt, calls) = in_backups(lenient);
    click(&mut rt, "row-4-date");
    press(&mut rt, Key::Enter, NONE);
    settle(&mut rt);
    assert_eq!(rt.state.listing.folder.id.as_deref(), Some(MAIL_ID));
    (rt, calls)
}

fn section(rt: &Runtime<State>) -> Option<Section> {
    match rt.state.shell.app.screen() {
        Screen::Drive(route) => Some(route.section),
        _ => None,
    }
}

/// The open message's words. (State, not elements: a path called on the state directly draws
/// no frame; `the_message_is_modal…` reads the drawn dialog.)
fn alert_words(rt: &Runtime<State>) -> Option<(String, String)> {
    let words = rt.state.actions.protected.alert.as_ref()?;
    Some((words.title.clone(), words.body.clone()))
}

fn text(rt: &Runtime<State>, key: &str) -> String {
    info(rt, key).text.unwrap_or_default()
}

// ---- the ways in (DW-0422) ----

#[test]
fn the_sidebars_backups_shows_every_backup_and_new_is_dimmed() {
    let (rt, calls) = in_backups(false);
    assert_eq!(section(&rt), Some(Section::Backups));
    assert!(rt.state.sidebar_section == SidebarSection::Backups);
    assert_eq!(text(&rt, "folder-title"), "Backups");
    assert_eq!(
        names(&rt),
        [
            "Anna's iPhone",
            "Anna's iPad",
            "Anna-Laptop",
            "Old iPhone",
            "Mail Backup"
        ]
    );
    assert_eq!(
        [text(&rt, "col-date"), text(&rt, "col-shared")],
        ["Last backup", "Status"]
    );
    let statuses: Vec<String> = (0..5).map(|i| text(&rt, &format!("row-{i}-dash"))).collect();
    assert_eq!(
        statuses,
        [
            "12,418 photos",
            "4,120 photos",
            "This PC",
            "Not signed in",
            "Managed by Beebeeb"
        ]
    );
    let sizes: Vec<String> = (0..5).map(|i| text(&rt, &format!("row-{i}-size"))).collect();
    assert_eq!(sizes, ["106 GB", "38 GB", "9.8 GB", "72 GB", "6.2 GB"]);
    // Only the service's folder carries the lock.
    let locks: Vec<bool> = (0..5).map(|i| find(&rt, &format!("row-{i}-lock")).is_some()).collect();
    assert_eq!(locks, [false, false, false, false, true]);
    assert_eq!(
        info(&rt, "row-2").label.as_deref(),
        Some("Anna-Laptop, last backup Today, 09:30, 9.8 GB, This PC, online only")
    );
    assert_eq!(text(&rt, "status-details"), "0 on this PC \u{00B7} 5 online only");
    let new = info(&rt, "new-btn");
    assert!(!new.focusable && (new.opacity - 0.4).abs() < 0.01, "New is dimmed");
    assert!(asked(&calls).is_empty());
}

#[test]
fn ctrl_4_and_the_backups_row_in_files_lead_to_the_same_place_and_back_returns() {
    let (mut rt, _) = app(false);
    click(&mut rt, "row-6-date");
    press(&mut rt, Key::Command('4'), CTRL);
    settle(&mut rt);
    assert_eq!(section(&rt), Some(Section::Backups));
    assert_eq!(rt.state.listing.folder.id.as_deref(), Some(BACKUPS_ID));
    // The sidebar's Files: the root again.
    click(&mut rt, "Files-row");
    settle(&mut rt);
    assert_eq!(section(&rt), Some(Section::Files));
    assert_eq!(rt.state.listing.folder.id, None);
    assert_eq!(names(&rt).len(), 11);
    // The Backups row of Files opens the section, the sidebar with it.
    click(&mut rt, "row-0-date");
    press(&mut rt, Key::Enter, NONE);
    settle(&mut rt);
    assert_eq!(section(&rt), Some(Section::Backups));
    assert!(rt.state.sidebar_section == SidebarSection::Backups);
    assert_eq!(text(&rt, "folder-title"), "Backups");
    assert_eq!(names(&rt).len(), 5);
    // Back (the toolbar's): Files, the sidebar with it.
    click(&mut rt, "back");
    settle(&mut rt);
    assert_eq!(rt.state.listing.folder.id, None);
    assert_eq!(section(&rt), Some(Section::Files));
    assert!(rt.state.sidebar_section == SidebarSection::Files);
}

#[test]
fn a_source_that_knows_no_backups_folder_shows_no_sample() {
    let mut state = init();
    state.actions = super::Actions::default();
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, crate::view, (1278.0, 798.0), env);
    rt.on_key = Some(host::on_key);
    rt.poll = Some(host::poll);
    rt.frame(0.0);
    settle(&mut rt);
    rt.state.sidebar_section = SidebarSection::Backups;
    settle(&mut rt);
    assert_eq!(section(&rt), Some(Section::Backups));
    assert_eq!(rt.state.listing.folder.id, None, "no folder was opened");
    let shown: Vec<String> = rt.elements().into_iter().filter_map(|e| e.text).collect();
    for name in ["Anna's iPhone", "Mail Backup", "Anna-Laptop"] {
        assert!(!shown.iter().any(|t| t.contains(name)), "{name} is shown");
    }
    assert!(find(&rt, "row-0").is_none(), "no rows");
}

// ---- what one device backed up, and a service's folder (DW-0423, DW-0424) ----

#[test]
fn a_device_opens_what_it_backed_up_read_only() {
    let (mut rt, calls) = in_backups(true);
    click(&mut rt, "row-2-date");
    press(&mut rt, Key::Enter, NONE);
    settle(&mut rt);
    assert_eq!(rt.state.listing.folder.id.as_deref(), Some(LAPTOP_ID));
    assert_eq!(section(&rt), Some(Section::Backups), "still Backups");
    assert_eq!(text(&rt, "crumb-0"), "Backups");
    assert_eq!(text(&rt, "crumb-here"), "Anna-Laptop");
    assert_eq!(names(&rt), ["Desktop", "Documents", "Projects", "Contacts", "Calendar"]);
    let sizes: Vec<String> = (0..5).map(|i| text(&rt, &format!("row-{i}-size"))).collect();
    assert_eq!(sizes, ["214 items", "1,820 items", "46 items", "1 item", "1 item"]);
    assert_eq!(text(&rt, "row-2-note"), "Chosen folder");
    assert_eq!(
        info(&rt, "row-2").label.as_deref(),
        Some("Projects, changed Today, 09:28, 46 items, Chosen folder, online only")
    );
    assert_eq!(
        [text(&rt, "col-date"), text(&rt, "col-shared")],
        ["Date changed", "Shared"]
    );
    assert!(!info(&rt, "new-btn").focusable, "New is dimmed");
    // Rename and Duplicate there: said, and the source is not asked.
    click(&mut rt, "row-1-date");
    press(&mut rt, Key::Function(2), NONE);
    assert!(rt.state.actions.rename.is_none());
    assert_eq!(
        rt.state.actions.toast,
        Some(Toast::Failed(Action::Rename, ActionError::Protected))
    );
    press(&mut rt, Key::Command('d'), CTRL);
    assert_eq!(rt.state.listing.items.len(), 5);
    assert!(asked(&calls).is_empty(), "{:?}", asked(&calls));
    // Delete there is Delete this backup, of this device, behind its code (DW-0742).
    press(&mut rt, Key::Delete, NONE);
    settle(&mut rt);
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::Settings(SettingsPane::Backups),
        "Settings' Backups"
    );
    assert_eq!(
        rt.state.backups.dialog,
        Some(crate::frames::backups::Dialog::DeleteDevice(LAPTOP_ID.into()))
    );
    assert!(rt.state.actions.intents.contains(&Intent::Frame("DW-0742")));
    assert!(asked(&calls).is_empty(), "{:?}", asked(&calls));
}

#[test]
fn the_crumb_goes_back_up_to_backups() {
    let (mut rt, _) = in_mail_backup(false);
    click(&mut rt, "crumb-0");
    settle(&mut rt);
    assert_eq!(rt.state.listing.folder.id.as_deref(), Some(BACKUPS_ID));
    assert_eq!(section(&rt), Some(Section::Backups));
}

#[test]
fn a_services_folder_says_who_manages_it_and_settings_leads_to_settings() {
    let (mut rt, calls) = in_mail_backup(false);
    assert_eq!(
        names(&rt),
        ["2026", "2025", "2024"],
        "the service's order, not the name's"
    );
    assert_eq!(
        info(&rt, "banner").label.as_deref(),
        Some("Mail Backup manages this folder. You can open and download its files.")
    );
    let settings = info(&rt, "banner-action");
    assert_eq!(
        (settings.role, settings.label.as_deref(), settings.focusable),
        ("button", Some("Settings"), true)
    );
    // Opening a year is allowed: it is asked of the listing, and nothing else is asked.
    assert!(!info(&rt, "new-btn").focusable, "New is dimmed");
    click(&mut rt, "banner-action");
    settle(&mut rt);
    assert_eq!(rt.state.shell.app.screen(), Screen::Settings(SettingsPane::Backups));
    assert_eq!(rt.state.actions.intents.last(), Some(&Intent::Frame("DW-0740")));
    assert!(asked(&calls).is_empty());
}

#[test]
fn the_file_menu_there_offers_only_what_reads() {
    let (mut rt, _) = in_mail_backup(true);
    right_click(&mut rt, "row-1-date");
    assert_eq!(
        menu_labels(&rt),
        ["Open", "Preview", "Show in File Explorer", "Details"]
    );
}

// ---- the refusal, per path (DW-0425) ----

/// Every way to change something in a service's folder, over a source that would accept it:
/// each ends in the message, and the source hears nothing.
#[test]
fn every_way_to_change_a_services_folder_ends_in_the_message_and_asks_nothing() {
    type Try = fn(&mut Runtime<State>);
    let ways: [(&str, Try); 7] = [
        ("Delete", |rt| press(rt, Key::Delete, NONE)),
        ("F2", |rt| press(rt, Key::Function(2), NONE)),
        ("Ctrl+D", |rt| press(rt, Key::Command('d'), CTRL)),
        ("Move", |rt| {
            let mut tx = rune::Tx::default();
            rt.state.begin_move(&mut tx);
        }),
        ("Copy link", |rt| {
            let mut tx = rune::Tx::default();
            rt.state.copy_link(&mut tx);
        }),
        ("Move to Trash from a menu", |rt| {
            let mut tx = rune::Tx::default();
            rt.state.trash_selection(&mut tx);
        }),
        ("several selected, Delete", |rt| {
            click_with(rt, "row-0-date", CTRL);
            click_with(rt, "row-2-date", CTRL);
            assert_eq!(rt.state.selected_rows().len(), 3);
            press(rt, Key::Delete, NONE);
        }),
    ];
    let mut refused = 0;
    for (name, way) in ways {
        let (mut rt, calls) = in_mail_backup(true);
        click(&mut rt, "row-1-date");
        way(&mut rt);
        settle(&mut rt);
        assert_eq!(
            alert_words(&rt),
            Some(("Beebeeb manages this folder".to_string(), OF_WHAT_IS_INSIDE.to_string())),
            "{name}"
        );
        assert!(asked(&calls).is_empty(), "{name}: {:?}", asked(&calls));
        assert_eq!(rt.state.listing.items.len(), 3, "{name}: nothing changed");
        assert!(
            rt.state.actions.rename.is_none() && rt.state.actions.dialog.is_none(),
            "{name}"
        );
        refused += 1;
    }
    assert_eq!(refused, 7);
}

#[test]
fn nothing_is_added_to_a_protected_folder() {
    for open in [in_backups as fn(bool) -> (Runtime<State>, Calls), in_mail_backup] {
        let (mut rt, calls) = open(true);
        press(&mut rt, Key::Command('n'), Mods { shift: true, ..CTRL });
        press(&mut rt, Key::Command('n'), CTRL);
        press(&mut rt, Key::Command('o'), CTRL);
        assert!(rt.state.actions.dialog.is_none(), "no New folder dialog");
        click(&mut rt, "new-btn");
        assert!(rt.state.actions.menu.is_none(), "New opens nothing");
        let files = vec![std::path::PathBuf::from("a.txt")];
        let (x, y) = centre(&rt, "rows");
        rt.drop_files(files.clone(), Some((x, y)));
        let (x, y) = centre(&rt, "row-0-date");
        rt.drop_files(files, Some((x, y)));
        settle(&mut rt);
        assert!(asked(&calls).is_empty(), "{:?}", asked(&calls));
        assert!(find(&rt, "dlg").is_none(), "a refused drop is silent");
    }
}

#[test]
fn the_message_is_modal_and_ok_esc_and_open_settings_do_what_they_say() {
    let (mut rt, calls) = in_mail_backup(true);
    click(&mut rt, "row-1-date");
    press(&mut rt, Key::Delete, NONE);
    let dialog = info(&rt, "dlg");
    assert_eq!(
        (dialog.role, dialog.label.as_deref()),
        ("alertdialog", Some("Beebeeb manages this folder"))
    );
    assert_eq!(
        [text(&rt, "dlg-title"), text(&rt, "dlg-sub")],
        ["Beebeeb manages this folder", OF_WHAT_IS_INSIDE]
    );
    for (key, name) in [("btn-0", "OK"), ("btn-1", "Open Settings")] {
        let b = info(&rt, key);
        assert_eq!((b.role, b.label.as_deref(), b.focusable), ("button", Some(name), true));
    }
    // Nothing reaches the list under it.
    press(&mut rt, Key::Down, NONE);
    assert_eq!(rt.state.selected, Some(1));
    press(&mut rt, Key::Escape, NONE);
    assert!(find(&rt, "dlg").is_none(), "Esc closes it");
    assert_eq!(
        rt.state.listing.folder.id.as_deref(),
        Some(MAIL_ID),
        "and stays in the folder"
    );
    assert_eq!(focused(&rt).as_deref(), Some("rows"), "the list has the keyboard again");
    press(&mut rt, Key::Delete, NONE);
    click(&mut rt, "btn-0");
    assert!(find(&rt, "dlg").is_none(), "OK closes it");
    assert_eq!(rt.state.listing.items.len(), 3);
    press(&mut rt, Key::Delete, NONE);
    click(&mut rt, "btn-1");
    settle(&mut rt);
    assert!(rt.state.actions.protected.alert.is_none());
    assert_eq!(rt.state.shell.app.screen(), Screen::Settings(SettingsPane::Backups));
    assert_eq!(rt.state.actions.intents.last(), Some(&Intent::Frame("DW-0740")));
    assert!(asked(&calls).is_empty());
}

// ---- in Backups itself (DW-1110, DW-0422's rules) ----

#[test]
fn deleting_the_services_folder_in_backups_says_its_own_sentence() {
    let (mut rt, calls) = in_backups(true);
    click(&mut rt, "row-4-date");
    press(&mut rt, Key::Delete, NONE);
    assert_eq!(
        alert_words(&rt),
        Some(("Beebeeb manages this folder".to_string(), OF_THE_FOLDER.to_string()))
    );
    click(&mut rt, "btn-0");
    assert_eq!(
        rt.state.listing.folder.id.as_deref(),
        Some(BACKUPS_ID),
        "back on DW-0422"
    );
    assert_eq!(names(&rt).len(), 5);
    press(&mut rt, Key::Function(2), NONE);
    assert!(alert_words(&rt).is_some(), "F2 too");
    assert!(rt.state.actions.rename.is_none());
    assert!(asked(&calls).is_empty(), "{:?}", asked(&calls));
}

#[test]
fn a_devices_backup_is_deleted_only_through_delete_this_backup() {
    let (mut rt, calls) = in_backups(true);
    click(&mut rt, "row-0-date");
    press(&mut rt, Key::Function(2), NONE);
    assert!(rt.state.actions.rename.is_none(), "no rename");
    assert!(find(&rt, "dlg").is_none(), "and not the service's message");
    press(&mut rt, Key::Delete, NONE);
    settle(&mut rt);
    assert_eq!(rt.state.shell.app.screen(), Screen::Settings(SettingsPane::Backups));
    assert_eq!(
        rt.state.backups.dialog,
        Some(crate::frames::backups::Dialog::DeleteDevice("b-iphone".into()))
    );
    assert!(asked(&calls).is_empty(), "{:?}", asked(&calls));
}

#[test]
fn backups_are_selected_one_at_a_time() {
    let (mut rt, _) = in_backups(false);
    click(&mut rt, "row-0-date");
    click_with(&mut rt, "row-2-date", CTRL);
    assert_eq!(rt.state.selected_rows(), [2], "Ctrl+click");
    click_with(&mut rt, "row-4-date", SHIFT);
    assert_eq!(rt.state.selected_rows(), [4], "Shift+click");
    press(&mut rt, Key::Up, SHIFT);
    assert_eq!(rt.state.selected_rows(), [3], "Shift+Up");
    press(&mut rt, Key::Command('a'), CTRL);
    assert_eq!(rt.state.selected_rows(), [3], "Ctrl+A");
    // Inside a backup several may be selected (they can be downloaded together).
    click(&mut rt, "row-4-date");
    press(&mut rt, Key::Enter, NONE);
    settle(&mut rt);
    click(&mut rt, "row-0-date");
    press(&mut rt, Key::Command('a'), CTRL);
    assert_eq!(rt.state.selected_rows(), [0, 1, 2]);
}

// ---- the words are the source's ----

#[test]
fn the_fixture_carries_the_boards_two_sentences_and_none_for_the_backups_row() {
    let (rt, _) = in_mail_backup(false);
    let inside = rt.state.listing.items[0].clone();
    let Some(Protection::Managed(words)) = rt.state.actions.source.protection(Some(MAIL_ID), Some(&inside)) else {
        panic!("a year in the service's folder is managed");
    };
    assert_eq!(
        (words.title.as_str(), words.body.as_str()),
        ("Beebeeb manages this folder", OF_WHAT_IS_INSIDE)
    );
    assert_eq!(
        rt.state.actions.source.protection(Some(LAPTOP_ID), None),
        Some(Protection::DeviceBackup(LAPTOP_ID.into()))
    );
    assert_eq!(
        rt.state.actions.source.protection(Some(BACKUPS_ID), None),
        Some(Protection::BackupsRoot)
    );
    assert_eq!(
        rt.state.actions.source.protection(None, None),
        None,
        "Files is not protected"
    );
    // A source without the methods (the default bodies) protects nothing and knows no Backups.
    let unwired = super::source::NoEngine;
    use super::source::FilesSource;
    assert_eq!(unwired.backups_root(), None);
    assert_eq!(unwired.protection(Some(MAIL_ID), Some(&inside)), None);
    assert_eq!(unwired.backup_entry(&inside), None);
    assert_eq!(unwired.folder_notice(Some(MAIL_ID)), None);
}

// ---- the scenes are what the controls reach ----

#[test]
fn each_scene_is_the_state_its_controls_reach() {
    let scene = |frame: &str| {
        let mut state = init();
        let run = super::protected::SCENES
            .iter()
            .find(|(id, _)| *id == frame)
            .unwrap_or_else(|| panic!("{frame} is a scene"))
            .1;
        run(&mut state);
        (
            state.listing.folder.id.clone(),
            state.selected,
            state.actions.protected.alert.clone().map(|w| w.body),
            state.sidebar_section == SidebarSection::Backups,
        )
    };
    let of = |rt: &Runtime<State>| {
        (
            rt.state.listing.folder.id.clone(),
            rt.state.selected,
            rt.state.actions.protected.alert.clone().map(|w| w.body),
            rt.state.sidebar_section == SidebarSection::Backups,
        )
    };
    let (rt, _) = in_backups(false);
    assert_eq!(of(&rt), scene("DW-0422"));
    let (mut rt, _) = in_backups(false);
    click(&mut rt, "row-2-date");
    press(&mut rt, Key::Enter, NONE);
    settle(&mut rt);
    assert_eq!(of(&rt), scene("DW-0423"));
    let (rt, _) = in_mail_backup(false);
    assert_eq!(of(&rt), scene("DW-0424"));
    let (mut rt, _) = in_mail_backup(false);
    click(&mut rt, "row-1-date");
    press(&mut rt, Key::Delete, NONE);
    assert_eq!(of(&rt), scene("DW-0425"));
    let (mut rt, _) = in_backups(false);
    click(&mut rt, "row-4-date");
    press(&mut rt, Key::Delete, NONE);
    assert_eq!(of(&rt), scene("DW-1110"));
    assert_eq!(super::protected::SCENES.len(), 5);
}
