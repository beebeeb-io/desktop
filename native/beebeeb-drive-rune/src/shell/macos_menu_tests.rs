//! The menu table of `docs/macos/menu-bar-and-windows.md` §1, read as data (tests ported from §9.2).

use std::collections::HashSet;

use super::*;

fn rows() -> impl Iterator<Item = &'static Row> {
    TABLE.iter().filter(|r| !r.id.is_empty())
}

fn entries(menu: &str) -> Vec<&'static str> {
    TABLE
        .iter()
        .take_while(|r| r.menu == menu || r.id.is_empty())
        .map(|r| r.label)
        .collect()
}

#[test]
fn the_macos_items_have_the_words_keys_and_targets_of_f1_to_f11() {
    let expect: &[(&str, &str, &str, Act)] = &[
        ("about", "About Beebeeb", "", Act::Native),
        ("settings", "Settings…", "cmd+,", Act::Settings),
        (
            "check-for-updates",
            "Check for Updates…",
            "shift+cmd+u",
            Act::CheckForUpdates,
        ),
        ("pause-resume", "Pause Sync", "shift+cmd+p", Act::PauseResume),
        ("sign-out", "Sign Out…", "shift+cmd+l", Act::SignOut),
        ("hide", "Hide Beebeeb", "cmd+h", Act::Native),
        ("quit", "Quit Beebeeb", "cmd+q", Act::Native),
        ("open-in-finder", "Open in Finder", "cmd+o", Act::OpenInFinder),
        ("upload-files", "Upload Files…", "cmd+u", Act::UploadFiles),
        ("new-folder", "New Folder…", "shift+cmd+n", Act::NewFolder),
        ("open-web-app", "Open Web App", "shift+cmd+o", Act::OpenWebApp),
        ("home", "Home", "cmd+1", Act::Home),
        ("back", "Back", "cmd+[", Act::Back),
        ("zoom-in", "Zoom in", "cmd+=", Act::ZoomIn),
        ("zoom-out", "Zoom out", "cmd+-", Act::ZoomOut),
        ("actual-size", "Actual size", "cmd+0", Act::ActualSize),
        ("shortcuts", "Keyboard shortcuts", "cmd+/", Act::Shortcuts),
    ];
    for (id, label, key, act) in expect {
        let row = row(id).unwrap_or_else(|| panic!("{id} is in the table"));
        assert_eq!((row.label, row.key, row.act), (*label, *key, *act), "{id}");
    }
}

#[test]
fn each_key_is_bound_once_and_the_legacy_items_are_gone() {
    let mut seen = HashSet::new();
    for row in rows().filter(|r| !r.key.is_empty()) {
        let key: Shortcut = row.key.parse().unwrap_or_else(|_| panic!("{} parses", row.key));
        assert!(seen.insert(key), "{} is bound twice", row.key);
    }
    // F10 and F11 (the legacy File › Settings and the duplicates) are not in the table.
    assert_eq!(rows().filter(|r| r.act == Act::Settings).count(), 1);
    assert!(rows().all(|r| !r.key.to_lowercase().contains("f10") && !r.key.to_lowercase().contains("f11")));
    let ids: HashSet<_> = rows().map(|r| r.id).collect();
    assert_eq!(ids.len(), rows().count(), "ids are unique");
}

#[test]
fn the_menu_is_the_order_of_section_1() {
    let app: Vec<_> = entries(APP);
    assert_eq!(
        app,
        [
            "About Beebeeb",
            "Settings…",
            "Check for Updates…",
            "Pause Sync",
            "",
            "Sign Out…",
            "",
            "Hide Beebeeb",
            "Hide Others",
            "Show All",
            "",
            "Quit Beebeeb"
        ]
    );
    let file: Vec<_> = rows().filter(|r| r.menu == "File").map(|r| r.label).collect();
    assert_eq!(file, ["Open in Finder", "Upload Files…", "New Folder…", "Open Web App"]);
    let view: Vec<_> = rows().filter(|r| r.menu == "View").map(|r| r.label).collect();
    assert_eq!(view, ["Home", "Back", "Zoom in", "Zoom out", "Actual size"]);
    let help: Vec<_> = rows().filter(|r| r.menu == "Help").map(|r| r.label).collect();
    assert_eq!(
        help,
        [
            "Documentation",
            "Keyboard shortcuts",
            "Report a problem",
            "Service status"
        ]
    );
    // Menus come in the order App, File, View, Help (Edit and Window are rune's).
    let mut menus: Vec<&str> = Vec::new();
    for r in rows() {
        if menus.last() != Some(&r.menu) {
            menus.push(r.menu);
        }
    }
    assert_eq!(menus, [APP, "File", "View", "Help"]);
}

#[test]
fn the_pause_label_follows_the_pause_state() {
    assert_eq!(pause_label(false), "Pause Sync");
    assert_eq!(pause_label(true), "Resume Sync");
    assert_eq!(row("pause-resume").map(|r| r.label), Some(PAUSE_LABEL));
}

#[test]
fn every_item_is_installed_once_and_a_missing_action_is_disabled_not_hidden() {
    let installed = items(false);
    let own = rows().filter(|r| r.act != Act::Native).count();
    let lines = TABLE.iter().filter(|r| r.id.is_empty()).count();
    assert_eq!(installed.len(), own + lines);
    // Nothing is dropped for being unbuilt: the unbuilt rows are in the table and in the install.
    assert!(rows().any(|r| !r.act.built() && r.act != Act::Native));
    assert!(Act::Settings.built() && !Act::UploadFiles.built());
}
