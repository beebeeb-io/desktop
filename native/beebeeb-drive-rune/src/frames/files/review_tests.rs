//! Tests for the independent review of PR 294 (2026-11-11): one per finding, named for what the
//! window must do. Helpers are `tests.rs`'s.

use super::source::{Action, ActionError, BoardDrive, Call, FilesFixture};
use super::tests::*;
use super::{Intent, MenuKind, SortBy, Toast};
use crate::State;
use crate::init;
use crate::source::DriveSource;
use rune::{Key, Mods, Runtime};

/// A `DriveSource` whose Backups folder opens (the board's fixture does not draw its inside).
struct WithBackups(BoardDrive);

impl DriveSource for WithBackups {
    fn list(&self, folder: Option<&str>) -> Result<crate::source::Listing, crate::source::SourceError> {
        if folder != Some(super::source::BACKUPS_ID) {
            return self.0.list(folder);
        }
        let root = self.0.list(None)?;
        let here = crate::source::Crumb {
            id: folder.map(str::to_string),
            name: "Backups".into(),
        };
        Ok(crate::source::Listing {
            path: vec![root.folder, here.clone()],
            folder: here,
            items: Vec::new(),
        })
    }
    fn storage(&self) -> Option<crate::source::StorageSummary> {
        self.0.storage()
    }
}

fn in_backups() -> (Runtime<State>, Calls, Fails) {
    let mut state = init();
    state.source = Box::new(WithBackups(BoardDrive::new()));
    let (mut rt, calls, fails) = spied_on(state);
    rt.update(|s, _| s.open_folder(Some(super::source::BACKUPS_ID.to_string())));
    settle(&mut rt);
    (rt, calls, fails)
}

fn group(rt: &Runtime<State>) -> crate::shell::registry::FrameGroup {
    crate::shell::registry::group_for(&rt.state.shell.app)
}

// ---- P1: every drawn control does something ----

#[test]
fn the_details_panels_buttons_and_switch_act_with_the_pointer_and_the_keyboard() {
    let (mut rt, calls, _) = spied();
    for (key, role) in [
        ("share-box", "button"),
        ("versions-box", "button"),
        ("offline-switch", "switch"),
    ] {
        let e = info(&rt, key);
        assert_eq!((e.role, e.focusable), (role, true), "{key}");
    }
    click(&mut rt, "share-box");
    // Each opens its dialog (`actions_tests.rs`): Esc closes it.
    press(&mut rt, Key::Escape, NONE);
    click(&mut rt, "versions-box");
    press(&mut rt, Key::Escape, NONE);
    assert_eq!(
        rt.state.actions.intents,
        [Intent::Frame("DW-0407"), Intent::Frame("DW-0413")]
    );
    click(&mut rt, "offline-switch");
    assert_eq!(asked(&calls), [Call::KeepOffline(ids(&["contract"]), true)]);
    assert_eq!(
        rt.state.listing.items[6].pc_status.as_deref(),
        Some("available_offline")
    );
    assert_eq!(rt.state.live_toast(), Some(Toast::Keeping));

    // The keyboard: Space on each, with focus on it.
    rt.state.actions.intents.clear();
    for key in ["share-box", "versions-box", "offline-switch"] {
        rt.update(move |_, tx| tx.focus(key, true));
        settle(&mut rt);
        assert_eq!(focused(&rt).as_deref(), Some(key));
        press(&mut rt, Key::Text(" ".into()), NONE);
        // Share and Version history are dialogs now: Esc closes each.
        if rt.state.actions.dialog.is_some() {
            press(&mut rt, Key::Escape, NONE);
        }
    }
    assert_eq!(
        rt.state.actions.intents,
        [Intent::Frame("DW-0407"), Intent::Frame("DW-0413")]
    );
    assert_eq!(
        asked(&calls).last(),
        Some(&Call::KeepOffline(ids(&["contract"]), false)),
        "the switch, switched back"
    );
    assert_eq!(rt.state.listing.items[6].pc_status.as_deref(), Some("online_only"));
}

#[test]
fn the_grid_switch_the_column_headers_and_the_status_line_act() {
    let (mut rt, calls, _) = spied();
    click(&mut rt, "view-grid");
    assert_eq!(rt.state.actions.intents, [Intent::Frame("DW-1102")]);
    // The folder is a grid now (`frames::global::grid`, DW-0119): no column headers. Back to the list.
    assert!(find(&rt, "grid-0").is_some() && find(&rt, "column-headers").is_none());
    click(&mut rt, "view-list");
    assert!(find(&rt, "grid").is_none());

    // A column header sorts by that column; a second click reverses the order (DW-0404).
    let header = info(&rt, "col-size-cell");
    assert_eq!((header.role, header.focusable), ("columnheader", true));
    click(&mut rt, "col-size-cell");
    assert_eq!(rt.state.actions.sort.by, SortBy::Size);
    assert!(!rt.state.actions.sort.reversed);
    assert_eq!(names(&rt)[0], "Backups", "Backups stays first");
    let sizes = |rt: &Runtime<State>| -> Vec<i64> {
        rt.state
            .listing
            .items
            .iter()
            .filter(|i| !i.is_folder)
            .map(|i| i.size.unwrap_or(0))
            .collect()
    };
    let largest_first = sizes(&rt);
    assert!(largest_first.windows(2).all(|w| w[0] >= w[1]), "{largest_first:?}");
    click(&mut rt, "col-size-cell");
    assert!(rt.state.actions.sort.reversed);
    let smallest_first = sizes(&rt);
    assert!(smallest_first.windows(2).all(|w| w[0] <= w[1]), "{smallest_first:?}");
    // The arrow stands beside the sorted column.
    let arrow = info(&rt, "sort-arrow").rect;
    let size = info(&rt, "col-size-cell").rect;
    assert!(
        arrow[0] >= size[0] && arrow[0] <= size[0] + size[2],
        "the arrow is in Size"
    );
    click(&mut rt, "col-name-cell");
    assert_eq!(
        (rt.state.actions.sort.by, rt.state.actions.sort.reversed),
        (SortBy::Name, false)
    );
    assert!(asked(&calls).iter().all(|c| matches!(c, Call::Sort(None, _))));
    assert_eq!(asked(&calls).len(), 3);

    // The status line shows the b's flyout (DW-0401).
    let status = info(&rt, "status-bar");
    assert_eq!((status.role, status.focusable), ("button", true));
    assert!(status.label.as_deref().unwrap().starts_with("Up to date. "));
    click(&mut rt, "status-text");
    assert_eq!(rt.state.actions.intents.last(), Some(&Intent::Frame("DW-1201")));
}

// ---- P2.1: Settings and Available offline go where the board says ----

#[test]
fn settings_is_dw_0701_and_available_offline_is_dw_0715_and_the_shell_goes_there() {
    use crate::shell::app::{Section, SettingsPane};
    use crate::shell::registry::FrameGroup;
    let mut rt = runtime();
    click(&mut rt, "Sync history-row");
    assert_eq!(rt.state.actions.intents, [Intent::Frame("DW-1301")]);
    assert_eq!(group(&rt), FrameGroup::Section(Section::SyncHistory));

    let mut rt = runtime();
    press(&mut rt, Key::Command('6'), CTRL);
    assert_eq!(group(&rt), FrameGroup::Section(Section::SyncHistory));

    let mut rt = runtime();
    click(&mut rt, "Available offline-row");
    assert_eq!(rt.state.actions.intents, [Intent::Frame("DW-0715")]);
    assert_eq!(group(&rt), FrameGroup::Section(Section::AvailableOffline));

    let mut rt = runtime();
    press(&mut rt, Key::Command('7'), CTRL);
    assert_eq!(rt.state.actions.intents, [Intent::Frame("DW-0715")]);
    assert_eq!(group(&rt), FrameGroup::Section(Section::AvailableOffline));

    let mut rt = runtime();
    click(&mut rt, "settings-btn");
    assert_eq!(rt.state.actions.intents, [Intent::Frame("DW-0701")]);
    assert_eq!(group(&rt), FrameGroup::Settings(SettingsPane::General));

    let mut rt = runtime();
    press(&mut rt, Key::Command(','), CTRL);
    assert_eq!(rt.state.actions.intents, [Intent::Frame("DW-0701")]);
}

// ---- P2.2, P2.3: nothing is claimed that did not happen; a failure says so (DW-0109) ----

#[test]
fn link_copied_is_said_only_once_the_link_is_copied() {
    let (mut rt, calls, fails) = spied();
    fails.lock().unwrap().push((Action::CopyLink, ActionError::Offline));
    right_click(&mut rt, "row-6-date");
    rt.update(|s, tx| s.pick_menu(4, tx));
    settle(&mut rt);
    assert_eq!(
        rt.state.live_toast(),
        Some(Toast::Failed(Action::CopyLink, ActionError::Offline))
    );
    let toast = info(&rt, "toast");
    assert_eq!(toast.role, "alert");
    assert_eq!(
        toast.label.as_deref(),
        Some("Couldn't copy the link. Check your connection.")
    );
    assert!(asked(&calls).is_empty(), "nothing was copied");

    // Try again, once the connection is back: the same action, and now it is said.
    fails.lock().unwrap().clear();
    let again = info(&rt, "toast-action");
    assert_eq!((again.role, again.label.as_deref()), ("button", Some("Try again")));
    click(&mut rt, "toast-action");
    assert_eq!(asked(&calls), [Call::CopyLink(ids(&["contract"]))]);
    assert_eq!(rt.state.live_toast(), Some(Toast::LinkCopied));
}

#[test]
fn a_failed_trash_restore_duplicate_or_keep_offline_says_so_and_changes_nothing() {
    let (mut rt, calls, fails) = spied();
    *fails.lock().unwrap() = vec![
        (Action::Trash, ActionError::Failed),
        (Action::Duplicate, ActionError::Offline),
        (Action::KeepOffline, ActionError::Failed),
    ];
    press(&mut rt, Key::Delete, NONE);
    assert_eq!(names(&rt).len(), 11, "the row is still there");
    assert_eq!(
        rt.state.live_toast(),
        Some(Toast::Failed(Action::Trash, ActionError::Failed))
    );
    assert_eq!(info(&rt, "toast-text").text.as_deref(), Some("Couldn't move to Trash."));
    assert_eq!(info(&rt, "Trash-badge").text.as_deref(), Some("12"));

    press(&mut rt, Key::Command('d'), CTRL);
    assert_eq!(names(&rt).len(), 11);
    assert_eq!(
        info(&rt, "toast-text").text.as_deref(),
        Some("Couldn't duplicate. Check your connection.")
    );

    click(&mut rt, "offline-switch");
    assert_eq!(rt.state.listing.items[6].pc_status.as_deref(), Some("online_only"));
    assert_eq!(
        info(&rt, "toast-text").text.as_deref(),
        Some("Couldn't keep on this PC.")
    );
    assert!(asked(&calls).is_empty());

    // Trash works, then putting it back fails: the row stays in Trash, Undo stays possible.
    *fails.lock().unwrap() = vec![(Action::Restore, ActionError::Offline)];
    click(&mut rt, "row-6-date");
    press(&mut rt, Key::Delete, NONE);
    assert_eq!(names(&rt).len(), 10);
    press(&mut rt, Key::Command('z'), CTRL);
    assert_eq!(names(&rt).len(), 10, "nothing came back");
    assert_eq!(
        info(&rt, "toast-text").text.as_deref(),
        Some("Couldn't put it back. Check your connection.")
    );
    fails.lock().unwrap().clear();
    click(&mut rt, "toast-action");
    assert_eq!(names(&rt).len(), 11, "Try again puts it back");
    assert_eq!(names(&rt)[6], "Contract draft.pdf");
}

#[test]
fn an_action_that_fails_after_it_was_accepted_says_so_too() {
    let mut state = init();
    let mut fixture = FilesFixture::default();
    fixture.fail_later.push((Action::Duplicate, ActionError::Failed));
    state.actions.source = Box::new(fixture);
    let mut rt = runtime_of(state);
    click(&mut rt, "row-6-date");
    press(&mut rt, Key::Command('d'), CTRL);
    assert_eq!(names(&rt).len(), 11);
    assert_eq!(
        rt.state.live_toast(),
        Some(Toast::Failed(Action::Duplicate, ActionError::Failed))
    );
}

// ---- P2.4: the menu acts on the whole selection ----

#[test]
fn duplicate_keep_offline_copy_link_and_open_act_on_every_selected_item() {
    let (mut rt, calls, _) = spied();
    click(&mut rt, "row-6-date");
    click_with(&mut rt, "row-8-date", CTRL);
    let both = ids(&[&rt.state.listing.items[6].id, &rt.state.listing.items[8].id]);
    let (first, second) = (names(&rt)[6].clone(), names(&rt)[8].clone());

    press(&mut rt, Key::Enter, NONE);
    assert_eq!(asked(&calls), [Call::Open(both.clone())]);

    right_click(&mut rt, "row-6-date");
    let pick = |rt: &mut Runtime<State>, label: &str| {
        let key = rt
            .elements()
            .into_iter()
            .find(|e| !e.exiting && e.role == "menuitem" && e.label.as_deref() == Some(label))
            .unwrap_or_else(|| panic!("no menu item {label}"))
            .key
            .unwrap();
        click(rt, &key);
    };
    pick(&mut rt, "Copy link");
    assert_eq!(asked(&calls).last(), Some(&Call::CopyLink(both.clone())));

    right_click(&mut rt, "row-6-date");
    pick(&mut rt, "Make available offline");
    assert_eq!(asked(&calls).last(), Some(&Call::KeepOffline(both.clone(), true)));
    for row in [6, 8] {
        assert_eq!(
            rt.state.listing.items[row].pc_status.as_deref(),
            Some("available_offline")
        );
    }
    // All of it is kept now: the menu offers to remove it.
    right_click(&mut rt, "row-6-date");
    assert!(menu_labels(&rt).iter().any(|l| l == "Remove from this PC"));
    press(&mut rt, Key::Escape, NONE);

    press(&mut rt, Key::Command('d'), CTRL);
    assert_eq!(asked(&calls).last(), Some(&Call::Duplicate(both)));
    assert_eq!(names(&rt).len(), 13, "two copies");
    let all = names(&rt);
    let at = |name: &str| all.iter().position(|n| n == name).unwrap();
    assert_eq!(
        all[at(&first) + 1],
        first.replace('.', " copy."),
        "the copy stands under its original"
    );
    assert_eq!(all[at(&second) + 1], second.replace('.', " copy."));
    assert_eq!(
        rt.state.selected_rows(),
        [at(&first) + 1, at(&second) + 1],
        "the copies are selected"
    );
    assert_eq!(rt.state.live_toast(), Some(Toast::Duplicated));
}

// ---- P2.5: Preview on a folder ----

#[test]
fn preview_is_dimmed_on_a_folder_and_does_nothing() {
    let mut rt = runtime();
    click(&mut rt, "row-1-date");
    assert!(rt.state.listing.items[1].is_folder);
    press(&mut rt, Key::Text(" ".into()), NONE);
    assert!(
        rt.state.actions.intents.is_empty(),
        "Space on a folder asks for nothing"
    );

    right_click(&mut rt, "row-1-date");
    let preview = rt
        .elements()
        .into_iter()
        .find(|e| !e.exiting && e.role == "menuitem" && e.label.as_deref() == Some("Preview"))
        .unwrap();
    assert!(!preview.focusable, "a dimmed item takes no focus");
    let label = info(&rt, &format!("{}-label", preview.key.clone().unwrap()));
    let open = info(&rt, "menu-item-0-label");
    assert_ne!(label.text_color, open.text_color, "it is drawn dimmed");
    click(&mut rt, preview.key.as_deref().unwrap());
    assert!(rt.state.actions.menu.is_some(), "a click on it leaves the menu open");
    assert!(rt.state.actions.intents.is_empty());
    // The arrows pass it: Open, then Share….
    press(&mut rt, Key::Down, NONE);
    press(&mut rt, Key::Down, NONE);
    let hot = rt.state.actions.menu.unwrap().hot.unwrap();
    assert_eq!(rt.state.menu_entries(MenuKind::File)[hot].id(), Some("share"));
    // And a pick of it by index does nothing.
    rt.update(|s, tx| s.pick_menu(1, tx));
    assert!(rt.state.actions.intents.is_empty());
}

// ---- P2.6: each folder has its own sort, and the rows are in it ----

#[test]
fn each_folder_keeps_its_own_sort_and_the_rows_follow_the_menus_tick() {
    let (mut rt, calls, _) = spied();
    click(&mut rt, "col-size-cell");
    let root_by_size = names(&rt);
    rt.update(|s, _| s.open_folder(Some("f-lisbon".to_string())));
    settle(&mut rt);
    assert_eq!(
        rt.state.actions.sort.by,
        SortBy::Name,
        "Lisbon trip was never sorted by size"
    );
    assert_eq!(
        names(&rt),
        [
            "Day 1",
            "Day 2",
            "Alfama.heic",
            "Belem.heic",
            "Sunset.heic",
            "Tickets.pdf",
            "Tram 28.mov"
        ],
        "and its rows are in the order its menu ticks"
    );
    click(&mut rt, "sort-btn");
    let ticked: Vec<String> = rt
        .elements()
        .into_iter()
        .filter(|e| !e.exiting && e.role == "menuitemcheckbox")
        .filter_map(|e| e.label)
        .collect();
    assert_eq!(ticked, ["Name", "A to Z", "Folders first"]);
    press(&mut rt, Key::Escape, NONE);
    click(&mut rt, "col-date-cell");
    assert_eq!(names(&rt)[2], "Sunset.heic", "newest first");
    assert_eq!(
        asked(&calls).last(),
        Some(&Call::Sort(
            Some("f-lisbon".into()),
            super::Sort {
                by: SortBy::DateChanged,
                reversed: false,
                folders_first: true
            }
        ))
    );

    rt.update(|s, _| {
        s.back();
    });
    settle(&mut rt);
    assert_eq!(rt.state.actions.sort.by, SortBy::Size, "the root is still by size");
    assert_eq!(names(&rt), root_by_size);
    rt.update(|s, _| {
        s.forward();
    });
    settle(&mut rt);
    assert_eq!(rt.state.actions.sort.by, SortBy::DateChanged);
    assert_eq!(names(&rt)[2], "Sunset.heic");
}

// ---- P2.7: a protected folder takes nothing ----

#[test]
fn nothing_is_added_to_backups_new_is_dimmed_its_keys_are_dead_and_a_drop_is_refused() {
    let (mut rt, calls, _) = in_backups();
    assert_eq!(info(&rt, "crumb-here").text.as_deref(), Some("Backups"));
    let new = info(&rt, "new-btn");
    assert!(!new.focusable, "New takes no focus");
    assert!((new.opacity - 0.4).abs() < 0.01, "and is dimmed: {}", new.opacity);
    click(&mut rt, "new-btn");
    assert!(rt.state.actions.menu.is_none(), "a click opens nothing");
    press(&mut rt, Key::Command('n'), Mods { shift: true, ..CTRL });
    press(&mut rt, Key::Command('n'), CTRL);
    press(&mut rt, Key::Command('o'), CTRL);
    assert!(rt.state.actions.intents.is_empty(), "{:?}", rt.state.actions.intents);
    // Not the empty state of a folder that takes files: no "Add files".
    assert!(find(&rt, "empty-add").is_none() && find(&rt, "empty-title").is_none());
    assert!(find(&rt, "column-headers").is_some());
    let (x, y) = centre(&rt, "rows");
    rt.drop_files(vec![std::path::PathBuf::from("a.txt")], Some((x, y)));
    settle(&mut rt);
    assert!(asked(&calls).is_empty(), "the drop was refused: {:?}", asked(&calls));
}

#[test]
fn a_drop_on_a_folder_row_uploads_into_that_folder_and_backups_refuses() {
    let (mut rt, calls, _) = spied();
    let files = vec![std::path::PathBuf::from("a.txt")];
    let (x, y) = centre(&rt, "row-0-date");
    assert!(rt.state.listing.items[0].is_protected);
    rt.drop_files(files.clone(), Some((x, y)));
    settle(&mut rt);
    assert!(asked(&calls).is_empty(), "Backups refuses");
    let (x, y) = centre(&rt, "row-1-date");
    rt.drop_files(files.clone(), Some((x, y)));
    settle(&mut rt);
    assert_eq!(asked(&calls), [Call::Upload(Some("f-docs".into()), files)]);
}

#[test]
fn a_folder_the_fixture_does_not_hold_is_not_shown_as_empty() {
    let mut rt = runtime();
    click(&mut rt, "row-1-date");
    press(&mut rt, Key::Enter, NONE);
    assert!(
        find(&rt, "empty-title").is_none(),
        "Documents holds 48 items: never \"Nothing here yet\""
    );
    assert!(find(&rt, "file-list-error-text").is_some());
    assert!(
        !info(&rt, "new-btn").focusable,
        "and nothing can be added to what could not be read"
    );
}

// ---- P2.8: accessibility ----

#[test]
fn keyboard_focus_follows_the_menus_highlight() {
    let mut rt = runtime();
    right_click(&mut rt, "row-6-date");
    for _ in 0..5 {
        press(&mut rt, Key::Down, NONE);
        let hot = rt.state.actions.menu.unwrap().hot.unwrap();
        let entries = rt.state.menu_entries(MenuKind::File);
        let key = crate::ui::menu::item_key("menu", &entries, hot).unwrap();
        assert_eq!(focused(&rt), Some(key.clone()), "focus is on the highlighted item");
        let item = info(&rt, &key);
        assert_eq!(
            item.label.as_deref(),
            match entries[hot] {
                crate::ui::menu::Entry::Item { label, .. } => Some(label),
                _ => None,
            }
        );
    }
    press(&mut rt, Key::End, NONE);
    assert_eq!(
        info(&rt, &focused(&rt).unwrap()).label.as_deref(),
        Some("Move to Trash")
    );
}

#[test]
fn the_list_is_a_table_whose_rows_say_everything_and_it_is_one_tab_stop() {
    let mut rt = runtime();
    let table = info(&rt, "rows");
    assert_eq!(
        (table.role, table.label.as_deref(), table.focusable),
        ("table", Some("Files"), true)
    );
    let row = info(&rt, "row-6");
    assert_eq!(row.role, "row");
    assert_eq!(
        row.label.as_deref(),
        Some("Contract draft.pdf, changed Yesterday, 16:42, 1.2 MB, shared with 1 person, online only")
    );
    assert_eq!(
        info(&rt, "row-0").label.as_deref(),
        Some("Backups, protected, changed Today, 07:02, 4 devices, not shared, online only")
    );
    assert!(info(&rt, "row-7").label.unwrap().contains("shared by link"));

    // Keyboard focus is on the table, and stays there while the arrows move the selection.
    assert_eq!(focused(&rt).as_deref(), Some("rows"));
    assert!(!info(&rt, "row-6").focusable, "a row is no Tab stop of its own");
    press(&mut rt, Key::Down, NONE);
    assert_eq!((rt.state.selected, focused(&rt).as_deref()), (Some(7), Some("rows")));

    // One Tab leaves the list for what comes after it; one Shift+Tab comes back; one more leaves it the other way.
    press(&mut rt, Key::Tab, NONE);
    let after = focused(&rt).unwrap_or_default();
    assert_eq!(after, "offline-switch", "Tab went on to the details panel");
    press(&mut rt, Key::Tab, SHIFT);
    assert_eq!(
        focused(&rt).as_deref(),
        Some("rows"),
        "Shift+Tab comes back to the table"
    );
    press(&mut rt, Key::Tab, SHIFT);
    let before = focused(&rt).unwrap_or_default();
    assert!(!before.starts_with("row"), "and one more leaves it for {before}");
    // Back on the table the list's keys work.
    press(&mut rt, Key::Tab, NONE);
    assert_eq!(focused(&rt).as_deref(), Some("rows"));
    press(&mut rt, Key::Down, NONE);
    assert_eq!(rt.state.selected, Some(8));
    press(&mut rt, Key::ContextMenu, NONE);
    assert_eq!(rt.state.actions.menu.map(|m| m.kind), Some(MenuKind::File));
}

#[test]
fn undo_reads_as_undo_with_its_shortcut_apart() {
    let mut rt = runtime();
    press(&mut rt, Key::Delete, NONE);
    let undo = info(&rt, "toast-action");
    assert_eq!(undo.label.as_deref(), Some("Undo"));
    assert_eq!(info(&rt, "toast-action-text").text.as_deref(), Some("Undo Ctrl+Z"));
}

// ---- P2.9: the data seam ----

#[test]
fn a_window_with_no_engine_shows_no_sample_and_claims_no_change() {
    // What `--db` builds: a listing, and nothing behind it.
    let state = crate::shell::state::init_from(
        crate::shell::host::Shell::fixture(),
        Box::new(crate::source::BoardFixtureSource::new()),
        None,
    );
    let mut rt = runtime_of(state);
    for key in [
        "Trash-badge",
        "Shared-badge",
        "Available offline-badge",
        "account-name",
        "account-email",
        "avatar",
        "status-text",
        "status-dot",
        "storage-files",
    ] {
        assert!(find(&rt, key).is_none(), "{key} would be a sample");
    }
    assert!(find(&rt, "settings-btn").is_some());
    assert!(!info(&rt, "new-btn").focusable);
    click(&mut rt, "row-6-date");
    press(&mut rt, Key::Delete, NONE);
    assert_eq!(names(&rt).len(), 11, "nothing was moved to Trash");
    assert_eq!(
        rt.state.live_toast(),
        Some(Toast::Failed(Action::Trash, ActionError::Unavailable))
    );
    assert_eq!(info(&rt, "toast-text").text.as_deref(), Some("Couldn't move to Trash."));
    press(&mut rt, Key::Command('d'), CTRL);
    assert_eq!(names(&rt).len(), 11);
}

#[test]
fn the_sample_account_counts_and_sync_state_come_from_the_source() {
    let rt = runtime();
    assert_eq!(info(&rt, "account-name").text.as_deref(), Some("Anna Visser"));
    assert_eq!(info(&rt, "storage-files").text.as_deref(), Some("Files 24 GB"));
    assert_eq!(info(&rt, "storage-photos").text.as_deref(), Some("Photos 216 GB"));
    assert_eq!(info(&rt, "status-text").text.as_deref(), Some("Up to date"));
}

// ---- P3.2, P3.3: Undo ----

#[test]
fn ctrl_z_undoes_only_while_nothing_else_was_done_and_restores_into_the_sort() {
    let mut rt = runtime();
    press(&mut rt, Key::Delete, NONE);
    click(&mut rt, "row-7-date");
    press(&mut rt, Key::Command('d'), CTRL);
    let rows = names(&rt).len();
    press(&mut rt, Key::Command('z'), CTRL);
    assert_eq!(names(&rt).len(), rows, "something else was done: nothing comes back");

    // Undo while the toast still stands: the row goes where the order puts it.
    let mut rt = runtime();
    rt.update(|s, _| s.sort_by_column(SortBy::Size));
    settle(&mut rt);
    let before = names(&rt);
    let row = before.iter().position(|n| n == "Contract draft.pdf").unwrap();
    click(&mut rt, &format!("row-{row}-date"));
    press(&mut rt, Key::Delete, NONE);
    press(&mut rt, Key::Command('z'), CTRL);
    assert_eq!(names(&rt), before);
    assert_eq!(rt.state.selected, Some(row));
}
