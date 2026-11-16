//! A video in Photos plays only the item the window started (slice M6): the stage is live for the held item and for
//! no other, and Space on a video starts it through the source. Written without a build (the media plan's code-only
//! batch); first seen at the media checkpoint.

use std::sync::{Arc, Mutex};

use rune::Pixels;

use super::super::Ui;
use super::super::source::{
    BackupStatus, Day, Detail, Device, FixturePhotos, Folder, Kind, Left, Photo, PhotoId, PhotosSource, Shape,
    SourceRef, Totals, Viewing, board,
};
use super::live_for;
use crate::media::audio_out::AudioError;
use crate::media::player::{
    AudioFactory, AudioSink, Link, LinkFactory, MediaBytes, MediaOpener, PlayerHandle, Refusal,
};
use crate::media::video::VideoSurface;

struct NoOpener;
impl MediaOpener for NoOpener {
    fn open(&self, _file_id: &str) -> Result<Box<dyn MediaBytes>, Refusal> {
        Err(Refusal::CouldNotPlay)
    }
}

struct NoLinks;
impl LinkFactory for NoLinks {
    fn start(&self) -> Result<Box<dyn Link>, Refusal> {
        Err(Refusal::NotInstalled)
    }
}

struct NoAudio;
impl AudioFactory for NoAudio {
    fn start(&self, _rate_hint: u32) -> Result<Box<dyn AudioSink>, AudioError> {
        Err(AudioError::NoDevice)
    }
}

/// A player whose seams answer nothing (the way `engine_host/photos_video_tests.rs` builds one).
fn a_player() -> PlayerHandle {
    PlayerHandle::new(
        Arc::new(NoOpener),
        Arc::new(NoLinks),
        Arc::new(NoAudio),
        None,
        VideoSurface::new(),
    )
}

fn video_id() -> PhotoId {
    PhotoId((board::VIDEO.0 * 1000 + board::VIDEO.1) as u64)
}

/// The board's library, and a `play_video` that records what it was asked and hands a player.
struct Recording {
    library: FixturePhotos,
    asked: Arc<Mutex<Vec<PhotoId>>>,
}

impl PhotosSource for Recording {
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
    fn play_video(&mut self, id: PhotoId) -> Option<PlayerHandle> {
        self.asked.lock().unwrap().push(id);
        Some(a_player())
    }
}

/// The stage is live for the video the window holds and for no other: no held video, or another item's, draws
/// the item's own controls. Needs no global and no test order.
#[test]
fn a_video_the_player_does_not_hold_draws_its_own_controls() {
    let shown = video_id();
    let other = PhotoId(shown.0 + 1);
    assert!(live_for(shown, None).is_none(), "nothing held: the item's own controls");
    let held = (other, a_player());
    assert!(
        live_for(shown, Some(&held)).is_none(),
        "another item's player is never drawn"
    );
    let held = (shown, a_player());
    assert!(live_for(shown, Some(&held)).is_some(), "the held item shows live");
}

/// Space on an open video asks the source to play it and holds the player it hands; at the base nothing called
/// `play_video`.
#[test]
fn photos_space_starts_the_video_it_shows() {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let mut ui = Ui::new(Box::new(Recording {
        library: FixturePhotos::new(),
        asked: asked.clone(),
    }));
    ui.open(video_id());
    assert!(
        ui.viewing().is_some_and(|v| matches!(v.kind, Kind::Video(_))),
        "the board's video is open"
    );
    assert!(ui.playing.is_none(), "opening starts nothing");
    ui.play_pause();
    assert_eq!(
        *asked.lock().unwrap(),
        vec![video_id()],
        "the source was asked for this video once"
    );
    assert!(
        ui.playing.as_ref().is_some_and(|(id, _)| *id == video_id()),
        "the window holds the player it was handed"
    );
    // Leaving the item lets the player go.
    ui.close_viewer();
    assert!(ui.playing.is_none(), "closing the viewer lets it go");
}
