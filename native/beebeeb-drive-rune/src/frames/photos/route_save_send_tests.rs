//! Photos' Save to PC and Send a copy on the fake platform: Send a copy hands the photo's own path in the Beebeeb
//! folder to the share call and writes no other file; with no such path it is refused in words. Save to PC asks the
//! save panel for a place named as the photo is and copies through the saver.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use beebeeb_engine::account::SessionGeneration;
use rune::{Env, Pixels, Runtime};

use super::super::source::{
    BackupStatus, Day, Detail, Device, FixturePhotos, Folder, Left, Photo, PhotoId, PhotosSource, Shape, SourceRef,
    Totals, Viewing,
};
use super::super::{Toast, Ui};
use super::*;
use crate::engine_host::save_copy::{Progress, SaveRefused};
use crate::shell::platform::fake::FakePlatform;
use crate::shell::platform::SaveAnswer;
use crate::source::DriveItem;
use crate::{State, init, view};

/// The board's library, and the files behind the photos it was given.
struct Held {
    library: FixturePhotos,
    files: Vec<(PhotoId, DriveItem)>,
    kept: Option<PathBuf>,
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
    fn kept_path(&self, _id: PhotoId) -> Option<PathBuf> {
        self.kept.clone()
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

fn window(kept: Option<PathBuf>) -> Runtime<State> {
    let mut rt = Runtime::new(init(), view, (1278.0, 798.0), Env { system_dark: false, scale: 2.0 });
    rt.frame(0.0);
    rt.state.photos = Ui::new(Box::new(Held {
        library: FixturePhotos::new(),
        files: vec![(A, file("f-a", "harbour.jpg", false))],
        kept,
    }));
    rt
}

fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> =
        std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into()).collect();
    names.sort();
    names
}

/// Send a copy shares the photo's own file in the Beebeeb folder and writes no other file anywhere near it.
/// Mutant: copy the bytes to a temp file and share that -> `shared` holds another path / the folder gains a file.
#[test]
fn send_a_copy_shares_the_photos_own_path_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("harbour.jpg");
    std::fs::write(&path, b"photo").unwrap();
    let mut rt = window(Some(path.clone()));
    let platform = FakePlatform::default();
    let before = listing(dir.path());
    rt.update(|s, _| send_copy(s, &platform, A));
    assert_eq!(*platform.shared.lock().unwrap(), vec![path.clone()]);
    assert_eq!(listing(dir.path()), before);
    assert_eq!(std::fs::read(&path).unwrap(), b"photo");
    assert_eq!(rt.state.photos.toast, None);
}

/// Sync off, or a photo with no file in the Beebeeb folder: refused in words, nothing shared.
#[test]
fn send_a_copy_with_no_path_is_refused_in_words_and_shares_nothing() {
    let mut rt = window(None);
    let platform = FakePlatform::default();
    rt.update(|s, _| send_copy(s, &platform, A));
    assert!(platform.shared.lock().unwrap().is_empty());
    assert_eq!(rt.state.photos.toast, Some(Toast::Words(NEEDS_SYNC_ELSEWHERE)));
}

/// A share sheet that cannot open says so; no file is left behind.
#[test]
fn send_a_copy_says_so_when_the_sheet_cannot_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("harbour.jpg");
    std::fs::write(&path, b"photo").unwrap();
    let mut rt = window(Some(path));
    let platform = FakePlatform::unbuilt();
    rt.update(|s, _| send_copy(s, &platform, A));
    assert_eq!(rt.state.photos.toast, Some(Toast::Words(SHEET_FAILED)));
}

struct Saver {
    saved: Mutex<Vec<(String, PathBuf)>>,
}

impl CopySaver for Saver {
    fn turn(&self) -> SessionGeneration {
        SessionGeneration::for_test(1)
    }
    fn save(
        &self,
        _session: SessionGeneration,
        file_id: &str,
        dest: &Path,
        _progress: &Progress,
        _cancel: &AtomicBool,
    ) -> Result<(), SaveRefused> {
        self.saved.lock().unwrap().push((file_id.to_string(), dest.to_path_buf()));
        std::fs::write(dest, b"copy").map_err(|_| SaveRefused::Failed)
    }
}

/// Save to PC: the panel is asked with the photo's name, and the saver copies that file there.
#[test]
fn save_to_pc_asks_the_panel_with_the_photos_name_and_saves_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("harbour.jpg");
    let platform = Arc::new(FakePlatform::default());
    platform.saves.lock().unwrap().push_back(SaveAnswer::Chosen(dest.clone()));
    let saver = Arc::new(Saver { saved: Mutex::default() });
    let mut rt = window(None);
    rt.update(|s, tx| save_to_pc(s, tx, platform.clone(), saver.clone(), A));
    for _ in 0..200 {
        if !saver.saved.lock().unwrap().is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(platform.save_asks.lock().unwrap()[0].0, "harbour.jpg");
    assert_eq!(*saver.saved.lock().unwrap(), vec![("f-a".to_string(), dest.clone())]);
    assert_eq!(std::fs::read(&dest).unwrap(), b"copy");
    assert!(platform.shared.lock().unwrap().is_empty(), "Save to PC shares nothing");
}
