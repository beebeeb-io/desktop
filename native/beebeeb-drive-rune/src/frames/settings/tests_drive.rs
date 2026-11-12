//! Tests of Drive's own settings panes (Storage, Storage insights, Sync and File Explorer):
//! keyboard, roles and names, and that every drawn control does something.
//!
//! Input is scripted: keys and presses go through Rune's `Runtime`, as a person's would.

use super::drive::{BoardDrive, DriveSettings, Insights, Kept, Repair, SyncStatus};
use super::{Dialog, Intent, Pane};
use crate::shell::app::{Screen, SettingsPane};
use crate::shell::host;
use crate::{State, init, view};
use rune::{Env, Key, KeyEvent, Mods, Runtime};

fn runtime(frame: &str) -> Runtime<State> {
    let mut state = init();
    assert!(super::scene(frame, &mut state), "{frame} is a built frame");
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (958.0, 718.0), env);
    rt.on_key = Some(host::on_key);
    rt.poll = Some(host::poll);
    rt.frame(0.0);
    settle(&mut rt);
    rt
}

fn settle(rt: &mut Runtime<State>) {
    rt.frame(0.016);
    rt.settle_all();
    rt.frame(0.016);
}

fn find(rt: &Runtime<State>, key: &str) -> Option<rune::ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn info(rt: &Runtime<State>, key: &str) -> rune::ElementInfo {
    find(rt, key).unwrap_or_else(|| panic!("no element keyed {key}"))
}

fn key(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    settle(rt);
}

fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.focused && !e.exiting)
        .and_then(|e| e.key)
}

/// Tab until the control keyed `target` has the focus.
fn tab_to(rt: &mut Runtime<State>, target: &str) {
    for _ in 0..80 {
        key(rt, Key::Tab);
        if focused(rt).as_deref() == Some(target) {
            return;
        }
    }
    panic!("Tab never reaches {target}");
}

fn press(rt: &mut Runtime<State>, target: &str) {
    tab_to(rt, target);
    key(rt, Key::Enter);
}

fn ui(rt: &Runtime<State>) -> &super::SettingsUi {
    &rt.state.settings
}

#[test]
fn storage_is_the_shells_storage_section_with_its_item_selected() {
    let rt = runtime("DW-0712");
    assert_eq!(rt.state.shell.app.screen(), Screen::Settings(SettingsPane::Storage));
    assert_eq!(info(&rt, "set-title").text.as_deref(), Some("Storage"));
    assert!(
        info(&rt, "Storage-row").fill.a > 0.0,
        "the Storage item is the selected one"
    );
    assert_eq!(info(&rt, "usage").label.as_deref(), Some("240 GB used of 1 TB"));
    assert_eq!(info(&rt, "meter-0").role, "meter");
}

#[test]
fn the_fixture_holds_the_boards_storage() {
    let d = BoardDrive::new();
    let st = d.storage();
    assert_eq!((st.used.as_str(), st.total.as_str()), ("240 GB", "1 TB"));
    assert_eq!(st.kinds.len(), 4);
    assert_eq!((st.downloaded.limit.gb, st.previews.limit.gb), (50, 20));
    let Insights::Ready(data) = d.insights() else {
        panic!("the board's insights are loaded")
    };
    assert_eq!((data.kinds.len(), data.largest.len()), (3, 7));
}

#[test]
fn storage_rows_go_where_they_point() {
    let mut rt = runtime("DW-0712");
    press(&mut rt, "item-0-5");
    assert_eq!(ui(&rt).pane, Pane::Insights, "Storage insights opens its page");
    press(&mut rt, "crumb-row");
    assert_eq!(ui(&rt).pane, Pane::Storage, "the way back leads to Storage");
    press(&mut rt, "item-0-4");
    assert!(
        ui(&rt).intents.contains(&Intent::Show("DW-0747")),
        "Trash is asked of the shell"
    );
    press(&mut rt, "item-1-2");
    assert!(ui(&rt).intents.contains(&Intent::Show("DW-0715")));
}

#[test]
fn a_limit_is_changed_with_the_keyboard_and_kept_only_on_save() {
    let mut rt = runtime("DW-0712");
    press(&mut rt, "btn-3");
    assert_eq!(ui(&rt).dialog, Some(Dialog::Limit(Kept::Previews)));
    assert_eq!(info(&rt, "dlg").role, "dialog");
    assert_eq!(info(&rt, "dlg-figure").text.as_deref(), Some("20 GB"));
    let slider = info(&rt, "slider");
    assert_eq!(slider.role, "slider");
    assert_eq!(slider.label.as_deref(), Some("Thumbnails and previews"));

    tab_to(&mut rt, "slider");
    key(&mut rt, Key::Right);
    key(&mut rt, Key::Right);
    assert_eq!(
        info(&rt, "dlg-figure").text.as_deref(),
        Some("22 GB"),
        "an arrow is one GB"
    );
    key(&mut rt, Key::Home);
    assert_eq!(
        info(&rt, "dlg-figure").text.as_deref(),
        Some("1 GB"),
        "Home is the least"
    );
    key(&mut rt, Key::End);
    assert_eq!(
        info(&rt, "dlg-figure").text.as_deref(),
        Some("93 GB"),
        "End is the most"
    );

    // Esc keeps the old limit.
    key(&mut rt, Key::Escape);
    assert_eq!(ui(&rt).dialog, None);
    assert_eq!(ui(&rt).drive.storage().previews.limit.gb, 20);
    assert_eq!(ui(&rt).pane, Pane::Storage, "Esc closed the dialog, not settings");

    // Save keeps the new one, and the row says it.
    press(&mut rt, "btn-1");
    assert_eq!(ui(&rt).dialog, Some(Dialog::Limit(Kept::Downloaded)));
    tab_to(&mut rt, "slider");
    key(&mut rt, Key::Left);
    press(&mut rt, "btn-4");
    assert_eq!(ui(&rt).dialog, None);
    assert_eq!(ui(&rt).drive.storage().downloaded.limit.gb, 49);
    assert_eq!(info(&rt, "item-1-0-sub").text.as_deref(), Some("18.4 GB of 49 GB"));
}

#[test]
fn clearing_downloads_asks_first_and_clearing_previews_says_so() {
    let mut rt = runtime("DW-0712");
    press(&mut rt, "btn-0");
    assert_eq!(ui(&rt).dialog, Some(Dialog::FreeUp));
    assert_eq!(info(&rt, "dlg").role, "alertdialog");
    assert_eq!(info(&rt, "dlg-title").text.as_deref(), Some("Free up 18.4 GB?"));
    // Cancel changes nothing.
    press(&mut rt, "btn-5");
    assert_eq!(ui(&rt).dialog, None);
    assert_eq!(ui(&rt).drive.storage().downloaded.used, "18.4 GB");
    // Enter is the dialog's primary button.
    press(&mut rt, "btn-0");
    key(&mut rt, Key::Enter);
    assert_eq!(ui(&rt).dialog, None);
    assert_eq!(ui(&rt).drive.storage().downloaded.used, "0 GB");
    assert!(find(&rt, "toast-0").is_some(), "what happened is said");

    press(&mut rt, "btn-2");
    assert_eq!(ui(&rt).drive.storage().previews.used, "0 GB");
}

#[test]
fn insights_say_loading_failed_and_empty_and_try_again_loads() {
    let rt = runtime("DW-0783b");
    assert_eq!(info(&rt, "card-0").label.as_deref(), Some("Loading"));
    assert!(find(&rt, "skel-8").is_some(), "three cards of three shapes");

    let rt = runtime("DW-0783d");
    assert_eq!(info(&rt, "card-0-head").text.as_deref(), Some("Nothing is stored yet"));

    let mut rt = runtime("DW-0783c");
    assert_eq!(info(&rt, "card-0").role, "alert");
    assert!(find(&rt, "insbar-0").is_none(), "no stale numbers beside the failure");
    press(&mut rt, "btn-0");
    assert_eq!(ui(&rt).drive.insights(), Insights::Loading, "Try again loads again");
    assert!(find(&rt, "skel-0").is_some());
}

#[test]
fn a_largest_file_is_a_link_named_in_full_that_shows_it_in_files() {
    let mut rt = runtime("DW-0783");
    let row = info(&rt, "lrow-2");
    assert_eq!(row.role, "link");
    assert_eq!(
        row.label.as_deref(),
        Some("Backups/Anna-Laptop/Documents/Archive 2024.zip, Other, 2.6 GB"),
        "the name is the whole path, not the shortened one"
    );
    assert_eq!(
        info(&rt, "kind-0").label.as_deref(),
        Some("Media: 221.6 GB · 3,140 files")
    );
    press(&mut rt, "lrow-0");
    assert!(ui(&rt).intents.contains(&Intent::ShowInFiles(
        "Backups/Anna's iPhone/Photos/2026/IMG_4410.MOV".into()
    )));
    assert!(
        !matches!(rt.state.shell.app.screen(), Screen::Settings(_)),
        "the window goes to Files"
    );
}

#[test]
fn sync_switches_turn_by_keyboard_and_are_never_amber() {
    let mut rt = runtime("DW-0746");
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::Settings(SettingsPane::SyncAndFileExplorer)
    );
    let amber = crate::theme::tokens::theme(false).accent;
    for (switch, read) in [
        (
            "item-0-1-switch",
            (|d: &dyn DriveSettings| d.sync().status_on_files) as fn(&dyn DriveSettings) -> bool,
        ),
        ("item-1-1-switch", |d: &dyn DriveSettings| d.sync().open_at_login),
    ] {
        let sw = info(&rt, switch);
        assert_eq!(sw.role, "switch");
        assert_ne!(sw.fill, amber, "a switch is ink, never Amber");
        assert!(read(ui(&rt).drive.as_ref()));
        tab_to(&mut rt, switch);
        key(&mut rt, Key::Text(" ".into()));
        assert!(!read(ui(&rt).drive.as_ref()), "Space turns {switch} off");
    }
}

#[test]
fn sync_pauses_resumes_and_syncs_anyway_on_a_metered_connection() {
    let mut rt = runtime("DW-0746");
    assert_eq!(info(&rt, "btn-2").label.as_deref(), Some("Pause syncing"));
    press(&mut rt, "btn-2");
    assert_eq!(ui(&rt).drive.sync().status, SyncStatus::Paused);
    assert_eq!(info(&rt, "btn-2").label.as_deref(), Some("Resume syncing"));
    press(&mut rt, "btn-2");
    assert!(matches!(ui(&rt).drive.sync().status, SyncStatus::Running(_)));

    let mut rt = runtime("DW-0746e");
    assert_eq!(
        info(&rt, "item-1-0-sub").text.as_deref(),
        Some("Waiting · this PC is on a metered connection · 7 changes wait")
    );
    assert_eq!(info(&rt, "btn-2").label.as_deref(), Some("Sync anyway"));
    press(&mut rt, "btn-2");
    assert!(matches!(ui(&rt).drive.sync().status, SyncStatus::Running(_)));
    assert!(
        find(&rt, "list-1-foot-words").is_none(),
        "the metered note leaves with the wait"
    );
}

#[test]
fn a_repair_asks_first_says_repairing_and_a_failed_one_offers_it_again() {
    let mut rt = runtime("DW-0746");
    press(&mut rt, "btn-1");
    assert_eq!(ui(&rt).dialog, Some(Dialog::Repair));
    assert_eq!(info(&rt, "dlg").role, "alertdialog");
    key(&mut rt, Key::Enter);
    assert_eq!(ui(&rt).drive.sync().repair, Repair::Running);
    assert_eq!(info(&rt, "item-0-2-sub").text.as_deref(), Some("Repairing…"));
    assert!(ui(&rt).intents.contains(&Intent::Show("DW-1212")));

    let mut rt = runtime("DW-0746d");
    assert_eq!(info(&rt, "note-0").role, "status");
    assert_eq!(info(&rt, "btn-0").label.as_deref(), Some("Repair…"));
    press(&mut rt, "btn-0");
    assert_eq!(ui(&rt).dialog, Some(Dialog::Repair), "the notice's button asks again");
}

#[test]
fn turning_file_explorer_off_is_destructive_asks_and_cancel_keeps_it() {
    let mut rt = runtime("DW-0746");
    let row = info(&rt, "item-3-0");
    assert_eq!(row.role, "button");
    press(&mut rt, "item-3-0");
    assert_eq!(ui(&rt).dialog, Some(Dialog::TurnOffExplorer));
    assert!(
        info(&rt, "dlg-sub")
            .text
            .unwrap_or_default()
            .starts_with("4 files made in File Explorer"),
        "the alert counts what has not uploaded"
    );
    // Enter does NOT turn it off: a destructive answer is never the default button.
    key(&mut rt, Key::Escape);
    assert_eq!(ui(&rt).dialog, None);
    assert!(ui(&rt).drive.sync().in_explorer);

    press(&mut rt, "item-3-0");
    press(&mut rt, "btn-5");
    assert!(!ui(&rt).drive.sync().in_explorer);
    assert!(find(&rt, "item-3-0").is_none(), "nothing left to turn off");
}

#[test]
fn sync_rows_open_the_limit_and_free_up_space_and_the_metered_link_asks_for_bandwidth() {
    let mut rt = runtime("DW-0746");
    press(&mut rt, "item-2-1");
    assert_eq!(ui(&rt).dialog, Some(Dialog::Limit(Kept::Downloaded)));
    assert_eq!(info(&rt, "dlg-figure").text.as_deref(), Some("50 GB"));
    key(&mut rt, Key::Escape);
    press(&mut rt, "btn-4");
    assert_eq!(ui(&rt).dialog, Some(Dialog::FreeUp));
    key(&mut rt, Key::Escape);
    press(&mut rt, "btn-3");
    assert!(
        ui(&rt).intents.contains(&Intent::Show("DW-0784")),
        "the folder tree is not built: asked of the shell"
    );
    press(&mut rt, "btn-0");
    assert!(ui(&rt).intents.contains(&Intent::Open("file-explorer")));
}

#[test]
fn a_build_that_ships_draws_no_sample_value_on_drives_panes() {
    let mut state = init();
    assert!(super::scene("DW-0712", &mut state));
    state.settings.drive = super::drive::first(false);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (958.0, 638.0), env);
    rt.frame(0.0);
    assert!(find(&rt, "usage").is_none());
    assert!(
        !rt.elements().iter().any(|e| e.text.as_deref() == Some("216 GB")),
        "no sample figure"
    );
}

/// A section this lane has not built stays the shell's: its "not built yet" frame, never
/// another pane's content under that section's route.
#[test]
fn a_section_not_built_yet_is_left_to_the_shell() {
    let mut rt = runtime("DW-0712");
    press(&mut rt, "Thumbnails and previews-row");
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::Settings(SettingsPane::ThumbnailsAndPreviews)
    );
    assert!(find(&rt, "set-main").is_none(), "no settings pane is drawn for it");
}
