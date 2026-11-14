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
    opened: usize,
    forgot: usize,
    refreshed: usize,
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
        self.script.lock().unwrap().refreshed += 1;
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
