//! SelectiveSync wave-2 tests, split out of `tests.rs` (file-size ratchet; moved unchanged).

use super::*;
use crate::tests::{at, file, folder};
use std::collections::HashSet;

// ── SelectiveSync wave-2: tree-build + aggregation + exclude-diff ──────────

#[test]
fn folder_leaf_name_takes_last_path_segment() {
    assert_eq!(folder_leaf_name("/Docs/Taxes").as_deref(), Some("Taxes"));
    assert_eq!(folder_leaf_name("Photos").as_deref(), Some("Photos"));
    assert_eq!(folder_leaf_name("/Photos/").as_deref(), Some("Photos"));
    assert_eq!(folder_leaf_name(""), None);
    assert_eq!(folder_leaf_name("/"), None);
}

#[test]
fn build_tree_nests_folders_and_aggregates_subtree() {
    // root
    //  ├─ A (folder)
    //  │   ├─ a1.txt  100  on_disk
    //  │   └─ B (folder)
    //  │        └─ b1.txt 50  cloud-only
    //  └─ C (folder)  (empty)
    let rows = vec![
        folder("A", None, "Alpha"),
        folder("B", Some("A"), "Bravo"),
        folder("C", None, "Charlie"),
        file("a1", Some("A"), 100, true),
        file("b1", Some("B"), 50, false),
    ];
    let excluded = HashSet::new();
    let tree = build_vault_tree(&rows, &excluded);

    // Two roots, sorted by name: Alpha, Charlie.
    assert_eq!(tree.len(), 2);
    assert_eq!(tree[0].id, "A");
    assert_eq!(tree[1].id, "C");

    let a = &tree[0];
    // A aggregates a1 (100) + b1 (50) = 150 bytes, 2 files, 100 on disk.
    assert_eq!(a.size_bytes, 150);
    assert_eq!(a.file_count, 2);
    assert_eq!(a.on_disk_bytes, 100);
    assert!(a.is_folder);

    // A has one child folder B.
    assert_eq!(a.children.len(), 1);
    let b = &a.children[0];
    assert_eq!(b.id, "B");
    assert_eq!(b.size_bytes, 50);
    assert_eq!(b.file_count, 1);
    assert_eq!(b.on_disk_bytes, 0);

    // C is empty.
    let c = &tree[1];
    assert_eq!(c.size_bytes, 0);
    assert_eq!(c.file_count, 0);
    assert!(c.children.is_empty());
}

#[test]
fn build_tree_marks_excluded_and_dangling_parents_become_roots() {
    let rows = vec![
        folder("A", None, "Alpha"),
        // D's parent points at an unknown folder → must surface as a root,
        // never silently dropped.
        folder("D", Some("ghost"), "Delta"),
        file("a1", Some("A"), 10, true),
    ];
    let mut excluded = HashSet::new();
    excluded.insert("A".to_string());

    let tree = build_vault_tree(&rows, &excluded);
    let ids: Vec<&str> = tree.iter().map(|n| n.id.as_str()).collect();
    assert!(ids.contains(&"A"));
    assert!(ids.contains(&"D"), "dangling-parent folder must still appear as a root");

    let a = tree.iter().find(|n| n.id == "A").unwrap();
    assert!(a.excluded, "folder in the excluded set must be marked excluded");
    let d = tree.iter().find(|n| n.id == "D").unwrap();
    assert!(!d.excluded);
}

/// 0.8.15 (Guus's PC, 0.8.14): Selective sync listed one folder twice. One row the server never confirmed (a folder
/// this PC created on 4 Oct whose `create_folder` stayed paused) and the server's row at the same path. The tree shows
/// one `Archive`, the server's; the never-sent child with a server twin folds into it too; a never-sent child with no
/// twin stays, under the server's folder; the same file twice is counted once.
#[test]
fn selective_sync_shows_one_folder_per_path_preferring_the_server_row() {
    let rows = vec![
        at(folder("ghost-archive", None, "Archive"), "Archive", false),
        at(folder("srv-archive", None, "Archive"), "Archive", true),
        at(
            folder("ghost-other", Some("ghost-archive"), "other"),
            "Archive/other",
            false,
        ),
        at(folder("srv-other", Some("srv-archive"), "other"), "Archive/other", true),
        at(folder("ghost-only", Some("ghost-archive"), "new"), "Archive/new", false),
        at(file("f1", Some("srv-other"), 100, true), "Archive/other/a.txt", true),
        at(file("f2", Some("srv-archive"), 50, false), "Archive/b.txt", true),
        at(file("f2-again", Some("srv-archive"), 50, true), "Archive/b.txt", false),
    ];
    let tree = build_vault_tree(&rows, &HashSet::new());

    let roots: Vec<(&str, &str)> = tree.iter().map(|n| (n.id.as_str(), n.name.as_str())).collect();
    assert_eq!(roots, vec![("srv-archive", "Archive")], "one Archive, the server's");
    let archive = &tree[0];
    assert_eq!(
        (archive.size_bytes, archive.file_count, archive.on_disk_bytes),
        (150, 2, 100),
        "two files, 150 bytes, 100 of them on disk: nothing counted twice"
    );
    let children: Vec<(&str, &str)> = archive
        .children
        .iter()
        .map(|c| (c.id.as_str(), c.name.as_str()))
        .collect();
    assert_eq!(children, vec![("ghost-only", "new"), ("srv-other", "other")]);
    let other = &archive.children[1];
    assert_eq!((other.size_bytes, other.file_count, other.on_disk_bytes), (100, 1, 100));
    assert!(archive.children[0].children.is_empty() && other.children.is_empty());
    assert_eq!(rows.len(), 8, "the rows themselves are untouched (display only)");
}

/// The merge is narrow: two rows the server confirmed at one path are two server items and both stay (with their
/// own online-only state); two never-confirmed rows at one path are one entry (the smallest id, for a stable choice);
/// a file and a folder at one path are never merged; a row with no path is left alone.
#[test]
fn per_path_merge_keeps_two_server_items_and_never_mixes_kinds() {
    let rows = vec![
        at(folder("srv-a", None, "Photos"), "Photos", true),
        at(folder("srv-b", None, "Photos"), "Photos", true),
        at(folder("ghost-2", None, "Docs"), "Docs", false),
        at(folder("ghost-1", None, "Docs"), "Docs", false),
        at(file("a-file-named-docs", Some("ghost-2"), 7, true), "/Docs/", true),
        at(folder("no-path-1", None, "Folder 1"), "", false),
        at(folder("no-path-2", None, "Folder 2"), "", true),
    ];
    let mut excluded = HashSet::new();
    excluded.insert("srv-b".to_string());
    let tree = build_vault_tree(&rows, &excluded);

    let ids: Vec<&str> = tree.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, vec!["ghost-1", "no-path-1", "no-path-2", "srv-a", "srv-b"]);
    assert_eq!(
        tree.iter().map(|n| n.excluded).collect::<Vec<_>>(),
        vec![false, false, false, false, true]
    );
    assert_eq!(
        (tree[0].file_count, tree[0].size_bytes),
        (1, 7),
        "the file under the folded row counts once"
    );
}

/// What "the server knows the row" is read from: a server version, or a server timestamp (Guus's DB had 16 server
/// files with a timestamp and no version filled yet). A folder this PC created and never sent has neither.
#[test]
fn a_row_is_server_known_by_its_version_or_its_server_timestamp() {
    let row = |current_version: i64, remote_updated_at: i64, status: crate::state_db::FileStatus| {
        vault_entry_row(&crate::state_db::TreeRow {
            file_id: "id".into(),
            path: "Archive".into(),
            status,
            size_bytes: 0,
            parent_id: None,
            item_kind: crate::state_db::ItemKind::Folder,
            current_version,
            remote_updated_at,
        })
    };
    use crate::state_db::FileStatus::{CloudOnly, Local};
    assert!(!row(0, 0, Local).server_known && row(0, 0, Local).local_status);
    assert!(row(3, 0, CloudOnly).server_known && !row(3, 0, CloudOnly).local_status);
    assert!(row(0, 1_761_480_000, CloudOnly).server_known);
    assert_eq!(
        (row(0, 0, Local).name, row(0, 0, Local).path),
        ("Archive".to_string(), "Archive".to_string())
    );
}

/// The rows behind `tests/fixtures/selective-sync-tree.json`, the browser fixture `tests/render-sync-activity.mjs`
/// screenshots: Guus's shape with synthetic names. Every top folder the server knows also has a row this PC created
/// on 4 Oct and never sent (and so do two subfolders, under the never-sent parents); every file hangs under the
/// server's folders; an `error` row with no parent repeats a file's path. `Music` has no twin.
fn guus_shaped_rows() -> Vec<VaultEntryRow> {
    vec![
        at(folder("srv-documents", None, "Documents"), "Documents", true),
        at(folder("srv-photos", None, "Photos"), "Photos", true),
        at(folder("srv-archive", None, "Archive"), "Archive", true),
        at(folder("srv-music", None, "Music"), "Music", true),
        at(
            folder("srv-taxes", Some("srv-documents"), "Taxes"),
            "Documents/Taxes",
            true,
        ),
        at(folder("srv-2026", Some("srv-photos"), "2026"), "Photos/2026", true),
        at(folder("srv-other", Some("srv-archive"), "other"), "Archive/other", true),
        at(folder("ghost-documents", None, "Documents"), "Documents", false),
        at(folder("ghost-photos", None, "Photos"), "Photos", false),
        at(folder("ghost-archive", None, "Archive"), "Archive", false),
        at(
            folder("ghost-taxes", Some("ghost-documents"), "Taxes"),
            "Documents/Taxes",
            false,
        ),
        at(
            folder("ghost-other", Some("ghost-archive"), "other"),
            "Archive/other",
            false,
        ),
        at(
            file("f-report", Some("srv-documents"), 2_400_000, true),
            "Documents/report.pdf",
            true,
        ),
        at(
            file("f-return", Some("srv-taxes"), 860_000, false),
            "Documents/Taxes/return.pdf",
            true,
        ),
        at(
            file("f-beach", Some("srv-2026"), 4_100_000, false),
            "Photos/2026/beach.jpg",
            true,
        ),
        at(
            file("f-hike", Some("srv-2026"), 3_700_000, true),
            "Photos/2026/hike.jpg",
            true,
        ),
        at(
            file("f-disk", Some("srv-other"), 512_000_000, false),
            "Archive/other/disk.img",
            true,
        ),
        at(
            file("f-song", Some("srv-music"), 9_800_000, false),
            "Music/song.flac",
            true,
        ),
        at(
            file("err-report", None, 2_400_000, false),
            "Documents/report.pdf",
            false,
        ),
    ]
}

/// The browser fixture is what `build_vault_tree` returns for [`guus_shaped_rows`] (`after`) and what 0.8.14 returned
/// (`before`, no merge), so the Selective sync screenshots show the real output of both. `BB_UPDATE_FIXTURES=1`
/// rewrites the file.
#[test]
fn the_selective_sync_browser_fixture_is_the_trees_real_output() {
    let rows = guus_shaped_rows();
    let excluded = HashSet::new();
    let fixture = serde_json::json!({
        "before": build_vault_tree_of_rows(&rows, &excluded),
        "after": build_vault_tree(&rows, &excluded),
    });
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../tests/fixtures/selective-sync-tree.json"
    );
    if std::env::var_os("BB_UPDATE_FIXTURES").is_some() {
        std::fs::write(path, format!("{}\n", serde_json::to_string_pretty(&fixture).unwrap())).unwrap();
    }
    let on_disk: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(
        on_disk, fixture,
        "rerun with BB_UPDATE_FIXTURES=1 after a deliberate change"
    );
    let roots = |tree: &serde_json::Value| -> Vec<String> {
        tree.as_array()
            .unwrap()
            .iter()
            .map(|n| n["name"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        roots(&fixture["before"]),
        [
            "Archive",
            "Archive",
            "Documents",
            "Documents",
            "Music",
            "Photos",
            "Photos"
        ]
    );
    assert_eq!(roots(&fixture["after"]), ["Archive", "Documents", "Music", "Photos"]);
}

/// 0.8.15 evidence, not part of the suite: build the tree from a COPY of a real state DB (opening it migrates it),
/// named by `BB_SELECTIVE_SYNC_DB`. Prints counts only, never a name.
#[test]
#[ignore = "reads a real state DB copy named by BB_SELECTIVE_SYNC_DB"]
fn selective_sync_tree_from_a_real_state_db_copy() {
    fn walk(nodes: &[VaultItem], total: &mut usize, sibling_dupes: &mut usize, files: &mut i64, depth: usize) {
        let mut names: Vec<String> = nodes.iter().map(|n| n.name.to_lowercase()).collect();
        names.sort();
        *sibling_dupes += names.windows(2).filter(|pair| pair[0] == pair[1]).count();
        for node in nodes {
            *total += 1;
            if depth == 0 {
                *files += node.file_count;
            }
            walk(&node.children, total, sibling_dupes, files, depth + 1);
        }
    }
    let path = std::env::var("BB_SELECTIVE_SYNC_DB").expect("BB_SELECTIVE_SYNC_DB names a state.db copy");
    let db = crate::state_db::StateDb::open(&path).unwrap();
    let rows: Vec<VaultEntryRow> = db.list_tree_rows().unwrap().iter().map(vault_entry_row).collect();
    let mut folder_paths: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for row in rows.iter().filter(|row| row.is_folder) {
        *folder_paths.entry(row.path.trim_matches('/')).or_default() += 1;
    }
    let tree = build_vault_tree(&rows, &HashSet::new());
    let (mut nodes, mut sibling_dupes, mut files) = (0, 0, 0);
    walk(&tree, &mut nodes, &mut sibling_dupes, &mut files, 0);
    println!(
        "rows={} folder_rows={} folder_paths={} folder_paths_with_2+_rows={} tree_roots={} tree_folders={} \
         sibling_name_duplicates={} files_counted_under_roots={}",
        rows.len(),
        rows.iter().filter(|row| row.is_folder).count(),
        folder_paths.len(),
        folder_paths.values().filter(|n| **n > 1).count(),
        tree.len(),
        nodes,
        sibling_dupes,
        files
    );
    assert_eq!(nodes, folder_paths.len(), "one tree entry per folder path");
    assert_eq!(sibling_dupes, 0);
}

#[test]
fn build_tree_is_cycle_safe() {
    // A → B → A parent cycle (corrupt data). With edges A→B and B→A,
    // BOTH nodes have a known parent inside the folder set, so neither
    // qualifies as a root. The builder produces an empty tree (roots = [])
    // rather than infinite-looping. The assertion verifies termination —
    // tree.len() is 0 in practice; the ≤ 2 upper bound accommodates any
    // future builder change that surfaces cycle members as roots.
    let rows = vec![folder("A", Some("B"), "Alpha"), folder("B", Some("A"), "Bravo")];
    let tree = build_vault_tree(&rows, &HashSet::new());
    // No panic / hang; at most the reachable nodes are emitted.
    assert!(tree.len() <= 2);
}

#[test]
fn newly_excluded_ids_diffs_against_old() {
    let old = vec!["a".to_string(), "b".to_string()];
    let new = vec!["b".to_string(), "c".to_string(), "d".to_string(), "c".to_string()];
    // c and d are new; b was already excluded; duplicate c de-duped.
    assert_eq!(newly_excluded_ids(&old, &new), vec!["c".to_string(), "d".to_string()]);

    // Un-excluding (removing) yields nothing newly-excluded.
    assert!(newly_excluded_ids(&old, &[]).is_empty());
    // No change → empty.
    assert!(newly_excluded_ids(&old, &old).is_empty());
}

#[test]
fn subtree_file_ids_collects_descendant_files_only() {
    // A ─ a1, A ─ B ─ b1, A ─ B ─ b2(folder) ─ c1; sibling root Z ─ z1.
    let rows = vec![
        ("A".to_string(), None, true),
        ("B".to_string(), Some("A".to_string()), true),
        ("a1".to_string(), Some("A".to_string()), false),
        ("b1".to_string(), Some("B".to_string()), false),
        ("b2".to_string(), Some("B".to_string()), true),
        ("c1".to_string(), Some("b2".to_string()), false),
        ("Z".to_string(), None, true),
        ("z1".to_string(), Some("Z".to_string()), false),
    ];
    let mut got = subtree_file_ids(&rows, &["A".to_string()]);
    got.sort();
    // Files under A: a1, b1, c1. NOT z1 (under Z), and folders excluded.
    assert_eq!(got, vec!["a1".to_string(), "b1".to_string(), "c1".to_string()]);
}

#[test]
fn subtree_file_ids_dedupes_overlapping_excluded_roots() {
    // Excluding both A and its child B must not list B's files twice.
    let rows = vec![
        ("A".to_string(), None, true),
        ("B".to_string(), Some("A".to_string()), true),
        ("b1".to_string(), Some("B".to_string()), false),
    ];
    let got = subtree_file_ids(&rows, &["A".to_string(), "B".to_string()]);
    assert_eq!(got, vec!["b1".to_string()]);
}

#[test]
fn bandwidth_sample_rates_convert_bytes_window_to_bytes_per_second() {
    assert_eq!(super::bandwidth_sample_rates(1_500, 3_000, 3), (500, 1_000));
    assert_eq!(super::bandwidth_sample_rates(1_500, 3_000, 0), (0, 0));
}

#[test]
fn normalized_process_cpu_percent_divides_by_logical_core_count_and_caps() {
    assert_eq!(super::normalize_process_cpu_percent(200.0, 8), 25.0);
    assert_eq!(super::normalize_process_cpu_percent(12.34, 1), 12.3);
    assert_eq!(super::normalize_process_cpu_percent(900.0, 8), 100.0);
    assert_eq!(super::normalize_process_cpu_percent(-4.0, 8), 0.0);
    assert_eq!(super::normalize_process_cpu_percent(f32::NAN, 8), 0.0);
    assert_eq!(super::normalize_process_cpu_percent(42.0, 0), 42.0);
}

#[test]
fn native_menu_specs_cover_required_real_actions() {
    let specs = desktop_menu_specs();
    let ids: Vec<&str> = specs.iter().map(|spec| spec.id).collect();

    assert!(ids.contains(&MENU_PREFERENCES_ID));
    assert!(ids.contains(&MENU_SETTINGS_ID));
    assert!(ids.contains(&MENU_CHECK_UPDATES_ID));
    assert!(ids.contains(&MENU_TOGGLE_SYNC_ID));
    assert!(ids.contains(&MENU_SIGN_OUT_ID));
    assert!(ids.contains(&MENU_QUIT_ID));
    assert!(ids.contains(&MENU_OPEN_FOLDER_ID));
    assert!(ids.contains(&MENU_UPLOAD_FILES_ID));
    assert!(ids.contains(&MENU_NEW_FOLDER_ID));
    assert!(ids.contains(&MENU_OPEN_WEB_APP_ID));
    assert!(ids.contains(&MENU_VIEW_FILES_ID));
    assert!(ids.contains(&MENU_VIEW_ACTIVITY_ID));
    assert!(
        !ids.contains(&MENU_VIEW_SHARED_ID),
        "hide the dead-end Shared native menu item until real sharing ships"
    );
    assert!(ids.contains(&MENU_VIEW_TRASH_ID));
    assert!(ids.contains(&MENU_ZOOM_IN_ID));
    assert!(ids.contains(&MENU_ZOOM_OUT_ID));
    assert!(ids.contains(&MENU_ZOOM_RESET_ID));
    assert!(ids.contains(&MENU_HELP_DOCS_ID));
    assert!(ids.contains(&MENU_HELP_SHORTCUTS_ID));
    assert!(ids.contains(&MENU_HELP_REPORT_ID));
    assert!(ids.contains(&MENU_HELP_STATUS_ID));

    for spec in specs {
        if let DesktopMenuAction::OpenExternal(url) = spec.action {
            assert!(!url.is_empty(), "menu item {} must have a real URL", spec.id);
        }
    }
}

#[test]
fn preferences_and_settings_open_same_settings_surface() {
    let specs = desktop_menu_specs();
    let preferences = specs.iter().find(|spec| spec.id == MENU_PREFERENCES_ID).unwrap();
    let settings = specs.iter().find(|spec| spec.id == MENU_SETTINGS_ID).unwrap();

    assert_eq!(preferences.action, DesktopMenuAction::OpenSettings);
    assert_eq!(settings.action, DesktopMenuAction::OpenSettings);
    assert_eq!(preferences.accelerator, Some("CmdOrCtrl+Comma"));
    assert_eq!(settings.accelerator, Some("CmdOrCtrl+Comma"));
}

/// Keys a Chromium web view (WebView2 on Windows) acts on itself, with a meaning other than the menu item's.
/// Microsoft's list (ICoreWebView2Settings3: find, print, reload, DevTools, caret browsing, back/forward) plus
/// Ctrl+Shift+P, which printed the page on Guus's PC in 0.8.14 instead of pausing sync. The zoom keys are left out on
/// purpose: the View menu's Ctrl+= / Ctrl+- / Ctrl+0 mean what WebView2 means by them.
const WEB_VIEW_BROWSER_KEYS: &[&str] = &[
    "Ctrl+P",
    "Ctrl+Shift+P",
    "Ctrl+F",
    "F3",
    "Ctrl+G",
    "Ctrl+Shift+G",
    "Ctrl+R",
    "Ctrl+Shift+R",
    "F5",
    "Ctrl+Shift+C",
    "Ctrl+Shift+I",
    "Ctrl+Shift+J",
    "F7",
    "F12",
    "Alt+Left",
    "Alt+Right",
];

/// Ctrl+Alt+<key> is AltGr+<key> on most European layouts (it types @, €, { ...), so a menu shortcut there eats a
/// character on Windows and Linux.
fn is_altgr_chord(accelerator: &str) -> bool {
    let parts: Vec<&str> = accelerator.split('+').collect();
    parts.contains(&"Alt") && (parts.contains(&"Ctrl") || parts.contains(&"CmdOrCtrl"))
}

// the_browser_keys_plugin_is_registered_before_any_window_is_built: deleted (webview_browser_keys removed; v2-native
// has no web views).

#[test]
fn no_windows_or_linux_menu_shortcut_is_a_web_view_browser_key_or_an_altgr_chord() {
    let mut checked = 0;
    for platform in [MenuPlatform::Windows, MenuPlatform::Linux] {
        for spec in desktop_menu_specs() {
            let Some(accelerator) = menu_accelerator(spec, platform) else {
                continue;
            };
            checked += 1;
            let on_this_platform = accelerator.replace("CmdOrCtrl", "Ctrl");
            assert!(
                !WEB_VIEW_BROWSER_KEYS.contains(&on_this_platform.as_str()),
                "{:?}: \"{}\" uses {on_this_platform}, which the web view acts on first",
                platform,
                spec.label
            );
            assert!(
                !is_altgr_chord(accelerator),
                "{:?}: \"{}\" uses {accelerator}, which is AltGr on European keyboards",
                platform,
                spec.label
            );
        }
    }
    // Every other item keeps its shortcut on both platforms: 16 of the 20 items have one there.
    assert_eq!(checked, 32, "the check must have looked at the shortcuts it guards");
}

#[test]
fn pause_and_resume_keeps_its_shortcut_on_macos_only() {
    let toggle = desktop_menu_spec_required(MENU_TOGGLE_SYNC_ID);
    assert_eq!(menu_accelerator(toggle, MenuPlatform::MacOs), Some("CmdOrCtrl+Shift+P"));
    assert_eq!(menu_accelerator(toggle, MenuPlatform::Windows), None);
    assert_eq!(menu_accelerator(toggle, MenuPlatform::Linux), None);
    // Nothing else changes per platform.
    for spec in desktop_menu_specs()
        .iter()
        .filter(|spec| spec.id != MENU_TOGGLE_SYNC_ID)
    {
        for platform in [MenuPlatform::MacOs, MenuPlatform::Windows, MenuPlatform::Linux] {
            assert_eq!(menu_accelerator(spec, platform), spec.accelerator, "{}", spec.label);
        }
    }
}

#[test]
fn trash_view_routes_to_trash_when_supported_otherwise_files() {
    assert_eq!(
        menu_view_nav_target(DesktopMenuView::Trash, &["files", "activity", "shared", "trash"]),
        "trash"
    );
    assert_eq!(
        menu_view_nav_target(DesktopMenuView::Trash, &["files", "activity", "shared"]),
        "files"
    );
}

#[test]
fn shared_view_falls_back_to_files_until_windows_has_real_shared_route() {
    assert_eq!(
        menu_view_nav_target(DesktopMenuView::Shared, WINDOWS_MAIN_APP_VIEW_ROUTES),
        "files"
    );
}

#[test]
fn compact_shared_view_falls_back_to_finder_until_real_shared_route() {
    assert_eq!(compact_menu_page_for_view(DesktopMenuView::Shared), "finder");
}

#[test]
fn zoom_scale_steps_clamps_and_resets() {
    assert_eq!(next_menu_zoom_scale(1.0, MenuZoomAction::In), 1.1);
    assert_eq!(next_menu_zoom_scale(1.0, MenuZoomAction::Out), 0.9);
    assert_eq!(next_menu_zoom_scale(1.95, MenuZoomAction::In), 2.0);
    assert_eq!(next_menu_zoom_scale(0.55, MenuZoomAction::Out), 0.5);
    assert_eq!(next_menu_zoom_scale(1.4, MenuZoomAction::Reset), 1.0);
}

#[test]
fn unused_upload_destination_preserves_extension() {
    let root = std::env::temp_dir().join(format!("beebeeb-menu-test-{}", now_unix_seconds()));
    std::fs::create_dir_all(&root).unwrap();
    let existing = root.join("Report.pdf");
    std::fs::write(&existing, b"existing").unwrap();

    let next = unused_child_path(&root, "Report.pdf").unwrap();

    assert_eq!(next.file_name().unwrap().to_string_lossy(), "Report 2.pdf");
    let _ = std::fs::remove_dir_all(root);
}
