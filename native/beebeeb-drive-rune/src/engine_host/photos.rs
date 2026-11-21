//! Photos on the real engine: [`EnginePhotos`], the `PhotosSource` of the running app.
//!
//! Where each thing comes from:
//! - the library (days, photos, totals, the open photo, details, kept): [`Engine::photos_library`], the photos and
//!   videos of the signed-in account's LOCAL STATE (what this PC knows of the account, as Files shows it), read
//!   through the account-bound handle. A day is the local date of a file's modified time: the capture time is
//!   inside the encrypted file.
//! - devices: the device folders under the Backups root that hold a photo (`LibraryPhoto::device`). Whether one
//!   shows is kept here, in memory, for the session (the engine has no setting for it).
//! - thumbnails: the shared store (`engine_host/thumbs.rs`), in memory only.
//! - Move to Trash and Undo: the Files window's calls ([`TrashCalls`]: `Engine::trash_items_for` /
//!   `restore_items_for`), bound to the session and the account of the press; Undo sends the stamp of the move.
//! - Keep on this PC: [`Engine::set_kept_for`], for the session of the press.
//! - Photo sources' folders and the devices' backup line: `folders`, a folder's `set_shown`, `add_folder` and
//!   `remove_folder`, and `backup_status` (`photos_folders.rs`; the engine keeps the folders from Files the account
//!   added). The window's folder dialog (Files' Move-to dialog, in its choose-only mode) hands the chosen folder to
//!   `add_folder`.
//! - Move…: `file_items` hands the files behind the photos to Files' folder dialog, which moves them. A refused
//!   folder change is told to the window once (`folder_refused`).
//! - not bound (the PR lists why): the window's controls whose work no source method carries (Original,
//!   playing a video, Share, Rename, Search, Version history) say "not available in this version yet"
//!   (`frames/photos/session.rs`).
//!
//! Nothing is drawn before the engine answered: a change is only asked in the call; what it did arrives through
//! [`PhotosSource::pump`], per item. A photo Beebeeb moved to Trash leaves the library here at that answer (the
//! local state follows later), and is put back at the answer of its Undo.
//!
//! When it reads: when the Photos section opens and the library is missing or older than [`FRESH_FOR`], after a
//! change was answered, and when a sync check finished (`PhotosSource::refresh`). Never from a draw call: every accessor only reads what is here.
//!
//! Session rule: the library carries the session it was read for; every accessor compares it with
//! [`Engine::session_turn`] before it hands out a name or an id, and `forget` (the window's poll, on a gate or
//! account change) clears everything. An answer that lands under another session or account is dropped. A
//! `PhotoId` is a number this source hands out per file id; a number is never handed out twice, so one held across
//! an account change names nothing.
//!
//! Thread rule: the read and each change run on a worker of their own and send their answer down one channel,
//! taken in by `pump`; the worker wakes the window ([`Engine::wake_host`]).
//!
//! Must not: log a name, an id or a path.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, mpsc};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::photos_api::{LibraryPhoto, ThumbCut};
use beebeeb_engine::app::{CredentialStores, Engine, LocalStateRefused};
use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone};
use rune::Pixels;

use super::files_trash::TrashCalls;
use super::log;
use super::stamp::{Stamp, stamp};
use super::thumbs::EngineThumbStore;
use crate::frames::files::source::{Action, ActionError, Change};
use crate::frames::photos::source::{
    Answered, BackupStatus, Day, Detail, Device, DeviceKind, Folder, Kind, Left, Loaded, Photo, PhotoId, PhotosSource,
    OriginalAnswer, Pumped, Shape, SourceRef, StripItem, Totals, Viewing,
};
use crate::engine_host::viewer::viewer_picture;
use crate::media::StopReason;
use crate::media::player::{MediaItem, PlayerHandle, Press, Refusal};
use crate::source::DriveItem;
use beebeeb_media_protocol::KindHint;

#[cfg(test)]
#[path = "photos_tests.rs"]
mod tests;
// plan40 batch 1: Photo sources' folders and the devices' backup line (slice B1).
#[path = "photos_folders.rs"]
mod photos_folders;

/// Whether this PC can play video: Windows, with the helper next to the app.
fn playback_here() -> bool {
    cfg!(windows) && crate::media::transport::helper_exe().is_some()
}

/// A library this recent is not read again when the Photos section opens.
pub const FRESH_FOR: Duration = Duration::from_secs(30);

/// How many neighbours the strip under an open photo shows on each side.
const STRIP_REACH: usize = 4;

/// One day of one view of the library: its title and the places of its photos in `Library::photos`.
struct DayOf {
    title: String,
    items: Vec<usize>,
}

/// The library as it was read, grouped once: the accessors only index it.
struct Library {
    session: SessionGeneration,
    /// Newest first.
    photos: Vec<LibraryPhoto>,
    /// The number handed out for each photo, by place.
    ids: Vec<PhotoId>,
    /// The place of a number.
    place: HashMap<PhotoId, usize>,
    /// The days of what shows (hidden devices left out).
    all: Vec<DayOf>,
    /// The days of each device, by `devices` place.
    by_device: Vec<Vec<DayOf>>,
    /// The places of what shows, in order: the open photo's neighbours and its position.
    shown: Vec<usize>,
}

/// Never the photos: they hold decrypted names.
impl std::fmt::Debug for Library {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Library")
            .field("session", &self.session)
            .field("photos", &self.photos.len())
            .finish_non_exhaustive()
    }
}

enum Answer {
    /// The read with this number, for the stamp it was started under.
    Read(
        u64,
        Stamp,
        Result<(SessionGeneration, Vec<LibraryPhoto>), LocalStateRefused>,
    ),
    /// Keep on this PC, or stop: the file, what was asked, the stamp of the press, and whether it was done.
    Kept(String, bool, Stamp, Result<(), ()>),
    /// The original with this job number has this many bytes of this many.
    OriginalProgress(u64, u64, u64),
    /// The original with this job number: decoded, or not.
    OriginalDone(u64, Result<Pixels, ()>),
}

/// The original that is downloading for the open photo (one at a time), or is held decoded: memory only.
struct OriginalJob {
    number: u64,
    file_id: String,
    pressed: Stamp,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    /// `Some` once it is decoded.
    pixels: Option<Pixels>,
}

/// w31: why the original of the photo `name` is never downloaded for the Original tab: the decoder here opens PNG,
/// JPEG, GIF and WebP only (`viewer_picture::check`), so a HEIC or a RAW came down whole (2 MB to 80 MB each on
/// the owner's account) and was then refused. `None`: a format that opens here.
pub(crate) fn original_refused_for(name: &str) -> Option<&'static str> {
    use super::viewer::{Shows, shows};
    if shows(name) == Shows::Picture {
        return None;
    }
    let ext = name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default();
    Some(match ext.as_str() {
        "heic" | "heif" => crate::frames::viewers::refusal_text::HEIC_WORDS,
        "dng" | "cr2" | "cr3" | "arw" | "nef" | "raf" | "orf" | "rw2" => crate::frames::viewers::refusal_text::RAW_WORDS,
        _ => "This format can\u{2019}t be shown at full size on this PC yet. This is the preview.",
    })
}

/// An original over this is not downloaded into memory.
const ORIGINAL_MAX: u64 = 256 * 1024 * 1024;

/// What a test's download answers in the engine's place: calls `progress` as it goes, then hands back the bytes.
#[cfg(test)]
type FakeFetch = Box<dyn FnOnce(&(dyn Fn(u64, u64) + Send + Sync), &std::sync::atomic::AtomicBool) -> Result<Vec<u8>, ()> + Send>;

/// The picture decoders of this source, started with the first original.
fn decoders() -> &'static rune::ImageCache {
    static DECODERS: std::sync::OnceLock<rune::ImageCache> = std::sync::OnceLock::new();
    DECODERS.get_or_init(|| rune::ImageCache::new(Arc::new(|| {})))
}

/// The bytes as pixels, on this (a worker's) thread. What the file declares is held to the viewer's limits first.
/// The decoder reads a copy of the bytes; it is overwritten once the decoder let go of it.
fn decode(bytes: &[u8]) -> Option<Pixels> {
    use zeroize::Zeroize;
    viewer_picture::check(bytes).ok()?;
    let mut encoded: Arc<[u8]> = Arc::from(bytes);
    let image = decoders().load(encoded.clone());
    let decoded = image.wait().ok().map(|[width, height]| {
        let pixels = image.pixels().clone();
        for _ in 0..2_000 {
            if pixels.size() == (width, height) {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        pixels
    });
    for _ in 0..2_000 {
        if let Some(copy) = Arc::get_mut(&mut encoded) {
            copy.zeroize();
            return decoded;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    log("photos: a decoder kept the original's bytes");
    decoded
}

/// One Keep for the worker: the file, what was asked and the stamp of the press.
struct KeepJob {
    file_id: String,
    keep: bool,
    pressed: Stamp,
    #[cfg(test)]
    says: Option<Result<(), ()>>,
}

/// Photos on the real engine. See the module's doc.
pub struct EnginePhotos<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    thumbs: Arc<EngineThumbStore<C>>,
    trash: TrashCalls<C>,
    answers: mpsc::Sender<Answer>,
    inbox: mpsc::Receiver<Answer>,
    /// The session and account the last look saw: when they move, everything held goes.
    seen: Option<Stamp>,
    library: Option<Library>,
    /// What the view draws while there is no library.
    loaded: Loaded,
    /// When the last read was answered with a library (a failure never stamps it).
    read_at: Option<Instant>,
    /// The read on its way, by number.
    reading: Option<u64>,
    reads: u64,
    /// A read was asked for while one was on its way: start it when that one lands.
    again: bool,
    /// The Photos section was opened in this session: a change elsewhere is worth a read.
    asked: bool,
    /// A read with a number up to this began before a change was answered: its library is not taken.
    stale_up_to: u64,
    /// The numbers handed out, by file id. A number is never handed out twice (`next_id` only grows).
    numbers: HashMap<String, PhotoId>,
    next_id: u64,
    /// The devices seen in this session, in the order they were first seen (a place never moves), and whether
    /// each shows.
    devices: Vec<(String, bool)>,
    /// The photos this source moved to Trash in this session, as they were: what Undo puts back.
    gone: HashMap<String, LibraryPhoto>,
    /// Moved to Trash by this source and still listed by the local state (it follows later): left out of a read
    /// until one no longer lists them.
    hidden: HashSet<String>,
    /// The photos Beebeeb put back: added to a read that does not list them yet.
    back: HashMap<String, LibraryPhoto>,
    /// The files a Keep is on its way for, each with the session of its press.
    keeping: HashMap<String, SessionGeneration>,
    /// The one worker that does the Keeps, in the order they were pressed (started at the first press).
    keep_jobs: Option<mpsc::Sender<KeepJob>>,
    /// How many Keep workers were started (a selection of many is ONE).
    #[cfg(test)]
    keep_workers: usize,
    /// The photos a move to Trash was pressed for, as they were at the press: what Undo names even when a library
    /// read in between no longer lists one. Dropped as the answer comes.
    offered: HashMap<String, LibraryPhoto>,
    /// The original of the open photo: downloading or held (`download_original`).
    original: Option<OriginalJob>,
    original_answers: Vec<OriginalAnswer>,
    originals: u64,
    /// Tests only: what the next original's download answers.
    #[cfg(test)]
    fetch_says: Option<FakeFetch>,
    /// What the changes did and the window has not taken yet.
    answered: Vec<Answered>,
    /// Something changed outside an answer: the next pump says so.
    dirty: bool,
    /// Tests only: what the next Keep answers in the engine's place (this PC's sync folder is not a test's to set).
    #[cfg(test)]
    keep_says: Option<Result<(), ()>>,
    /// Slice B1 (`photos_folders.rs`): what it keeps between a press and its answer: the held folder list and the calls on their way.
    photos_folders: photos_folders::PhotoFolderCalls,
}

impl<C: CredentialStores> EnginePhotos<C> {
    /// Starts no thread and reads nothing.
    pub fn new(engine: Arc<Engine<C>>, thumbs: Arc<EngineThumbStore<C>>) -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            trash: TrashCalls::new(engine.clone()),
            engine,
            thumbs,
            answers,
            inbox,
            seen: None,
            library: None,
            loaded: Loaded::SignedOut,
            read_at: None,
            reading: None,
            reads: 0,
            again: false,
            asked: false,
            stale_up_to: 0,
            numbers: HashMap::new(),
            next_id: 1,
            devices: Vec::new(),
            gone: HashMap::new(),
            hidden: HashSet::new(),
            back: HashMap::new(),
            keeping: HashMap::new(),
            keep_jobs: None,
            #[cfg(test)]
            keep_workers: 0,
            original: None,
            original_answers: Vec::new(),
            originals: 0,
            #[cfg(test)]
            fetch_says: None,
            offered: HashMap::new(),
            answered: Vec::new(),
            dirty: false,
            #[cfg(test)]
            keep_says: None,
            photos_folders: Default::default(),
        }
    }

    /// The session or the account is not the one the last look saw: nothing of the earlier one is kept.
    fn look(&mut self, now: &Stamp) {
        if self.seen.as_ref() == Some(now) {
            return;
        }
        self.seen = Some(now.clone());
        self.drop_everything();
    }

    fn drop_everything(&mut self) {
        self.library = None;
        self.loaded = if self.engine.has_session() {
            Loaded::Loading
        } else {
            Loaded::SignedOut
        };
        self.read_at = None;
        self.reading = None;
        self.again = false;
        self.asked = false;
        self.stale_up_to = self.reads;
        // The numbers go; `next_id` stays, so none of them is ever handed out again.
        self.numbers.clear();
        self.devices.clear();
        self.gone.clear();
        self.hidden.clear();
        self.back.clear();
        self.keeping.clear();
        self.end_original();
        self.original_answers.clear();
        self.offered.clear();
        self.answered.clear();
        self.thumbs.forget();
        self.dirty = true;
    }

    /// The library, if it is the session's that is in memory now.
    fn current(&self) -> Option<&Library> {
        self.library
            .as_ref()
            .filter(|library| self.engine.has_session() && library.session == self.engine.session_turn())
    }

    fn days_of(&self, device: Option<usize>) -> Option<&Vec<DayOf>> {
        let library = self.current()?;
        match device {
            None => Some(&library.all),
            Some(place) => library.by_device.get(place),
        }
    }

    /// The file id behind a number, in the library that is here.
    fn file_of(&self, id: PhotoId) -> Option<&LibraryPhoto> {
        let library = self.current()?;
        library.photos.get(*library.place.get(&id)?)
    }

    fn number_of(&mut self, file_id: &str) -> PhotoId {
        if let Some(id) = self.numbers.get(file_id) {
            return *id;
        }
        let id = PhotoId(self.next_id);
        self.next_id += 1;
        self.numbers.insert(file_id.to_string(), id);
        id
    }

    /// Starts a read, or notes that one is wanted when the one on its way lands.
    fn read(&mut self) {
        if self.reading.is_some() {
            self.again = true;
            return;
        }
        let pressed = stamp(&self.engine);
        self.look(&pressed);
        if !self.engine.has_session() {
            self.loaded = Loaded::SignedOut;
            return;
        }
        self.reads += 1;
        let number = self.reads;
        let (engine, answers) = (self.engine.clone(), self.answers.clone());
        let started = std::thread::Builder::new().name("photos-read".into()).spawn(move || {
            let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                engine.photos_library().map(|library| (library.session, library.photos))
            }))
            .unwrap_or(Err(LocalStateRefused::Unreadable));
            let _ = answers.send(Answer::Read(number, pressed, read));
            engine.wake_host();
        });
        if started.is_err() {
            log("photos: could not start the read");
            self.loaded = Loaded::Failed;
            return;
        }
        self.reading = Some(number);
        if self.library.is_none() {
            self.loaded = Loaded::Loading;
        }
    }

    /// Takes what a read answered in: the local state follows a move to Trash and its Undo only later, so what
    /// this source moved to Trash is left out while the local state still lists it, and what Beebeeb put back is
    /// added while the local state does not list it yet.
    fn take_read(&mut self, session: SessionGeneration, mut photos: Vec<LibraryPhoto>) {
        // The local state caught up with these: from now on it alone says whether they are there.
        self.hidden.retain(|id| photos.iter().any(|photo| photo.id == *id));
        self.back.retain(|id, _| !photos.iter().any(|photo| photo.id == *id));
        photos.retain(|photo| !self.hidden.contains(&photo.id));
        photos.extend(self.back.values().cloned());
        // A file that changed since the last read: its pictures are the earlier version's.
        if let Some(before) = &self.library {
            let was: HashMap<&str, (i64, i64, u64)> = before
                .photos
                .iter()
                .map(|photo| (photo.id.as_str(), (photo.modified_at, photo.versions, photo.bytes)))
                .collect();
            for photo in &photos {
                if was
                    .get(photo.id.as_str())
                    .is_some_and(|was| *was != (photo.modified_at, photo.versions, photo.bytes))
                {
                    self.thumbs.drop_file(&photo.id);
                }
            }
        }
        self.take_in(session, photos);
    }

    /// Takes `photos` in as the library of `session`, and cuts the days once.
    fn take_in(&mut self, session: SessionGeneration, mut photos: Vec<LibraryPhoto>) {
        photos.sort_by(|a, b| {
            b.modified_at
                .cmp(&a.modified_at)
                .then_with(|| (&a.folders, &a.name).cmp(&(&b.folders, &b.name)))
        });
        for photo in &photos {
            if let Some(device) = &photo.device
                && !self.devices.iter().any(|(name, _)| name == device)
            {
                self.devices.push((device.clone(), true));
            }
        }
        let ids: Vec<PhotoId> = photos.iter().map(|photo| self.number_of(&photo.id)).collect();
        self.library = Some(group(session, photos, ids, &self.devices, Local::now().date_naive()));
        self.loaded = Loaded::Ready;
        self.dirty = true;
    }

    /// The days again, from the library that is here (a device was shown or hidden, a photo left or came back).
    fn regroup(&mut self) {
        if let Some(library) = self.library.take() {
            self.library = Some(group(
                library.session,
                library.photos,
                library.ids,
                &self.devices,
                Local::now().date_naive(),
            ));
            self.dirty = true;
        }
    }

    /// What the moves to and from Trash answered: the photos Beebeeb moved leave the library, the ones it put back
    /// stand in it again, and the window is told per item.
    fn take_trash_answers(&mut self) {
        for change in self.trash.take() {
            let Change::Trash { action, items } = change else {
                continue;
            };
            let mut said = Vec::new();
            let items_ids: Vec<String> = items.iter().map(|item| item.id.clone()).collect();
            for item in items {
                let Some(id) = self.numbers.get(&item.id).copied() else {
                    continue;
                };
                if item.left.is_none() {
                    if action == Action::Trash {
                        // In the library now, or as it was at the press: a read in between may not list it.
                        let pressed = self.offered.get(&item.id).cloned();
                        let photo = self
                            .library
                            .as_mut()
                            .and_then(|library| {
                                let at = library.photos.iter().position(|photo| photo.id == item.id)?;
                                library.ids.remove(at);
                                Some(library.photos.remove(at))
                            })
                            .or(pressed);
                        if let Some(photo) = photo {
                            self.hidden.insert(item.id.clone());
                            self.back.remove(&item.id);
                            self.gone.insert(item.id.clone(), photo);
                        }
                    } else if let Some(photo) = self.gone.remove(&item.id) {
                        self.hidden.remove(&item.id);
                        if let Some(library) = &mut self.library {
                            library.photos.push(photo.clone());
                            library.ids.push(id);
                        }
                        self.back.insert(item.id.clone(), photo);
                    }
                }
                said.push((id, item.left.map(left_of)));
            }
            for item in &items_ids {
                self.offered.remove(item);
            }
            // A read that began before this answer may not know of it.
            self.stale_up_to = self.reads;
            if let Some(library) = self.library.take() {
                let session = library.session;
                self.take_in(session, library.photos);
            }
            if !said.is_empty() {
                self.answered.push(if action == Action::Trash {
                    Answered::Trashed(said)
                } else {
                    Answered::Restored(said)
                });
            }
            self.read();
        }
    }

    /// Opens the video `id` in `player` for the press made now. `None`: not a video, no playback on this PC, the
    /// player refused, or the session or the account moved while it opened (the player is then stopped: a late
    /// open under another session is dropped). `between` runs after the open and before that check (tests only).
    fn play_with(
        &mut self,
        id: PhotoId,
        player: PlayerHandle,
        playable: bool,
        open: impl FnOnce(&PlayerHandle, MediaItem) -> Result<(), Refusal>,
        between: impl FnOnce(),
    ) -> Option<PlayerHandle> {
        // The session and account of the press, read before anything else.
        let pressed = stamp(&self.engine);
        self.look(&pressed);
        if !playable || !self.engine.has_session() {
            return None;
        }
        let file = self.file_of(id)?;
        if !file.video {
            return None;
        }
        // The press goes WITH the item: the player opens the video for this session of this account only.
        let item = MediaItem {
            file_id: file.id.clone(),
            kind: KindHint::Video,
            length_ms: u64::from(self.thumbs.length(&file.id).unwrap_or(0)) * 1000,
            press: Press {
                session: pressed.0,
                account: pressed.1.clone(),
            },
        };
        let opened = open(&player, item);
        between();
        if opened.is_err() {
            log("photos: the video could not be opened");
            return None;
        }
        if stamp(&self.engine) != pressed {
            log("photos: the open of an earlier session was dropped");
            player.stop(StopReason::AccountChanged);
            return None;
        }
        Some(player)
    }

    /// The worker of the Keeps: it takes the jobs in order, asks the engine for each under the session of its press
    /// (the engine decides whether that is still the session) and answers each; a panic answers as failed.
    /// Stops the original's download and wipes what is held of it.
    fn end_original(&mut self) {
        if let Some(job) = self.original.take() {
            job.cancel.store(true, Ordering::SeqCst);
        }
    }

    fn start_keep_worker(&mut self) -> Result<mpsc::Sender<KeepJob>, ()> {
        let (jobs, queue) = mpsc::channel::<KeepJob>();
        let (engine, answers) = (self.engine.clone(), self.answers.clone());
        std::thread::Builder::new()
            .name("photos-keep".into())
            .spawn(move || {
                for job in queue {
                    let KeepJob {
                        file_id,
                        keep,
                        pressed,
                        #[cfg(test)]
                        says,
                    } = job;
                    let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        #[cfg(test)]
                        if let Some(says) = says {
                            return says;
                        }
                        engine.set_kept_for(pressed.0, &file_id, keep).map_err(|_| ())
                    }))
                    .unwrap_or(Err(()));
                    let _ = answers.send(Answer::Kept(file_id, keep, pressed, done));
                    engine.wake_host();
                }
            })
            .map_err(|_| ())?;
        #[cfg(test)]
        {
            self.keep_workers += 1;
        }
        Ok(jobs)
    }

    fn rows(&self, ids: &[PhotoId]) -> Option<Vec<DriveItem>> {
        ids.iter()
            .map(|id| {
                // In the library, or moved to Trash by this source (Undo).
                let file_id = self.file_of(*id).map(|photo| photo.id.clone()).or_else(|| {
                    self.numbers
                        .iter()
                        .find(|(file_id, number)| *number == id && self.gone.contains_key(*file_id))
                        .map(|(file_id, _)| file_id.clone())
                })?;
                Some(row_of(file_id))
            })
            .collect()
    }
}

/// A row for the Files window's Trash calls, which read its id alone.
fn row_of(id: String) -> DriveItem {
    DriveItem {
        id,
        parent_id: None,
        name: String::new(),
        is_folder: false,
        size: None,
        child_count: None,
        modified_at: String::new(),
        mime: None,
        has_thumbnail: false,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: None,
        is_protected: false,
    }
}

fn left_of(why: ActionError) -> Left {
    match why {
        ActionError::Protected => Left::Protected,
        ActionError::Offline => Left::Offline,
        ActionError::Unavailable => Left::Unavailable,
        ActionError::InFlight => Left::InFlight,
        _ => Left::Failed,
    }
}

fn loaded_of(refused: LocalStateRefused) -> Loaded {
    match refused {
        LocalStateRefused::SessionEnded => Loaded::SessionEnded,
        LocalStateRefused::NothingSynced => Loaded::NothingSynced,
        LocalStateRefused::SignedOut | LocalStateRefused::Locked | LocalStateRefused::SessionChanged => {
            Loaded::SignedOut
        }
        LocalStateRefused::Unreadable => Loaded::Failed,
        refused if refused.not_this_accounts() => Loaded::NotThisAccounts,
        _ => Loaded::Failed,
    }
}

fn local(at: i64) -> DateTime<Local> {
    Local
        .timestamp_opt(at, 0)
        .single()
        .unwrap_or_else(|| Local.timestamp_opt(0, 0).single().unwrap_or_else(Local::now))
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// A day's title: "Today", "Yesterday", the weekday within the last week, else "12 October" (with the year when it
/// is not this one).
pub(super) fn day_title(day: NaiveDate, today: NaiveDate) -> String {
    let ago = (today - day).num_days();
    match ago {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        2..=6 => day.format("%A").to_string(),
        _ => {
            let month = MONTHS[day.month0() as usize];
            if day.year() == today.year() {
                format!("{} {month}", day.day())
            } else {
                format!("{} {month} {}", day.day(), day.year())
            }
        }
    }
}

/// `places` (newest first) cut into days.
fn days(photos: &[LibraryPhoto], places: impl Iterator<Item = usize>, today: NaiveDate) -> Vec<DayOf> {
    let mut out: Vec<(NaiveDate, DayOf)> = Vec::new();
    for place in places {
        let day = local(photos[place].modified_at).date_naive();
        match out.last_mut() {
            Some((last, of)) if *last == day => of.items.push(place),
            _ => out.push((
                day,
                DayOf {
                    title: day_title(day, today),
                    items: vec![place],
                },
            )),
        }
    }
    out.into_iter().map(|(_, of)| of).collect()
}

fn group(
    session: SessionGeneration,
    photos: Vec<LibraryPhoto>,
    ids: Vec<PhotoId>,
    devices: &[(String, bool)],
    today: NaiveDate,
) -> Library {
    let hidden = |photo: &LibraryPhoto| {
        photo
            .device
            .as_ref()
            .is_some_and(|device| devices.iter().any(|(name, shown)| name == device && !shown))
    };
    let shown: Vec<usize> = (0..photos.len()).filter(|&place| !hidden(&photos[place])).collect();
    let all = days(&photos, shown.iter().copied(), today);
    let by_device = devices
        .iter()
        .map(|(name, _)| {
            let of = (0..photos.len()).filter(|&place| photos[place].device.as_deref() == Some(name.as_str()));
            days(&photos, of, today)
        })
        .collect();
    let place = ids.iter().enumerate().map(|(place, id)| (*id, place)).collect();
    Library {
        session,
        photos,
        ids,
        place,
        all,
        by_device,
        shown,
    }
}

/// "A, B and C".
fn names(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_string(),
        [most @ .., last] => format!("{} and {last}", most.join(", ")),
    }
}

fn megabytes(bytes: u64) -> String {
    crate::source::format_size(i64::try_from(bytes).unwrap_or(i64::MAX))
}

impl<C: CredentialStores> PhotosSource for EnginePhotos<C> {
    fn wired(&self) -> bool {
        true
    }

    fn answers_later(&self) -> bool {
        true
    }

    fn loaded(&self) -> Loaded {
        if !self.engine.has_session() {
            return Loaded::SignedOut;
        }
        match self.current() {
            Some(_) => Loaded::Ready,
            // Signed in and nothing read yet for this session (or what was read is an earlier session's).
            None if matches!(self.loaded, Loaded::Ready | Loaded::SignedOut) => Loaded::Loading,
            None => self.loaded,
        }
    }

    fn opened(&mut self) {
        let now = stamp(&self.engine);
        self.look(&now);
        self.asked = true;
        let fresh = self.current().is_some() && self.read_at.is_some_and(|at| at.elapsed() < FRESH_FOR);
        if !fresh {
            self.read();
        }
    }

    fn refresh(&mut self) {
        let now = stamp(&self.engine);
        self.look(&now);
        // Only a library someone looked at in this session is read again; the next opening reads otherwise.
        if self.asked {
            self.read();
        } else {
            self.read_at = None;
        }
    }

    fn forget(&mut self) {
        self.seen = Some(stamp(&self.engine));
        self.drop_everything();
    }

    fn pump(&mut self) -> Pumped {
        let now = stamp(&self.engine);
        self.look(&now);
        let here = self.engine.has_session();
        let mut progress: Option<(u64, u64, u64)> = None;
        while let Ok(answer) = self.inbox.try_recv() {
            match answer {
                Answer::OriginalProgress(number, done, total) => {
                    if self.original.as_ref().is_some_and(|job| job.number == number && job.pixels.is_none()) {
                        progress = Some((number, done, total));
                    }
                }
                Answer::OriginalDone(number, decoded) => {
                    let Some(job) = self.original.as_mut().filter(|job| job.number == number) else {
                        continue;
                    };
                    let id = self.numbers.get(&job.file_id).copied();
                    match (decoded, id) {
                        (Ok(pixels), Some(id)) => {
                            job.pixels = Some(pixels);
                            progress = None;
                            self.original_answers.push(OriginalAnswer::Done(id));
                        }
                        (_, id) => {
                            self.original = None;
                            if let Some(id) = id {
                                self.original_answers.push(OriginalAnswer::Failed(id));
                            }
                        }
                    }
                }
                Answer::Read(number, pressed, read) => {
                    if self.reading == Some(number) {
                        self.reading = None;
                    }
                    // Only for the session AND the account it was started under, and only while they are here.
                    if !(here && pressed == now) {
                        continue;
                    }
                    match read {
                        // A read that began before a change was answered does not know of it: read again.
                        Ok(_) if number <= self.stale_up_to => self.again = true,
                        Ok((session, photos)) if session == now.0 => {
                            self.take_read(session, photos);
                            self.read_at = Some(Instant::now());
                        }
                        Ok(_) => {}
                        // A failed read never stamps freshness; a library that is here stays.
                        Err(refused) => {
                            if self.library.is_none() {
                                self.loaded = loaded_of(refused);
                                self.dirty = true;
                            }
                        }
                    }
                }
                Answer::Kept(file_id, keep, pressed, done) => {
                    if self.keeping.get(&file_id) == Some(&pressed.0) {
                        self.keeping.remove(&file_id);
                    }
                    if !(here && pressed == now) {
                        log("photos: the answer of an earlier session was dropped");
                        continue;
                    }
                    let Some(id) = self.numbers.get(&file_id).copied() else {
                        continue;
                    };
                    if done.is_ok()
                        && let Some(photo) = self
                            .library
                            .as_mut()
                            .and_then(|library| library.photos.iter_mut().find(|photo| photo.id == file_id))
                    {
                        photo.kept = keep;
                    }
                    self.answered.push(Answered::Kept {
                        id,
                        keep,
                        left: done.err().map(|()| Left::Failed),
                    });
                    self.stale_up_to = self.reads;
                    self.again = true;
                }
            }
        }
        if let Some((number, done, total)) = progress
            && let Some(id) = self
                .original
                .as_ref()
                .filter(|job| job.number == number)
                .and_then(|job| self.numbers.get(&job.file_id).copied())
        {
            self.original_answers.push(OriginalAnswer::Progress { id, done, total });
        }
        self.take_trash_answers();
        if self.again && self.reading.is_none() {
            self.again = false;
            self.read();
        }
        let arrived = self.thumbs.take_arrived();
        Pumped {
            changed: std::mem::take(&mut self.dirty) || arrived,
            busy: self.reading.is_some()
                || !self.keeping.is_empty()
                || self.thumbs.busy()
                || self.original.as_ref().is_some_and(|job| job.pixels.is_none()),
            answered: std::mem::take(&mut self.answered),
            originals: std::mem::take(&mut self.original_answers),
        }
    }

    fn devices(&self) -> Vec<Device> {
        let Some(library) = self.current() else {
            return Vec::new();
        };
        self.devices
            .iter()
            .map(|(name, shown)| {
                let lower = name.to_lowercase();
                Device {
                    kind: if lower.contains("ipad") || lower.contains("tablet") {
                        DeviceKind::Tablet
                    } else {
                        DeviceKind::Phone
                    },
                    photos: library
                        .photos
                        .iter()
                        .filter(|photo| photo.device.as_deref() == Some(name.as_str()))
                        .count() as u64,
                    name: name.clone(),
                    shown: *shown,
                }
            })
            .collect()
    }

    /// The folders from Files the account added, as the engine last answered (slice K4, `photos_folders.rs`).
    fn folders(&self) -> Vec<Folder> {
        self.photos_folders_folders()
    }

    fn folder_refused(&mut self) -> bool {
        self.photos_folders_refused()
    }

    /// The files behind the photos, for Move…: each with its name (the Files window's move reads the id, and the
    /// name for "already used there"). A photo that is not in the library: none for all.
    fn file_items(&self, ids: &[PhotoId]) -> Option<Vec<DriveItem>> {
        ids.iter()
            .map(|id| {
                let photo = self.file_of(*id)?;
                let mut row = row_of(photo.id.clone());
                row.name = photo.name.clone();
                Some(row)
            })
            .collect()
    }

    /// The photo's file in the Beebeeb folder, while sync is on and the engine has a path for it (a device backup
    /// has none). The same check as every hand-over of a file to another app.
    fn kept_path(&self, id: PhotoId) -> Option<std::path::PathBuf> {
        let file_id = self.file_of(id)?.id.clone();
        if !self.engine.is_syncing() {
            return None;
        }
        let root = self.engine.sync_root().ok()?;
        self.engine
            .local_state_for_changes()
            .ok()?
            .with(|db| super::path_the_system_opens(db, &root, &file_id).ok())
            .ok()
            .flatten()
    }

    fn add_folder(&mut self, folder_id: &str) -> bool {
        self.photos_folders_add(folder_id)
    }

    fn remove_folder(&mut self, source: SourceRef) {
        self.photos_folders_remove(source)
    }

    fn set_shown(&mut self, source: SourceRef, shown: bool) {
        let place = match source {
            SourceRef::Device(place) => place,
            // A folder from Files: asked of the engine (slice K4, `photos_folders.rs`).
            SourceRef::Folder(index) => return self.photos_folders_set_shown(index, shown),
        };
        if self.current().is_none() {
            return;
        }
        if let Some(device) = self.devices.get_mut(place) {
            device.1 = shown;
            self.regroup();
        }
    }

    fn totals(&self, device: Option<usize>) -> Totals {
        let (Some(library), Some(days)) = (self.current(), self.days_of(device)) else {
            return Totals::default();
        };
        let mut totals = Totals::default();
        for place in days.iter().flat_map(|day| day.items.iter()) {
            let photo = &library.photos[*place];
            if photo.video {
                totals.videos += 1;
            } else {
                totals.photos += 1;
            }
            totals.bytes += photo.bytes;
        }
        totals
    }

    /// The newest backup pass of the other devices (slice B1, `photos_folders.rs`).
    fn backup_status(&self) -> Option<BackupStatus> {
        self.photos_folders_backup_status()
    }

    fn origin(&self) -> String {
        let Some(library) = self.current() else {
            return String::new();
        };
        let shown: Vec<&str> = self
            .devices
            .iter()
            .filter(|(_, shown)| *shown)
            .map(|(name, _)| name.as_str())
            .collect();
        let others = library
            .shown
            .iter()
            .filter(|place| library.photos[**place].device.is_none())
            .count();
        let from_files = match others {
            0 => "none from your other folders".to_string(),
            n => format!("{n} from your other folders"),
        };
        if shown.is_empty() {
            return match others {
                0 => String::new(),
                _ => "Photos and videos from your folders".into(),
            };
        }
        format!("Photos from {} \u{00B7} {from_files}", names(&shown))
    }

    fn days(&self, device: Option<usize>) -> Vec<Day> {
        self.days_of(device)
            .map(|days| {
                days.iter()
                    .map(|day| Day {
                        title: day.title.clone(),
                        len: day.items.len(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn photo(&self, device: Option<usize>, day: usize, index: usize) -> Option<Photo> {
        let library = self.current()?;
        let place = *self.days_of(device)?.get(day)?.items.get(index)?;
        let photo = &library.photos[place];
        Some(Photo {
            id: library.ids[place],
            kind: if photo.video {
                // The length, where a copy on this PC says it.
                Kind::Video(self.thumbs.length(&photo.id).unwrap_or(0))
            } else {
                Kind::Photo
            },
            uploading: photo.uploading,
            bytes: photo.bytes,
        })
    }

    fn thumbnail(&self, id: PhotoId, shape: Shape) -> Option<Pixels> {
        let photo = self.file_of(id)?;
        let cut = match shape {
            Shape::Square => ThumbCut::Square,
            Shape::Strip => ThumbCut::Strip,
            Shape::Preview => ThumbCut::Whole,
        };
        self.thumbs.picture(&photo.id, cut)
    }

    fn viewing(&self, id: PhotoId) -> Option<Viewing> {
        let library = self.current()?;
        let place = *library.place.get(&id)?;
        let photo = &library.photos[place];
        let taken = local(photo.modified_at);
        let today = Local::now().date_naive();
        // Its place among what shows; a photo of a hidden device is shown alone.
        let at = library.shown.iter().position(|shown| *shown == place);
        let strip = match at {
            Some(at) => {
                let from = at.saturating_sub(STRIP_REACH);
                let to = (at + STRIP_REACH + 1).min(library.shown.len());
                library.shown[from..to]
                    .iter()
                    .map(|other| StripItem {
                        id: library.ids[*other],
                        current: *other == place,
                    })
                    .collect()
            }
            None => vec![StripItem { id, current: true }],
        };
        Some(Viewing {
            id,
            kind: if photo.video {
                Kind::Video(self.thumbs.length(&photo.id).unwrap_or(0))
            } else {
                Kind::Photo
            },
            taken: format!("{} {}", day_title(taken.date_naive(), today), taken.format("%H:%M")),
            // A photo outside Backups: it is in the account's own folders.
            device: photo.device.clone().unwrap_or_else(|| "Files".into()),
            name: photo.name.clone(),
            // The stored picture's format is not known here; it is shown as it was stored.
            preview_format: "as stored".into(),
            original_bytes: photo.bytes,
            position: (
                at.map_or(1, |at| at as u64 + 1),
                at.map_or(1, |_| library.shown.len() as u64),
            ),
            strip,
            played_ms: 0,
        })
    }

    fn details(&self, id: PhotoId) -> Vec<Detail> {
        let Some(photo) = self.file_of(id) else {
            return Vec::new();
        };
        let line = |label, value: String, mono| Detail {
            label,
            value,
            mono,
            // Version history is not bound from Photos: the line says the number and opens nothing.
            opens: false,
        };
        let taken = local(photo.modified_at);
        let mut lines = vec![line(
            "Changed",
            format!(
                "{}, {}",
                day_title(taken.date_naive(), Local::now().date_naive()),
                taken.format("%H:%M")
            ),
            true,
        )];
        if let Some(device) = &photo.device {
            lines.push(line("From", device.clone(), false));
        }
        lines.push(line("Size", megabytes(photo.bytes), true));
        if !photo.folders.is_empty() {
            lines.push(line("Where", photo.folders.join(" / "), false));
        }
        lines.push(line(
            "Version history",
            match photo.versions {
                1 => "1 version".into(),
                n => format!("{n} versions"),
            },
            false,
        ));
        lines
    }

    fn kept_offline(&self, id: PhotoId) -> bool {
        self.file_of(id).is_some_and(|photo| photo.kept)
    }

    fn keep_offline(&mut self, id: PhotoId, keep: bool) -> Result<(), Left> {
        // The session and account of the press, read before anything else.
        let pressed = stamp(&self.engine);
        self.look(&pressed);
        if !self.engine.has_session() {
            return Err(Left::Unavailable);
        }
        let file_id = self
            .file_of(id)
            .map(|photo| photo.id.clone())
            .ok_or(Left::Unavailable)?;
        if self.keeping.contains_key(&file_id) {
            return Err(Left::InFlight);
        }
        let job = KeepJob {
            file_id: file_id.clone(),
            keep,
            pressed: pressed.clone(),
            #[cfg(test)]
            says: self.keep_says.take(),
        };
        // One worker does every Keep, in the order pressed: a selection of many is one job, not one thread each.
        let mut job = Some(job);
        let mut sent = false;
        for _ in 0..2 {
            if self.keep_jobs.is_none() {
                match self.start_keep_worker() {
                    Ok(jobs) => self.keep_jobs = Some(jobs),
                    Err(()) => break,
                }
            }
            let Some(jobs) = self.keep_jobs.as_ref() else {
                break;
            };
            match jobs.send(job.take().expect("a job that was not sent")) {
                Ok(()) => {
                    sent = true;
                    break;
                }
                // The worker is gone: the next try starts another.
                Err(mpsc::SendError(back)) => {
                    job = Some(back);
                    self.keep_jobs = None;
                }
            }
        }
        if !sent {
            log("photos: could not start the worker");
            return Err(Left::Failed);
        }
        self.keeping.insert(file_id, pressed.0);
        Ok(())
    }

    fn download_original(&mut self, id: PhotoId) -> bool {
        let pressed = stamp(&self.engine);
        self.look(&pressed);
        if !self.engine.has_session() {
            return false;
        }
        let Some(photo) = self.file_of(id) else { return false };
        if photo.video || photo.bytes > ORIGINAL_MAX || original_refused_for(&photo.name).is_some() {
            return false;
        }
        let file_id = photo.id.clone();
        // One original at a time: the earlier one is stopped and wiped.
        self.end_original();
        self.originals += 1;
        let number = self.originals;
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        #[cfg(test)]
        let fake = self.fetch_says.take();
        let (engine, answers, flag, wanted) = (self.engine.clone(), self.answers.clone(), cancel.clone(), file_id.clone());
        let started = std::thread::Builder::new().name("photos-original".into()).spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let reply = answers.clone();
                let progress = move |done: u64, total: u64| {
                    let _ = reply.send(Answer::OriginalProgress(number, done, total));
                    // Not the window's wake on every chunk: the poll takes the latest.
                };
                #[cfg(test)]
                if let Some(fake) = fake {
                    return fake(&progress, &flag).ok().and_then(|bytes| decode(&bytes)).ok_or(());
                }
                let opened = engine
                    .open_file_in_memory(&wanted, ORIGINAL_MAX, &progress, &flag)
                    .map_err(|_| ())?;
                if flag.load(Ordering::SeqCst) {
                    return Err(());
                }
                decode(&opened.bytes).ok_or(())
            }))
            .unwrap_or(Err(()));
            let _ = answers.send(Answer::OriginalDone(number, outcome));
            engine.wake_host();
        });
        if started.is_err() {
            log("photos: could not start the worker");
            return false;
        }
        self.original = Some(OriginalJob { number, file_id, pressed, cancel, pixels: None });
        true
    }

    fn original_refused(&self, id: PhotoId) -> Option<&'static str> {
        let photo = self.file_of(id)?;
        (!photo.video).then(|| original_refused_for(&photo.name)).flatten()
    }

    fn holds_unshowable(&self) -> bool {
        self.current().is_some_and(|library| {
            library.photos.iter().any(|photo| !photo.video && original_refused_for(&photo.name).is_some())
        })
    }

    fn stop_original(&mut self, _id: PhotoId) {
        self.end_original();
    }

    fn original(&self, id: PhotoId) -> Option<Pixels> {
        let job = self.original.as_ref()?;
        let held = job.pixels.as_ref()?;
        (self.numbers.get(&job.file_id) == Some(&id) && self.engine.has_session() && job.pressed == stamp(&self.engine))
            .then(|| held.clone())
    }

    fn play_video(&mut self, id: PhotoId) -> Option<crate::media::player::PlayerHandle> {
        let player = crate::media::shell_player()?;
        self.play_with(
            id,
            player,
            playback_here(),
            |player, item| player.open(item, Vec::new()),
            || {},
        )
    }

    fn trash(&mut self, ids: &[PhotoId]) -> Result<(), Left> {
        let now = stamp(&self.engine);
        self.look(&now);
        let rows = self.rows(ids).ok_or(Left::Unavailable)?;
        // What the press names, as it is now: Undo is for exactly these, whatever a read in between lists.
        let before: Vec<(String, Option<LibraryPhoto>)> = ids
            .iter()
            .zip(&rows)
            .map(|(id, row)| (row.id.clone(), self.file_of(*id).cloned()))
            .collect();
        let asked = self.trash.trash(&rows).map_err(left_of);
        if asked.is_ok() {
            for (file_id, photo) in before {
                if let Some(photo) = photo {
                    self.offered.insert(file_id, photo);
                }
            }
        }
        asked
    }

    fn restore(&mut self, ids: &[PhotoId]) -> Result<(), Left> {
        let now = stamp(&self.engine);
        self.look(&now);
        let rows = self.rows(ids).ok_or(Left::Unavailable)?;
        self.trash.restore(&rows).map_err(left_of)
    }
}
