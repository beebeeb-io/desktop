//! Photos and thumbnails, for a host without a UI toolkit (the native Photos window and every grid of thumbnails):
//! the photos and videos of the signed-in account ([`Engine::photos_library`]), one thumbnail as pixels
//! ([`Engine::thumbnail_pixels_for`]), a video's length where a copy is on this PC ([`Engine::media_facts_for`]) and
//! Keep on this PC for the session of a press ([`Engine::set_kept_for`]).
//!
//! Where it comes from: the library is read from the signed-in account's local state, through the account-bound
//! handle (task 2488): a state that is not the session's answers nothing. A thumbnail is the encrypted blob another
//! client stored with the file, fetched and decrypted IN MEMORY by the path every other thumbnail uses
//! (`EngineBridge::finder_thumbnail`: the same keys, and never the previous version's picture while a new version
//! is on its way). No crypto of its own.
//!
//! Session rule: a thumbnail is asked FOR a session (the one the host read at its press or when the tile came into
//! view). The token and the vault key leave the session's lock only while that session is the one in memory, and the
//! pixels are handed over only while it still is ([`Engine::session_is_still`], asked with no session change half
//! done). Every answer carries that generation, for the host to compare.
//!
//! What is written: nothing by a thumbnail or the library (resolving the key of a file in a shared folder may record
//! that folder's custody, the engine's own ratchet, as every download of such a file does). The decrypted picture
//! and its pixels live in [`Zeroizing`] buffers, wiped when dropped; the picture decoder's own scratch memory is
//! freed, not overwritten. A picture is refused before it is decoded when it is larger than [`MAX_ENCODED`], and
//! while decoding when it is wider or higher than [`MAX_SIDE`]: a thumbnail in a shared folder is someone else's input.
//!
//! BLOCKS: every call here is a worker's (the state database, the network, a decode). Must not: log or return a
//! name, a path, a key or a pixel outside the answer types.

use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use zeroize::{Zeroize, Zeroizing};

use super::settings_api::{AccountCallError, kind_of};
use super::{CredentialStores, Engine, LocalStateRefused};
use crate::account::SessionGeneration;
use crate::api_client::ApiClient;
use crate::drive_v2::model::{is_photo_or_video, leaf_name, live_rows, mime_for_name};
use crate::drive_v2::photo_meta::{self, PhotoMeta};
use crate::engine_bridge::{
    EngineBridge, ThumbnailNotYet, UnknownItem, error_http_status, local_file_path_under_sync_root,
};
use crate::state_db::{FileStatus, PinState, StateDb};

#[cfg(test)]
#[path = "photos_api_tests.rs"]
mod tests;

/// How often a thumbnail under way asks whether its session ended.
const ASK_EVERY: Duration = Duration::from_millis(50);

/// The largest encrypted-then-decrypted picture that is decoded at all.
pub const MAX_ENCODED: usize = 8 * 1024 * 1024;

/// The widest and highest stored picture that is decoded.
pub const MAX_SIDE: u32 = 4096;

/// One photo or video of the library. Holds a decrypted name and path: its `Debug` prints neither.
#[derive(Clone, PartialEq, Eq)]
pub struct LibraryPhoto {
    pub id: String,
    pub name: String,
    /// The folders above it, outermost first.
    pub folders: Vec<String>,
    pub bytes: u64,
    /// Modified, in Unix seconds: the capture time is inside the encrypted file, not in the local state.
    pub modified_at: i64,
    pub video: bool,
    /// Kept on this PC (Available offline), by its own pin or a folder's.
    pub kept: bool,
    /// A copy is on this PC now.
    pub on_this_pc: bool,
    /// Uploading from this PC now.
    pub uploading: bool,
    /// The device whose backup holds it: the device folder under the Backups root (LD-0113).
    pub device: Option<String>,
    /// The version number Beebeeb recorded (1: the only one).
    pub versions: i64,
}

impl std::fmt::Debug for LibraryPhoto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LibraryPhoto")
            .field("bytes", &self.bytes)
            .field("video", &self.video)
            .finish_non_exhaustive()
    }
}

/// The library, newest first, and the session it was read for.
#[derive(Clone, PartialEq, Eq)]
pub struct PhotosLibrary {
    pub session: SessionGeneration,
    pub photos: Vec<LibraryPhoto>,
}

impl std::fmt::Debug for PhotosLibrary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhotosLibrary")
            .field("session", &self.session)
            .field("photos", &self.photos.len())
            .finish()
    }
}

/// Which stored picture is asked for, and how it is cut. The host never crops or scales a picture itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThumbCut {
    /// The middle of the picture as a square (the stored medium picture), at most `MAX_SQUARE` a side.
    Square,
    /// The middle of the picture as an upright strip of 36 by 52 (the stored small picture).
    Strip,
    /// The whole stored large picture, at its own proportions.
    Whole,
}

/// A square is handed over no larger than this a side.
pub const MAX_SQUARE: u32 = 512;

impl ThumbCut {
    /// The server's variant this cut is made from.
    pub fn variant(self) -> &'static str {
        match self {
            ThumbCut::Square => "medium",
            ThumbCut::Strip => "small",
            ThumbCut::Whole => "large",
        }
    }
}

/// A thumbnail as pixels (RGBA, 8 bit, row by row), and the session it was decrypted for.
pub struct ThumbPixels {
    pub session: SessionGeneration,
    pub width: u32,
    pub height: u32,
    pub rgba: Zeroizing<Vec<u8>>,
}

impl std::fmt::Debug for ThumbPixels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThumbPixels")
            .field("session", &self.session)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// Why no thumbnail was handed over. Never the server's words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThumbRefused {
    /// No session in memory, or not the one it was asked for.
    NotSignedIn,
    /// The server ended the session.
    SessionEnded,
    /// The local state is another account's.
    NotYours,
    Offline,
    /// The file has no stored picture of this size, is not this account's, or a new version's is still on its way.
    NoThumbnail,
    /// The stored picture is too large to decode, or does not decode.
    Unreadable,
    Failed,
}

impl ThumbRefused {
    /// A fixed word for a log line.
    pub fn kind(self) -> &'static str {
        match self {
            ThumbRefused::NotSignedIn => "not signed in",
            ThumbRefused::SessionEnded => "session ended",
            ThumbRefused::NotYours => "another account's state",
            ThumbRefused::Offline => "offline",
            ThumbRefused::NoThumbnail => "no thumbnail",
            ThumbRefused::Unreadable => "unreadable",
            ThumbRefused::Failed => "failed",
        }
    }
}

fn refused(refusal: LocalStateRefused) -> ThumbRefused {
    match refusal {
        LocalStateRefused::SessionEnded => ThumbRefused::SessionEnded,
        LocalStateRefused::NothingSynced => ThumbRefused::NoThumbnail,
        LocalStateRefused::Unreadable => ThumbRefused::Failed,
        refusal if refusal.not_this_accounts() => ThumbRefused::NotYours,
        _ => ThumbRefused::NotSignedIn,
    }
}

/// The kind of a failed fetch, read from the error's chain (never its text).
fn failure(error: anyhow::Error) -> ThumbRefused {
    // Not yet (a new version's picture is on its way), or an id the local state does not know.
    if error
        .chain()
        .any(|cause| cause.is::<ThumbnailNotYet>() || cause.is::<UnknownItem>())
    {
        return ThumbRefused::NoThumbnail;
    }
    if matches!(error_http_status(&error), Some(403 | 404)) {
        return ThumbRefused::NoThumbnail;
    }
    match kind_of(&error) {
        AccountCallError::SessionEnded => ThumbRefused::SessionEnded,
        AccountCallError::Offline => ThumbRefused::Offline,
        AccountCallError::NotSignedIn => ThumbRefused::NotSignedIn,
        AccountCallError::ServerError | AccountCallError::Unreadable => ThumbRefused::Failed,
    }
}

/// The decrypted picture as pixels, cut as asked. `None`: too large, or it does not decode.
fn cut_pixels(encoded: &[u8], cut: ThumbCut) -> Option<(u32, u32, Zeroizing<Vec<u8>>)> {
    if encoded.len() > MAX_ENCODED {
        return None;
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(encoded))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(u64::from(MAX_SIDE) * u64::from(MAX_SIDE) * 4);
    reader.limits(limits);
    let mut whole = reader.decode().ok()?.into_rgba8();
    let (w, h) = whole.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    // The part that is kept: the middle, at the proportions of the cut.
    let (keep_w, keep_h) = match cut {
        ThumbCut::Whole => (w, h),
        ThumbCut::Square => (w.min(h), w.min(h)),
        // 36 by 52: as high as the picture where it is wide enough, else as wide as it is.
        ThumbCut::Strip => {
            let wide = (u64::from(h) * 36 / 52).max(1) as u32;
            if wide <= w {
                (wide, h)
            } else {
                (w, ((u64::from(w) * 52 / 36).max(1) as u32).min(h))
            }
        }
    };
    let part = image::imageops::crop(&mut whole, (w - keep_w) / 2, (h - keep_h) / 2, keep_w, keep_h).to_image();
    let out = if cut == ThumbCut::Square && keep_w > MAX_SQUARE {
        let small = image::imageops::thumbnail(&part, MAX_SQUARE, MAX_SQUARE);
        let mut part = part.into_raw();
        part.zeroize();
        small
    } else {
        part
    };
    let mut whole = whole.into_raw();
    whole.zeroize();
    let (out_w, out_h) = out.dimensions();
    Some((out_w, out_h, Zeroizing::new(out.into_raw())))
}

impl<C: CredentialStores> Engine<C> {
    /// Every photo and video of the signed-in account's local state, newest first (then by path), in one pass.
    /// Read through the account-bound handle: a state that is not the session's answers nothing. No network.
    pub fn photos_library(&self) -> Result<PhotosLibrary, LocalStateRefused> {
        // First: a session change during the read makes the library older than the session, never newer.
        let session = self.acct.session_generation();
        let photos = self
            .local_state()?
            .with(library)?
            .ok_or(LocalStateRefused::Unreadable)?;
        Ok(PhotosLibrary { session, photos })
    }

    /// The thumbnail of the file `file_id`, decrypted in memory FOR `session` and cut as `cut` says. See the
    /// module's doc for the session rule and for what is refused before and while decoding.
    pub fn thumbnail_pixels_for(
        &self,
        session: SessionGeneration,
        file_id: &str,
        cut: ThumbCut,
    ) -> Result<ThumbPixels, ThumbRefused> {
        // The account-bound handle first (it answers nothing for a session the server ended), then the token and
        // the key of the session asked for: copied under the session's lock, and only while it is that session.
        let state = self.local_state_for_changes().map_err(refused)?;
        let (token, key) = {
            let guard = self.acct.session.lock().map_err(|_| ThumbRefused::NotSignedIn)?;
            let current = guard.as_ref().ok_or(ThumbRefused::NotSignedIn)?;
            if !self.acct.session_unchanged_since(session) {
                return Err(ThumbRefused::NotSignedIn);
            }
            (current.token.clone(), Zeroizing::new(current.master_key))
        };
        let fetched = state
            .with_shared(|db| {
                let api = Arc::new(ApiClient::new(self.config.api_base.clone(), token, *key));
                let bridge = EngineBridge::new(self.identity, db, api);
                self.runtime.block_on(async {
                    let mut fetch = std::pin::pin!(bridge.finder_thumbnail(file_id, cut.variant()));
                    loop {
                        match tokio::time::timeout(ASK_EVERY, fetch.as_mut()).await {
                            Ok(done) => break Ok(done),
                            Err(_) if !self.acct.session_unchanged_since(session) => {
                                break Err(ThumbRefused::NotSignedIn);
                            }
                            Err(_) => {}
                        }
                    }
                })
            })
            .map_err(refused)??;
        let encoded = fetched.map_err(failure)?;
        let (width, height, rgba) = cut_pixels(&encoded, cut).ok_or(ThumbRefused::Unreadable)?;
        // The hand-over: only while the session in memory is the one the token and the key were copied from.
        if !self.session_is_still(session, &key) {
            return Err(ThumbRefused::NotSignedIn);
        }
        Ok(ThumbPixels {
            session,
            width,
            height,
            rgba,
        })
    }

    /// What the head of the copy ON THIS PC says about the photo or video `file_id` (a video's length, a picture's
    /// size and camera), for `session`. A file that is online only is never downloaded for this: every field stays
    /// absent. Reads at most [`photo_meta::HEAD_BYTES`] of the file. No network.
    pub fn media_facts_for(&self, session: SessionGeneration, file_id: &str) -> Result<PhotoMeta, LocalStateRefused> {
        let state = self.local_state()?;
        if state.session() != session {
            return Err(LocalStateRefused::SessionChanged);
        }
        let entry = state
            .with(|db| db.get_file(file_id))?
            .map_err(|_| LocalStateRefused::Unreadable)?;
        let Some(entry) = entry.filter(|entry| !entry.is_dir() && entry.status == FileStatus::Local) else {
            return Ok(PhotoMeta::default());
        };
        let name = leaf_name(&entry);
        let video = mime_for_name(&name).is_some_and(|mime| mime.starts_with("video/"));
        let Some(sync_root) = crate::config::DesktopConfig::load_for(self.identity)
            .ok()
            .and_then(|config| config.sync_root)
        else {
            return Ok(PhotoMeta::default());
        };
        let facts = facts_of_copy(&sync_root, &entry.path, video);
        // Handed over only for the session it was read for.
        state.still_this_session()?;
        Ok(facts)
    }

    /// [`Engine::set_kept`] for the session of a press: nothing is changed unless the session in memory is still
    /// `session`. `Err` in the same fixed words.
    pub fn set_kept_for(&self, session: SessionGeneration, file_id: &str, kept: bool) -> Result<(), String> {
        if !self.acct.session_unchanged_since(session) {
            return Err(super::kept_api::NOT_SIGNED_IN.to_string());
        }
        self.set_kept_in(Some(session), file_id, kept)
    }
}

/// What the head of the copy at `rel_path` under `sync_root` says. Nothing for a path that leaves the folder or a
/// file that cannot be read.
fn facts_of_copy(sync_root: &std::path::Path, rel_path: &str, video: bool) -> PhotoMeta {
    let Ok(path) = local_file_path_under_sync_root(sync_root, rel_path) else {
        return PhotoMeta::default();
    };
    let mut head = Zeroizing::new(Vec::new());
    let read =
        std::fs::File::open(path).and_then(|file| file.take(photo_meta::HEAD_BYTES as u64).read_to_end(&mut head));
    if read.is_err() {
        return PhotoMeta::default();
    }
    photo_meta::from_head(&head, video)
}

/// The library of a state database. `None`: it could not be read.
fn library(db: &StateDb) -> Option<Vec<LibraryPhoto>> {
    let roots: Vec<String> = db.backups_roots().ok()?.into_iter().map(|(_, path)| path).collect();
    let mut rows: Vec<_> = live_rows(db)
        .ok()?
        .into_iter()
        .filter(|row| !row.is_dir())
        .filter_map(|row| {
            let name = leaf_name(&row);
            is_photo_or_video(&name).then_some((row, name))
        })
        .collect();
    rows.sort_by(|a, b| {
        b.0.modified_at
            .cmp(&a.0.modified_at)
            .then_with(|| a.0.path.cmp(&b.0.path))
    });
    let mut photos = Vec::with_capacity(rows.len());
    for (row, name) in rows {
        let contract = db.get_file_contract_state(&row.file_id).ok()?;
        let mut folders: Vec<String> = row
            .path
            .split('/')
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect();
        folders.pop();
        photos.push(LibraryPhoto {
            video: mime_for_name(&name).is_some_and(|mime| mime.starts_with("video/")),
            name,
            folders,
            bytes: u64::try_from(row.size_bytes).unwrap_or(0),
            modified_at: row.modified_at,
            kept: contract
                .as_ref()
                .is_some_and(|contract| contract.effective_pin_state() == PinState::Pinned),
            on_this_pc: row.status == FileStatus::Local,
            uploading: row.status == FileStatus::Uploading,
            device: photo_meta::backup_device(&roots, &row.path),
            versions: contract.as_ref().map_or(1, |contract| contract.current_version.max(1)),
            id: row.file_id,
        });
    }
    Some(photos)
}
