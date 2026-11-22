//! The folder the Files window shows is read again after every finished check, and merged into the view
//! (`frames::files::reread`, `host::read_again_after_a_check`): what arrived, left or was renamed shows without a
//! click; the selection, the focused row and the rows in view stay with their items; a name being typed in a row
//! holds the read until it closes.

use super::super::super::tray::EngineState;
use super::*;
use crate::source::{Crumb, DriveItem, DriveSource, Listing, SourceError, StorageSummary};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

/// The root of an account whose files the test changes "elsewhere" (another device, this PC's folder): each read
/// answers what it holds now, and counts.
#[derive(Clone, Default)]
struct Elsewhere {
    items: Arc<Mutex<Vec<DriveItem>>>,
    reads: Arc<AtomicU32>,
}

impl Elsewhere {
    fn with(names: &[&str]) -> Elsewhere {
        let shelf = Elsewhere::default();
        shelf.add(names);
        shelf
    }
    fn add(&self, names: &[&str]) {
        self.items.lock().unwrap().extend(names.iter().map(|name| file(name)));
    }
    fn remove(&self, name: &str) {
        self.items.lock().unwrap().retain(|item| item.id != name);
    }
    fn reads(&self) -> u32 {
        self.reads.load(Ordering::SeqCst)
    }
}

impl DriveSource for Elsewhere {
    fn list(&self, folder_id: Option<&str>) -> Result<Listing, SourceError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if folder_id.is_some() {
            return Err(SourceError::NotFound);
        }
        let root = Crumb {
            id: None,
            name: "Files".into(),
        };
        Ok(Listing {
            folder: root.clone(),
            path: vec![root],
            items: self.items.lock().unwrap().clone(),
        })
    }
    fn storage(&self) -> Option<StorageSummary> {
        None
    }
}

/// Seven files that land in the folder, each sorting above every `m-` file already there.
const SEVEN: [&str; 7] = [
    "a-1.txt", "a-2.txt", "a-3.txt", "a-4.txt", "a-5.txt", "a-6.txt", "a-7.txt",
];

/// A file whose id is its name.
fn file(name: &str) -> DriveItem {
    DriveItem {
        id: name.to_string(),
        parent_id: None,
        name: name.to_string(),
        is_folder: false,
        size: Some(1),
        child_count: None,
        modified_at: "2026-11-16T06:00:00Z".into(),
        mime: None,
        has_thumbnail: false,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: None,
        is_protected: false,
    }
}

/// Signed in, the root read by the opening gate, the engine running: the window over `shelf`.
fn window(shelf: &Elsewhere) -> (Arc<FixtureEngine>, Runtime<State>) {
    let engine = Arc::new(FixtureEngine::with(GateState::SignedOut, TrayInputs::default()));
    let shell = Shell::fixture_on(engine.clone(), Arc::new(FakePlatform::default()));
    let mut rt = runtime(state::init_from(shell, Box::new(shelf.clone()), None));
    engine.set_inputs(inputs(EngineState::NotRunning));
    engine.set_gate(GateState::Ready);
    settle(&mut rt);
    assert_eq!(rt.state.list_error, None);
    (engine, rt)
}

fn inputs(engine: EngineState) -> TrayInputs {
    TrayInputs {
        signed_in: true,
        engine,
        ..TrayInputs::default()
    }
}

/// The engine checks and finishes the check (the b's state turns Idle).
fn a_check_finishes(engine: &FixtureEngine, rt: &mut Runtime<State>) {
    engine.set_inputs(inputs(EngineState::Checking));
    settle(rt);
    engine.set_inputs(inputs(EngineState::Idle));
    settle(rt);
}

fn ids(s: &State) -> Vec<String> {
    s.listing.items.iter().map(|item| item.id.clone()).collect()
}

fn id_at(s: &State, row: Option<usize>) -> Option<String> {
    row.and_then(|row| s.listing.items.get(row)).map(|item| item.id.clone())
}

fn selection_ids(s: &State) -> Vec<String> {
    s.actions
        .selection
        .iter()
        .filter_map(|&row| s.listing.items.get(row))
        .map(|item| item.id.clone())
        .collect()
}

/// The fake-listing finding: a root with 2 rows, 7 files land (this PC's folder or another device), a check
/// finishes: 9 rows, with no click. The row that was selected and focused still is, though 7 rows landed above it.
#[test]
fn a_finished_check_shows_new_files_and_keeps_the_selection_and_the_focus() {
    let shelf = Elsewhere::with(&["m-1.txt", "m-2.txt"]);
    let (engine, mut rt) = window(&shelf);
    rt.update(|s, _| s.select_only(1));
    assert_eq!(id_at(&rt.state, rt.state.selected).as_deref(), Some("m-2.txt"));

    shelf.add(&SEVEN);
    a_check_finishes(&engine, &mut rt);

    assert_eq!(
        rt.state.listing.items.len(),
        9,
        "the 7 new files show: {:?}",
        ids(&rt.state)
    );
    assert_eq!(
        id_at(&rt.state, rt.state.selected).as_deref(),
        Some("m-2.txt"),
        "the focused row is the same file"
    );
    assert_eq!(selection_ids(&rt.state), ["m-2.txt"], "the selection is the same file");
    assert_eq!(id_at(&rt.state, rt.state.actions.anchor).as_deref(), Some("m-2.txt"));
    assert_eq!(
        ids(&rt.state),
        [
            "a-1.txt", "a-2.txt", "a-3.txt", "a-4.txt", "a-5.txt", "a-6.txt", "a-7.txt", "m-1.txt", "m-2.txt"
        ],
        "in the folder's sort"
    );
}

/// An item removed elsewhere leaves the rows and the selection; the rest of the selection stays.
#[test]
fn a_finished_check_drops_an_item_removed_elsewhere_from_the_rows_and_the_selection() {
    let shelf = Elsewhere::with(&["m-1.txt", "m-2.txt", "m-3.txt"]);
    let (engine, mut rt) = window(&shelf);
    rt.update(|s, _| {
        s.select_only(0);
        s.toggle_row(2);
    });
    assert_eq!(selection_ids(&rt.state), ["m-1.txt", "m-3.txt"]);
    assert_eq!(id_at(&rt.state, rt.state.selected).as_deref(), Some("m-3.txt"));

    shelf.remove("m-3.txt");
    a_check_finishes(&engine, &mut rt);

    assert_eq!(ids(&rt.state), ["m-1.txt", "m-2.txt"], "the removed file left the rows");
    assert_eq!(selection_ids(&rt.state), ["m-1.txt"], "and the selection");
    assert_eq!(rt.state.selected, None, "the focused row went with its file");
    assert!(
        rt.state.actions.selection.iter().all(|&row| row < 2),
        "no row past the end is selected"
    );
}

/// A name being typed in a row when the check finishes: the read waits (the row under the field stays where it
/// is), and runs on the first poll after the field closed.
#[test]
fn a_finished_check_during_a_rename_waits_until_the_rename_closes() {
    let shelf = Elsewhere::with(&["m-1.txt", "m-2.txt"]);
    let (engine, mut rt) = window(&shelf);
    rt.update(|s, _| {
        s.select_only(1);
        s.actions.rename = Some(crate::frames::files::rename::Rename {
            row: 1,
            id: "m-2.txt".into(),
        });
    });
    let before = shelf.reads();

    shelf.add(&SEVEN);
    a_check_finishes(&engine, &mut rt);
    assert_eq!(shelf.reads(), before, "nothing is read while the name is a field");
    assert_eq!(ids(&rt.state), ["m-1.txt", "m-2.txt"]);
    assert!(rt.state.actions.rename.is_some(), "the field stays open");

    rt.update(|s, _| s.actions.rename = None);
    settle(&mut rt);
    assert_eq!(
        rt.state.listing.items.len(),
        9,
        "read once the field closed: {:?}",
        ids(&rt.state)
    );
    assert_eq!(id_at(&rt.state, rt.state.selected).as_deref(), Some("m-2.txt"));
}

/// Files another device added come with a check that moves no file here: the engine reports Idle before and after it,
/// with a new time of its last finished check. That is a finished check too; the same report again is not.
#[test]
fn a_finished_check_that_stays_idle_still_shows_what_another_device_added() {
    let shelf = Elsewhere::with(&["m-1.txt", "m-2.txt"]);
    let (engine, mut rt) = window(&shelf);
    let checked = |at: i64| TrayInputs {
        last_check: Some(at),
        ..inputs(EngineState::Idle)
    };
    engine.set_inputs(checked(1_000));
    settle(&mut rt);
    assert_eq!(rt.state.listing.items.len(), 2);

    shelf.add(&SEVEN);
    engine.set_inputs(checked(1_030));
    settle(&mut rt);
    assert_eq!(
        rt.state.listing.items.len(),
        9,
        "the files of the check that stayed Idle show: {:?}",
        ids(&rt.state)
    );

    let read = shelf.reads();
    settle(&mut rt);
    engine.set_inputs(checked(1_030));
    settle(&mut rt);
    assert_eq!(
        shelf.reads(),
        read,
        "the same report again is no new check: nothing is read"
    );
}

/// Scrolled down a long folder, files land that sort above the rows in view: the rows in view stay where they
/// were on screen (anchored to the first row in view, by its item), not pushed down by the new rows.
#[test]
fn a_finished_check_keeps_the_rows_in_view_where_they_were_when_rows_land_above() {
    let names: Vec<String> = (0..60).map(|i| format!("m-{i:02}.txt")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let shelf = Elsewhere::with(&names);
    let (engine, mut rt) = window(&shelf);
    let (x, y) = rt.find("rows").expect("the list shows");
    rt.wheel(x, y, -600.0);
    settle(&mut rt);
    let (row, _) = rt.presented("row-25").expect("row 25 is in view");
    assert!(row[1] > 0.0 && row[1] < 800.0, "row 25 is on screen: {row:?}");
    assert_eq!(rt.state.listing.items[25].id, "m-25.txt");

    shelf.add(&SEVEN);
    a_check_finishes(&engine, &mut rt);

    assert_eq!(rt.state.listing.items.len(), 67);
    assert_eq!(rt.state.listing.items[32].id, "m-25.txt", "7 rows landed above it");
    let (now, _) = rt.presented("row-32").expect("its row is in view");
    assert!(
        (now[1] - row[1]).abs() < 0.5,
        "m-25.txt is where it was on screen: {} then {}",
        row[1],
        now[1]
    );
}

/// Every finished check reaches Shared, This PC and Trash (and Photos' own rule): each is asked to read again, and an
/// open menu holds it. Their sources here are the board's, which read nothing: the due flag is what shows it.
#[test]
fn a_finished_check_asks_shared_this_pc_and_trash_to_read_again_and_a_menu_holds_it() {
    use crate::frames::shared::parts::menu::Open;
    let shelf = Elsewhere::with(&["m-1.txt"]);
    let (engine, mut rt) = window(&shelf);
    rt.update(|s, _| {
        s.frames.shared.menu = Some(Open::at(1.0, 1.0));
        s.frames.trash.menu = Some(Open::at(1.0, 1.0));
        s.this_pc.menu = Some(crate::frames::this_pc::RowMenu {
            row: 0,
            at: (1.0, 1.0),
            hot: 0,
        });
    });
    a_check_finishes(&engine, &mut rt);
    let due = |rt: &Runtime<State>| {
        (
            rt.state.frames.shared.reread_due,
            rt.state.this_pc.reread_due,
            rt.state.frames.trash.reread_due,
        )
    };
    assert_eq!(due(&rt), (true, true, true), "asked, and held by the open menus");
    rt.update(|s, _| {
        s.frames.shared.menu = None;
        s.frames.trash.menu = None;
        s.this_pc.menu = None;
    });
    settle(&mut rt);
    assert_eq!(
        due(&rt),
        (false, false, false),
        "read on the first poll after they closed"
    );
}

/// w31 (the Windows test machine, 2026-11-20): the first file list lands in the middle of a long tick (the engine is still syncing,
/// no check has finished). The moment the window stops saying it is loading, the folder shown is read again: the
/// files show then, not minutes later at the end of that tick.
#[test]
fn w31_when_the_first_list_lands_the_folder_is_read_again_without_waiting_for_the_check_to_finish() {
    use crate::frames::global::source::{Loading, Step};
    use crate::frames::global::{GlobalFixture, GlobalStatus, Ui};
    let shelf = Elsewhere::with(&["m-1.txt"]);
    let (engine, mut rt) = window(&shelf);
    engine.set_inputs(inputs(EngineState::Syncing));
    rt.update(|s, _| {
        s.global = Ui::new(Box::new(GlobalFixture::of(GlobalStatus {
            loading: Some(Loading {
                listed: 0,
                total: 0,
                left: None,
                file_list: Step::InProgress,
                thumbnails: Step::WaitingForFileList,
            }),
            ..GlobalStatus::default()
        })));
    });
    settle(&mut rt);
    assert_eq!(rt.state.listing.items.len(), 1);

    // The list landed; the tick goes on (still Syncing, no finished check).
    shelf.add(&SEVEN);
    rt.update(|s, _| s.global = Ui::new(Box::new(GlobalFixture::of(GlobalStatus::default()))));
    settle(&mut rt);
    assert_eq!(
        rt.state.listing.items.len(),
        8,
        "the first list's files show as soon as it landed: {:?}",
        ids(&rt.state)
    );
}

/// w36 (the Windows test machine, bff1af87): New file made its file, and the window showed no row for it until the engine's next
/// finished check, minutes later on a busy engine. The source lists the file as it makes it, and the window reads
/// the folder shown again in the same poll: the row stands there with no check. Mutant: drop `reread_files()` from
/// `State::create_file` -> red (the row is missing).
#[test]
fn a_new_file_made_here_is_listed_at_once_with_no_finished_check() {
    use crate::frames::files::Actions;
    use crate::frames::files::source::FilesFixture;
    use crate::frames::viewers::Intent as ViewerIntent;
    let shelf = Elsewhere::with(&["m-1.txt"]);
    let (_engine, mut rt) = window(&shelf);
    rt.state.actions = Actions::new(Box::new(FilesFixture::default()));
    settle(&mut rt);
    let reads = shelf.reads();

    // Create file: the source makes it and lists it before it answers.
    shelf.add(&["a-new.txt"]);
    rt.state.viewer.asked.push(ViewerIntent::CreateFile {
        folder: None,
        name: "a-new.txt".into(),
    });
    settle(&mut rt);

    assert_eq!(rt.state.live_toast(), None, "nothing failed");
    assert_eq!(
        ids(&rt.state),
        ["a-new.txt", "m-1.txt"],
        "listed with no finished check"
    );
    assert_eq!(shelf.reads(), reads + 1, "the folder is read once for it");
}

/// Task 050: the running app reads on a worker. `shelf` is read only once the test lets it (`let_one_read`): until
/// then the read is out, and the window goes on.
fn read_on_a_worker(rt: &mut Runtime<State>, shelf: &Elsewhere) -> std::sync::mpsc::Sender<()> {
    let (let_one_read, wait) = std::sync::mpsc::channel::<()>();
    let (wait, shelf) = (Mutex::new(wait), shelf.clone());
    rt.state.folder_reads.reader = Some(Arc::new(move |folder: Option<&str>| {
        let _ = wait.lock().unwrap().recv();
        state::load(&shelf, folder)
    }));
    let_one_read
}

/// Polls until the read out answered and was taken (a worker answers in its own time).
fn until_taken(rt: &mut Runtime<State>) {
    for _ in 0..200 {
        if rt.state.folder_reads.pending.is_none() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        settle(rt);
    }
    panic!("the read never answered");
}

/// Task 050: the read after a check runs on the worker. While it is out the rows shown stay (no flash, no wait); its
/// answer merges by id and the selection stays with its file.
#[test]
fn a_finished_check_is_read_on_a_worker_and_the_rows_stay_until_it_answers() {
    let shelf = Elsewhere::with(&["m-1.txt", "m-2.txt"]);
    let (engine, mut rt) = window(&shelf);
    let let_one_read = read_on_a_worker(&mut rt, &shelf);
    rt.update(|s, _| s.select_only(1));
    shelf.add(&SEVEN);
    a_check_finishes(&engine, &mut rt);

    assert!(rt.state.folder_reads.pending.is_some(), "the read is out");
    assert_eq!(
        ids(&rt.state),
        ["m-1.txt", "m-2.txt"],
        "the rows shown stay while it is out"
    );

    let_one_read.send(()).unwrap();
    until_taken(&mut rt);
    assert_eq!(rt.state.listing.items.len(), 9, "merged: {:?}", ids(&rt.state));
    assert_eq!(selection_ids(&rt.state), ["m-2.txt"], "the selection is the same file");
}

/// Task 050: an answer for a folder the window no longer shows is dropped; it never replaces the rows shown.
#[test]
fn a_read_for_a_folder_no_longer_shown_is_dropped() {
    let shelf = Elsewhere::with(&["m-1.txt"]);
    let (engine, mut rt) = window(&shelf);
    let let_one_read = read_on_a_worker(&mut rt, &shelf);
    shelf.add(&SEVEN);
    a_check_finishes(&engine, &mut rt);
    assert!(rt.state.folder_reads.pending.is_some(), "the read is out");
    // The window shows another folder now (as an open made at once leaves it).
    rt.state.listing.folder.id = Some("elsewhere".into());

    let_one_read.send(()).unwrap();
    until_taken(&mut rt);
    assert_eq!(
        ids(&rt.state),
        ["m-1.txt"],
        "the root's answer is not put in another folder"
    );
}

/// Task 050: an answer that comes after the session moved (a lock, a sign-out) is dropped.
#[test]
fn a_read_that_answers_after_the_gate_closed_is_dropped() {
    let shelf = Elsewhere::with(&["m-1.txt"]);
    let (engine, mut rt) = window(&shelf);
    let let_one_read = read_on_a_worker(&mut rt, &shelf);
    shelf.add(&SEVEN);
    a_check_finishes(&engine, &mut rt);
    assert!(rt.state.folder_reads.pending.is_some(), "the read is out");
    engine.set_gate(GateState::Locked);
    settle(&mut rt);
    assert!(rt.state.folder_reads.pending.is_none(), "the gate change forgot it");

    let _ = let_one_read.send(());
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(5));
        settle(&mut rt);
    }
    assert!(
        ids(&rt.state).is_empty(),
        "nothing of the read shows behind a closed gate: {:?}",
        ids(&rt.state)
    );
}
