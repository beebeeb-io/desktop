//! The Photos window on a source that answers later: nothing is drawn at a press, what the source says is drawn
//! per item, Try again asks for exactly what was left, and a gate or account change leaves nothing that names a
//! photo. The source here is the board's library behind a script: it notes what it is asked, changes nothing in the
//! call, and answers when the test says so. Real events (keys, clicks, the shell's poll), read off the element tree.

use std::sync::{Arc, Mutex};

use rune::{Env, Key, KeyEvent, Mods, Pixels, Runtime};

use super::super::source::{
    Answered, BackupStatus, Day, Detail, Device, FixturePhotos, Folder, Kind, Left, Loaded, Photo, PhotoId,
    PhotosSource, Pumped, Shape, SourceRef, Totals, Viewing, board,
};
use super::super::{Toast, Ui};
use super::What;
use crate::shell::app::{Event, Route, Section};
use crate::{State, init, view};

#[derive(Default)]
struct Script {
    /// What was asked, in order: "trash", "restore", "keep", and the ids.
    asked: Vec<(&'static str, Vec<PhotoId>)>,
    /// The next ask is refused with this, unasked.
    refuse: Option<Left>,
    /// What the next pump hands over.
    answers: Vec<Answered>,
    loaded: Loaded,
    /// No video's length is known (an engine that has no copy on this PC).
    no_length: bool,
    /// These left the library some other way: the next pump no longer lists them.
    leaves: Vec<PhotoId>,
    /// The next pump says the library changed.
    dirty: bool,
    /// These leave the library when the source is asked to read again (`refresh`): a finished check brings it.
    leave_on_refresh: Vec<PhotoId>,
    opened: usize,
    forgot: usize,
    refreshed: usize,
    /// The folders of Files that Add a folder… handed over, and the rows Remove was pressed on.
    folders_added: Vec<String>,
    folders_removed: Vec<SourceRef>,
    /// The source refuses to add a folder (`add_folder` answers `false`).
    refuse_folder: bool,
    /// The engine refused a folder change after the press: told once.
    folder_refused: bool,
    /// The files behind the photos for Move…; none: the source holds no file for them.
    items: Vec<crate::source::DriveItem>,
}

struct Later {
    library: FixturePhotos,
    script: Arc<Mutex<Script>>,
}

impl Later {
    fn ask(&mut self, what: &'static str, ids: &[PhotoId]) -> Result<(), Left> {
        let mut script = self.script.lock().unwrap();
        if let Some(why) = script.refuse.take() {
            return Err(why);
        }
        script.asked.push((what, ids.to_vec()));
        Ok(())
    }
}

impl PhotosSource for Later {
    fn answers_later(&self) -> bool {
        true
    }
    fn loaded(&self) -> Loaded {
        self.script.lock().unwrap().loaded
    }
    fn opened(&mut self) {
        self.script.lock().unwrap().opened += 1;
    }
    fn forget(&mut self) {
        self.script.lock().unwrap().forgot += 1;
    }
    fn refresh(&mut self) {
        let mut script = self.script.lock().unwrap();
        script.refreshed += 1;
        let leaving = std::mem::take(&mut script.leave_on_refresh);
        if !leaving.is_empty() {
            script.leaves.extend(leaving);
            script.dirty = true;
        }
    }
    fn pump(&mut self) -> Pumped {
        let (answered, mut dirty, leaves) = {
            let mut script = self.script.lock().unwrap();
            (
                std::mem::take(&mut script.answers),
                std::mem::take(&mut script.dirty),
                std::mem::take(&mut script.leaves),
            )
        };
        if !leaves.is_empty() {
            self.library.trash(&leaves).unwrap();
            dirty = true;
        }
        // What Beebeeb did is in the library once it answered, never before.
        for answer in &answered {
            let done = |items: &[(PhotoId, Option<Left>)]| -> Vec<PhotoId> {
                items
                    .iter()
                    .filter(|item| item.1.is_none())
                    .map(|item| item.0)
                    .collect()
            };
            match answer {
                Answered::Trashed(items) => self.library.trash(&done(items)).unwrap(),
                Answered::Restored(items) => self.library.restore(&done(items)).unwrap(),
                Answered::Kept { id, keep, left: None } => self.library.keep_offline(*id, *keep).unwrap(),
                Answered::Kept { .. } => {}
            }
        }
        Pumped {
            changed: dirty || !answered.is_empty(),
            busy: false,
            answered,
        }
    }
    fn devices(&self) -> Vec<Device> {
        self.library.devices()
    }
    fn folders(&self) -> Vec<Folder> {
        self.library.folders()
    }
    fn set_shown(&mut self, source: SourceRef, shown: bool) {
        self.library.set_shown(source, shown);
    }
    fn add_folder(&mut self, folder_id: &str) -> bool {
        let mut script = self.script.lock().unwrap();
        if script.refuse_folder {
            return false;
        }
        script.folders_added.push(folder_id.to_string());
        true
    }
    fn folder_refused(&mut self) -> bool {
        std::mem::take(&mut self.script.lock().unwrap().folder_refused)
    }
    fn remove_folder(&mut self, source: SourceRef) {
        self.script.lock().unwrap().folders_removed.push(source);
    }
    fn file_items(&self, _ids: &[PhotoId]) -> Option<Vec<crate::source::DriveItem>> {
        let items = self.script.lock().unwrap().items.clone();
        (!items.is_empty()).then_some(items)
    }
    fn totals(&self, device: Option<usize>) -> Totals {
        self.library.totals(device)
    }
    fn backup_status(&self) -> Option<BackupStatus> {
        self.library.backup_status()
    }
    fn origin(&self) -> String {
        self.library.origin()
    }
    fn days(&self, device: Option<usize>) -> Vec<Day> {
        if self.loaded() != Loaded::Ready {
            return Vec::new();
        }
        self.library.days(device)
    }
    fn photo(&self, device: Option<usize>, day: usize, index: usize) -> Option<Photo> {
        let mut photo = self.library.photo(device, day, index)?;
        if self.script.lock().unwrap().no_length && matches!(photo.kind, Kind::Video(_)) {
            photo.kind = Kind::Video(0);
        }
        Some(photo)
    }
    fn thumbnail(&self, id: PhotoId, shape: Shape) -> Option<Pixels> {
        self.library.thumbnail(id, shape)
    }
    fn viewing(&self, id: PhotoId) -> Option<Viewing> {
        let mut viewing = self.library.viewing(id)?;
        if self.script.lock().unwrap().no_length && matches!(viewing.kind, Kind::Video(_)) {
            viewing.kind = Kind::Video(0);
        }
        Some(viewing)
    }
    fn details(&self, id: PhotoId) -> Vec<Detail> {
        self.library.details(id)
    }
    fn kept_offline(&self, id: PhotoId) -> bool {
        self.library.kept_offline(id)
    }
    fn keep_offline(&mut self, id: PhotoId, keep: bool) -> Result<(), Left> {
        self.ask(if keep { "keep" } else { "stop keeping" }, &[id])
    }
    fn trash(&mut self, ids: &[PhotoId]) -> Result<(), Left> {
        self.ask("trash", ids)
    }
    fn restore(&mut self, ids: &[PhotoId]) -> Result<(), Left> {
        self.ask("restore", ids)
    }
}

/// A window on Photos with the scripted source, polled as the app polls.
fn window() -> (Runtime<State>, Arc<Mutex<Script>>) {
    let script = Arc::new(Mutex::new(Script::default()));
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(init(), view, (1278.0, 798.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.poll = Some(crate::shell::host::poll);
    // The shell reads its session on the first polls; the window is on Photos once that is settled.
    rt.frame(0.0);
    rt.frame(0.0);
    crate::shell::host::dispatch(&mut rt.state, Event::Navigate(Route::section(Section::Photos)));
    rt.state.photos = Ui::new(Box::new(Later {
        library: FixturePhotos::new(),
        script: script.clone(),
    }));
    rt.frame(0.0);
    rt.frame(0.0);
    (rt, script)
}

fn has(rt: &Runtime<State>, key: &str) -> bool {
    rt.elements()
        .iter()
        .any(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn text(rt: &Runtime<State>, key: &str) -> String {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
        .text
        .unwrap_or_default()
}

fn click(rt: &mut Runtime<State>, key: &str) {
    rt.settle_all();
    rt.frame(0.0);
    let r = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
        .rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
}

fn press(rt: &mut Runtime<State>, key: Key, cmd: bool) {
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

fn id(place: (usize, usize)) -> PhotoId {
    PhotoId((place.0 * 1000 + place.1) as u64)
}

fn answer(rt: &mut Runtime<State>, script: &Arc<Mutex<Script>>, answer: Answered) {
    script.lock().unwrap().answers.push(answer);
    rt.frame(0.0);
    rt.frame(0.0);
}

/// The view runs again (a state the test set by hand is drawn).
fn redraw(rt: &mut Runtime<State>, script: &Arc<Mutex<Script>>) {
    script.lock().unwrap().dirty = true;
    rt.frame(0.0);
    rt.frame(0.0);
}

fn asked(script: &Arc<Mutex<Script>>) -> Vec<(&'static str, Vec<PhotoId>)> {
    script.lock().unwrap().asked.clone()
}

#[test]
fn the_section_opening_asks_for_the_library_once_and_no_library_is_said_in_words() {
    let (mut rt, script) = window();
    assert_eq!(
        script.lock().unwrap().opened,
        1,
        "asked when the section came on screen"
    );
    rt.frame(0.0);
    assert_eq!(script.lock().unwrap().opened, 1, "not on every frame");
    assert!(has(&rt, "ph-tile-0-0") && !has(&rt, "ph-not-here"));

    for (loaded, title) in [
        (Loaded::Loading, "Loading your photos\u{2026}"),
        (Loaded::Failed, "Couldn\u{2019}t load your photos"),
        (Loaded::NothingSynced, "Nothing has synced to this PC yet"),
        (Loaded::NotThisAccounts, "This PC holds another account\u{2019}s files"),
        (Loaded::SessionEnded, "Your session ended"),
        (Loaded::SignedOut, "Signed out"),
    ] {
        {
            let mut script = script.lock().unwrap();
            script.loaded = loaded;
            script.dirty = true;
        }
        rt.frame(0.0);
        rt.frame(0.0);
        assert_eq!(text(&rt, "ph-not-here-title"), title);
        assert!(!has(&rt, "ph-tile-0-0"), "no tile under {loaded:?}");
        assert!(
            !has(&rt, "ph-empty"),
            "and never \"No photos yet\" for a library that is not read"
        );
    }
}

#[test]
fn a_selection_leaves_only_when_beebeeb_said_so_and_what_was_left_can_be_tried_again() {
    let (mut rt, script) = window();
    let (a, b) = (id((0, 0)), id((0, 3)));
    rt.state.photos.check_tile(0, 0);
    rt.state.photos.check_tile(0, 3);
    rt.frame(0.0);
    click(&mut rt, "ph-bulk-btn-3");

    // Asked, not answered: nothing left, nothing is said.
    assert_eq!(asked(&script), [("trash", vec![a, b])]);
    assert!(rt.state.photos.toast.is_none() && !has(&rt, "ph-toast"));
    assert!(rt.state.photos.selecting && rt.state.photos.selected.len() == 2);
    assert!(has(&rt, "ph-tile-0-6"), "seven tiles in Today still");

    // Beebeeb moved one and refused the other.
    answer(
        &mut rt,
        &script,
        Answered::Trashed(vec![(a, None), (b, Some(Left::Failed))]),
    );
    assert!(!has(&rt, "ph-tile-0-6"), "the one that was moved left");
    assert_eq!(
        rt.state.photos.toast,
        Some(Toast::Left {
            what: What::Trash,
            why: Left::Failed,
            count: 1,
            again: vec![b]
        })
    );
    assert_eq!(text(&rt, "ph-toast-text"), "Couldn\u{2019}t move to Trash");
    assert_eq!(
        rt.state.photos.selected.keys().copied().collect::<Vec<_>>(),
        [b],
        "the one that was left is still checked"
    );

    // Try again: exactly the one that was left.
    click(&mut rt, "ph-toast-action");
    assert_eq!(asked(&script)[1], ("trash", vec![b]));
    assert!(rt.state.photos.toast.is_none());
    answer(&mut rt, &script, Answered::Trashed(vec![(b, None)]));
    assert_eq!(rt.state.photos.toast, Some(Toast::Trashed(vec![b])));
    assert!(!rt.state.photos.selecting, "nothing is checked any more");
    assert_eq!(text(&rt, "ph-toast-text"), "Moved to Trash");

    // Undo (Ctrl+Z): asked, and the photo is back only when Beebeeb put it back.
    press(&mut rt, Key::Command('z'), true);
    assert_eq!(asked(&script)[2], ("restore", vec![b]));
    assert!(rt.state.photos.toast.is_none());
    assert!(!has(&rt, "ph-tile-0-5"), "five in Today until it is answered");
    answer(&mut rt, &script, Answered::Restored(vec![(b, None)]));
    assert!(has(&rt, "ph-tile-0-5"));
}

#[test]
fn a_refusal_at_the_press_is_said_and_a_change_on_its_way_offers_no_second_try() {
    let (mut rt, script) = window();
    rt.state.photos.check_tile(0, 0);
    rt.frame(0.0);
    script.lock().unwrap().refuse = Some(Left::Offline);
    click(&mut rt, "ph-bulk-btn-3");
    assert_eq!(asked(&script), [], "nothing was asked");
    assert_eq!(
        text(&rt, "ph-toast-text"),
        "Couldn\u{2019}t move to Trash \u{00B7} you\u{2019}re offline"
    );
    assert!(has(&rt, "ph-toast-action") && has(&rt, "ph-tile-0-6"));

    script.lock().unwrap().refuse = Some(Left::InFlight);
    click(&mut rt, "ph-toast-action");
    assert_eq!(
        text(&rt, "ph-toast-text"),
        "Still on its way \u{00B7} nothing was sent twice"
    );
    assert!(
        !has(&rt, "ph-toast-action"),
        "the first press's answer says what was moved"
    );
    assert_eq!(asked(&script), []);
}

#[test]
fn an_open_photo_stays_on_the_stage_until_beebeeb_moved_it_and_undo_waits_for_its_answer_too() {
    let (mut rt, script) = window();
    let harbour = id(board::HARBOUR);
    rt.state.photos.open(harbour);
    rt.frame(0.0);
    press(&mut rt, Key::Delete, false);
    assert_eq!(asked(&script), [("trash", vec![harbour])]);
    assert_eq!(
        rt.state.photos.viewer.as_ref().map(|v| v.id),
        Some(harbour),
        "still on the stage"
    );
    assert!(rt.state.photos.toast.is_none());

    answer(&mut rt, &script, Answered::Trashed(vec![(harbour, None)]));
    assert_eq!(rt.state.photos.toast, Some(Toast::Trashed(vec![harbour])));
    assert_eq!(
        rt.state.photos.viewer.as_ref().map(|v| v.id),
        Some(id((0, 1))),
        "the next one is on the stage"
    );
    assert_eq!(
        text(&rt, "folder-title"),
        "Today 09:12",
        "the bar still shows what can come back"
    );

    // Undo: asked; the next photo stays on the stage until Beebeeb put the first back.
    click(&mut rt, "ph-toast-action");
    assert_eq!(asked(&script)[1], ("restore", vec![harbour]));
    assert_eq!(rt.state.photos.viewer.as_ref().map(|v| v.id), Some(id((0, 1))));
    // Beebeeb refused: said, and Undo is still for it.
    answer(
        &mut rt,
        &script,
        Answered::Restored(vec![(harbour, Some(Left::Offline))]),
    );
    assert_eq!(
        text(&rt, "ph-toast-text"),
        "Couldn\u{2019}t put it back \u{00B7} you\u{2019}re offline"
    );
    assert_eq!(rt.state.photos.viewer.as_ref().map(|v| v.id), Some(id((0, 1))));
    click(&mut rt, "ph-toast-action");
    assert_eq!(asked(&script)[2], ("restore", vec![harbour]));
    answer(&mut rt, &script, Answered::Restored(vec![(harbour, None)]));
    assert_eq!(rt.state.photos.viewer.as_ref().map(|v| v.id), Some(harbour));
    assert!(rt.state.photos.toast.is_none());
}

#[test]
fn keeping_a_photo_is_said_only_once_this_pc_answered() {
    let (mut rt, script) = window();
    let harbour = id(board::HARBOUR);
    rt.state.photos.open(harbour);
    rt.frame(0.0);
    rt.state.photos.press_offline();
    rt.frame(0.0);
    assert_eq!(asked(&script), [("keep", vec![harbour])]);
    assert!(rt.state.photos.toast.is_none(), "not before the answer");

    answer(
        &mut rt,
        &script,
        Answered::Kept {
            id: harbour,
            keep: true,
            left: Some(Left::Failed),
        },
    );
    assert_eq!(text(&rt, "ph-toast-text"), "Couldn\u{2019}t keep it on this PC");
    click(&mut rt, "ph-toast-action");
    assert_eq!(asked(&script)[1], ("keep", vec![harbour]));
    answer(
        &mut rt,
        &script,
        Answered::Kept {
            id: harbour,
            keep: true,
            left: None,
        },
    );
    assert_eq!(rt.state.photos.toast, Some(Toast::Kept));
}

#[test]
fn another_account_finds_nothing_of_the_one_before_in_the_window() {
    let (mut rt, script) = window();
    let harbour = id(board::HARBOUR);
    rt.state.photos.check_tile(0, 0);
    rt.state.photos.open(harbour);
    rt.frame(0.0);
    press(&mut rt, Key::Delete, false);
    answer(&mut rt, &script, Answered::Trashed(vec![(harbour, None)]));
    assert!(rt.state.photos.toast.is_some() && rt.state.photos.viewer.is_some());
    let before = script.lock().unwrap().forgot;

    // Another account is here (the shell's session says so; the poll sees it).
    crate::shell::host::dispatch(
        &mut rt.state,
        Event::Session(crate::shell::app::Session::of(
            crate::shell::app::GateState::Ready,
            "someone-else",
        )),
    );
    rt.frame(0.0);
    let ui = &mut rt.state.photos;
    assert!(script.lock().unwrap().forgot > before, "the source was told to forget");
    assert!(ui.viewer.is_none() && ui.toast.is_none() && ui.selected.is_empty() && !ui.selecting);
    assert!(ui.menu.is_none() && ui.dialog.is_none() && ui.filter.is_none());
    // Undo has nothing to put back: nothing of the first account's is asked for under the second.
    let asks = asked(&script).len();
    assert!(!ui.undo());
    assert_eq!(asked(&script).len(), asks);
}

#[test]
fn what_was_left_is_counted_and_reasoned_by_what_asking_again_can_change() {
    let (mut rt, script) = window();
    let (a, b, c) = (id((0, 0)), id((0, 3)), id((1, 2)));
    for (day, index) in [(0, 0), (0, 3), (1, 2)] {
        rt.state.photos.check_tile(day, index);
    }
    rt.frame(0.0);
    click(&mut rt, "ph-bulk-btn-3");
    // One is protected, two could not be reached: the message is about the two, and Try again is for them.
    answer(
        &mut rt,
        &script,
        Answered::Trashed(vec![
            (a, Some(Left::Protected)),
            (b, Some(Left::Offline)),
            (c, Some(Left::Offline)),
        ]),
    );
    assert_eq!(
        text(&rt, "ph-toast-text"),
        "Couldn\u{2019}t move 2 to Trash \u{00B7} you\u{2019}re offline"
    );
    click(&mut rt, "ph-toast-action");
    assert_eq!(asked(&script)[1], ("trash", vec![b, c]));
    // Only protected ones left: said of all of them, and there is nothing to try again.
    answer(
        &mut rt,
        &script,
        Answered::Trashed(vec![(b, Some(Left::Protected)), (c, Some(Left::Protected))]),
    );
    assert_eq!(
        text(&rt, "ph-toast-text"),
        "Couldn\u{2019}t move 2 to Trash \u{00B7} backups can\u{2019}t be changed here"
    );
    assert!(!has(&rt, "ph-toast-action"));
}

#[test]
fn a_control_whose_work_is_not_connected_says_so_and_what_the_source_can_do_is_done() {
    let (mut rt, script) = window();
    let harbour = id(board::HARBOUR);
    rt.state.photos.open(harbour);
    rt.frame(0.0);
    // Original: no download is started, and the tab does not say one runs.
    rt.state.photos.show_original();
    rt.frame(0.0);
    rt.frame(0.0);
    assert_eq!(rt.state.photos.toast, Some(Toast::NotYet));
    assert_eq!(text(&rt, "ph-toast-text"), "Not available in this version yet");
    assert_eq!(
        rt.state.photos.viewer.as_ref().map(|v| v.original),
        Some(super::super::Original::Preview)
    );
    assert_eq!(
        rt.state.photos.take_intents(),
        vec![],
        "nothing waits for a shell that never takes it"
    );

    // "On this PC", pressed on a kept photo: it stops being kept, asked of the source.
    answer(
        &mut rt,
        &script,
        Answered::Kept {
            id: harbour,
            keep: true,
            left: None,
        },
    );
    rt.state.photos.press_offline();
    rt.frame(0.0);
    assert_eq!(asked(&script), [("stop keeping", vec![harbour])]);

    // A video: the line does not say it streams, and an unknown length is not written as 0:00.
    script.lock().unwrap().no_length = true;
    rt.state.photos.close_viewer();
    rt.state.photos.open(id(board::VIDEO));
    redraw(&mut rt, &script);
    assert!(
        rt.elements().iter().any(|e| e
            .text
            .as_deref()
            .is_some_and(|t| t.contains("playing it here is not available"))),
        "the open video's line says what is"
    );
    assert!(
        !rt.elements()
            .iter()
            .any(|e| e.text.as_deref().is_some_and(|t| t.contains("Streaming")))
    );
    assert!(!rt.elements().iter().any(|e| e.text.as_deref() == Some("0:00")));
    rt.state.photos.close_viewer();
    redraw(&mut rt, &script);
    assert!(has(&rt, "ph-tile-0-2-badge-icon"), "the video's mark is there");
    assert!(!has(&rt, "ph-tile-0-2-badge-label"), "without a length");

    // Keep on a selection: asked per photo.
    rt.state.photos.check_tile(0, 3);
    rt.state.photos.check_tile(0, 5);
    rt.frame(0.0);
    click(&mut rt, "ph-bulk-btn-1");
    // The next poll takes what the press asked for.
    rt.frame(0.0);
    assert_eq!(
        asked(&script)[1..],
        [("keep", vec![id((0, 3))]), ("keep", vec![id((0, 5))])]
    );
}

#[test]
fn a_checked_photo_that_left_the_library_is_no_longer_checked_and_a_finished_check_reads_again() {
    let (mut rt, script) = window();
    rt.state.photos.check_tile(0, 0);
    rt.state.photos.check_tile(0, 3);
    rt.frame(0.0);
    // It left some other way (moved to Trash on the phone): the next read no longer lists it, and nothing answers.
    script.lock().unwrap().leaves = vec![id((0, 0))];
    rt.frame(0.0);
    rt.frame(0.0);
    assert_eq!(
        rt.state.photos.selected.keys().copied().collect::<Vec<_>>(),
        [id((0, 3))]
    );

    let before = script.lock().unwrap().refreshed;
    super::after_a_check(&mut rt.state);
    assert_eq!(script.lock().unwrap().refreshed, before + 1);

    // On screen with nothing read and nothing on its way: the library is asked for again.
    let opened = script.lock().unwrap().opened;
    {
        let mut script = script.lock().unwrap();
        script.loaded = Loaded::Loading;
        script.dirty = true;
    }
    rt.frame(0.0);
    assert!(script.lock().unwrap().opened > opened);
}

#[test]
fn while_the_session_gate_is_closed_no_change_is_asked_of_the_source() {
    let (mut rt, script) = window();
    let harbour = id(board::HARBOUR);
    rt.state.photos.check_tile(0, 0);
    rt.state.photos.later.undo = vec![harbour];
    // The shell's gate closed (the session ended) and no poll has run since.
    rt.state.photos.later.closed = true;
    rt.state.photos.trash_selected();
    assert!(matches!(
        rt.state.photos.toast,
        Some(Toast::Left {
            what: What::Trash,
            why: Left::Unavailable,
            ..
        })
    ));
    assert!(rt.state.photos.undo(), "said, not done");
    rt.state.photos.open(harbour);
    rt.state.photos.press_offline();
    assert_eq!(asked(&script), [], "the source heard of none of the three");
    // The next poll asks the gate again: it is open, and a press is asked of the source.
    rt.frame(0.0);
    rt.state.photos.press_offline();
    assert_eq!(asked(&script), [("keep", vec![harbour])]);
}

#[test]
fn keep_several_failures_are_one_message_with_the_count() {
    let (mut rt, script) = window();
    let (a, b) = (id((0, 0)), id((0, 3)));
    rt.state.photos.check_tile(0, 0);
    rt.state.photos.check_tile(0, 3);
    rt.frame(0.0);
    click(&mut rt, "ph-bulk-btn-1");
    // The press is dispatched in the click's last frame; the window's poll takes the intent in the next one.
    rt.frame(0.0);
    assert_eq!(asked(&script), [("keep", vec![a]), ("keep", vec![b])]);
    assert!(rt.state.photos.toast.is_none(), "asked, not answered: nothing is said");

    // Both left: ONE message that counts them, not one that the second overwrites.
    for left in [a, b] {
        script.lock().unwrap().answers.push(Answered::Kept {
            id: left,
            keep: true,
            left: Some(Left::Failed),
        });
    }
    rt.frame(0.0);
    rt.frame(0.0);
    assert_eq!(
        rt.state.photos.toast,
        Some(Toast::Left {
            what: What::Keep(true),
            why: Left::Failed,
            count: 2,
            again: vec![a, b]
        })
    );
    assert_eq!(text(&rt, "ph-toast-text"), "Couldn\u{2019}t keep 2 on this PC");
}

// ---- Add a folder… and Move… (CP2 8.1), Remove (8.2) ----

fn folder_item(id: &str, name: &str) -> crate::source::DriveItem {
    crate::source::DriveItem {
        id: id.to_string(),
        parent_id: None,
        name: name.to_string(),
        is_folder: true,
        size: None,
        child_count: Some(3),
        modified_at: String::new(),
        mime: None,
        has_thumbnail: false,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: None,
        is_protected: false,
    }
}

/// The folder dialog of Files as Photos opened it, with `docs` offered and chosen.
fn picker(rt: &mut Runtime<State>) -> Option<super::super::super::files::dialogs::Purpose> {
    use super::super::super::files::dialogs::Dialog as Files;
    rt.frame(0.0);
    rt.frame(0.0);
    match &mut rt.state.actions.dialog {
        Some(Files::Move(m)) => {
            m.choices = vec![folder_item("docs-id", "Documents")];
            m.picked = Some(0);
            Some(m.purpose)
        }
        _ => None,
    }
}

#[test]
fn add_a_folder_opens_the_folder_dialog_and_hands_the_chosen_folder_to_the_source() {
    use super::super::super::files::dialogs::Purpose;
    let (mut rt, script) = window();
    rt.state.photos.open_sources();
    rt.state.photos.ask(super::super::Intent::AddFolder);
    assert_eq!(picker(&mut rt), Some(Purpose::PhotosFolder));
    assert!(
        rt.state.photos.toast.is_none(),
        "no \"not available\" for a control that works"
    );
    click(&mut rt, "btn-0");
    assert_eq!(script.lock().unwrap().folders_added, ["docs-id"]);
    assert!(rt.state.actions.dialog.is_none(), "the folder dialog closed");
}

#[test]
fn add_a_folder_does_not_add_the_root_of_files() {
    let (mut rt, script) = window();
    rt.state.photos.ask(super::super::Intent::AddFolder);
    rt.frame(0.0);
    rt.frame(0.0);
    assert!(rt.state.actions.dialog.is_some(), "the dialog is open");
    // Nothing chosen: the dialog shows the root, and "Add" is not offered there.
    if let Some(super::super::super::files::dialogs::Dialog::Move(m)) = &mut rt.state.actions.dialog {
        m.picked = None;
    }
    click(&mut rt, "btn-0");
    assert_eq!(script.lock().unwrap().folders_added, Vec::<String>::new());
}

#[test]
fn add_a_folder_the_source_refuses_is_said() {
    let (mut rt, script) = window();
    script.lock().unwrap().refuse_folder = true;
    rt.state.photos.ask(super::super::Intent::AddFolder);
    picker(&mut rt);
    click(&mut rt, "btn-0");
    assert!(matches!(rt.state.photos.toast, Some(Toast::Words(_))));
    assert!(text(&rt, "ph-toast-text").starts_with("Couldn\u{2019}t add that folder"));
}

#[test]
fn move_from_photos_opens_the_folder_dialog_with_the_files_of_the_photos() {
    use super::super::super::files::dialogs::{Dialog as Files, Purpose};
    let (mut rt, script) = window();
    script.lock().unwrap().items = vec![folder_item("photo-file", "Harbour.jpg")];
    rt.state.photos.check_tile(0, 0);
    rt.frame(0.0);
    click(&mut rt, "ph-bulk-btn-2");
    assert_eq!(picker(&mut rt), Some(Purpose::PhotosMove));
    assert!(rt.state.photos.toast.is_none());
    match &rt.state.actions.dialog {
        Some(Files::Move(m)) => assert_eq!(m.items.len(), 1),
        _ => panic!("no Move dialog"),
    }
}

#[test]
fn move_from_photos_with_no_file_behind_the_photos_says_so() {
    let (mut rt, _script) = window();
    rt.state.photos.check_tile(0, 0);
    rt.frame(0.0);
    click(&mut rt, "ph-bulk-btn-2");
    rt.frame(0.0);
    assert!(rt.state.actions.dialog.is_none());
    assert!(matches!(rt.state.photos.toast, Some(Toast::Words(_))));
    assert!(text(&rt, "ph-toast-text").starts_with("Couldn\u{2019}t move"));
}

#[test]
fn remove_on_a_folder_row_takes_the_folder_out_of_the_sources() {
    let (mut rt, script) = window();
    rt.state.photos.open_sources();
    rt.settle_all();
    rt.frame(0.0);
    assert!(has(&rt, "ph-item-1-0-remove"), "a folder row has Remove");
    click(&mut rt, "ph-item-1-0-remove");
    assert_eq!(script.lock().unwrap().folders_removed, [SourceRef::Folder(0)]);
}

#[test]
fn a_folder_change_the_engine_refused_later_is_said_once() {
    let (mut rt, script) = window();
    assert!(rt.state.photos.toast.is_none());
    script.lock().unwrap().folder_refused = true;
    rt.frame(0.0);
    rt.frame(0.0);
    assert_eq!(
        rt.state.photos.toast,
        Some(Toast::Words("Couldn\u{2019}t change that folder"))
    );
    assert_eq!(text(&rt, "ph-toast-text"), "Couldn\u{2019}t change that folder");
}

fn opacity(rt: &Runtime<State>, key: &str) -> f32 {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
        .opacity
}

#[test]
fn a_folder_rows_remove_shows_on_hover_and_on_keyboard_focus_only() {
    let (mut rt, _script) = window();
    rt.state.photos.open_sources();
    rt.settle_all();
    rt.frame(0.0);
    let key = "ph-item-1-0-remove";
    // At rest: not drawn, still a button the keyboard reaches.
    assert_eq!(opacity(&rt, key), 0.0, "Remove is hidden at rest");
    let info = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key))
        .unwrap();
    assert!(info.focusable && info.role == "button", "Remove stays focusable");
    // Pointer over the row.
    let row = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("ph-item-1-0"))
        .unwrap()
        .rect;
    rt.pointer_move(row[0] + 20.0, row[1] + row[3] / 2.0);
    rt.settle_all();
    rt.frame(0.0);
    assert_eq!(opacity(&rt, key), 1.0, "Remove shows under the pointer");
    // Pointer away, then Tab to it: it shows on focus.
    rt.pointer_move(1.0, 1.0);
    rt.frame(0.0);
    rt.frame(0.5);
    rt.settle_all();
    rt.frame(0.0);
    assert_eq!(opacity(&rt, key), 0.0, "Remove hides again");
    for _ in 0..40 {
        if rt.elements().iter().any(|e| e.focused && e.key.as_deref() == Some(key)) {
            break;
        }
        press(&mut rt, Key::Tab, false);
    }
    rt.settle_all();
    rt.frame(0.0);
    assert!(
        rt.elements().iter().any(|e| e.focused && e.key.as_deref() == Some(key)),
        "Tab reaches Remove"
    );
    assert_eq!(opacity(&rt, key), 1.0, "Remove shows on keyboard focus");
}

#[test]
fn move_is_refused_while_the_photos_move_to_trash_is_on_its_way() {
    let (mut rt, script) = window();
    script.lock().unwrap().items = vec![folder_item("photo-file", "Harbour.jpg")];
    let a = id((0, 0));
    rt.state.photos.check_tile(0, 0);
    rt.frame(0.0);
    click(&mut rt, "ph-bulk-btn-3");
    assert_eq!(asked(&script), [("trash", vec![a])]);
    // Move… while the Trash request runs: said, no folder dialog.
    click(&mut rt, "ph-bulk-btn-2");
    rt.frame(0.0);
    rt.frame(0.0);
    assert!(
        rt.state.actions.dialog.is_none(),
        "no Move dialog while Trash is on its way"
    );
    assert!(
        matches!(rt.state.photos.toast, Some(Toast::Words(w)) if w == "Couldn\u{2019}t move. The last change is still on its way."),
        "the refusal names the Move that did not happen and says the last change is still on its way"
    );
    // Beebeeb answered (refused): the photo is movable again.
    answer(&mut rt, &script, Answered::Trashed(vec![(a, Some(Left::Failed))]));
    rt.state.photos.toast = None;
    rt.state.photos.check_tile(0, 0);
    rt.frame(0.0);
    click(&mut rt, "ph-bulk-btn-2");
    assert_eq!(
        picker(&mut rt),
        Some(super::super::super::files::dialogs::Purpose::PhotosMove)
    );
}

/// A finished check brings a change from another device: a photo leaves the library above the tile the keys are on
/// and the checked ones. Each stays with its PHOTO (an index would name the next one); the one that left is dropped.
#[test]
fn a_finished_check_keeps_the_cursor_and_the_checked_photos_with_their_photos() {
    let (mut rt, script) = window();
    rt.state.photos.check_tile(0, 3);
    rt.state.photos.check_tile(0, 5);
    rt.state.photos.cursor = Some((0, 4));
    assert_eq!(rt.state.photos.cursor, Some((0, 4)));
    let (cursor_id, kept) = (id((0, 4)), [id((0, 3)), id((0, 5))]);
    rt.frame(0.0);

    script.lock().unwrap().leave_on_refresh = vec![id((0, 0))];
    super::after_a_check(&mut rt.state);
    rt.frame(0.0);
    rt.frame(0.0);

    let ui = &rt.state.photos;
    assert_eq!(
        ui.selected.keys().copied().collect::<Vec<_>>(),
        kept,
        "the checked photos stay checked"
    );
    let at = ui.cursor.expect("the cursor stays");
    assert_eq!(
        ui.source.photo(ui.filter, at.0, at.1).map(|p| p.id),
        Some(cursor_id),
        "the cursor is on its photo: {at:?}"
    );
    assert_eq!(at, (0, 3), "one photo left above it");
}

/// The read of a finished check waits while a menu is open over the grid, and runs on the first poll after it closed.
#[test]
fn a_finished_check_waits_while_a_menu_is_open_and_reads_once_it_closed() {
    let (mut rt, script) = window();
    rt.state.photos.menu = Some(crate::frames::photos::OpenMenu {
        which: crate::frames::photos::Menu::Across,
        hot: None,
    });
    let before = script.lock().unwrap().refreshed;
    super::after_a_check(&mut rt.state);
    rt.frame(0.0);
    assert_eq!(
        script.lock().unwrap().refreshed,
        before,
        "nothing is read under an open menu"
    );
    rt.state.photos.menu = None;
    rt.frame(0.0);
    assert_eq!(
        script.lock().unwrap().refreshed,
        before + 1,
        "read once, after it closed"
    );
    rt.frame(0.0);
    assert_eq!(script.lock().unwrap().refreshed, before + 1, "and only once");
}

#[test]
fn what_session_does_not_handle_goes_to_one_route_per_feature() {
    let (mut rt, _script) = window();
    let harbour = id(board::HARBOUR);
    super::super::route::reached_clear();
    rt.state.photos.ask(super::super::Intent::Duplicate(harbour));
    rt.frame(0.0);
    rt.frame(0.0);
    assert_eq!(super::super::route::reached(), ["route_items"]);
    assert_eq!(rt.state.photos.toast, Some(Toast::NotYet), "a stub's Err says so as before");

    // The tab is no longer reset inside session.rs: the route for the original owns it.
    super::super::route::reached_clear();
    rt.state.photos.open(harbour);
    rt.frame(0.0);
    rt.state.photos.show_original();
    rt.frame(0.0);
    rt.frame(0.0);
    assert_eq!(super::super::route::reached(), ["route_original"]);
}
