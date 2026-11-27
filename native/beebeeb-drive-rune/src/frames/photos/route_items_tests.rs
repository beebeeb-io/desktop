//! Rename and Duplicate of Photos on the fake platform: a source of Files that notes its calls, and a library that
//! knows the files behind its photos. A name another file of the folder has is refused before the source is asked;
//! Duplicate asks once.

use std::sync::{Arc, Mutex};

use rune::{Env, Pixels, Runtime};

use super::super::source::{
    BackupStatus, Day, Detail, Device, FixturePhotos, Folder, Left, Photo, PhotoId, PhotosSource, Shape, SourceRef,
    Totals, Viewing,
};
use super::super::{Dialog, Intent, Toast, Ui};
use crate::frames::files::source::{Call, FilesFixture};
use crate::shell::app::{Event, Route, Section};
use crate::source::DriveItem;
use crate::{State, init, view};

/// The board's library, and the files behind the photos it was given.
struct Held {
    library: FixturePhotos,
    files: Vec<(PhotoId, DriveItem)>,
    /// w31: the library holds HEIC/RAW files; photo A is one.
    unshowable: bool,
}

fn file(id: &str, name: &str, protected: bool) -> DriveItem {
    DriveItem {
        id: id.into(),
        parent_id: Some("folder".into()),
        name: name.into(),
        is_folder: false,
        size: Some(10),
        child_count: None,
        modified_at: String::new(),
        mime: Some("image/jpeg".into()),
        has_thumbnail: true,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: Some("on_this_pc".into()),
        is_protected: protected,
    }
}

impl PhotosSource for Held {
    fn holds_unshowable(&self) -> bool {
        self.unshowable
    }
    fn original_refused(&self, id: PhotoId) -> Option<&'static str> {
        (self.unshowable && id == A).then_some("HEIC")
    }
    fn answers_later(&self) -> bool {
        true
    }
    fn file_items(&self, ids: &[PhotoId]) -> Option<Vec<DriveItem>> {
        ids.iter()
            .map(|id| self.files.iter().find(|(photo, _)| photo == id).map(|(_, item)| item.clone()))
            .collect()
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
        self.library.days(device)
    }
    fn photo(&self, device: Option<usize>, day: usize, index: usize) -> Option<Photo> {
        self.library.photo(device, day, index)
    }
    fn thumbnail(&self, id: PhotoId, shape: Shape) -> Option<Pixels> {
        self.library.thumbnail(id, shape)
    }
    fn viewing(&self, id: PhotoId) -> Option<Viewing> {
        self.library.viewing(id)
    }
    fn details(&self, id: PhotoId) -> Vec<Detail> {
        self.library.details(id)
    }
    fn kept_offline(&self, id: PhotoId) -> bool {
        self.library.kept_offline(id)
    }
    fn keep_offline(&mut self, id: PhotoId, keep: bool) -> Result<(), Left> {
        self.library.keep_offline(id, keep)
    }
    fn trash(&mut self, ids: &[PhotoId]) -> Result<(), Left> {
        self.library.trash(ids)
    }
    fn restore(&mut self, ids: &[PhotoId]) -> Result<(), Left> {
        self.library.restore(ids)
    }
}

const A: PhotoId = PhotoId(0);
const B: PhotoId = PhotoId(1);

/// A window on Photos whose photos 0 and 1 are "harbour.jpg" and "dock.jpg" of one folder; `protected` makes the
/// first one's file a protected one. The calls the source of Files was asked.
fn window(protected: bool) -> (Runtime<State>, Arc<Mutex<Vec<Call>>>) {
    window_with(protected, false)
}

fn window_with(protected: bool, unshowable: bool) -> (Runtime<State>, Arc<Mutex<Vec<Call>>>) {
    let mut rt = Runtime::new(init(), view, (1278.0, 798.0), Env { system_dark: false, scale: 2.0 });
    rt.on_key = Some(crate::shell::host::on_key);
    rt.poll = Some(crate::shell::host::poll);
    rt.frame(0.0);
    rt.frame(0.0);
    crate::shell::host::dispatch(&mut rt.state, Event::Navigate(Route::section(Section::Photos)));
    rt.state.photos = Ui::new(Box::new(Held {
        library: FixturePhotos::new(),
        files: vec![(A, file("f-a", "harbour.jpg", protected)), (B, file("f-b", "dock.jpg", false))],
        unshowable,
    }));
    let fixture = FilesFixture::default();
    let calls = fixture.calls.clone();
    rt.state.actions.source = Box::new(fixture);
    rt.frame(0.0);
    rt.frame(0.0);
    (rt, calls)
}

fn settle(rt: &mut Runtime<State>) {
    rt.frame(0.0);
    rt.frame(0.0);
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

fn type_name(rt: &mut Runtime<State>, name: &str) {
    match &mut rt.state.photos.dialog {
        Some(Dialog::Rename(draft)) => draft.edit.0 = rune::TextEdit::new(name.to_string()),
        other => panic!("no rename dialog: {other:?}"),
    }
    rt.frame(0.0);
}

fn renames(calls: &Arc<Mutex<Vec<Call>>>) -> Vec<(String, String)> {
    calls
        .lock()
        .unwrap()
        .iter()
        .filter_map(|c| match c {
            Call::Rename(id, name) => Some((id.clone(), name.clone())),
            _ => None,
        })
        .collect()
}

fn duplicates(calls: &Arc<Mutex<Vec<Call>>>) -> Vec<Vec<String>> {
    calls
        .lock()
        .unwrap()
        .iter()
        .filter_map(|c| match c {
            Call::Duplicate(ids) => Some(ids.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn rename_to_a_taken_name_is_refused_before_the_source_is_asked() {
    let (mut rt, calls) = window(false);
    rt.state.photos.ask(Intent::Rename(A));
    settle(&mut rt);
    assert!(matches!(rt.state.photos.dialog, Some(Dialog::Rename(_))), "the name dialog opened");

    // Another file of the folder has it, with other capitals: Files' rule.
    type_name(&mut rt, "DOCK.jpg");
    click(&mut rt, "ph-dlg-btn-0");
    assert!(renames(&calls).is_empty(), "the source was not asked");
    let Some(Dialog::Rename(draft)) = &rt.state.photos.dialog else {
        panic!("the dialog stays open");
    };
    assert!(draft.error_now().is_some(), "and says why");

    // A free name goes through, once.
    type_name(&mut rt, "sunset.jpg");
    click(&mut rt, "ph-dlg-btn-0");
    assert_eq!(renames(&calls), [("f-a".to_string(), "sunset.jpg".to_string())]);
    assert!(rt.state.photos.dialog.is_none());
    assert_eq!(rt.state.photos.toast, Some(Toast::Words("Renamed")));
}

#[test]
fn duplicate_asks_once() {
    let (mut rt, calls) = window(false);
    rt.state.photos.ask(Intent::Duplicate(A));
    settle(&mut rt);
    settle(&mut rt);
    assert_eq!(duplicates(&calls), [vec!["f-a".to_string()]]);
    assert!(matches!(rt.state.photos.toast, Some(Toast::Words(w)) if w.starts_with("Duplicated")));
}

#[test]
fn a_protected_place_says_so_and_nothing_is_asked() {
    let (mut rt, calls) = window(true);
    rt.state.photos.ask(Intent::Duplicate(A));
    settle(&mut rt);
    assert!(duplicates(&calls).is_empty());
    assert!(matches!(rt.state.photos.toast, Some(Toast::Words(w)) if w.contains("Couldn\u{2019}t duplicate")));

    rt.state.photos.ask(Intent::Rename(A));
    settle(&mut rt);
    assert!(rt.state.photos.dialog.is_none(), "no name dialog for it");
    assert!(matches!(rt.state.photos.toast, Some(Toast::Words(w)) if w.contains("Couldn\u{2019}t rename")));
}

#[test]
fn show_in_folder_reveals_the_photos_file_and_nothing_else() {
    let (mut rt, calls) = window(false);
    rt.state.photos.ask(Intent::ShowInFolder(A));
    settle(&mut rt);
    assert_eq!(*calls.lock().unwrap(), [Call::ShowInFileExplorer(vec!["f-a".to_string()])]);
    assert_eq!(rt.state.photos.toast, None);
    // A photo with no file on this PC: the line, no call.
    calls.lock().unwrap().clear();
    rt.state.photos.ask(Intent::ShowInFolder(PhotoId(99)));
    settle(&mut rt);
    assert!(calls.lock().unwrap().is_empty());
    assert!(matches!(rt.state.photos.toast, Some(Toast::Words(_))));
}

fn has(rt: &mut Runtime<State>, key: &str) -> bool {
    rt.settle_all();
    rt.frame(0.0);
    rt.elements().into_iter().any(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

#[test]
fn photos_says_heic_and_raw_cannot_be_shown_only_when_the_view_holds_such_files() {
    let (mut rt, _) = window_with(false, false);
    assert!(!has(&mut rt, "ph-heic-notice"));
    let (mut rt, _) = window_with(false, true);
    // Dm-87: a Mac decodes HEIC and RAW (dm-81), so the notice is never said there.
    assert_eq!(has(&mut rt, "ph-heic-notice"), !cfg!(target_os = "macos"));
}

/// Dm-87 (RC16): the notice is absent on macOS, also when the view holds HEIC and RAW photos; elsewhere it stays
/// until those systems decode them.
#[test]
fn the_heic_and_raw_notice_is_absent_on_macos() {
    use super::super::grid::{DECODES_HEIC_AND_RAW, says_unshowable};
    assert_eq!(DECODES_HEIC_AND_RAW, cfg!(target_os = "macos"));
    assert!(!says_unshowable(true, true), "a system that decodes them says nothing");
    assert!(says_unshowable(false, true));
    assert!(!says_unshowable(false, false));
    #[cfg(target_os = "macos")]
    {
        let (mut rt, _) = window_with(false, true);
        assert!(!has(&mut rt, "ph-heic-notice"));
    }
}

#[test]
fn the_open_heic_photo_has_the_notice_and_open_in_the_os_app_and_a_jpeg_has_neither() {
    let (mut rt, calls) = window_with(false, true);
    rt.state.photos.ask(Intent::OpenInOsApp(A));
    settle(&mut rt);
    // The shared open chain's outside steps: the default app; never a reveal-only call here.
    assert_eq!(*calls.lock().unwrap(), [Call::OpenDefault("f-a".to_string())]);

    rt.state.photos.viewer = Some(viewer(A));
    settle(&mut rt);
    assert!(has(&mut rt, "ph-viewer-notice"));
    assert!(has(&mut rt, "ph-open-in-os-app"));
    rt.state.photos.viewer = Some(viewer(B));
    settle(&mut rt);
    assert!(!has(&mut rt, "ph-viewer-notice"));
    assert!(!has(&mut rt, "ph-open-in-os-app"));
}

#[test]
fn a_pc_with_no_default_app_gets_the_file_selected_in_its_folder() {
    let (mut rt, calls) = window_with(false, true);
    let fixture = FilesFixture::default();
    fixture.no_default_app.store(true, std::sync::atomic::Ordering::SeqCst);
    let c = fixture.calls.clone();
    rt.state.actions.source = Box::new(fixture);
    rt.state.photos.ask(Intent::OpenInOsApp(A));
    settle(&mut rt);
    assert_eq!(*c.lock().unwrap(), [Call::OpenRevealed("f-a".to_string())]);
    let _ = calls;
}

fn viewer(id: PhotoId) -> super::super::Viewer {
    super::super::Viewer { id, original: super::super::Original::Preview, playing: false, gone: None }
}
