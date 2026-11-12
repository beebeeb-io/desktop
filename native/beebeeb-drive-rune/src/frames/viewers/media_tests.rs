//! Tests of the photo, audio, PDF, table and archive viewers: every frame's words, the router by
//! file type, what each control does on the board's fixture, and that a source with no pictures
//! is never answered for by a sample.

use std::cell::RefCell;
use std::rc::Rc;

use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::kinds::{Viewer, viewer_of};
use super::source::{Body, FixtureViewerSource, Match, OpenError, OpenFile, Picture, ViewerSource, board};
use super::{Intent, open_from, open_item, set_scene};
use crate::shell::app::Screen;
use crate::source::DriveItem;
use crate::{State, init, view};

type Calls = Rc<RefCell<Vec<String>>>;

/// The fixture, with what the viewer asked of it kept where a test can read it.
struct Recording {
    inner: FixtureViewerSource,
    calls: Calls,
}

impl ViewerSource for Recording {
    fn open(&mut self, id: &str) -> Result<OpenFile, OpenError> {
        self.inner.open(id)
    }
    fn poll(&mut self, id: &str) -> Option<Body> {
        self.inner.poll(id)
    }
    fn cancel(&mut self, _: &str) {}
    fn retry(&mut self, id: &str) -> Result<OpenFile, OpenError> {
        self.inner.retry(id)
    }
    fn set_offline(&mut self, id: &str, on: bool) -> bool {
        self.calls.borrow_mut().push(format!("offline {id} {on}"));
        true
    }
    fn picture(&self, id: &str, which: Picture) -> Option<rune::Pixels> {
        self.inner.picture(id, which)
    }
    fn find(&mut self, id: &str, words: &str) -> Vec<Match> {
        self.calls.borrow_mut().push(format!("find {id} {words}"));
        self.inner.find(id, words)
    }
    fn play(&mut self, id: &str, on: bool) -> bool {
        self.calls.borrow_mut().push(format!("play {id} {on}"));
        true
    }
}

/// A source that has the board's files and none of the trait's optional answers: what an
/// engine-backed source is before it renders pictures.
struct Bare(FixtureViewerSource);

impl ViewerSource for Bare {
    fn open(&mut self, id: &str) -> Result<OpenFile, OpenError> {
        self.0.open(id)
    }
    fn poll(&mut self, _: &str) -> Option<Body> {
        None
    }
    fn cancel(&mut self, _: &str) {}
    fn retry(&mut self, id: &str) -> Result<OpenFile, OpenError> {
        self.0.retry(id)
    }
}

fn runtime_of(state: State, dark: bool) -> Runtime<State> {
    let env = Env {
        system_dark: dark,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.poll = Some(crate::shell::host::poll);
    rt.frame(0.0);
    rt
}

fn scene(frame: &str) -> Runtime<State> {
    let mut state = init();
    set_scene(&mut state, frame);
    runtime_of(state, false)
}

fn recording(file: &str) -> (Runtime<State>, Calls) {
    let calls = Calls::default();
    let mut state = init();
    let source = Recording {
        inner: FixtureViewerSource::new(),
        calls: calls.clone(),
    };
    assert!(open_from(&mut state, Box::new(source), file));
    (runtime_of(state, false), calls)
}

fn info(rt: &Runtime<State>, key: &str) -> rune::ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
}

fn has(rt: &Runtime<State>, key: &str) -> bool {
    rt.elements()
        .iter()
        .any(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn words(rt: &Runtime<State>, key: &str) -> String {
    info(rt, key).text.unwrap_or_default()
}

fn press_with(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    rt.frame(0.0);
    rt.frame(0.0);
    rt.settle_all();
}

fn press(rt: &mut Runtime<State>, key: Key) {
    press_with(rt, key, Mods::default());
}

fn ctrl(rt: &mut Runtime<State>, key: Key) {
    press_with(
        rt,
        key,
        Mods {
            cmd: true,
            ..Mods::default()
        },
    );
}

fn click(rt: &mut Runtime<State>, key: &str) {
    rt.settle_all();
    rt.frame(0.0);
    let r = info(rt, key).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
    rt.frame(0.0);
    rt.settle_all();
}

fn viewer(rt: &Runtime<State>) -> &super::Ui {
    rt.state.viewer.ui.as_ref().expect("a file is open")
}

fn intents(rt: &mut Runtime<State>) -> Vec<Intent> {
    rt.state.viewer.ui.as_mut().expect("a file is open").take_intents()
}

fn on_a_file(rt: &Runtime<State>) -> bool {
    matches!(rt.state.shell.app.screen(), Screen::Drive(route) if route.item.is_some())
}

fn item(id: &str, name: &str) -> DriveItem {
    DriveItem {
        id: id.into(),
        parent_id: None,
        name: name.into(),
        is_folder: false,
        size: None,
        child_count: None,
        modified_at: "2026-11-04T16:40:00Z".into(),
        mime: None,
        has_thumbnail: false,
        shared_type: None,
        shared_badges: vec![],
        pc_status: None,
        is_protected: false,
    }
}

/// Every frame of this file, a key it draws and the words there.
const BUILT: [(&str, &str, &str); 7] = [
    ("DW-1501", "tb-pill-label", "Camera preview \u{00B7} Sony ARW"),
    ("DW-1502", "detail-camera-value", "Sony ILCE-7RM5"),
    ("DW-1503", "audio-title", "Harbour sessions, take 3"),
    ("DW-1505", "item-count", "Page 2 of 6 \u{00B7} 2.3 MB"),
    ("DW-1506", "find-count", "2 of 3"),
    ("DW-1514", "csv-2-5", "440.00"),
    (
        "DW-1515",
        "banner-text",
        "214 files \u{00B7} 3.8 GB unpacked \u{00B7} files inside can't be opened here.",
    ),
];

#[test]
fn every_frame_draws_its_words_in_both_themes() {
    for (frame, key, text) in BUILT {
        for dark in [false, true] {
            let mut state = init();
            set_scene(&mut state, frame);
            let rt = runtime_of(state, dark);
            assert_eq!(words(&rt, key), text, "{frame} dark={dark}");
            assert!(has(&rt, "status-text"), "{frame}: the status line");
        }
    }
}

#[test]
fn the_router_gives_each_type_its_viewer() {
    for (name, want) in [
        ("DSC04182.ARW", Some(Viewer::Photo)),
        ("Harbour.heic", Some(Viewer::Photo)),
        ("take 3.m4a", Some(Viewer::Audio)),
        ("Studio plan.PDF", Some(Viewer::Pdf)),
        ("Rates 2026.csv", Some(Viewer::Table)),
        ("rates.tsv", Some(Viewer::Table)),
        ("export.zip", Some(Viewer::Archive)),
        ("notes.md", Some(Viewer::Markdown)),
        ("server.log", Some(Viewer::Text)),
        ("Q4 plan.key", None),
        ("no extension", None),
    ] {
        assert_eq!(viewer_of(name), want, "{name}");
    }
}

#[test]
fn a_file_of_each_type_opens_its_viewer_through_the_router() {
    for (id, name, key) in [
        (board::RAW, "DSC04182.ARW", "photo"),
        (board::AUDIO, "Harbour sessions, take 3.m4a", "artwork"),
        (board::PDF, "Studio plan.pdf", "page-area"),
        (board::CSV, "Rates 2026.csv", "csv-table"),
        (board::ZIP, "Shoot selects export.zip", "column-headers"),
    ] {
        let mut state = init();
        assert!(open_item(&mut state, &item(id, name)), "{name}");
        let rt = runtime_of(state, false);
        assert!(on_a_file(&rt), "{name}: the route is on the file");
        assert_eq!(viewer(&rt).file.name, name);
        assert!(has(&rt, key), "{name}: its viewer draws {key}");
    }
    // A type with no viewer, a folder, and a file the source does not have: not this group's.
    let mut state = init();
    assert!(!open_item(&mut state, &item("q4plan", "Q4 plan.key")));
    assert!(!open_item(&mut state, &item("nowhere", "Nowhere.pdf")));
    let folder = DriveItem {
        is_folder: true,
        ..item(board::PDF, "Studio plan.pdf")
    };
    assert!(!open_item(&mut state, &folder));
    assert!(state.viewer.ui.is_none());
}

#[test]
fn enter_on_a_files_row_opens_the_viewer_and_escape_returns_to_the_list() {
    let mut rt = runtime_of(init(), false);
    let row = rt
        .state
        .listing
        .items
        .iter()
        .position(|i| i.id == "contract")
        .expect("the board's Files list has Contract draft.pdf");
    rt.state.select_only(row);
    press(&mut rt, Key::Enter);
    assert!(on_a_file(&rt));
    assert_eq!(viewer(&rt).file.name, "Contract draft.pdf");
    assert!(has(&rt, "thumb-sidebar"));
    press(&mut rt, Key::Escape);
    press(&mut rt, Key::Tab);
    assert!(!on_a_file(&rt));
    assert!(rt.state.viewer.ui.is_none(), "what was decrypted is dropped");
    assert!(has(&rt, "row-0"), "the list is back");
}

#[test]
fn a_window_on_another_drive_opens_none_of_these_from_the_boards_samples() {
    for (id, name) in [(board::RAW, "DSC04182.ARW"), (board::PDF, "Studio plan.pdf")] {
        let drive = Box::new(crate::source::LargeFixtureSource::new(3));
        let mut state = crate::shell::state::init_from(crate::shell::host::Shell::fixture(), drive, None);
        assert!(!open_item(&mut state, &item(id, name)));
        assert!(state.viewer.ui.is_none());
    }
}

#[test]
fn a_source_without_pictures_gets_plain_tiles_and_no_sample() {
    let mut bare = Bare(FixtureViewerSource::new());
    for which in [Picture::Photo, Picture::Strip(0), Picture::Artwork, Picture::Page(1)] {
        assert!(bare.picture(board::RAW, which).is_none(), "{which:?}");
    }
    assert!(bare.find(board::PDF_FIND, "terrace").is_empty());
    assert!(!bare.play(board::AUDIO, false));
    // The frames still draw, on their plain tiles; and a play the source did not do is not shown as done.
    for (id, key) in [
        (board::RAW, "photo"),
        (board::AUDIO, "artwork"),
        (board::PDF_FIND, "page"),
    ] {
        let mut state = init();
        assert!(open_from(&mut state, Box::new(Bare(FixtureViewerSource::new())), id));
        let mut rt = runtime_of(state, false);
        assert!(has(&rt, key), "{id}");
        if id == board::AUDIO {
            click(&mut rt, "pause");
            assert_eq!(info(&rt, "pause").label.as_deref(), Some("Pause"));
        }
        if id == board::PDF_FIND {
            ctrl(&mut rt, Key::Command('f'));
            rt.state
                .viewer
                .ui
                .as_mut()
                .unwrap()
                .media
                .find
                .as_mut()
                .unwrap()
                .edit
                .insert("terrace");
            rt.frame(0.0);
            rt.frame(0.0);
            assert_eq!(words(&rt, "find-count"), "No matches");
            assert!(!has(&rt, "mark-0"));
        }
    }
    // The fixture has them: the board's sample window only.
    let fixture = FixtureViewerSource::new();
    assert!(fixture.picture(board::RAW, Picture::Photo).is_some());
    assert!(fixture.picture(board::PDF, Picture::Page(2)).is_some());
    assert!(
        fixture.picture(board::PDF, Picture::Page(3)).is_none(),
        "a page the board does not draw"
    );
    assert!(fixture.picture("voice-over-draft.m4a", Picture::Artwork).is_none());
}

#[test]
fn a_photo_steps_through_its_folder_and_back_returns_to_the_list() {
    let (mut rt, calls) = recording(board::RAW);
    assert_eq!(
        words(&rt, "status-details"),
        "RAW \u{00B7} the camera's own preview, not a developed RAW"
    );
    assert_eq!(info(&rt, "gallery-4").target[2], 52.0);
    assert_eq!(info(&rt, "gallery-3").target[2], 36.0);
    press(&mut rt, Key::Right);
    assert_eq!(viewer(&rt).file.name, "DSC04183.ARW");
    assert_eq!(info(&rt, "gallery-5").target[2], 52.0, "the strip follows");
    press(&mut rt, Key::Left);
    press(&mut rt, Key::Left);
    assert_eq!(viewer(&rt).file.name, "DSC04181.ARW");
    click(&mut rt, "gallery-0");
    assert_eq!(viewer(&rt).file.name, "DSC04178.ARW");
    press(&mut rt, Key::Left);
    assert_eq!(
        viewer(&rt).file.name,
        "DSC04178.ARW",
        "the first of the folder: no step"
    );
    // Back is the list, not the photo before.
    click(&mut rt, "back");
    assert!(!on_a_file(&rt));
    assert!(rt.state.viewer.ui.is_none());
    assert!(calls.borrow().is_empty());
}

#[test]
fn a_photos_actions_share_keep_offline_show_details_and_trash() {
    let (mut rt, calls) = recording(board::RAW);
    click(&mut rt, "action-0");
    click(&mut rt, "action-3");
    assert_eq!(intents(&mut rt), [Intent::Share, Intent::Trash]);
    assert_eq!(words(&rt, "action-1-label"), "Offline");
    click(&mut rt, "action-1");
    assert_eq!(*calls.borrow(), [format!("offline {} true", board::RAW)]);
    assert_eq!(words(&rt, "action-1-label"), "On this PC");
    // Details: the panel with the RAW's facts (DW-1502); the photo stands alone beside it.
    click(&mut rt, "action-2");
    assert!(viewer(&rt).show_details);
    assert_eq!(words(&rt, "detail-lens-value"), "FE 35mm F1.4 GM");
    assert_eq!(words(&rt, "status-details"), "Space for Preview");
    assert!(!has(&rt, "gallery") && !has(&rt, "action-0"));
    assert_eq!(info(&rt, "photo").target[2], 500.0);
    // Alt+Shift+P hides it again (DW-1501).
    press_with(
        &mut rt,
        Key::Text("P".into()),
        Mods {
            alt: true,
            shift: true,
            ..Mods::default()
        },
    );
    assert!(has(&rt, "gallery"));
    assert_eq!(info(&rt, "photo").target[2], 600.0);
}

#[test]
fn a_recording_plays_pauses_and_walks_its_queue() {
    let (mut rt, calls) = recording(board::AUDIO);
    assert_eq!(words(&rt, "time-elapsed"), "1:12");
    assert_eq!(words(&rt, "time-remain"), "\u{2212}2:50");
    assert_eq!(words(&rt, "status-details"), "Playing \u{00B7} Studio demos, 1 of 5");
    assert_eq!(info(&rt, "pause").label.as_deref(), Some("Pause"));
    assert!(!info(&rt, "prev").focusable, "the first of the queue has no previous");
    press(&mut rt, Key::Text(" ".into()));
    assert_eq!(*calls.borrow(), [format!("play {} false", board::AUDIO)]);
    assert_eq!(info(&rt, "pause").label.as_deref(), Some("Play"));
    assert_eq!(words(&rt, "status-details"), "Paused \u{00B7} Studio demos, 1 of 5");
    click(&mut rt, "pause");
    assert_eq!(calls.borrow().len(), 2);
    // Space with the keyboard on the button is one press, not two.
    assert_eq!(info(&rt, "pause").label.as_deref(), Some("Pause"));
    press(&mut rt, Key::Text(" ".into()));
    assert_eq!(calls.borrow().len(), 3);
    assert_eq!(info(&rt, "pause").label.as_deref(), Some("Play"));
    press(&mut rt, Key::Text(" ".into()));
    // A press on a track plays it; Ctrl+Left and the buttons step.
    click(&mut rt, "track-3");
    assert_eq!(viewer(&rt).file.name, "Client call, 30 Oct.m4a");
    assert_eq!(words(&rt, "status-details"), "Playing \u{00B7} Studio demos, 4 of 5");
    assert!(!has(&rt, "audio-artist"), "no artist in its tags: none shown");
    ctrl(&mut rt, Key::Left);
    assert_eq!(words(&rt, "audio-title"), "Room tone, Studio B");
    click(&mut rt, "next");
    click(&mut rt, "next");
    assert_eq!(words(&rt, "audio-title"), "Voice-over draft");
    assert!(!info(&rt, "next").focusable, "the last of the queue has no next");
    ctrl(&mut rt, Key::Right);
    assert_eq!(words(&rt, "audio-title"), "Voice-over draft");
    click(&mut rt, "tool-0");
    assert_eq!(intents(&mut rt), [Intent::Share]);
    press(&mut rt, Key::Escape);
    press(&mut rt, Key::Tab);
    assert!(!on_a_file(&rt), "Esc is the folder, not the track before");
}

#[test]
fn a_pdf_turns_pages_and_zooms() {
    let (mut rt, _) = recording(board::PDF);
    assert_eq!(words(&rt, "item-count"), "Page 2 of 6 \u{00B7} 2.3 MB");
    assert_eq!(info(&rt, "page").target[2], 420.0);
    click(&mut rt, "thumb-4");
    assert_eq!(words(&rt, "item-count"), "Page 5 of 6 \u{00B7} 2.3 MB");
    press(&mut rt, Key::PageDown);
    press(&mut rt, Key::PageDown);
    assert_eq!(viewer(&rt).media.page, 6, "the last page: no further");
    press(&mut rt, Key::PageUp);
    assert_eq!(viewer(&rt).media.page, 5);
    click(&mut rt, "zoom-plus");
    assert_eq!(words(&rt, "zoom-pct"), "125%");
    assert_eq!(info(&rt, "page").target[2], 525.0);
    ctrl(&mut rt, Key::Command('-'));
    ctrl(&mut rt, Key::Command('-'));
    assert_eq!(words(&rt, "zoom-pct"), "75%");
    for _ in 0..9 {
        ctrl(&mut rt, Key::Command('='));
    }
    assert_eq!(words(&rt, "zoom-pct"), "400%");
    assert!(!info(&rt, "zoom-plus").focusable, "the largest: no further");
    click(&mut rt, "tool-4");
    assert_eq!(intents(&mut rt), [Intent::Share]);
}

#[test]
fn find_in_a_pdf_marks_the_matches_steps_through_them_and_escape_closes_it_first() {
    let (mut rt, calls) = recording(board::PDF_FIND);
    assert!(!has(&rt, "find-bar"));
    ctrl(&mut rt, Key::Command('f'));
    assert!(has(&rt, "find-bar"));
    assert!(
        info(&rt, super::media::FIND_KEY).focused,
        "the keyboard is in the field"
    );
    assert_eq!(words(&rt, "find-count"), "");
    for ch in "terrace".chars() {
        press(&mut rt, Key::Text(ch.to_string()));
    }
    assert_eq!(viewer(&rt).media.find.as_ref().unwrap().edit.text, "terrace");
    assert_eq!(words(&rt, "find-count"), "1 of 3");
    assert_eq!(
        calls.borrow().last().unwrap(),
        &format!("find {} terrace", board::PDF_FIND)
    );
    assert!(
        has(&rt, "mark-0") && has(&rt, "mark-1") && !has(&rt, "mark-2"),
        "two on page 1"
    );
    press(&mut rt, Key::Enter);
    assert_eq!(words(&rt, "find-count"), "2 of 3");
    press(&mut rt, Key::Enter);
    assert_eq!(words(&rt, "find-count"), "3 of 3");
    assert_eq!(viewer(&rt).media.page, 3, "the third is on page 3");
    assert!(has(&rt, "mark-2") && !has(&rt, "mark-0"));
    press(&mut rt, Key::Enter);
    assert_eq!(words(&rt, "find-count"), "1 of 3", "around the end");
    assert_eq!(viewer(&rt).media.page, 1);
    // The second chevron: the previous one. (Shift+Enter in the field asks the same of
    // `Tx::mods`; the test runtime's submit carries no modifiers, so only the button is proven.)
    click(&mut rt, "find-nav-1");
    assert_eq!(words(&rt, "find-count"), "3 of 3");
    click(&mut rt, "find-nav-1");
    assert_eq!(words(&rt, "find-count"), "2 of 3");
    click(&mut rt, "find-nav-0");
    assert_eq!(words(&rt, "find-count"), "3 of 3");
    // Esc closes find and the file stays; Esc again leaves the file.
    press(&mut rt, Key::Escape);
    press(&mut rt, Key::Tab);
    assert!(!has(&rt, "find-bar"));
    assert!(on_a_file(&rt) && rt.state.viewer.ui.is_some());
    press(&mut rt, Key::Escape);
    press(&mut rt, Key::Tab);
    assert!(!on_a_file(&rt));
    // Done closes it too.
    let mut rt = scene("DW-1506");
    click(&mut rt, "find-done");
    assert!(!has(&rt, "find-bar") && !has(&rt, "mark-1"));
}

#[test]
fn a_table_selects_a_cell_moves_it_and_sorts_by_a_column() {
    let (mut rt, _) = recording(board::CSV);
    assert_eq!(
        words(&rt, "item-count"),
        "Comma-separated \u{00B7} 6 columns \u{00B7} 48 rows"
    );
    assert_eq!(viewer(&rt).media.cell, None);
    click(&mut rt, "csv-2-5");
    assert_eq!(viewer(&rt).media.cell, Some((2, 5)));
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Left);
    assert_eq!(viewer(&rt).media.cell, Some((3, 4)));
    press(&mut rt, Key::Right);
    press(&mut rt, Key::Right);
    assert_eq!(viewer(&rt).media.cell, Some((3, 5)), "the last column: no further");
    // Amount, ascending: by value (0.00, 385.00, …), not by letter.
    click(&mut rt, "csv-th-5");
    assert_eq!(viewer(&rt).media.cell, None);
    let amounts = |rt: &Runtime<State>| (0..8).map(|r| words(rt, &format!("csv-{r}-5"))).collect::<Vec<_>>();
    assert_eq!(
        amounts(&rt),
        [
            "0.00", "385.00", "440.00", "475.00", "617.50", "660.00", "665.00", "760.00"
        ]
    );
    assert_eq!(words(&rt, "csv-0-1"), "Studio", "a row moves whole");
    click(&mut rt, "csv-th-5");
    assert_eq!(amounts(&rt)[0], "760.00");
    click(&mut rt, "csv-th-5");
    assert_eq!(amounts(&rt)[0], "760.00", "the file's order again");
    assert_eq!(words(&rt, "csv-1-0"), "2026-10-03");
    // Rate: 95.00 before 110.00 (a sort by letter would put 110.00 first).
    click(&mut rt, "csv-th-4");
    assert_eq!(words(&rt, "csv-0-4"), "0.00");
    assert_eq!(words(&rt, "csv-1-4"), "95.00");
    assert_eq!(words(&rt, "csv-7-4"), "110.00");
    // Client: by letter.
    click(&mut rt, "csv-th-1");
    assert_eq!(words(&rt, "csv-0-1"), "Harbourlight");
    assert_eq!(words(&rt, "csv-7-1"), "Studio");
    // Edit opens it in Sheets: the button and Ctrl+E.
    click(&mut rt, "tb-btn-0");
    ctrl(&mut rt, Key::Command('e'));
    assert_eq!(intents(&mut rt), [Intent::EditInSheets, Intent::EditInSheets]);
    assert!(viewer(&rt).editor.is_none(), "a table is not edited as text");
}

#[test]
fn an_archive_lists_steps_into_a_folder_and_out_again() {
    let (mut rt, _) = recording(board::ZIP);
    assert_eq!(words(&rt, "row-0-name"), "Shoot selects/");
    assert_eq!(words(&rt, "row-0-date"), "212 files");
    assert_eq!(words(&rt, "row-0-size"), "\u{2014}");
    assert_eq!(words(&rt, "row-2-size"), "4.1 MB");
    assert!(
        has(&rt, "row-4") && !has(&rt, "row-5"),
        "the whole list, as the board draws it"
    );
    // A file inside cannot be opened here.
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Down);
    assert_eq!(viewer(&rt).media.entry, Some(2));
    press(&mut rt, Key::Enter);
    assert_eq!(viewer(&rt).media.dir, "");
    press(&mut rt, Key::Up);
    // The folder: its list.
    press(&mut rt, Key::Up);
    press(&mut rt, Key::Enter);
    assert_eq!(viewer(&rt).media.dir, "Shoot selects/");
    assert_eq!(words(&rt, "row-0-name"), "Shoot selects/Day 1/");
    assert!(!has(&rt, "row-1"));
    press(&mut rt, Key::Backspace);
    assert_eq!(words(&rt, "row-0-name"), "Shoot selects/");
    // Back steps out of the folder before it leaves the archive.
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Enter);
    assert_eq!(viewer(&rt).media.dir, "Shoot selects/");
    click(&mut rt, "back");
    assert!(on_a_file(&rt));
    assert_eq!(viewer(&rt).media.dir, "");
    click(&mut rt, "back");
    assert!(!on_a_file(&rt));
    // The way to the files themselves.
    let (mut rt, _) = recording(board::ZIP);
    click(&mut rt, "tb-btn-0");
    assert_eq!(intents(&mut rt), [Intent::OpenInAnotherApp]);
}

#[test]
fn share_says_not_built_yet_until_something_takes_it() {
    let (mut rt, _) = recording(board::AUDIO);
    assert!(!has(&rt, "toast-text"));
    click(&mut rt, "tool-0");
    assert_eq!(words(&rt, "toast-text"), "Not built yet: Share");
    assert_eq!(
        intents(&mut rt),
        [Intent::Share],
        "the intent is still recorded for the shell"
    );
    // The notice goes with the next key the window gets.
    press(&mut rt, Key::Down);
    assert_eq!(viewer(&rt).toast, None);
}

#[test]
fn trash_says_not_built_yet_until_something_takes_it() {
    let (mut rt, _) = recording(board::RAW);
    click(&mut rt, "action-3");
    assert_eq!(words(&rt, "toast-text"), "Not built yet: Move to Trash");
    assert_eq!(viewer(&rt).file.name, "DSC04182.ARW", "nothing was trashed");
}

#[test]
fn open_in_another_app_says_not_built_yet_until_something_takes_it() {
    let (mut rt, _) = recording(board::ZIP);
    click(&mut rt, "tb-btn-0");
    assert_eq!(words(&rt, "toast-text"), "Not built yet: Open in another app");
}

#[test]
fn edit_in_sheets_says_not_built_yet_until_something_takes_it() {
    let (mut rt, _) = recording(board::CSV);
    ctrl(&mut rt, Key::Command('e'));
    assert_eq!(words(&rt, "toast-text"), "Not built yet: Edit in Sheets");
    press(&mut rt, Key::Down);
    assert!(!has(&rt, "toast-text"));
    click(&mut rt, "tb-btn-0");
    assert_eq!(words(&rt, "toast-text"), "Not built yet: Edit in Sheets");
}
