//! Tests of the Photos group: every board frame's state, the keyboard, names and roles, what a
//! press asks of the shell and the engine, and that a 20,000-photo library builds only the
//! rows in view.

use rune::{Env, Key, KeyEvent, Mods, Pixels, Runtime};

use super::source::{
    BackupStatus, Day, Detail, Device, FixturePhotos, Folder, LargePhotos, Photo, PhotoId, PhotosSource, Shape,
    SourceRef, Totals, Viewing, board,
};
use super::{Dialog, Intent, Menu, Original, Toast, Ui, scene};
use crate::shell::app::{Event, Route, Section};
use crate::{State, init, view};

const FRAMES: [&str; 13] = [
    "DW-0601", "DW-0116", "DW-0602", "DW-0603", "DW-0604", "DW-0605", "DW-0605b", "DW-0606", "DW-0607", "DW-0608",
    "DW-0609", "DW-0610", "DW-0611",
];

fn runtime_of(ui: Ui) -> Runtime<State> {
    let mut state = init();
    crate::shell::host::dispatch(&mut state, Event::Navigate(Route::section(Section::Photos)));
    state.photos = ui;
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    // As the app runs: the shell's keys first, then this group's through the registry.
    rt.on_key = Some(crate::shell::host::on_key);
    rt.frame(0.0);
    rt
}

fn runtime(frame: &str) -> Runtime<State> {
    runtime_of(scene(frame).unwrap_or_else(|| panic!("no scene {frame}")))
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

fn text(rt: &Runtime<State>, key: &str) -> String {
    info(rt, key).text.unwrap_or_default()
}

fn key(rt: &mut Runtime<State>, key: Key, cmd: bool) {
    rt.key(KeyEvent {
        key,
        mods: Mods {
            cmd,
            ctrl: cmd,
            ..Mods::default()
        },
    });
    rt.frame(0.0);
}

fn press(rt: &mut Runtime<State>, k: Key) {
    key(rt, k, false);
}

fn click(rt: &mut Runtime<State>, key: &str) {
    rt.settle_all();
    rt.frame(0.0);
    let r = info(rt, key).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
}

fn id(place: (usize, usize)) -> PhotoId {
    PhotoId((place.0 * 1000 + place.1) as u64)
}

#[test]
fn every_board_frame_has_a_scene_that_draws_its_marks() {
    // One element only that frame has, and the toolbar's title and sub line.
    let marks = [
        (
            "DW-0601",
            "ph-tile-2-6",
            "Photos",
            "12,430 photos \u{00B7} 842 videos \u{00B7} 216 GB",
        ),
        ("DW-0116", "ph-empty-title", "Photos", "No photos"),
        (
            "DW-0602",
            "ph-bulk",
            "Photos",
            "12,430 photos \u{00B7} 842 videos \u{00B7} 216 GB",
        ),
        (
            "DW-0603",
            "ph-menu-4-check",
            "Photos",
            "12,430 photos \u{00B7} 842 videos \u{00B7} 216 GB",
        ),
        (
            "DW-0604",
            "ph-item-1-1-title",
            "Photos",
            "12,430 photos \u{00B7} 842 videos \u{00B7} 216 GB",
        ),
        (
            "DW-0605",
            "ph-strip-8",
            "Today 09:12",
            "Anna's iPhone \u{00B7} Harbour.heic",
        ),
        (
            "DW-0605b",
            "ph-progress-fill",
            "Today 09:12",
            "Anna's iPhone \u{00B7} Harbour.heic",
        ),
        (
            "DW-0606",
            "ph-menu-3",
            "Today 09:12",
            "Anna's iPhone \u{00B7} Harbour.heic",
        ),
        (
            "DW-0607",
            "ph-dlg-thumb",
            "Today 09:12",
            "Anna's iPhone \u{00B7} Harbour.heic",
        ),
        (
            "DW-0608",
            "ph-toast",
            "Today 09:12",
            "Anna's iPhone \u{00B7} Harbour.heic",
        ),
        (
            "DW-0609",
            "ph-item-0-6-chev",
            "Today 09:12",
            "Anna's iPhone \u{00B7} Harbour.heic",
        ),
        (
            "DW-0610",
            "ph-toast-action",
            "Today 09:12",
            "Anna's iPhone \u{00B7} Harbour.heic",
        ),
        ("DW-0611", "ph-player", "Sunday 14:20", "Anna's iPhone \u{00B7} 0:24"),
    ];
    assert_eq!(marks.len(), FRAMES.len());
    for (frame, mark, title, sub) in marks {
        let rt = runtime(frame);
        assert!(has(&rt, mark), "{frame}: {mark}");
        assert_eq!(text(&rt, "folder-title"), title, "{frame}");
        assert_eq!(text(&rt, "item-count"), sub, "{frame}");
        assert!(
            has(&rt, "caption-bar") && has(&rt, "Photos-row"),
            "{frame}: the window's chrome"
        );
    }
    assert!(scene("DW-0401").is_none(), "another group's frame is not this group's");
}

#[test]
fn the_board_copy_of_each_frame_is_drawn() {
    let rt = runtime("DW-0601");
    assert_eq!(text(&rt, "ph-day-0"), "Today");
    assert_eq!(text(&rt, "ph-day-2"), "12 October \u{00B7} Lisbon");
    assert_eq!(text(&rt, "ph-backup-strong"), "Anna's iPhone is backing up 12 photos");
    assert_eq!(text(&rt, "ph-tile-0-2-badge-label"), "0:24");
    assert_eq!(
        text(&rt, "status-details"),
        "Photos from Anna's iPhone, Old iPhone and iPad \u{00B7} none from this PC"
    );
    assert_eq!(text(&runtime("DW-0602"), "ph-bulk-count"), "3 photos \u{00B7} 42 MB");
    let menu = runtime("DW-0603");
    assert_eq!(text(&menu, "ph-menu-3-sc"), "Ctrl+Plus");
    assert_eq!(text(&menu, "ph-menu-4-sc"), "Ctrl\u{2212}");
    let open = runtime("DW-0605");
    assert_eq!(text(&open, "ph-seg-1-label"), "Original \u{00B7} 30.9 MB");
    assert_eq!(
        text(&open, "status-details"),
        "Photo 1 of 12,430 \u{00B7} \u{2190} \u{2192} for the next one"
    );
    assert_eq!(
        text(&runtime("DW-0605b"), "ph-seg-1-label"),
        "Original \u{00B7} 12.4 of 30.9 MB"
    );
    assert_eq!(text(&runtime("DW-0608"), "ph-act-1-label"), "On this PC");
    assert_eq!(
        text(&runtime("DW-0609"), "ph-item-0-3-value"),
        "9504 \u{00D7} 6336, 30.9 MB"
    );
    assert_eq!(
        text(&runtime("DW-0611"), "status-details"),
        "Streaming \u{00B7} decrypted on this PC as it plays"
    );
}

#[test]
fn names_and_roles() {
    let rt = runtime("DW-0602");
    for (key, role, label) in [
        ("ph-across", "button", "Photos across"),
        ("ph-sources", "button", "Photo sources"),
        ("ph-select", "button", "Select"),
        ("ph-pills", "radiogroup", "Source"),
        ("ph-pill-1", "radio", "Anna's iPhone"),
        ("ph-bulk", "toolbar", "Selection"),
        ("ph-bulk-btn-3", "button", "Move to Trash"),
        ("ph-tile-0-0", "checkbox", "Photo, Today, still uploading"),
        ("ph-tile-0-2", "checkbox", "Video, 0:24, Today"),
        ("ph-rows", "list", "Photos by day"),
    ] {
        let e = info(&rt, key);
        assert_eq!((e.role, e.label.as_deref()), (role, Some(label)), "{key}");
    }
    let rt = runtime("DW-0607");
    for (key, role, label) in [
        ("ph-dlg", "dialog", "Share"),
        ("ph-item-0-0-pop", "combobox", "Anyone with the link"),
        ("ph-item-0-2-switch", "switch", "Password"),
        ("ph-dlg-btn-1", "button", "Copy link"),
        ("ph-seg", "tablist", "Rendition"),
        ("ph-more", "button", "More"),
        ("ph-act-3", "button", "Trash"),
        ("ph-photo", "image", "Harbour.heic"),
    ] {
        let e = info(&rt, key);
        assert_eq!((e.role, e.label.as_deref()), (role, Some(label)), "{key}");
    }
    assert_eq!(info(&runtime("DW-0603"), "ph-menu").role, "menu");
    assert_eq!(info(&runtime("DW-0116"), "ph-empty-title").role, "heading");
}

#[test]
fn the_keyboard_walks_the_grid_and_opens_a_photo() {
    let mut rt = runtime("DW-0601");
    press(&mut rt, Key::Right);
    assert_eq!(
        rt.state.photos.cursor,
        Some((0, 0)),
        "the first arrow lands on the first photo"
    );
    press(&mut rt, Key::Left);
    assert_eq!(rt.state.photos.cursor, Some((0, 0)), "and stays at the start");
    for _ in 0..4 {
        press(&mut rt, Key::Right);
    }
    press(&mut rt, Key::Down);
    assert_eq!(rt.state.photos.cursor, Some((1, 4)));
    press(&mut rt, Key::Up);
    assert_eq!(rt.state.photos.cursor, Some(board::HARBOUR));
    press(&mut rt, Key::Enter);
    assert_eq!(rt.state.photos.viewer.as_ref().map(|v| v.id), Some(id(board::HARBOUR)));
    assert!(has(&rt, "ph-stage") && !has(&rt, "ph-rows"));
    press(&mut rt, Key::Escape);
    assert!(
        rt.state.photos.viewer.is_none() && has(&rt, "ph-rows"),
        "Esc: back to Photos"
    );
    press(&mut rt, Key::Text(" ".into()));
    assert!(rt.state.photos.viewer.is_some(), "Space previews the photo");
    press(&mut rt, Key::Text(" ".into()));
    assert!(rt.state.photos.viewer.is_none(), "and Space closes it");
}

#[test]
fn a_double_click_opens_and_the_back_button_returns() {
    let mut rt = runtime("DW-0601");
    rt.settle_all();
    let r = info(&rt, "ph-tile-0-4").rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    for _ in 0..2 {
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
    }
    rt.frame(0.0);
    assert_eq!(rt.state.photos.viewer.as_ref().map(|v| v.id), Some(id(board::HARBOUR)));
    click(&mut rt, "back");
    assert!(rt.state.photos.viewer.is_none());
}

#[test]
fn arrows_move_through_the_strip_and_a_thumbnail_jumps() {
    let mut rt = runtime("DW-0605");
    press(&mut rt, Key::Right);
    assert_eq!(rt.state.photos.viewer.as_ref().map(|v| v.id), Some(id((0, 1))));
    press(&mut rt, Key::Left);
    press(&mut rt, Key::Left);
    assert_eq!(rt.state.photos.viewer.as_ref().map(|v| v.id), Some(id((0, 6))));
    click(&mut rt, "ph-strip-0");
    assert_eq!(rt.state.photos.viewer.as_ref().map(|v| v.id), Some(id((0, 2))));
    press(&mut rt, Key::Left);
    assert_eq!(
        rt.state.photos.viewer.as_ref().map(|v| v.id),
        Some(id((0, 2))),
        "nothing before the first"
    );
}

#[test]
fn photos_across_by_menu_and_by_keys() {
    let mut rt = runtime("DW-0601");
    click(&mut rt, "ph-across");
    assert_eq!(rt.state.photos.menu.map(|m| m.which), Some(Menu::Across));
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Enter);
    assert_eq!(rt.state.photos.across, 4, "the second choice");
    assert!(rt.state.photos.menu.is_none());
    assert_eq!(rt.state.photos.take_intents(), vec![Intent::RememberAcross(4)]);
    // Seven photos a day, four across: two rows a day, the second without a heading.
    assert!(has(&rt, "ph-tile-0-6") && has(&rt, "ph-line-1") && !has(&rt, "ph-tile-0-7"));
    let side = info(&rt, "ph-tile-0-0").rect[2];
    assert!((side - 254.25).abs() < 0.6, "four across is {side} wide");
    key(&mut rt, Key::Command('+'), true);
    assert_eq!(rt.state.photos.across, 3);
    key(&mut rt, Key::Command('+'), true);
    assert_eq!(rt.state.photos.across, 3, "3 is the largest");
    for _ in 0..4 {
        key(&mut rt, Key::Command('-'), true);
    }
    assert_eq!(rt.state.photos.across, 7, "7 is the smallest");
    click(&mut rt, "ph-across");
    click(&mut rt, "ph-menu-3");
    assert_eq!(rt.state.photos.across, 5, "a click on 5");
    click(&mut rt, "ph-across");
    press(&mut rt, Key::Escape);
    assert!(
        rt.state.photos.menu.is_none() && rt.state.photos.across == 5,
        "Esc closes the menu, nothing else"
    );
}

#[test]
fn a_pill_filters_and_an_empty_device_says_so() {
    let mut rt = runtime("DW-0601");
    click(&mut rt, "ph-pill-1");
    assert_eq!(rt.state.photos.filter, Some(0));
    assert_eq!(text(&rt, "item-count"), "12,418 photos", "the totals follow");
    assert!(has(&rt, "ph-tile-0-0"));
    click(&mut rt, "ph-pill-2");
    assert!(
        has(&rt, "ph-empty-title") && !has(&rt, "ph-rows"),
        "DW-0116: a filter with nothing"
    );
    click(&mut rt, "ph-pill-0");
    assert!(rt.state.photos.filter.is_none() && has(&rt, "ph-tile-2-6"));
}

#[test]
fn selecting_checks_counts_trashes_and_undoes() {
    let mut rt = runtime("DW-0601");
    click(&mut rt, "ph-select");
    assert!(rt.state.photos.selecting && has(&rt, "ph-bulk"));
    click(&mut rt, "ph-tile-0-0");
    click(&mut rt, "ph-tile-0-3");
    assert_eq!(text(&rt, "ph-bulk-count"), "2 photos \u{00B7} 28 MB");
    click(&mut rt, "ph-tile-0-3");
    assert_eq!(text(&rt, "ph-bulk-count"), "1 photo \u{00B7} 14 MB");
    click(&mut rt, "ph-bulk-all");
    assert_eq!(rt.state.photos.selected.len(), 21);
    click(&mut rt, "ph-bulk-btn-4");
    assert!(!rt.state.photos.selecting && !has(&rt, "ph-bulk"), "Done");

    click(&mut rt, "ph-select");
    click(&mut rt, "ph-tile-0-1");
    click(&mut rt, "ph-bulk-btn-0");
    click(&mut rt, "ph-bulk-btn-1");
    click(&mut rt, "ph-bulk-btn-2");
    let one = vec![id((0, 1))];
    assert_eq!(
        rt.state.photos.take_intents(),
        vec![
            Intent::ShareSeveral(one.clone()),
            Intent::KeepSeveral(one.clone()),
            Intent::Move(one.clone())
        ]
    );
    click(&mut rt, "ph-bulk-btn-3");
    assert_eq!(rt.state.photos.toast, Some(Toast::Trashed(one)));
    assert!(!has(&rt, "ph-tile-0-6") && has(&rt, "ph-tile-0-5"), "six left in Today");
    assert!(has(&rt, "ph-toast-action"), "Undo is offered over the grid");
    key(&mut rt, Key::Command('z'), true);
    assert!(
        rt.state.photos.toast.is_none() && has(&rt, "ph-tile-0-6"),
        "Ctrl+Z brings it back"
    );

    click(&mut rt, "ph-select");
    click(&mut rt, "ph-tile-1-0");
    press(&mut rt, Key::Escape);
    assert!(
        !rt.state.photos.selecting && rt.state.photos.selected.is_empty(),
        "Esc ends selecting"
    );
}

#[test]
fn the_open_photo_actions() {
    let mut rt = runtime("DW-0605");
    let harbour = id(board::HARBOUR);

    click(&mut rt, "ph-seg-1");
    assert_eq!(
        rt.state.photos.viewer.as_ref().map(|v| v.original),
        Some(Original::Downloading { done: 0 })
    );
    assert!(has(&rt, "ph-progress"), "the thin bar; the preview stays");
    rt.state.photos.original_progress(harbour, 30_900_000, 30_900_000);
    assert_eq!(
        rt.state.photos.viewer.as_ref().map(|v| v.original),
        Some(Original::Whole)
    );
    click(&mut rt, "ph-seg-0");
    click(&mut rt, "ph-seg-1");
    click(&mut rt, "ph-seg-0");
    assert_eq!(
        rt.state.photos.take_intents(),
        vec![
            Intent::DownloadOriginal(harbour),
            Intent::DownloadOriginal(harbour),
            Intent::StopOriginal(harbour)
        ],
        "Preview stops a running download only"
    );

    click(&mut rt, "ph-more");
    click(&mut rt, "ph-menu-0");
    click(&mut rt, "ph-more");
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Enter);
    assert_eq!(
        rt.state.photos.take_intents(),
        vec![Intent::SaveToPc(harbour), Intent::Move(vec![harbour])]
    );

    click(&mut rt, "ph-act-1");
    assert_eq!(rt.state.photos.toast, Some(Toast::Kept));
    assert_eq!(text(&rt, "ph-act-1-label"), "On this PC");
    click(&mut rt, "ph-act-1");
    assert_eq!(rt.state.photos.take_intents(), vec![Intent::OfferRemoveFromPc(harbour)]);

    click(&mut rt, "ph-act-2");
    assert_eq!(rt.state.photos.dialog, Some(Dialog::Details));
    click(&mut rt, "ph-item-0-6");
    assert_eq!(rt.state.photos.take_intents(), vec![Intent::VersionHistory(harbour)]);
    press(&mut rt, Key::Escape);
    assert!(
        rt.state.photos.dialog.is_none() && rt.state.photos.viewer.is_some(),
        "Esc closes the dialog only"
    );
    click(&mut rt, "panel-toggle-btn");
    assert_eq!(
        rt.state.photos.dialog,
        Some(Dialog::Details),
        "the details button opens Details"
    );
    click(&mut rt, "ph-dlg-btn-0");
    assert!(rt.state.photos.dialog.is_none());
}

#[test]
fn share_says_which_way_keeps_the_encryption_and_asks_for_it() {
    let mut rt = runtime("DW-0607");
    let harbour = id(board::HARBOUR);
    assert!(text(&rt, "ph-dlg-note").starts_with("Copy link keeps it encrypted. Send a copy decrypts it"));
    click(&mut rt, "ph-item-0-0-pop");
    click(&mut rt, "ph-item-0-2-switch");
    assert_eq!(text(&rt, "ph-item-0-0-pop-label"), "Can edit");
    press(&mut rt, Key::Enter);
    assert_eq!(
        rt.state.photos.take_intents(),
        vec![Intent::CopyLink {
            id: harbour,
            can_edit: true,
            days: Some(30),
            password: true
        }]
    );
    assert!(rt.state.photos.dialog.is_none());
    click(&mut rt, "ph-act-0");
    click(&mut rt, "ph-dlg-btn-0");
    assert_eq!(rt.state.photos.take_intents(), vec![Intent::SendCopy(harbour)]);
    click(&mut rt, "ph-act-0");
    click(&mut rt, "ph-dlg-btn-2");
    assert!(
        rt.state.photos.dialog.is_none() && rt.state.photos.take_intents().is_empty(),
        "Cancel"
    );
}

#[test]
fn trash_shows_the_next_photo_and_undo_brings_it_back() {
    let mut rt = runtime("DW-0605");
    let harbour = id(board::HARBOUR);
    press(&mut rt, Key::Delete);
    assert_eq!(rt.state.photos.toast, Some(Toast::Trashed(vec![harbour])));
    assert_eq!(
        rt.state.photos.viewer.as_ref().map(|v| v.id),
        Some(id((0, 1))),
        "the next one is on the stage"
    );
    assert_eq!(
        text(&rt, "folder-title"),
        "Today 09:12",
        "DW-0610: the bar still shows what can come back"
    );
    click(&mut rt, "ph-toast-action");
    assert_eq!(rt.state.photos.viewer.as_ref().map(|v| v.id), Some(harbour));
    assert!(rt.state.photos.toast.is_none() && has(&rt, "ph-strip-8"));

    click(&mut rt, "ph-act-3");
    press(&mut rt, Key::Right);
    assert!(rt.state.photos.toast.is_none(), "moving on lets the message go");
    assert!(!has(&rt, "ph-strip-8"), "eight left in the strip");
    press(&mut rt, Key::Escape);
    assert!(!has(&rt, "ph-tile-0-6"), "and six in Today");
}

#[test]
fn a_video_plays_and_pauses_on_space() {
    let mut rt = runtime("DW-0611");
    assert!(!has(&rt, "ph-seg"), "a video has one rendition");
    assert_eq!(info(&rt, "ph-player-icon").label.as_deref(), Some("Pause"));
    press(&mut rt, Key::Text(" ".into()));
    assert_eq!(info(&rt, "ph-player-icon").label.as_deref(), Some("Play"));
    assert_eq!(
        rt.state.photos.take_intents(),
        vec![Intent::PlayPause(id(board::VIDEO))]
    );
    assert!(rt.state.photos.viewer.is_some(), "Space does not close a video");
    press(&mut rt, Key::Escape);
    assert!(rt.state.photos.viewer.is_none());
}

#[test]
fn photo_sources_applies_on_done_only() {
    let mut rt = runtime("DW-0601");
    click(&mut rt, "ph-sources");
    // The dialog's second row is the iPad (the third device).
    click(&mut rt, "ph-item-0-1-switch");
    press(&mut rt, Key::Escape);
    assert!(has(&rt, "ph-pill-3"), "Cancel: nothing changed");
    click(&mut rt, "ph-sources");
    click(&mut rt, "ph-item-0-1-switch");
    click(&mut rt, "ph-item-1-1");
    assert_eq!(rt.state.photos.take_intents(), vec![Intent::AddFolder]);
    click(&mut rt, "ph-dlg-btn-0");
    assert!(rt.state.photos.dialog.is_none());
    assert!(
        !has(&rt, "ph-pill-3") && text(&rt, "ph-pill-2-label") == "Old iPhone",
        "the iPad's pill went"
    );
    assert!(!rt.state.photos.source.devices()[2].shown);
}

#[test]
fn new_and_search_ask_the_lanes_that_own_them() {
    let mut rt = runtime("DW-0601");
    click(&mut rt, "new-btn");
    assert_eq!(rt.state.photos.take_intents(), vec![Intent::NewInFiles]);
    assert!(
        !info(&rt, "back").focusable && !info(&rt, "sort-btn").focusable,
        "nothing to go back to, nothing to sort"
    );
}

/// A source whose thumbnails have not arrived.
struct NoThumbnails(FixturePhotos);

impl PhotosSource for NoThumbnails {
    fn devices(&self) -> Vec<Device> {
        self.0.devices()
    }
    fn folders(&self) -> Vec<Folder> {
        self.0.folders()
    }
    fn set_shown(&mut self, source: SourceRef, shown: bool) {
        self.0.set_shown(source, shown);
    }
    fn totals(&self, device: Option<usize>) -> Totals {
        self.0.totals(device)
    }
    fn backup_status(&self) -> Option<BackupStatus> {
        self.0.backup_status()
    }
    fn origin(&self) -> String {
        self.0.origin()
    }
    fn days(&self, device: Option<usize>) -> Vec<Day> {
        self.0.days(device)
    }
    fn photo(&self, device: Option<usize>, day: usize, index: usize) -> Option<Photo> {
        self.0.photo(device, day, index)
    }
    fn thumbnail(&self, _: PhotoId, _: Shape) -> Option<Pixels> {
        None
    }
    fn viewing(&self, id: PhotoId) -> Option<Viewing> {
        self.0.viewing(id)
    }
    fn details(&self, id: PhotoId) -> Vec<Detail> {
        self.0.details(id)
    }
    fn kept_offline(&self, id: PhotoId) -> bool {
        self.0.kept_offline(id)
    }
    fn keep_offline(&mut self, id: PhotoId, keep: bool) {
        self.0.keep_offline(id, keep);
    }
    fn trash(&mut self, ids: &[PhotoId]) {
        self.0.trash(ids);
    }
    fn restore(&mut self, ids: &[PhotoId]) {
        self.0.restore(ids);
    }
}

#[test]
fn a_thumbnail_that_is_not_there_yet_is_a_plain_tile_that_still_works() {
    let mut rt = runtime_of(Ui::new(Box::new(NoThumbnails(FixturePhotos::new()))));
    assert!(
        has(&rt, "ph-tile-0-4") && has(&rt, "ph-tile-0-2-badge-label"),
        "the tile and its badge are there"
    );
    rt.state.photos.open_tile(0, 4);
    rt.frame(0.0);
    assert!(has(&rt, "ph-photo") && has(&rt, "ph-strip-4"), "and so is the viewer");
}

#[test]
fn twenty_thousand_photos_build_only_the_rows_in_view() {
    let mut rt = runtime_of(Ui::new(Box::new(LargePhotos::new(20_000))));
    let ui = &rt.state.photos;
    // 500 days of 40, seven across: six rows a day.
    assert_eq!((ui.days.len(), ui.lines.len()), (500, 3000));
    let tiles = rt
        .elements()
        .iter()
        .filter(|e| {
            e.key
                .as_deref()
                .is_some_and(|k| k.starts_with("ph-tile-") && !k.contains("-badge"))
        })
        .count();
    assert!((28..=140).contains(&tiles), "{tiles} tiles are built for 20,000 photos");
    let started = std::time::Instant::now();
    for i in 0..50 {
        rt.state.photos.cursor = Some((0, i % 7));
        rt.frame(0.0);
    }
    let per_frame = started.elapsed().as_secs_f64() * 1000.0 / 50.0;
    println!("photos: 20,000 photos, {tiles} tiles built, {per_frame:.2} ms per rebuilt frame (debug build)");
    assert!(per_frame < 50.0, "{per_frame} ms per frame");
}
