//! Slice M1 (`media-range`): the bytes of a file being played, decrypted in memory range by range, for the native
//! app's player (the media plan, (c)).
//!
//! [`Engine::open_media_for`] reads the file's metadata ONCE and pins its version (`EngineBridge::media_plan`);
//! [`MediaReader::read_at`] then fetches and opens only the chunks that cover the range, each authenticated by core at
//! its own index for the pinned version (`EngineBridge::media_chunks`, the index-bound partial read of
//! `chunk_frames.rs`), and hands the plaintext over in a `Zeroizing` buffer. Nothing is written to disk, nothing
//! reaches the transfer board, no row status changes. A file wholly kept on this PC is read from its copy in the sync
//! folder instead ([`MediaOrigin::ThisPc`]); nothing is decrypted for it, and nothing recalls a placeholder.
//!
//! Session-bound like its neighbours (`*_for(session)`): asked FOR the session the host read at its press. Every read
//! first asks whether that session is still the one in memory; a lock, a sign-out or an account change wipes what the
//! reader holds, and that read and every later one answer [`MediaReadError::NotSignedIn`]. The token and the vault key
//! are never held by the reader: each fetch copies them under the session-write lock (as `open_file_in_memory` does),
//! and a fetched chunk is handed over only while the session is still the one they were copied from.
//!
//! What a reader holds: the plan (its file key wipes itself on drop) and a cache of decrypted chunks, each in a
//! `Zeroizing` buffer, least recently used first out, at most [`MEDIA_CACHE_MAX`] bytes, chunk 0 never evicted (the
//! container's header is read again and again). A worker fetches ahead up to [`MEDIA_READ_AHEAD`] past each read; a
//! read far from it (a seek) gives that up. A playback never fetches more than twice the file's size plus
//! [`MEDIA_REFETCH_SLACK`] (read-ahead included): past it every read answers [`MediaReadError::OverBudget`].
//! Dropping the reader wipes the cache at once (the `Zeroizing` buffers zero themselves as they are dropped), and the
//! worker, told so, gives up its fetch within [`ASK_EVERY`] and drops what it fetched.
//!
//! Versions: a reader reads ONE version for its whole life. The server's named version (`download_version`) is
//! carried by every chunk request, and one that went away is [`MediaReadError::Gone`] for this and every later read
//! (never a restart on the newer one, which would splice two versions). A legacy file with no version named is read
//! by the unpinned route, as the whole-file read reads it: its v2 content is still bound to the version commitment of
//! the record read at open (a chunk of another upload does not open: [`MediaReadError::Failed`]). A v1 file is read:
//! v1 frames bind no position, version or truncation (core's I9 gap, the same as the whole-file read's). v2 content
//! whose record has no commitment is refused at open ([`OpenFileError::Failed`]; the player offers "Open in another
//! app…").
//!
//! The copy on this PC: chosen by the viewer's rules (`open_local_copy_in_memory`: the local state says the file is
//! here, it is not behind the version the local state holds, its name leads to it, and every byte of it is on disk),
//! asked with a limit of nothing, so the question reads no byte. Every read then opens the copy again, refusing a link
//! (unix), asks through the handle that it is still a plain file wholly on disk of the same length, holds this
//! process's own fetch of it off while it reads, and reads the range with ordinary reads. A copy that stops being
//! readable falls back to Beebeeb for this and every later read, but only to the version the copy is (the same object
//! version, else the same version number, and the same length): another one is [`MediaReadError::Gone`], one that
//! cannot be told is [`MediaReadError::Failed`]. A copy with a change that is not in Beebeeb yet never falls back
//! (Beebeeb's version is other content): [`MediaReadError::Failed`].
//!
//! BLOCKS: [`Engine::open_media_for`] and [`MediaReader::read_at`] are a worker's (the network, the disk); never from
//! the UI thread, never from inside an async runtime. Must not: log or return a name, a path, an id, a key or a byte
//! outside the answer; write a decrypted byte anywhere.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use zeroize::Zeroizing;

use crate::account::SessionGeneration;
use crate::api_client::ApiClient;
use crate::app::viewer_api::{LocalCopy, OpenFileError, failure};
use crate::app::{CredentialStores, Engine, LocalStateRefused};
use crate::download_pin::{PinLost, VersionGone};
use crate::engine_bridge::{EngineBridge, EngineError};
use crate::state_db::FileStatus;

#[cfg(test)]
#[path = "media_stream_tests.rs"]
mod tests;

/// The most decrypted chunk bytes one reader holds (a zeroized LRU; chunk 0 stays).
pub const MEDIA_CACHE_MAX: u64 = 32 << 20;
/// How far past the last read the reader fetches ahead, on a worker.
pub const MEDIA_READ_AHEAD: u64 = 8 << 20;
/// What one playback may fetch beyond twice the file's size (a helper that loops seeks cannot make the app download
/// without end); past it every read answers [`MediaReadError::OverBudget`].
pub const MEDIA_REFETCH_SLACK: u64 = 256 << 20;
/// The longest read a player may ask for: the media protocol's `MAX_READ` (`beebeeb-media-protocol`, 4 MiB). The
/// engine does not depend on that crate; the two must stay equal.
pub const MEDIA_MAX_READ: u32 = 4 << 20;

/// How often a fetch under way, or a read waiting for the read-ahead's fetch, asks whether it was cancelled or its
/// session ended.
const ASK_EVERY: Duration = Duration::from_millis(50);
/// What a chunk costs on the wire beyond its plaintext (nonce and tag), defined once in core.
const WIRE_OVERHEAD: u64 = beebeeb_core::chunk_stream::CHUNK_OVERHEAD;

/// Where the bytes a reader hands over come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaOrigin {
    /// The copy kept in the sync folder on this PC (nothing decrypted).
    ThisPc,
    /// Beebeeb's encrypted chunks, decrypted in memory.
    Beebeeb,
}

/// Why a range was not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaReadError {
    /// The session the reader was opened for is not the one in memory (a lock, a sign-out, another account).
    NotSignedIn,
    /// The pinned version is no longer served (a newer one replaced it, or the file is gone).
    Gone,
    /// Beebeeb could not be reached.
    Unreachable,
    /// The playback fetched more than twice the file's size plus [`MEDIA_REFETCH_SLACK`].
    OverBudget,
    /// The range is not inside the file.
    OutOfRange,
    /// The caller's cancel flag was set.
    Cancelled,
    /// Anything else (a chunk that did not authenticate among them). Nothing is handed over.
    Failed,
}

/// The answers a reader keeps for every later read: the session is gone, the version is gone, the budget is spent.
fn sticky(why: MediaReadError) -> bool {
    matches!(
        why,
        MediaReadError::NotSignedIn | MediaReadError::Gone | MediaReadError::OverBudget
    )
}

/// A reader's limits: the constants above. A test sets smaller ones (with the test server's 1 KiB chunks the real ones
/// are out of a test's reach).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Limits {
    cache_max: u64,
    read_ahead: u64,
    refetch_slack: u64,
}

impl Limits {
    const REAL: Self = Self {
        cache_max: MEDIA_CACHE_MAX,
        read_ahead: MEDIA_READ_AHEAD,
        refetch_slack: MEDIA_REFETCH_SLACK,
    };
}

/// One chunk of the pinned version, fetched and decrypted in memory: `(index, stop)`, `stop` asked while it runs.
type FetchChunk = dyn Fn(u32, &dyn Fn() -> bool) -> Result<Zeroizing<Vec<u8>>, MediaReadError> + Send + Sync;

/// Beebeeb's side of a file: the plan's geometry, and the fetch of one chunk of its version (the plan, with its file
/// key, lives inside `fetch`).
struct Network {
    len: u64,
    chunk_size: u64,
    chunk_count: u32,
    /// The version the plan pinned (its object id), when the server named one.
    object_version_id: Option<String>,
    /// The version number the metadata stated.
    version_number: Option<i64>,
    fetch: Box<FetchChunk>,
}

impl Network {
    /// Plaintext bytes chunk `index` holds: the stride, or what is left for the last. `None` outside the file.
    fn chunk_len(&self, index: u32) -> Option<u64> {
        if index >= self.chunk_count {
            return None;
        }
        let before = u64::from(index).checked_mul(self.chunk_size)?;
        Some(self.len.checked_sub(before)?.min(self.chunk_size))
    }
}

/// The copy on this PC a reader reads, as the local state and the disk had it at open.
struct Local {
    /// For this process's own fetch to be held off while the copy is read; never printed.
    file_id: String,
    root: PathBuf,
    path: PathBuf,
    len: u64,
    /// The copy holds a change that is not in Beebeeb yet (by the local state, or by its own length).
    unsent: bool,
    /// The version the local state holds for the file (its object id, its number).
    object_version_id: Option<String>,
    version_number: Option<i64>,
}

/// The engine, as a reader asks it. Type-erased: a reader is not generic over the engine's credential stores.
struct Host {
    /// Whether no session change happened since the reader's session was read.
    unchanged: Box<dyn Fn() -> bool + Send + Sync>,
    /// The hand-over question: the session in memory is still that one, with no session change half done.
    handover: Box<dyn Fn() -> bool + Send + Sync>,
    /// Beebeeb's side of the file, planned now (one metadata read): a local copy's fallback.
    plan: Box<dyn Fn() -> Result<Network, OpenFileError> + Send + Sync>,
}

/// What the read-ahead worker is asked to fetch: chunks `next..end` of the window that began at `start`.
#[derive(Debug, Default, Clone, Copy)]
struct Job {
    /// Moves on when a seek gives the window up; a fetch of an older one is dropped.
    generation: u64,
    start: u32,
    next: u32,
    end: u32,
}

/// The decrypted chunks a reader holds, and what its read-ahead does. Behind [`Shared::cache`].
#[derive(Default)]
struct Cache {
    chunks: HashMap<u32, Zeroizing<Vec<u8>>>,
    /// Least recently used first. Chunk 0 is never in it: it is never evicted.
    order: VecDeque<u32>,
    /// Bytes in `chunks`.
    held: u64,
    /// Bytes asked of Beebeeb so far, read-ahead included: each chunk's wire size, booked before it is asked for.
    fetched: u64,
    /// Why every later read is refused. Set once; what was held is wiped with it.
    ended: Option<MediaReadError>,
    /// The reader was dropped: the read-ahead leaves.
    closed: bool,
    /// The chunk the read-ahead is fetching now (a read that needs it waits for it).
    in_flight: Option<u32>,
    job: Job,
}

impl Cache {
    /// Every chunk dropped, each buffer zeroed as it goes (`Zeroizing`).
    fn wipe(&mut self) {
        self.chunks.clear();
        self.order.clear();
        self.held = 0;
    }

    /// `index` was just used.
    fn touch(&mut self, index: u32) {
        if index == 0 {
            return;
        }
        if let Some(at) = self.order.iter().position(|held| *held == index) {
            self.order.remove(at);
        }
        self.order.push_back(index);
    }

    /// Keeps `bytes` as chunk `index`, the least recently used chunks out first until it fits under `cap`. Not kept
    /// (and so wiped now) when the reader has ended or closed, when it is held already, or when it cannot fit.
    fn insert(&mut self, index: u32, bytes: Zeroizing<Vec<u8>>, cap: u64) {
        if self.ended.is_some() || self.closed || self.chunks.contains_key(&index) {
            return;
        }
        let size = bytes.len() as u64;
        while self.held.saturating_add(size) > cap {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(gone) = self.chunks.remove(&oldest) {
                self.held -= gone.len() as u64;
            }
        }
        if self.held.saturating_add(size) > cap {
            return;
        }
        self.held += size;
        self.chunks.insert(index, bytes);
        self.touch(index);
    }

    /// Books `cost` wire bytes against `budget`; `false` (nothing booked) when they would pass it.
    fn book(&mut self, cost: u64, budget: u64) -> bool {
        match self.fetched.checked_add(cost) {
            Some(total) if total <= budget => {
                self.fetched = total;
                true
            }
            _ => false,
        }
    }
}

/// What a reader and its read-ahead worker share.
#[derive(Default)]
struct Shared {
    cache: Mutex<Cache>,
    /// Told on every change a waiting read or the worker waits for.
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Ends the reader: this and every later read answer `why` (the first reason stays), and every decrypted byte
    /// it holds is wiped now.
    fn end(&self, why: MediaReadError) {
        let mut cache = self.lock();
        if cache.ended.is_none() {
            cache.ended = Some(why);
        }
        cache.wipe();
        drop(cache);
        self.changed.notify_all();
    }
}

/// A file opened for playback: its length, where its bytes come from, the pinned version (inside its fetch), the
/// session it was opened for and the zeroized chunk cache. `Send`, not `Clone`; its `Debug` prints no id and no byte.
pub struct MediaReader {
    len: u64,
    origin: MediaOrigin,
    host: Host,
    /// The copy on this PC, while it is what is read.
    local: Option<Local>,
    /// Beebeeb's side: at open for a file not kept here, at the first fallback for one that is.
    network: Option<Arc<Network>>,
    shared: Arc<Shared>,
    limits: Limits,
    /// The read-ahead worker was started.
    worker: bool,
}

/// The length and the origin, never the file's id or a byte of it.
impl fmt::Debug for MediaReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MediaReader")
            .field("len", &self.len)
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

/// The kind of a refusal of the local state, as the viewer's open names it.
fn state_refused(refusal: LocalStateRefused) -> OpenFileError {
    match refusal {
        LocalStateRefused::SessionEnded => OpenFileError::SessionEnded,
        LocalStateRefused::NothingSynced => OpenFileError::NothingSynced,
        LocalStateRefused::Unreadable => OpenFileError::Failed,
        refusal if refusal.not_this_accounts() => OpenFileError::NotYours,
        _ => OpenFileError::NotSignedIn,
    }
}

/// A refusal of the open, as a read answers it.
fn from_open(error: OpenFileError) -> MediaReadError {
    match error {
        OpenFileError::NotSignedIn | OpenFileError::SessionEnded | OpenFileError::NotYours => {
            MediaReadError::NotSignedIn
        }
        OpenFileError::NotFound => MediaReadError::Gone,
        OpenFileError::Offline => MediaReadError::Unreachable,
        OpenFileError::Cancelled => MediaReadError::Cancelled,
        OpenFileError::NothingSynced
        | OpenFileError::TooLarge { .. }
        | OpenFileError::OverLimit
        | OpenFileError::CopyNotReadable
        | OpenFileError::Failed => MediaReadError::Failed,
    }
}

/// The kind of a failed chunk read, read from the error's chain (never its text): the pinned version went away is
/// [`MediaReadError::Gone`]; the rest as the viewer's download names them.
fn read_failure(error: EngineError) -> MediaReadError {
    if error.caused_by::<VersionGone>().is_some() || error.caused_by::<PinLost>().is_some() {
        return MediaReadError::Gone;
    }
    from_open(failure(error))
}

impl<C: CredentialStores> Engine<C> {
    /// Opens `file_id` for playback FOR `session` (the session the host read at its press): the copy on this PC when
    /// the viewer's rules say it is here (no network), else one metadata read with the version pinned. Refused when
    /// `session` is not the one in memory. See the module's doc for what is (not) read and kept.
    pub fn open_media_for(
        self: &Arc<Self>,
        session: SessionGeneration,
        file_id: &str,
    ) -> Result<MediaReader, OpenFileError> {
        if self.acct.session.is_poisoned() || !self.acct.session_unchanged_since(session) {
            return Err(OpenFileError::NotSignedIn);
        }
        let host = self.media_host(session, file_id);
        let local = self.media_copy_here(session, file_id)?;
        let (origin, len, network) = match &local {
            Some(local) => (MediaOrigin::ThisPc, local.len, None),
            None => {
                let network = (host.plan)()?;
                (MediaOrigin::Beebeeb, network.len, Some(Arc::new(network)))
            }
        };
        if !(host.handover)() {
            return Err(OpenFileError::NotSignedIn);
        }
        Ok(MediaReader {
            len,
            origin,
            host,
            local,
            network,
            shared: Arc::new(Shared::default()),
            limits: Limits::REAL,
            worker: false,
        })
    }

    /// The engine as a reader of `file_id` for `session` asks it. Holds the engine weakly: a reader that outlives
    /// it reads nothing more.
    fn media_host(self: &Arc<Self>, session: SessionGeneration, file_id: &str) -> Host {
        let (unchanged, handover, plan) = (Arc::downgrade(self), Arc::downgrade(self), Arc::downgrade(self));
        let file_id = file_id.to_owned();
        Host {
            unchanged: Box::new(move || {
                unchanged
                    .upgrade()
                    .is_some_and(|engine| engine.acct.session_unchanged_since(session))
            }),
            handover: Box::new(move || {
                handover
                    .upgrade()
                    .is_some_and(|engine| engine.acct.while_session_is(&engine.writes, session, |_| ()).is_some())
            }),
            plan: Box::new(move || match plan.upgrade() {
                Some(engine) => engine.media_network(session, &file_id),
                None => Err(OpenFileError::NotSignedIn),
            }),
        }
    }

    /// The token and the vault key of `session`, copied the way every other hand-over copies them
    /// (`AccountRuntime::while_session_is`: under the session-write lock, never while a session change is half
    /// done, only while the generation is still `session`), in a client of their own that wipes them when dropped;
    /// the key also in a `Zeroizing`, for the hand-over question. `None`: not that session.
    fn media_client(&self, session: SessionGeneration) -> Option<(Arc<ApiClient>, Zeroizing<[u8; 32]>)> {
        if self.acct.session.is_poisoned() {
            return None;
        }
        let (token, key) = self.acct.while_session_is(&self.writes, session, |current| {
            (current.token.clone(), Zeroizing::new(current.master_key))
        })?;
        let api = Arc::new(ApiClient::new(self.config.api_base.clone(), token, *key));
        Some((api, key))
    }

    /// `work` on the handle's runtime until it ends, or until `stop` says so (asked every [`ASK_EVERY`]): `None`.
    /// What `work` held when it was stopped is dropped with it.
    fn media_until_stopped<T>(&self, work: impl std::future::Future<Output = T>, stop: &dyn Fn() -> bool) -> Option<T> {
        self.runtime.block_on(async {
            let mut work = std::pin::pin!(work);
            loop {
                match tokio::time::timeout(ASK_EVERY, work.as_mut()).await {
                    Ok(done) => break Some(done),
                    Err(_) if stop() => break None,
                    Err(_) => {}
                }
            }
        })
    }

    /// Beebeeb's side of `file_id` for `session`: the plan (ONE metadata read, the version pinned) and the fetch of
    /// one chunk of it. The plan lives inside the fetch, which copies the session's token and key afresh for every
    /// chunk and hands a chunk over only while the session is still the one they were copied from.
    fn media_network(self: &Arc<Self>, session: SessionGeneration, file_id: &str) -> Result<Network, OpenFileError> {
        let (api, key) = self.media_client(session).ok_or(OpenFileError::NotSignedIn)?;
        let state = self.local_state_for_changes().map_err(state_refused)?;
        let planned = state
            .with_shared(|db| {
                let bridge = EngineBridge::new(self.identity, db, api);
                self.media_until_stopped(bridge.media_plan(file_id), &|| {
                    !self.acct.session_unchanged_since(session)
                })
            })
            .map_err(state_refused)?;
        let plan = match planned {
            Some(planned) => planned.map_err(failure)?,
            None => return Err(OpenFileError::NotSignedIn),
        };
        if !self.session_is_still(session, &key) {
            return Err(OpenFileError::NotSignedIn);
        }
        let (len, chunk_size, chunk_count) = (plan.plaintext_size, plan.chunk_size, plan.chunk_count);
        let object_version_id = plan.pin.as_ref().map(|pin| pin.object_version_id.clone());
        let version_number = plan.version_number;
        let engine = Arc::downgrade(self);
        let fetch = move |index: u32, stop: &dyn Fn() -> bool| -> Result<Zeroizing<Vec<u8>>, MediaReadError> {
            let engine = engine.upgrade().ok_or(MediaReadError::NotSignedIn)?;
            let (api, key) = engine.media_client(session).ok_or(MediaReadError::NotSignedIn)?;
            let state = engine
                .local_state_for_changes()
                .map_err(|refusal| from_open(state_refused(refusal)))?;
            let fetched = state
                .with_shared(|db| {
                    let bridge = EngineBridge::new(engine.identity, db, api);
                    engine.media_until_stopped(bridge.media_chunks(&plan, index..index + 1), &|| {
                        stop() || !engine.acct.session_unchanged_since(session)
                    })
                })
                .map_err(|refusal| from_open(state_refused(refusal)))?;
            let bytes = match fetched {
                Some(fetched) => fetched.map_err(read_failure)?,
                // Stopped: what was decrypted was dropped (and wiped) with the read.
                None if engine.acct.session_unchanged_since(session) => return Err(MediaReadError::Cancelled),
                None => return Err(MediaReadError::NotSignedIn),
            };
            // The hand-over: only while the session in memory is the one the token and the key were copied from.
            if !engine.session_is_still(session, &key) {
                return Err(MediaReadError::NotSignedIn);
            }
            Ok(bytes)
        };
        Ok(Network {
            len,
            chunk_size,
            chunk_count,
            object_version_id,
            version_number,
            fetch: Box::new(fetch),
        })
    }

    /// The copy of `file_id` on this PC, when the viewer's rules say it is what is read; `None`: Beebeeb's version
    /// is (not here, behind, or no row). The rules are `open_local_copy_in_memory`'s own, asked with a limit of
    /// nothing: it answers the copy's length as `TooLarge` before it reads a byte (an empty copy reads nothing). A
    /// copy with a change that is not sent and cannot be read is that method's `CopyNotReadable`, never Beebeeb's
    /// version in its place.
    fn media_copy_here(&self, session: SessionGeneration, file_id: &str) -> Result<Option<Local>, OpenFileError> {
        let (entry, contract) = self
            .local_state()
            .map_err(state_refused)?
            .with(|db| Ok::<_, rusqlite::Error>((db.get_file(file_id)?, db.get_file_contract_state(file_id)?)))
            .map_err(state_refused)?
            .map_err(|_| OpenFileError::Failed)?;
        let Some(entry) = entry else {
            return Ok(None);
        };
        if entry.is_dir() {
            return Err(OpenFileError::NotFound);
        }
        let recorded_unsent = match entry.status {
            FileStatus::Local => false,
            FileStatus::Uploading => true,
            // Not a copy to read as the file: not here, half here, two versions to choose from, or on its way out.
            FileStatus::CloudOnly
            | FileStatus::Downloading
            | FileStatus::Conflict
            | FileStatus::Error
            | FileStatus::Trashing => return Ok(None),
        };
        let length = match self.open_local_copy_in_memory(file_id, 0, &AtomicBool::new(false)) {
            Err(OpenFileError::TooLarge { size }) => size,
            Ok(LocalCopy::Opened(copy)) => copy.bytes.len() as u64,
            Ok(LocalCopy::NotHere | LocalCopy::Behind) => return Ok(None),
            Err(error) => return Err(error),
        };
        if !self.acct.session_unchanged_since(session) {
            return Err(OpenFileError::NotSignedIn);
        }
        // A copy whose length is not the synced size was changed here, whatever the row says so far.
        let unsent = recorded_unsent || u64::try_from(entry.size_bytes).ok() != Some(length);
        let Some((root, path)) = self.media_copy_path(&entry.path) else {
            return if unsent {
                Err(OpenFileError::CopyNotReadable)
            } else {
                Ok(None)
            };
        };
        Ok(Some(Local {
            file_id: file_id.to_owned(),
            root,
            path,
            len: length,
            unsent,
            object_version_id: contract.as_ref().and_then(|c| c.current_object_version_id.clone()),
            version_number: contract.as_ref().map(|c| c.current_version),
        }))
    }

    /// The account's folder on this computer and where the file at `rel_path` of the local state is in it, as the
    /// viewer resolves it. `None`: no folder is known, the path is not one below it, or its name leads to a different
    /// file here (a Windows short name).
    fn media_copy_path(&self, rel_path: &str) -> Option<(PathBuf, PathBuf)> {
        let root = match self.configured_sync_root().ok()? {
            Some(root) => root,
            // No folder named in the settings: the identity's own, as `Engine::sync_root` answers. A test names its
            // folder or has none (it never reads the developer's own).
            #[cfg(not(any(test, feature = "test-support")))]
            None => crate::config::default_sync_root_suggestion(self.identity),
            #[cfg(any(test, feature = "test-support"))]
            None => return None,
        };
        let path = crate::file_ops::full_path(&root, rel_path).ok()?;
        matches!(crate::name_alias::conflict_below(&root, &path), Ok(None)).then_some((root, path))
    }
}

impl MediaReader {
    /// The file's plaintext length in bytes.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Where the bytes come from (Beebeeb's after a fallback).
    pub fn origin(&self) -> MediaOrigin {
        self.origin
    }

    /// Bytes `offset .. offset + len` of the file (`len` ≤ [`MEDIA_MAX_READ`], the range inside the file), decrypted
    /// in memory, or read from the copy on this PC. The session is asked first, before the range or the cache: a
    /// changed one wipes what the reader holds and answers [`MediaReadError::NotSignedIn`], now and for every later
    /// read. The bytes are handed over only while the session is still the reader's.
    pub fn read_at(
        &mut self,
        offset: u64,
        len: u32,
        cancel: &AtomicBool,
    ) -> Result<Zeroizing<Vec<u8>>, MediaReadError> {
        if let Some(why) = self.shared.lock().ended {
            return Err(why);
        }
        if !(self.host.unchanged)() {
            self.shared.end(MediaReadError::NotSignedIn);
            return Err(MediaReadError::NotSignedIn);
        }
        if len > MEDIA_MAX_READ {
            return Err(MediaReadError::OutOfRange);
        }
        let end = match offset.checked_add(u64::from(len)) {
            Some(end) if end <= self.len => end,
            _ => return Err(MediaReadError::OutOfRange),
        };
        if cancel.load(Ordering::SeqCst) {
            return Err(MediaReadError::Cancelled);
        }
        let local_read = self.local.as_ref().map(|local| read_local(local, offset, len));
        let bytes = match local_read {
            Some(Ok(bytes)) => bytes,
            Some(Err(())) => {
                self.fall_back()?;
                self.read_network(offset, end, cancel)?
            }
            None => self.read_network(offset, end, cancel)?,
        };
        // The hand-over: only while the session in memory is still the reader's, with no change half done.
        if !(self.host.handover)() {
            self.shared.end(MediaReadError::NotSignedIn);
            return Err(MediaReadError::NotSignedIn);
        }
        Ok(bytes)
    }

    /// The copy on this PC stopped being readable: Beebeeb's side from now on, but only the version the copy is.
    fn fall_back(&mut self) -> Result<(), MediaReadError> {
        let Some(local) = self.local.as_ref() else {
            return Ok(());
        };
        // A change that is not in Beebeeb yet: Beebeeb's version is other content, never spliced into it.
        if local.unsent {
            self.shared.end(MediaReadError::Failed);
            return Err(MediaReadError::Failed);
        }
        let network = match (self.host.plan)() {
            Ok(network) => network,
            Err(error) => {
                let why = from_open(error);
                if sticky(why) {
                    self.shared.end(why);
                }
                return Err(why);
            }
        };
        match same_version(local, &network) {
            Some(true) => {}
            Some(false) => {
                self.shared.end(MediaReadError::Gone);
                return Err(MediaReadError::Gone);
            }
            None => {
                self.shared.end(MediaReadError::Failed);
                return Err(MediaReadError::Failed);
            }
        }
        self.local = None;
        self.network = Some(Arc::new(network));
        self.origin = MediaOrigin::Beebeeb;
        Ok(())
    }

    /// The most a playback fetches: twice the file plus the slack.
    fn budget(&self) -> u64 {
        self.len.saturating_mul(2).saturating_add(self.limits.refetch_slack)
    }

    /// Bytes `offset..end` (inside the file) from Beebeeb's chunks: from the cache, from the read-ahead's fetch, or
    /// fetched now. One buffer of exactly the range's size; each chunk's part is copied in as it is had.
    fn read_network(
        &mut self,
        offset: u64,
        end: u64,
        cancel: &AtomicBool,
    ) -> Result<Zeroizing<Vec<u8>>, MediaReadError> {
        let network = self.network.clone().ok_or(MediaReadError::Failed)?;
        let want = usize::try_from(end - offset).map_err(|_| MediaReadError::OutOfRange)?;
        let mut out = Zeroizing::new(Vec::with_capacity(want));
        if want == 0 {
            return Ok(out);
        }
        let stride = network.chunk_size;
        let first = u32::try_from(offset / stride).map_err(|_| MediaReadError::OutOfRange)?;
        let last = u32::try_from((end - 1) / stride).map_err(|_| MediaReadError::OutOfRange)?;
        self.seek_to(first);
        for index in first..=last {
            let start = u64::from(index) * stride;
            let from = usize::try_from(offset.saturating_sub(start)).map_err(|_| MediaReadError::Failed)?;
            let to =
                usize::try_from(end.min(start.saturating_add(stride)) - start).map_err(|_| MediaReadError::Failed)?;
            self.with_chunk(&network, index, cancel, |chunk| match chunk.get(from..to) {
                Some(part) => {
                    out.extend_from_slice(part);
                    Ok(())
                }
                // A chunk shorter than the plan says: nothing of it is handed over.
                None => Err(MediaReadError::Failed),
            })??;
        }
        self.ahead(&network, end);
        Ok(out)
    }

    /// `take` over chunk `index`: the one in the cache, the one the read-ahead is fetching (waited for), or one
    /// fetched now, booked against the budget BEFORE it is asked for and kept in the cache after.
    fn with_chunk<R>(
        &self,
        network: &Network,
        index: u32,
        cancel: &AtomicBool,
        take: impl FnOnce(&[u8]) -> R,
    ) -> Result<R, MediaReadError> {
        let mut cache = self.shared.lock();
        loop {
            if let Some(why) = cache.ended {
                return Err(why);
            }
            if cache.chunks.contains_key(&index) {
                cache.touch(index);
                if let Some(chunk) = cache.chunks.get(&index) {
                    return Ok(take(chunk));
                }
            }
            if cache.in_flight != Some(index) {
                break;
            }
            // The read-ahead is fetching this very chunk: wait for it, asking the caller and the session meanwhile.
            if cancel.load(Ordering::SeqCst) {
                return Err(MediaReadError::Cancelled);
            }
            if !(self.host.unchanged)() {
                drop(cache);
                self.shared.end(MediaReadError::NotSignedIn);
                return Err(MediaReadError::NotSignedIn);
            }
            cache = match self.shared.changed.wait_timeout(cache, ASK_EVERY) {
                Ok((cache, _)) => cache,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
        let size = network.chunk_len(index).ok_or(MediaReadError::Failed)?;
        if !cache.book(size.saturating_add(WIRE_OVERHEAD), self.budget()) {
            drop(cache);
            self.shared.end(MediaReadError::OverBudget);
            return Err(MediaReadError::OverBudget);
        }
        drop(cache);
        let stop = || cancel.load(Ordering::SeqCst) || self.shared.lock().ended.is_some();
        let bytes = match (network.fetch)(index, &stop) {
            Ok(bytes) => bytes,
            Err(why) => {
                if sticky(why) {
                    self.shared.end(why);
                }
                return Err(why);
            }
        };
        let mut cache = self.shared.lock();
        // Ended while it was fetched: dropped here, and wiped with it.
        if let Some(why) = cache.ended {
            return Err(why);
        }
        let answer = take(&bytes);
        cache.insert(index, bytes, self.limits.cache_max);
        drop(cache);
        self.shared.changed.notify_all();
        Ok(answer)
    }

    /// A read that begins outside the read-ahead's window is a seek: the read-ahead under way is given up.
    fn seek_to(&self, first: u32) {
        let mut cache = self.shared.lock();
        let job = &mut cache.job;
        if job.next < job.end && !(job.start..=job.end).contains(&first) {
            job.generation += 1;
            job.next = job.end;
        }
    }

    /// After a read that ended at `end`: the read-ahead fetches up to the limit past it (the window grows while
    /// the reads follow each other; a new one begins after a seek).
    fn ahead(&mut self, network: &Arc<Network>, end: u64) {
        if self.limits.read_ahead == 0 || end >= self.len {
            return;
        }
        let stride = network.chunk_size;
        let until = end.saturating_add(self.limits.read_ahead).min(self.len);
        let (Ok(from), Ok(to)) = (u32::try_from(end / stride), u32::try_from((until - 1) / stride + 1)) else {
            return;
        };
        let to = to.min(network.chunk_count);
        {
            let mut cache = self.shared.lock();
            if cache.ended.is_some() || cache.closed {
                return;
            }
            let job = &mut cache.job;
            if job.next < job.end && (job.start..=job.end).contains(&from) {
                job.end = job.end.max(to);
            } else {
                job.generation += 1;
                job.start = from;
                job.next = from;
                job.end = to;
            }
        }
        self.shared.changed.notify_all();
        if !self.worker {
            self.worker = true;
            let (shared, network) = (Arc::clone(&self.shared), Arc::clone(network));
            let (cache_max, budget) = (self.limits.cache_max, self.budget());
            // No worker (the system refused a thread): no read-ahead, every read fetches what it needs itself.
            let _ = std::thread::Builder::new()
                .name("media-read-ahead".to_string())
                .spawn(move || read_ahead(&shared, &network, cache_max, budget));
        }
    }
}

/// Dropping the reader wipes every decrypted chunk it holds now (each `Zeroizing` buffer zeroes itself as it is
/// dropped) and tells the read-ahead to leave; never waits for it.
impl Drop for MediaReader {
    fn drop(&mut self) {
        let mut cache = self.shared.lock();
        cache.closed = true;
        cache.wipe();
        drop(cache);
        self.shared.changed.notify_all();
    }
}

/// The read-ahead worker: the chunks of the current window, one at a time, each booked against the budget before it
/// is asked for, kept only while the reader still wants that window. Leaves when the reader is dropped or ended.
fn read_ahead(shared: &Shared, network: &Network, cache_max: u64, budget: u64) {
    loop {
        let mut cache = shared.lock();
        while !(cache.closed || cache.ended.is_some() || cache.job.next < cache.job.end) {
            cache = match shared.changed.wait(cache) {
                Ok(cache) => cache,
                Err(poisoned) => poisoned.into_inner(),
            };
        }
        if cache.closed || cache.ended.is_some() {
            return;
        }
        let index = cache.job.next;
        cache.job.next += 1;
        let generation = cache.job.generation;
        if cache.chunks.contains_key(&index) {
            continue;
        }
        let Some(size) = network.chunk_len(index) else {
            continue;
        };
        // Past the budget the read-ahead stops; the next read that needs a chunk answers `OverBudget` itself.
        if !cache.book(size.saturating_add(WIRE_OVERHEAD), budget) {
            cache.job.next = cache.job.end;
            continue;
        }
        cache.in_flight = Some(index);
        drop(cache);
        let stop = || {
            let cache = shared.lock();
            cache.closed || cache.ended.is_some() || cache.job.generation != generation
        };
        let fetched = (network.fetch)(index, &stop);
        let mut cache = shared.lock();
        cache.in_flight = None;
        match fetched {
            Ok(bytes) if cache.job.generation == generation => cache.insert(index, bytes, cache_max),
            // A seek gave the window up: dropped, and wiped with it.
            Ok(_) => {}
            Err(MediaReadError::NotSignedIn) => {
                drop(cache);
                shared.end(MediaReadError::NotSignedIn);
                continue;
            }
            // The window is given up; the read that needs a chunk of it fetches it itself and answers the refusal.
            Err(_) => {
                if cache.job.generation == generation {
                    cache.job.next = cache.job.end;
                }
            }
        }
        drop(cache);
        shared.changed.notify_all();
    }
}

/// Whether Beebeeb's version is the one the copy on this PC is: the same length, and the same object version when
/// both are known, else the same version number. `None`: it cannot be told.
fn same_version(local: &Local, network: &Network) -> Option<bool> {
    if local.len != network.len {
        return Some(false);
    }
    if let (Some(here), Some(there)) = (&local.object_version_id, &network.object_version_id) {
        let parse = |id: &str| uuid::Uuid::parse_str(id.trim()).ok();
        return match (parse(here), parse(there)) {
            (Some(here), Some(there)) => Some(here == there),
            _ => None,
        };
    }
    match (local.version_number, network.version_number) {
        (Some(here), Some(there)) if here > 0 => Some(here == there),
        _ => None,
    }
}

/// Bytes `offset..offset + len` of the copy on this PC, by ordinary reads. `Err`: the name no longer leads to it,
/// it is not a plain file wholly on disk of the length it had at open, or the read failed or came up short.
fn read_local(local: &Local, offset: u64, len: u32) -> Result<Zeroizing<Vec<u8>>, ()> {
    if !matches!(crate::name_alias::conflict_below(&local.root, &local.path), Ok(None)) {
        return Err(());
    }
    // This process's own fetch of the file is refused until the read is over: a file that stops being wholly on
    // disk between the check and the read is not downloaded by the read.
    let _no_fetch = crate::no_fetch_scope::enter(&[local.file_id.as_str()]);
    let mut file = open_for_the_read(&local.path).map_err(|_| ())?;
    // Asked through the handle: it describes the file the read then reads, whatever was put at the path since.
    match file.metadata() {
        Ok(metadata) if is_plain_and_wholly_here(&metadata) && metadata.len() == local.len => {}
        _ => return Err(()),
    }
    let want = usize::try_from(len).map_err(|_| ())?;
    let mut bytes = Zeroizing::new(vec![0u8; want]);
    file.seek(SeekFrom::Start(offset)).map_err(|_| ())?;
    file.read_exact(&mut bytes).map_err(|_| ())?;
    Ok(bytes)
}

/// Whether `metadata` describes a plain file whose every byte is on this computer (core's reading of it, as the
/// viewer's copy read asks it): no folder, no link or special file, nothing offline.
fn is_plain_and_wholly_here(metadata: &std::fs::Metadata) -> bool {
    let attrs = beebeeb_backup::attrs_of(metadata);
    !(attrs.is_dir || attrs.is_link || attrs.not_local)
}

/// Opens `path` for a read. Unix: a link is never followed, and the open never waits (a pipe put where the file was
/// is opened at once, and refused by the check through the handle).
fn open_for_the_read(path: &Path) -> std::io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    options.open(path)
}
