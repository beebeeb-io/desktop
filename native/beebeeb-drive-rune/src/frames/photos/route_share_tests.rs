//! Photos' Share dialog, Copy link and Version history on the fake platform: Copy link with Week and a password asks
//! the source of Files for a link once, for that photo, with those choices; Share on a selection and Version
//! history open Files' own dialogs.

use std::sync::{Arc, Mutex};

use rune::{Env, Runtime};

use rune::Pixels;

use super::source::{
    BackupStatus, Day, Detail, Device, FixturePhotos, Folder, Left, Photo, PhotoId, PhotosSource, Shape, SourceRef,
    Totals, Viewing,
};
use super::{Dialog, Intent, Toast, Ui};
use crate::frames::files::actions_data::{LinkExpiry, ShareOptions};
use crate::frames::files::source::{Call, FilesFixture};
use crate::shell::app::{Event, Route, Section};
use crate::source::DriveItem;
use crate::{State, init, view};

const A: PhotoId = PhotoId(0);
const B: PhotoId = PhotoId(1);

struct Held {
    library: FixturePhotos,
    files: Vec<(PhotoId, DriveItem)>,
}

impl PhotosSource for Held {
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


fn file(id: &str, name: &str) -> DriveItem {
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
        is_protected: false,
    }
}

/// The library of the board, with files behind photos 0 and 1.
fn window() -> (Runtime<State>, Arc<Mutex<Vec<Call>>>) {
    let mut rt = Runtime::new(init(), view, (1278.0, 798.0), Env { system_dark: false, scale: 2.0 });
    rt.on_key = Some(crate::shell::host::on_key);
    rt.poll = Some(crate::shell::host::poll);
    rt.frame(0.0);
    rt.frame(0.0);
    crate::shell::host::dispatch(&mut rt.state, Event::Navigate(Route::section(Section::Photos)));
    rt.state.photos = Ui::new(Box::new(Held {
        library: FixturePhotos::new(),
        files: vec![(A, file("f-a", "harbour.jpg")), (B, file("f-b", "dock.jpg"))],
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

fn links(calls: &Arc<Mutex<Vec<Call>>>) -> Vec<(Vec<String>, ShareOptions)> {
    calls
        .lock()
        .unwrap()
        .iter()
        .filter_map(|c| match c {
            Call::ShareLink(ids, options) => Some((ids.clone(), options.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn copy_link_with_week_and_a_password_asks_once_with_those_choices_for_that_photo() {
    let (mut rt, calls) = window();
    rt.state.photos.open(A);
    rt.state.photos.open_share();
    settle(&mut rt);
    click(&mut rt, "ph-item-0-1-pop");
    click(&mut rt, "menu-0-item-1");
    click(&mut rt, "ph-item-0-2-switch");
    rt.state.photos.share_password = rune::TextEdit::new("hunter2").secure();
    settle(&mut rt);
    click(&mut rt, "ph-dlg-btn-1");
    settle(&mut rt);
    settle(&mut rt);
    assert_eq!(
        links(&calls),
        [(
            vec!["f-a".to_string()],
            ShareOptions { expires: LinkExpiry::Week, password: Some("hunter2".into()), ..Default::default() }
        )]
    );
    assert!(rt.state.photos.dialog.is_none());
    assert_eq!(rt.state.photos.toast, Some(Toast::Words("Link copied")));
    assert!(rt.state.photos.share_password.text.is_empty(), "the password is wiped once used");
}

#[test]
fn a_password_that_was_asked_for_and_not_typed_makes_no_link() {
    let (mut rt, calls) = window();
    rt.state.photos.open(A);
    rt.state.photos.open_share();
    settle(&mut rt);
    click(&mut rt, "ph-item-0-2-switch");
    click(&mut rt, "ph-dlg-btn-1");
    settle(&mut rt);
    assert!(links(&calls).is_empty());
    assert!(matches!(&rt.state.photos.dialog, Some(Dialog::Share(d)) if d.password_missing));
}

#[test]
fn share_several_and_version_history_open_files_dialogs() {
    let (mut rt, _calls) = window();
    rt.state.photos.ask(Intent::ShareSeveral(vec![A, B]));
    settle(&mut rt);
    assert!(matches!(&rt.state.actions.dialog,
        Some(crate::frames::files::dialogs::Dialog::Share(sh)) if sh.items.len() == 2));
    rt.state.actions.dialog = None;
    rt.state.photos.ask(Intent::VersionHistory(A));
    settle(&mut rt);
    assert!(matches!(&rt.state.actions.dialog,
        Some(crate::frames::files::dialogs::Dialog::Versions(item)) if item.id == "f-a"));
}
