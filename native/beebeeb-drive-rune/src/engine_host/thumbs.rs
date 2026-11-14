//! Thumbnails on the real engine: [`EngineThumbStore`], the one store behind every thumbnail the app draws, and
//! [`EngineThumbs`], the `ThumbSource` of the Files grid. The Photos source (`engine_host/photos.rs`) reads the
//! same store.
//!
//! - A thumbnail: [`Engine::thumbnail_pixels_for`]: the encrypted picture another client stored with the file,
//!   fetched and decrypted in memory by the engine, cut by the engine (the window never crops or scales).
//! - A video's length: [`Engine::video_seconds_for`]: read from the copy on this PC, when there is one. A video
//!   that is online only has no length here (it is never downloaded for it).
//!
//! Where the pixels live: in this process's memory, in this store, and nowhere else. Nothing is written to disk,
//! nothing is logged. At most [`MAX_PICTURES`] pictures and [`MAX_BYTES`] of pixels are held; the ones not looked at
//! for longest go first.
//!
//! Whose: everything held belongs to ONE session and account (the stamp). A look under another stamp drops all of
//! it before it answers, on the caller's (the window's) thread; an answer that lands under another stamp is dropped
//! and never stored. So another account never sees a picture of the one before, and neither does the same account
//! after a lock.
//!
//! Thread rule: a look never waits. A draw only says what it wants ([`EngineThumbStore::picture`]); the session of
//! that look is noted with it, on the window's thread. A few workers fetch, newest wish first, and wake the window
//! ([`Engine::wake_host`]) when a picture is here. A file with no picture is not asked again for [`ASK_AGAIN_AFTER`].
//!
//! Must not: log an id, a name or a pixel; hand out a picture without comparing the stamp.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use beebeeb_engine::app::photos_api::{ThumbCut, ThumbRefused};
use beebeeb_engine::app::{CredentialStores, Engine};
use rune::Pixels;

use super::log;
use super::stamp::{Stamp, stamp};
use crate::frames::global::grid::ThumbSource;

#[cfg(test)]
#[path = "thumbs_tests.rs"]
mod tests;

/// At most this many pictures are held.
pub const MAX_PICTURES: usize = 600;
/// At most this many bytes of pixels are held.
pub const MAX_BYTES: usize = 160 * 1024 * 1024;
/// At most this many wishes wait; older ones are dropped (a tile that is still on screen asks again).
pub const MAX_WISHES: usize = 240;
/// How many fetches run at once.
const WORKERS: usize = 3;
/// A file that had no picture, or whose picture could not be fetched, is asked again after this.
pub const ASK_AGAIN_AFTER: Duration = Duration::from_secs(60);
/// How long an idle worker sleeps before it looks whether the store is still there.
const IDLE: Duration = Duration::from_millis(500);

/// What is wished for of one file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Ask {
    Picture(ThumbCut),
    /// A video's length.
    Length,
}

type Key = (String, Ask);

enum Held {
    Picture {
        pixels: Pixels,
        bytes: usize,
        used: u64,
    },
    /// A video's length in seconds (`None`: this PC cannot say).
    Length(Option<u32>),
    /// No picture: not asked again before `until`.
    Nothing {
        until: Instant,
    },
}

#[derive(Default)]
struct Inner {
    /// Whose everything here is.
    stamp: Option<Stamp>,
    held: HashMap<Key, Held>,
    bytes: usize,
    /// What is wished for and not fetched yet, oldest first, each with the stamp of its look.
    wishes: VecDeque<(Key, Stamp)>,
    in_flight: Vec<Key>,
    /// Counts the looks: the picture with the lowest number goes first.
    looks: u64,
    workers: usize,
    /// Something arrived since the window last asked.
    arrived: bool,
    /// Tests only: the next fetch for this file waits on the first before it asks the engine, and says on the
    /// second that its answer was taken in (or dropped).
    #[cfg(test)]
    next_fetch: Option<(String, std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>)>,
}

impl Inner {
    fn clear(&mut self) {
        self.held.clear();
        self.bytes = 0;
        self.wishes.clear();
        self.in_flight.clear();
        self.arrived = false;
    }

    /// Everything here is `now`'s, or it goes.
    fn look(&mut self, now: &Stamp) {
        if self.stamp.as_ref() != Some(now) {
            self.clear();
            self.stamp = Some(now.clone());
        }
    }

    /// Room for one more: the pictures not looked at for longest go.
    fn make_room(&mut self) {
        loop {
            let pictures = self
                .held
                .values()
                .filter(|held| matches!(held, Held::Picture { .. }))
                .count();
            if pictures <= MAX_PICTURES && self.bytes <= MAX_BYTES {
                return;
            }
            let oldest = self
                .held
                .iter()
                .filter_map(|(key, held)| match held {
                    Held::Picture { used, .. } => Some((*used, key.clone())),
                    _ => None,
                })
                .min_by_key(|(used, _)| *used);
            let Some((_, key)) = oldest else { return };
            if let Some(Held::Picture { bytes, .. }) = self.held.remove(&key) {
                self.bytes = self.bytes.saturating_sub(bytes);
            }
        }
    }
}

/// See the module's doc.
pub struct EngineThumbStore<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    inner: Mutex<Inner>,
    wake: Condvar,
}

impl<C: CredentialStores> EngineThumbStore<C> {
    /// Starts no thread and asks nothing.
    pub fn new(engine: Arc<Engine<C>>) -> Arc<Self> {
        Arc::new(Self {
            engine,
            inner: Mutex::new(Inner::default()),
            wake: Condvar::new(),
        })
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What is held for `key` under the session that is here, or a wish for it. Never waits.
    fn held_or_wish<T>(
        self: &Arc<Self>,
        id: &str,
        ask: Ask,
        read: impl FnOnce(&mut Held, u64) -> Option<T>,
    ) -> Option<T> {
        // The session and account of this look, read before anything is handed out.
        let now = stamp(&self.engine);
        let mut inner = self.inner();
        inner.look(&now);
        if !self.engine.has_session() {
            return None;
        }
        inner.looks += 1;
        let looks = inner.looks;
        let key = (id.to_string(), ask);
        match inner.held.get_mut(&key) {
            Some(Held::Nothing { until }) if *until > Instant::now() => return None,
            Some(Held::Nothing { .. }) | None => {}
            Some(held) => return read(held, looks),
        }
        if inner.in_flight.contains(&key) {
            return None;
        }
        // Wished again: it moves to the front (the newest wish is fetched first).
        inner.wishes.retain(|(wished, _)| *wished != key);
        inner.wishes.push_back((key, now));
        while inner.wishes.len() > MAX_WISHES {
            inner.wishes.pop_front();
        }
        self.start_workers(&mut inner);
        self.wake.notify_one();
        None
    }

    /// The thumbnail of the file `id`, cut as `cut` says, if it is here; otherwise it is wished for, and the window
    /// is woken when it arrives.
    pub fn picture(self: &Arc<Self>, id: &str, cut: ThumbCut) -> Option<Pixels> {
        self.held_or_wish(id, Ask::Picture(cut), |held, looks| match held {
            Held::Picture { pixels, used, .. } => {
                *used = looks;
                Some(pixels.clone())
            }
            _ => None,
        })
    }

    /// The length of the video `id` in seconds, if this PC can say; asked for otherwise.
    pub fn length(self: &Arc<Self>, id: &str) -> Option<u32> {
        self.held_or_wish(id, Ask::Length, |held, _| match held {
            Held::Length(seconds) => *seconds,
            _ => None,
        })
    }

    /// Tests only: the next fetch for the file `id` waits, on its worker, before it asks the engine anything, until
    /// the first is sent on; the second says its answer was taken in or dropped.
    #[cfg(test)]
    pub(super) fn hold_next_fetch(&self, id: &str) -> (std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>) {
        let ((release, hold), (ended, has_ended)) = (std::sync::mpsc::channel(), std::sync::mpsc::channel());
        self.inner().next_fetch = Some((id.to_string(), hold, ended));
        (release, has_ended)
    }

    /// Nothing of the session that was here is kept.
    pub fn forget(&self) {
        let mut inner = self.inner();
        inner.clear();
        inner.stamp = None;
    }

    /// Did something arrive since the last call?
    pub fn take_arrived(&self) -> bool {
        std::mem::take(&mut self.inner().arrived)
    }

    /// Is something still on its way?
    pub fn busy(&self) -> bool {
        let inner = self.inner();
        !inner.wishes.is_empty() || !inner.in_flight.is_empty()
    }

    fn start_workers(self: &Arc<Self>, inner: &mut Inner) {
        while inner.workers < WORKERS && inner.workers < inner.wishes.len() + inner.in_flight.len() {
            let store = Arc::downgrade(self);
            let started = std::thread::Builder::new()
                .name("thumbnails".into())
                .spawn(move || work(store));
            if started.is_err() {
                log("thumbnails: could not start a worker");
                return;
            }
            inner.workers += 1;
        }
    }

    /// On a worker: the newest wish, fetched and taken in. `false`: there was none.
    fn fetch_one(&self) -> bool {
        let (key, wished_under) = {
            let mut inner = self.inner();
            let Some(wish) = inner.wishes.pop_back() else {
                return false;
            };
            inner.in_flight.push(wish.0.clone());
            wish
        };
        #[cfg(test)]
        let hooks = self.inner().next_fetch.take_if(|(id, _, _)| *id == key.0);
        #[cfg(test)]
        if let Some((_, hold, _)) = &hooks {
            let _ = hold.recv();
        }
        // The session of the look is handed over and the engine decides (with the token copy): a check here
        // could not be. A panic must not leave the wish in flight for good.
        let (id, ask) = (&key.0, key.1);
        let session = wished_under.0;
        let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match ask {
            Ask::Picture(cut) => match self.engine.thumbnail_pixels_for(session, id, cut) {
                Ok(picture) => {
                    let pixels = Pixels::new();
                    let bytes = picture.rgba.len();
                    pixels.set(picture.width, picture.height, picture.rgba.to_vec());
                    Ok(Held::Picture { pixels, bytes, used: 0 })
                }
                Err(refused) => Err(refused),
            },
            Ask::Length => match self.engine.video_seconds_for(session, id) {
                Ok(seconds) => Ok(Held::Length(
                    seconds.map(|seconds| u32::try_from(seconds).unwrap_or(u32::MAX)),
                )),
                Err(_) => Err(ThumbRefused::NotSignedIn),
            },
        }))
        .unwrap_or_else(|_| {
            log("thumbnails: a fetch ended early");
            Err(ThumbRefused::Failed)
        });
        let mut inner = self.inner();
        inner.in_flight.retain(|flying| *flying != key);
        // Only for the session AND the account of its look, and only while they are still here. The session's
        // turn only moves forward, so a stamp that is current now was current for the whole fetch.
        let now = stamp(&self.engine);
        let current = self.engine.has_session() && wished_under == now && inner.stamp.as_ref() == Some(&now);
        if current {
            let looks = inner.looks;
            match answer {
                Ok(Held::Picture { pixels, bytes, .. }) => {
                    inner.bytes += bytes;
                    inner.held.insert(
                        key,
                        Held::Picture {
                            pixels,
                            bytes,
                            used: looks,
                        },
                    );
                    inner.make_room();
                    inner.arrived = true;
                }
                Ok(held) => {
                    inner.held.insert(key, held);
                    inner.arrived = true;
                }
                // The session is not there for it: nothing is kept, not even "no picture".
                Err(ThumbRefused::NotSignedIn | ThumbRefused::SessionEnded | ThumbRefused::NotYours) => {}
                Err(_) => {
                    inner.held.insert(
                        key,
                        Held::Nothing {
                            until: Instant::now() + ASK_AGAIN_AFTER,
                        },
                    );
                }
            }
        }
        drop(inner);
        #[cfg(test)]
        if let Some((_, _, ended)) = &hooks {
            let _ = ended.send(());
        }
        if current {
            self.engine.wake_host();
        }
        true
    }
}

/// A worker: fetches while there are wishes; ends once the store is gone or nothing was wished for a while.
fn work<C: CredentialStores>(store: Weak<EngineThumbStore<C>>) {
    loop {
        let Some(store) = store.upgrade() else { return };
        if store.fetch_one() {
            continue;
        }
        let inner = store.inner();
        let (mut inner, _) = store
            .wake
            .wait_timeout_while(inner, IDLE, |inner| inner.wishes.is_empty())
            .unwrap_or_else(PoisonError::into_inner);
        if inner.wishes.is_empty() {
            inner.workers = inner.workers.saturating_sub(1);
            return;
        }
    }
}

/// A video's length as a tile writes it ("0:41").
pub(super) fn length_words(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// The Files grid's thumbnails on the real engine. See the module's doc.
pub struct EngineThumbs<C: CredentialStores> {
    store: Arc<EngineThumbStore<C>>,
}

impl<C: CredentialStores> EngineThumbs<C> {
    pub fn new(store: Arc<EngineThumbStore<C>>) -> Self {
        Self { store }
    }
}

impl<C: CredentialStores> ThumbSource for EngineThumbs<C> {
    fn arrived(&self, id: &str) -> bool {
        self.store.picture(id, ThumbCut::Square).is_some()
    }

    fn video_length(&self, id: &str) -> Option<String> {
        self.store.length(id).map(length_words)
    }

    fn pixels(&self, id: &str) -> Option<Pixels> {
        self.store.picture(id, ThumbCut::Square)
    }
}
