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
//! - not bound (the PR lists why): `folders` and a folder's `set_shown` (the engine has no "folder Photos shows"),
//!   `backup_status` (this PC does not know what another device's backup is doing).
//!
//! Nothing is drawn before the engine answered: a change is only asked in the call; what it did arrives through
//! [`PhotosSource::pump`], per item. A photo Beebeeb moved to Trash leaves the library here at that answer (the
//! local state follows later), and is put back at the answer of its Undo.
//!
//! When it reads: when the Photos section opens and the library is missing or older than [`FRESH_FOR`], and after
//! a change was answered. Never from a draw call: every accessor only reads what is here.
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
    Pumped, Shape, SourceRef, StripItem, Totals, Viewing,
};
use crate::source::DriveItem;

#[cfg(test)]
#[path = "photos_tests.rs"]
mod tests;

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
    /// What the changes did and the window has not taken yet.
    answered: Vec<Answered>,
    /// Something changed outside an answer: the next pump says so.
    dirty: bool,
    /// Tests only: what the next Keep answers in the engine's place (this PC's sync folder is not a test's to set).
    #[cfg(test)]
    keep_says: Option<Result<(), ()>>,
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
            stale_up_to: 0,
            numbers: HashMap::new(),
            next_id: 1,
            devices: Vec::new(),
            gone: HashMap::new(),
            hidden: HashSet::new(),
            back: HashMap::new(),
            keeping: HashMap::new(),
            answered: Vec::new(),
            dirty: false,
            #[cfg(test)]
            keep_says: None,
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
        self.stale_up_to = self.reads;
        // The numbers go; `next_id` stays, so none of them is ever handed out again.
        self.numbers.clear();
        self.devices.clear();
        self.gone.clear();
        self.hidden.clear();
        self.back.clear();
        self.keeping.clear();
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
            for item in items {
                let Some(id) = self.numbers.get(&item.id).copied() else {
                    continue;
                };
                if item.left.is_none() {
                    if action == Action::Trash {
                        let photo = self.library.as_mut().and_then(|library| {
                            let at = library.photos.iter().position(|photo| photo.id == item.id)?;
                            library.ids.remove(at);
                            Some(library.photos.remove(at))
                        });
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
        let fresh = self.current().is_some() && self.read_at.is_some_and(|at| at.elapsed() < FRESH_FOR);
        if !fresh {
            self.read();
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
        while let Ok(answer) = self.inbox.try_recv() {
            match answer {
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
        self.take_trash_answers();
        if self.again && self.reading.is_none() {
            self.again = false;
            self.read();
        }
        let arrived = self.thumbs.take_arrived();
        Pumped {
            changed: std::mem::take(&mut self.dirty) || arrived,
            busy: self.reading.is_some() || !self.keeping.is_empty() || self.thumbs.busy(),
            answered: std::mem::take(&mut self.answered),
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

    /// Not bound: the engine has no "folder from Files that Photos shows"; every photo and video of the account
    /// shows already.
    fn folders(&self) -> Vec<Folder> {
        Vec::new()
    }

    fn set_shown(&mut self, source: SourceRef, shown: bool) {
        let SourceRef::Device(place) = source else {
            return;
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

    /// Not bound: this PC does not know what another device's backup is doing now.
    fn backup_status(&self) -> Option<BackupStatus> {
        None
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
        let (engine, answers) = (self.engine.clone(), self.answers.clone());
        let (stamped, asked) = (pressed.clone(), file_id.clone());
        #[cfg(test)]
        let says = self.keep_says.take();
        let started = std::thread::Builder::new().name("photos-keep".into()).spawn(move || {
            // The engine decides whether the session is still the press's; a panic answers as failed.
            let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                #[cfg(test)]
                if let Some(says) = says {
                    return says;
                }
                engine.set_kept_for(stamped.0, &asked, keep).map_err(|_| ())
            }))
            .unwrap_or(Err(()));
            let _ = answers.send(Answer::Kept(asked, keep, stamped, done));
            engine.wake_host();
        });
        if started.is_err() {
            log("photos: could not start the worker");
            return Err(Left::Failed);
        }
        self.keeping.insert(file_id, pressed.0);
        Ok(())
    }

    fn trash(&mut self, ids: &[PhotoId]) -> Result<(), Left> {
        let now = stamp(&self.engine);
        self.look(&now);
        let rows = self.rows(ids).ok_or(Left::Unavailable)?;
        self.trash.trash(&rows).map_err(left_of)
    }

    fn restore(&mut self, ids: &[PhotoId]) -> Result<(), Left> {
        let now = stamp(&self.engine);
        self.look(&now);
        let rows = self.rows(ids).ok_or(Left::Unavailable)?;
        self.trash.restore(&rows).map_err(left_of)
    }
}
