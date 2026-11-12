//! Tests of Diagnostics (DW-0790, DW-0790b) and Keep on this PC (DW-0784, b, c, d): that each
//! is reached by the app's own controls, and what every control on it does.
//!
//! Input is scripted: keys and presses go through Rune's `Runtime`, as a person's would.

use super::diagnostics::Bundle;
use super::drive::{BoardDrive, DriveSettings};
use super::keep::{Change, Check, Folder, Keep, KeepDraft, size};
use super::more::board_folders;
use super::{Dialog, Intent, Pane};
use crate::shell::app::{Screen, SettingsPane};
use crate::shell::host;
use crate::{State, init, view};
use rune::{Env, Key, KeyEvent, Mods, Runtime};

fn window(state: State) -> Runtime<State> {
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

fn runtime(frame: &str) -> Runtime<State> {
    let mut state = init();
    assert!(super::scene(frame, &mut state), "{frame} is a built frame");
    window(state)
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

fn says(rt: &Runtime<State>, key: &str) -> String {
    info(rt, key).text.unwrap_or_default()
}

fn named(rt: &Runtime<State>, key: &str) -> String {
    info(rt, key).label.unwrap_or_default()
}

fn key(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    settle(rt);
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let r = info(rt, key).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}

/// Tab until the control keyed `target` has the focus.
fn tab_to(rt: &mut Runtime<State>, target: &str) {
    for _ in 0..120 {
        key(rt, Key::Tab);
        let focused = rt.elements().into_iter().find(|e| e.focused && !e.exiting);
        if focused.and_then(|e| e.key).as_deref() == Some(target) {
            return;
        }
    }
    panic!("Tab never reaches {target}");
}

fn ui(rt: &Runtime<State>) -> &super::SettingsUi {
    &rt.state.settings
}

fn stored(rt: &Runtime<State>) -> Vec<Folder> {
    match ui(rt).drive.keep() {
        Keep::Ready { folders, .. } => folders,
        Keep::Failed => panic!("the folder list is loaded"),
    }
}

fn folder<'a>(folders: &'a [Folder], id: &str) -> &'a Folder {
    folders
        .iter()
        .find_map(|f| {
            if f.id == id {
                Some(f)
            } else {
                f.children.iter().find(|c| c.id == id)
            }
        })
        .unwrap_or_else(|| panic!("no folder {id}"))
}

// ---- Diagnostics (DW-0790, DW-0790b) ----

#[test]
fn diagnostics_opens_from_general_and_the_way_back_leads_there() {
    let mut rt = runtime("DW-0701");
    tab_to(&mut rt, "item-1-2");
    key(&mut rt, Key::Enter);
    assert_eq!(ui(&rt).pane, Pane::Diagnostics);
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::Settings(SettingsPane::General),
        "a page of General"
    );
    assert!(info(&rt, "General-row").fill.a > 0.0, "General stays the selected item");
    assert_eq!(says(&rt, "set-title"), "Diagnostics");
    assert_eq!(info(&rt, "set-title").role, "heading");
    assert_eq!(ui(&rt).intents, Vec::new(), "nothing is asked of the shell");
    tab_to(&mut rt, "crumb-row");
    key(&mut rt, Key::Enter);
    assert_eq!(ui(&rt).pane, Pane::General);
}

#[test]
fn app_activity_is_four_measures_each_with_its_value_in_words() {
    let rt = runtime("DW-0790");
    let rows: Vec<String> = (0..4).map(|i| named(&rt, &format!("item-0-{i}"))).collect();
    assert_eq!(
        rows,
        vec!["CPU, 3.1 %", "Memory, 212 MB", "Upload, 3.2 MB/s", "Download, 410 KB/s"]
    );
    assert_eq!(says(&rt, "spark-0-value"), "3.1 %");
    // The sparkline is a picture of the value beside it: 72 x 20, not a control.
    let spark = info(&rt, "spark-2").rect;
    assert_eq!((spark[2], spark[3]), (72.0, 20.0));
    assert!(find(&rt, "note-0").is_none(), "no banner before an export");
}

#[test]
fn every_help_row_of_diagnostics_goes_where_it_points() {
    // Bandwidth is another lane's page: asked of the shell by its frame id.
    let mut rt = runtime("DW-0790");
    click(&mut rt, "item-2-0");
    assert_eq!(ui(&rt).intents, vec![Intent::Show("DW-0785")]);
    let mut rt = runtime("DW-0790");
    assert_eq!(info(&rt, "item-2-1").role, "link");
    assert!(named(&rt, "item-2-1").contains("opens in the browser"));
    click(&mut rt, "item-2-1");
    assert_eq!(ui(&rt).intents, vec![Intent::Web("support")]);
    assert_eq!(ui(&rt).pane, Pane::Diagnostics, "the page stays");
    let mut rt = runtime("DW-0790");
    tab_to(&mut rt, "item-2-2");
    key(&mut rt, Key::Enter);
    assert_eq!(
        ui(&rt).dialog,
        Some(Dialog::ReportProblem),
        "Report a problem (DW-0782)"
    );
    assert_eq!(says(&rt, "dlg-title"), "Report a problem");
}

#[test]
fn export_asks_for_the_bundle_and_the_banner_says_where_it_went() {
    let mut rt = runtime("DW-0790");
    assert_eq!(says(&rt, "btn-0-label"), "Export…");
    tab_to(&mut rt, "btn-0");
    key(&mut rt, Key::Enter);
    assert_eq!(ui(&rt).intents, vec![Intent::SaveBundleAndWriteEmail]);
    assert_eq!(ui(&rt).pane, Pane::Diagnostics);
    let banner = named(&rt, "note-0");
    assert_eq!(info(&rt, "note-0").role, "status");
    assert!(
        banner.starts_with("Support bundle saved. Saved to %LOCALAPPDATA%"),
        "{banner}"
    );
    assert!(banner.contains("beebeeb-diagnostics-1793956320.json"), "{banner}");
    assert!(
        banner.ends_with("A draft to support is opening in your email app; attach the file yourself."),
        "{banner}"
    );
    // The banner's button comes first; Export… is still there, for another one.
    assert_eq!(says(&rt, "btn-0-label"), "Show in File Explorer");
    assert_eq!(says(&rt, "btn-1-label"), "Export…");
    click(&mut rt, "btn-0");
    assert_eq!(
        ui(&rt).intents,
        vec![Intent::SaveBundleAndWriteEmail, Intent::Open("support-bundle")]
    );
}

#[test]
fn the_report_dialog_lands_on_diagnostics_with_the_banner() {
    let mut rt = runtime("DW-0782");
    click(&mut rt, "btn-0");
    assert_eq!(ui(&rt).pane, Pane::Diagnostics, "DW-0790b");
    assert_eq!(ui(&rt).dialog, None);
    assert!(named(&rt, "note-0").starts_with("Support bundle saved."));
}

#[test]
fn the_banner_says_so_when_no_email_app_opened_and_when_nothing_was_saved() {
    let with = |bundle: Bundle| {
        let mut state = init();
        state.settings.drive = Box::new(BoardDrive::new().with_bundle(bundle));
        super::go(&mut state, "DW-0790");
        window(state)
    };
    let rt = with(Bundle::Saved {
        folder: "%LOCALAPPDATA%\\io.beebeeb.app\\diagnostics\\".into(),
        file: "beebeeb-diagnostics-1.json".into(),
        draft_opened: false,
    });
    assert!(
        named(&rt, "note-0").ends_with("Your email app didn't open; send the file to support@beebeeb.io yourself."),
        "{}",
        named(&rt, "note-0")
    );
    assert_eq!(says(&rt, "btn-0-label"), "Show in File Explorer");
    let rt = with(Bundle::Failed("The disk is full.".into()));
    assert_eq!(
        named(&rt, "note-0"),
        "Couldn't save the support bundle. The disk is full."
    );
    assert_eq!(
        says(&rt, "btn-0-label"),
        "Export…",
        "no file, so nothing to show in File Explorer"
    );
    assert!(find(&rt, "btn-1").is_none());
}

// ---- Keep on this PC: the choices (no window) ----

#[test]
fn a_size_reads_as_the_tree_says_it() {
    assert_eq!(size(1400.0), "1.4 GB");
    assert_eq!(size(27.8), "27.8 MB");
    assert_eq!(size(18200.0), "18.2 GB");
}

#[test]
fn checking_a_folder_keeps_everything_in_it_and_the_folder_above_goes_mixed() {
    let mut draft = KeepDraft::of(board_folders(), 0.0);
    assert_eq!(folder(&draft.folders, "lisbon").check(), Check::Off);
    assert!(draft.changes().is_empty(), "nothing changed yet");
    assert_eq!(draft.delta(), (Vec::new(), Vec::new()));
    draft.toggle("lisbon/day-1");
    assert_eq!(
        folder(&draft.folders, "lisbon").check(),
        Check::Mixed,
        "some folders inside kept"
    );
    assert_eq!(folder(&draft.folders, "lisbon/day-1").check(), Check::On);
    let changes = draft.changes();
    assert_eq!(changes.len(), 1);
    assert_eq!(
        (changes[0].0.id.as_str(), &changes[0].1),
        ("lisbon/day-1", &Change::Downloads)
    );
    // The other two as well: now the folder itself is kept, and it is ONE change.
    draft.toggle("lisbon/day-2");
    draft.toggle("lisbon/day-3");
    assert_eq!(folder(&draft.folders, "lisbon").check(), Check::On);
    let changes = draft.changes();
    assert_eq!(
        changes.iter().map(|(f, _)| f.id.as_str()).collect::<Vec<_>>(),
        vec!["lisbon"]
    );
    // The source hears of that one folder, and of no folder that was kept all along.
    assert_eq!(draft.delta(), (vec!["lisbon".to_string()], Vec::new()));
    // Checking twice is no change.
    draft.toggle("lisbon");
    assert!(draft.changes().is_empty());
}

#[test]
fn unchecking_one_folder_inside_a_kept_folder_keeps_the_others() {
    let mut draft = KeepDraft::of(board_folders(), 0.0);
    draft.toggle("documents/taxes");
    assert_eq!(folder(&draft.folders, "documents").check(), Check::Mixed);
    assert_eq!(folder(&draft.folders, "documents/letters").check(), Check::On);
    let changes = draft.changes();
    assert_eq!(changes.len(), 1, "only Taxes leaves");
    assert_eq!(
        (changes[0].0.id.as_str(), &changes[0].1),
        ("documents/taxes", &Change::Leaves)
    );
    // Taxes alone is released: Documents itself is not, so what else is in it stays.
    assert_eq!(draft.delta(), (Vec::new(), vec!["documents/taxes".to_string()]));
    // The whole folder: one change, everything in it.
    draft.toggle("documents");
    draft.toggle("documents");
    let changes = draft.changes();
    assert_eq!(
        changes.iter().map(|(f, _)| f.id.as_str()).collect::<Vec<_>>(),
        vec!["documents"]
    );
    assert!((draft.kept_mb() - 27.8).abs() < 0.01, "only Invoices 2026 stays");
}

// ---- Keep on this PC: the dialog (DW-0784, b, c, d) ----

#[test]
fn choose_folders_opens_the_tree_on_the_stored_choices_with_apply_dimmed() {
    let mut rt = runtime("DW-0746");
    tab_to(&mut rt, "btn-3");
    assert_eq!(named(&rt, "btn-3"), "Choose folders…");
    key(&mut rt, Key::Enter);
    assert_eq!(ui(&rt).dialog, Some(Dialog::Keep));
    assert_eq!(ui(&rt).intents, Vec::new(), "nothing is asked of the shell");
    assert_eq!(says(&rt, "dlg-title"), "Keep on this PC");
    assert_eq!(info(&rt, "tree-rows").role, "tree");
    let boxes: Vec<String> = ["backups", "documents", "invoices-2026", "lisbon", "shoot-selects"]
        .iter()
        .map(|id| named(&rt, &format!("tree-{id}-check")))
        .collect();
    assert_eq!(
        boxes,
        vec![
            "Backups, online only",
            "Documents, kept on this PC",
            "Invoices 2026, kept on this PC",
            "Lisbon trip, online only",
            "Shoot selects, online only",
        ]
    );
    assert_eq!(info(&rt, "tree-documents-check").role, "checkbox");
    assert_eq!(says(&rt, "tree-backups-size"), "4 devices · 18.2 GB");
    assert_eq!(
        says(&rt, "dlg-sum"),
        "After Apply: 1.4 GB on this PC · 27.8 GB online only"
    );
    // Nothing changed: Apply does nothing, by pointer and by Enter.
    click(&mut rt, "btn-5");
    key(&mut rt, Key::Enter);
    assert_eq!(ui(&rt).dialog, Some(Dialog::Keep), "Apply is dimmed");
    assert!(folder(&stored(&rt), "documents").kept);
}

#[test]
fn the_disclosure_opens_a_folder_and_a_box_checks_by_keyboard() {
    let mut rt = runtime("DW-0746");
    click(&mut rt, "btn-3");
    assert!(find(&rt, "tree-lisbon/day-1").is_none(), "closed");
    assert!(
        find(&rt, "tree-invoices-2026-disc-icon").is_none(),
        "no folders inside: no disclosure"
    );
    assert_eq!(named(&rt, "tree-lisbon-disc"), "Open Lisbon trip");
    tab_to(&mut rt, "tree-lisbon-disc");
    key(&mut rt, Key::Enter);
    assert_eq!(named(&rt, "tree-lisbon-disc"), "Close Lisbon trip");
    assert_eq!(says(&rt, "tree-lisbon/day-1-label"), "Day 1");
    // A folder inside is drawn one step in.
    let (parent, child) = (info(&rt, "tree-lisbon-ck").rect, info(&rt, "tree-lisbon/day-1-ck").rect);
    assert!((child[0] - parent[0] - 22.0).abs() < 0.01);
    tab_to(&mut rt, "tree-lisbon/day-1-check");
    key(&mut rt, Key::Text(" ".into()));
    assert_eq!(named(&rt, "tree-lisbon/day-1-check"), "Day 1, kept on this PC");
    assert_eq!(named(&rt, "tree-lisbon-check"), "Lisbon trip, mixed");
    assert_eq!(says(&rt, "tree-lisbon-mixed"), "Some folders kept");
    assert_eq!(says(&rt, "tree-lisbon/day-1-note-text"), "Downloads · 1.3 GB");
    assert_eq!(
        says(&rt, "dlg-sum"),
        "After Apply: 2.7 GB on this PC · 26.5 GB online only"
    );
    // Nothing reached the source yet.
    assert!(
        !folder(&stored(&rt), "lisbon/day-1").kept,
        "nothing changes until Apply"
    );
    // Only checking: Apply applies without a question.
    click(&mut rt, "btn-5");
    assert_eq!(ui(&rt).dialog, None, "back on Sync and File Explorer (DW-0746)");
    assert_eq!(ui(&rt).pane, Pane::Sync);
    assert!(folder(&stored(&rt), "lisbon/day-1").kept);
    assert!(folder(&stored(&rt), "documents").kept, "the rest is as it was");
}

#[test]
fn the_board_scene_is_two_choices_in_and_says_what_each_does() {
    let rt = runtime("DW-0784");
    assert_eq!(says(&rt, "tree-documents-note-text"), "Leaves this PC · 1.4 GB");
    assert_eq!(says(&rt, "tree-lisbon/day-1-note-text"), "Downloads · 1.3 GB");
    assert_eq!(named(&rt, "tree-documents-check"), "Documents, online only");
    assert_eq!(
        says(&rt, "dlg-sum"),
        "After Apply: 1.3 GB on this PC · 27.9 GB online only"
    );
    // The tree is the board's: a head and eight rows of 32 pt in a 552 pt card.
    let tree = info(&rt, "tree").rect;
    assert_eq!((tree[2], tree[3]), (552.0, 282.0));
}

#[test]
fn unchecking_asks_first_and_cancel_returns_with_the_choices_kept() {
    let mut rt = runtime("DW-0784");
    click(&mut rt, "btn-5");
    assert_eq!(ui(&rt).dialog, Some(Dialog::KeepConfirm), "DW-0784b");
    assert_eq!(info(&rt, "dlg").role, "alertdialog");
    assert_eq!(says(&rt, "dlg-title"), "Stop keeping Documents on this PC?");
    assert_eq!(
        says(&rt, "dlg-sub"),
        "Its 48 items (1.4 GB) become online only. They stay in Beebeeb and download again when you open them. Changes that haven't uploaded yet stay on this PC until they have. Day 1 (1.3 GB) downloads now."
    );
    assert!(folder(&stored(&rt), "documents").kept, "asked, not done");
    click(&mut rt, "btn-6");
    assert_eq!(ui(&rt).dialog, Some(Dialog::Keep), "back to the dialog (DW-0784)");
    assert_eq!(
        says(&rt, "tree-documents-note-text"),
        "Leaves this PC · 1.4 GB",
        "the choices are kept"
    );
    // Esc in the question is Cancel too.
    click(&mut rt, "btn-5");
    key(&mut rt, Key::Escape);
    assert_eq!(ui(&rt).dialog, Some(Dialog::Keep));
    // Apply, then Apply: both changes reach the source, and the dialog closes.
    click(&mut rt, "btn-5");
    click(&mut rt, "btn-5");
    assert_eq!(ui(&rt).dialog, None);
    assert_eq!(ui(&rt).pane, Pane::Sync, "DW-0746");
    let now = stored(&rt);
    assert!(!folder(&now, "documents").kept);
    assert!(!folder(&now, "documents/taxes").kept, "and everything in it");
    assert!(folder(&now, "lisbon/day-1").kept);
    assert!(folder(&now, "invoices-2026").kept, "untouched");
    // Opened again, the tree shows the new state with nothing pending.
    click(&mut rt, "btn-3");
    assert!(find(&rt, "tree-documents-note-text").is_none());
    assert_eq!(named(&rt, "tree-documents-check"), "Documents, online only");
}

#[test]
fn cancel_and_esc_change_nothing_and_the_next_visit_starts_fresh() {
    for leave in ["cancel", "esc"] {
        let mut rt = runtime("DW-0784");
        if leave == "cancel" {
            click(&mut rt, "btn-6");
        } else {
            key(&mut rt, Key::Escape);
        }
        assert_eq!(ui(&rt).dialog, None, "{leave}");
        assert_eq!(ui(&rt).pane, Pane::Sync, "{leave}: DW-0746");
        let now = stored(&rt);
        assert!(folder(&now, "documents").kept, "{leave}");
        assert!(!folder(&now, "lisbon/day-1").kept, "{leave}");
        click(&mut rt, "btn-3");
        assert!(
            find(&rt, "tree-documents-note-text").is_none(),
            "{leave}: unapplied choices are gone"
        );
    }
}

#[test]
fn a_list_that_could_not_load_says_so_and_try_again_loads_it() {
    let mut rt = runtime("DW-0784c");
    assert_eq!(ui(&rt).dialog, Some(Dialog::Keep));
    assert_eq!(says(&rt, "tree-state-head"), "Couldn't load your folders.");
    assert_eq!(
        says(&rt, "tree-state-sub"),
        "Nothing was changed. The folders you keep stay on this PC."
    );
    assert!(find(&rt, "tree-rows").is_none());
    assert_eq!(says(&rt, "btn-5-label"), "Try again");
    // Apply (the second button here) is dimmed: it does nothing.
    click(&mut rt, "btn-6");
    assert_eq!(ui(&rt).dialog, Some(Dialog::Keep));
    tab_to(&mut rt, "btn-5");
    key(&mut rt, Key::Enter);
    assert_eq!(ui(&rt).dialog, Some(Dialog::Keep), "DW-0784");
    assert_eq!(named(&rt, "tree-documents-check"), "Documents, kept on this PC");
    assert!(find(&rt, "tree-state-head").is_none());
}

#[test]
fn no_folders_yet_says_where_single_files_are_kept_and_cancel_closes() {
    let mut rt = runtime("DW-0784d");
    assert_eq!(says(&rt, "tree-state-head"), "No folders yet");
    assert!(says(&rt, "tree-state-sub").ends_with("Files you keep one by one are in Available offline."));
    assert!(find(&rt, "btn-7").is_none(), "Apply and Cancel only");
    click(&mut rt, "btn-5");
    assert_eq!(ui(&rt).dialog, Some(Dialog::Keep), "Apply is dimmed");
    click(&mut rt, "btn-6");
    assert_eq!(ui(&rt).dialog, None);
    assert_eq!(ui(&rt).pane, Pane::Sync, "DW-0746");
}

#[test]
fn a_source_that_does_not_know_these_pages_shows_no_sample_values() {
    // The trait's default bodies: what an engine-backed source that has not bound these
    // pages yet answers.
    struct Bare(BoardDrive);
    impl DriveSettings for Bare {
        fn storage(&self) -> super::drive::Storage {
            self.0.storage()
        }
        fn set_limit(&mut self, _: super::drive::Kept, _: u32) {}
        fn clear(&mut self, _: super::drive::Kept) {}
        fn sync(&self) -> super::drive::SyncPane {
            self.0.sync()
        }
        fn set_status_on_files(&mut self, _: bool) {}
        fn set_open_at_login(&mut self, _: bool) {}
        fn set_paused(&mut self, _: bool) {}
        fn sync_anyway(&mut self) {}
        fn repair(&mut self) {}
        fn turn_off_explorer(&mut self) {}
        fn insights(&self) -> &super::drive::Insights {
            self.0.insights()
        }
        fn reload_insights(&mut self) {}
    }
    let mut state = init();
    state.settings.drive = Box::new(Bare(BoardDrive::new()));
    super::go(&mut state, "DW-0790b");
    let rt = window(state);
    assert!(find(&rt, "item-0-0").is_none(), "no App activity without a measurement");
    assert!(find(&rt, "note-0").is_none(), "no banner without a saved file");
    assert_eq!(says(&rt, "btn-0-label"), "Export…");
    let mut state = init();
    state.settings.drive = Box::new(Bare(BoardDrive::new()));
    super::go(&mut state, "DW-0784");
    let rt = window(state);
    assert_eq!(says(&rt, "tree-state-head"), "Couldn't load your folders.");
    assert!(find(&rt, "tree-rows").is_none());
}

// ---- What Apply tells the source: the person's changes, never the whole kept set ----

/// What a source was told, in order.
#[derive(Default)]
struct Told {
    /// Each `apply_keep`: the folders newly kept, the folders newly not kept.
    keeps: Vec<(Vec<String>, Vec<String>)>,
    exports: usize,
}

/// A source that records what it is told, and whose folder list a test can change while the
/// dialog is open (as another device does).
struct Recording {
    board: BoardDrive,
    folders: std::rc::Rc<std::cell::RefCell<Vec<Folder>>>,
    told: std::rc::Rc<std::cell::RefCell<Told>>,
}

impl DriveSettings for Recording {
    fn storage(&self) -> super::drive::Storage {
        self.board.storage()
    }
    fn set_limit(&mut self, _: super::drive::Kept, _: u32) {}
    fn clear(&mut self, _: super::drive::Kept) {}
    fn sync(&self) -> super::drive::SyncPane {
        self.board.sync()
    }
    fn set_status_on_files(&mut self, _: bool) {}
    fn set_open_at_login(&mut self, _: bool) {}
    fn set_paused(&mut self, _: bool) {}
    fn sync_anyway(&mut self) {}
    fn repair(&mut self) {}
    fn turn_off_explorer(&mut self) {}
    fn insights(&self) -> &super::drive::Insights {
        self.board.insights()
    }
    fn reload_insights(&mut self) {}
    fn export_bundle(&mut self) {
        self.told.borrow_mut().exports += 1;
    }
    fn keep(&self) -> Keep {
        Keep::Ready {
            folders: self.folders.borrow().clone(),
            total_mb: 29_227.8,
        }
    }
    fn apply_keep(&mut self, keep: &[String], release: &[String]) {
        self.told.borrow_mut().keeps.push((keep.to_vec(), release.to_vec()));
    }
}

type Shared<T> = std::rc::Rc<std::cell::RefCell<T>>;

/// Sync and File Explorer on a recording source holding `folders`.
fn recording(folders: Vec<Folder>) -> (Runtime<State>, Shared<Vec<Folder>>, Shared<Told>) {
    let folders = std::rc::Rc::new(std::cell::RefCell::new(folders));
    let told = std::rc::Rc::new(std::cell::RefCell::new(Told::default()));
    let mut state = init();
    state.settings.drive = Box::new(Recording {
        board: BoardDrive::new(),
        folders: folders.clone(),
        told: told.clone(),
    });
    super::go(&mut state, "DW-0746");
    (window(state), folders, told)
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn apply_tells_the_source_only_the_folder_that_was_checked() {
    // Backups is not loaded: whatever is kept inside it is not in the tree. Documents and
    // Invoices 2026 are kept and untouched. None of them is the person's change.
    let (mut rt, _, told) = recording(board_folders());
    click(&mut rt, "btn-3");
    click(&mut rt, "tree-lisbon-disc");
    click(&mut rt, "tree-lisbon/day-1-check");
    click(&mut rt, "btn-5");
    assert_eq!(ui(&rt).dialog, None);
    assert_eq!(
        told.borrow().keeps,
        vec![(ids(&["lisbon/day-1"]), Vec::new())],
        "one call, one folder, nothing about the kept set"
    );
}

#[test]
fn a_folder_kept_elsewhere_while_the_dialog_is_open_is_not_released() {
    let (mut rt, folders, told) = recording(board_folders());
    click(&mut rt, "btn-3");
    click(&mut rt, "tree-lisbon-disc");
    // Another device keeps Shoot selects while this dialog is open.
    folders
        .borrow_mut()
        .iter_mut()
        .find(|f| f.id == "shoot-selects")
        .unwrap()
        .kept = true;
    settle(&mut rt);
    click(&mut rt, "tree-lisbon/day-1-check");
    assert!(
        find(&rt, "tree-shoot-selects-note-text").is_none(),
        "not the person's change: no \"Leaves this PC\" on it"
    );
    click(&mut rt, "btn-5");
    assert_eq!(ui(&rt).dialog, None, "checking alone: no question");
    assert_eq!(told.borrow().keeps, vec![(ids(&["lisbon/day-1"]), Vec::new())]);
}

#[test]
fn unchecking_one_folder_inside_releases_that_folder_and_not_the_one_above() {
    let (mut rt, _, told) = recording(board_folders());
    click(&mut rt, "btn-3");
    click(&mut rt, "tree-documents-disc");
    // By pointer: a box is unchecked.
    click(&mut rt, "tree-documents/taxes-check");
    assert_eq!(named(&rt, "tree-documents-check"), "Documents, mixed");
    assert_eq!(says(&rt, "tree-documents/taxes-note-text"), "Leaves this PC · 600.0 MB");
    assert!(
        find(&rt, "tree-documents-note-text").is_none(),
        "Documents itself does not leave"
    );
    click(&mut rt, "btn-5");
    assert_eq!(ui(&rt).dialog, Some(Dialog::KeepConfirm), "copies leave: asked first");
    assert_eq!(says(&rt, "dlg-title"), "Stop keeping Taxes on this PC?");
    assert!(told.borrow().keeps.is_empty(), "asked, not done");
    click(&mut rt, "btn-5");
    assert_eq!(
        told.borrow().keeps,
        vec![(Vec::new(), ids(&["documents/taxes"]))],
        "Taxes, and not Documents: files directly in Documents stay"
    );
}

#[test]
fn a_source_that_marks_only_the_folder_above_as_kept_draws_its_folders_checked() {
    // The shape a source may well answer in: Lisbon trip kept, its folders not marked.
    let mut tree = board_folders();
    tree.iter_mut().find(|f| f.id == "lisbon").unwrap().kept = true;
    let (mut rt, _, told) = recording(tree);
    click(&mut rt, "btn-3");
    click(&mut rt, "tree-lisbon-disc");
    for day in ["day-1", "day-2", "day-3"] {
        assert!(
            named(&rt, &format!("tree-lisbon/{day}-check")).ends_with("kept on this PC"),
            "{day} is inside a kept folder"
        );
    }
    click(&mut rt, "tree-lisbon/day-2-check");
    assert_eq!(named(&rt, "tree-lisbon/day-2-check"), "Day 2, online only");
    assert_eq!(named(&rt, "tree-lisbon/day-1-check"), "Day 1, kept on this PC");
    assert_eq!(says(&rt, "tree-lisbon/day-2-note-text"), "Leaves this PC · 1.6 GB");
    assert!(
        find(&rt, "tree-lisbon/day-1-note-text").is_none(),
        "Day 1 stays, and says nothing"
    );
    click(&mut rt, "btn-5");
    assert_eq!(ui(&rt).dialog, Some(Dialog::KeepConfirm));
    click(&mut rt, "btn-5");
    assert_eq!(told.borrow().keeps, vec![(Vec::new(), ids(&["lisbon/day-2"]))]);
}

#[test]
fn the_question_by_its_frame_id_without_a_choice_asks_nothing_and_tells_nothing() {
    let (mut rt, _, told) = recording(board_folders());
    rt.update(|s, _| super::go(s, "DW-0784b"));
    settle(&mut rt);
    assert_eq!(ui(&rt).dialog, Some(Dialog::Keep), "nothing to ask: the tree opens");
    assert_eq!(says(&rt, "dlg-title"), "Keep on this PC");
    click(&mut rt, "btn-5");
    key(&mut rt, Key::Enter);
    assert!(told.borrow().keeps.is_empty(), "no choice, no call");
}

#[test]
fn a_list_that_arrives_after_the_dialog_opened_is_adopted_at_the_first_press() {
    let (mut rt, folders, told) = recording(Vec::new());
    click(&mut rt, "btn-3");
    assert_eq!(says(&rt, "tree-state-head"), "No folders yet");
    *folders.borrow_mut() = board_folders();
    settle(&mut rt);
    assert_eq!(named(&rt, "tree-documents-check"), "Documents, kept on this PC");
    click(&mut rt, "tree-shoot-selects-check");
    assert_eq!(says(&rt, "tree-shoot-selects-note-text"), "Downloads · 2.4 GB");
    click(&mut rt, "btn-5");
    assert_eq!(told.borrow().keeps, vec![(ids(&["shoot-selects"]), Vec::new())]);
}

#[test]
fn choose_folders_shows_the_state_the_source_is_in() {
    // DW-0784c and DW-0784d are reached by the same button, on a source in that state.
    let mut state = init();
    state.settings.drive = Box::new(BoardDrive::new().with_keep(Keep::Failed));
    super::go(&mut state, "DW-0746");
    let mut rt = window(state);
    tab_to(&mut rt, "btn-3");
    key(&mut rt, Key::Enter);
    assert_eq!(says(&rt, "tree-state-head"), "Couldn't load your folders.");
    let (mut rt, _, _) = recording(Vec::new());
    click(&mut rt, "btn-3");
    assert_eq!(says(&rt, "tree-state-head"), "No folders yet");
}

// ---- Showing Diagnostics never exports; a press does, once ----

#[test]
fn showing_the_saved_page_exports_nothing_and_a_press_exports_once() {
    let (mut rt, _, told) = recording(board_folders());
    rt.update(|s, _| super::go(s, "DW-0790b"));
    settle(&mut rt);
    assert_eq!(ui(&rt).pane, Pane::Diagnostics);
    assert_eq!(told.borrow().exports, 0, "showing a page writes no file");
    rt.update(|s, _| super::go(s, "DW-0790"));
    settle(&mut rt);
    assert_eq!(told.borrow().exports, 0);
    click(&mut rt, "btn-0");
    assert_eq!(told.borrow().exports, 1, "Export…: once");
    assert_eq!(ui(&rt).intents, vec![Intent::SaveBundleAndWriteEmail]);
    // Report a problem's "Save and write email": once too.
    let (mut rt, _, told) = recording(board_folders());
    rt.update(|s, _| super::go(s, "DW-0782"));
    settle(&mut rt);
    click(&mut rt, "btn-0");
    assert_eq!(ui(&rt).pane, Pane::Diagnostics);
    assert_eq!(told.borrow().exports, 1);
}
