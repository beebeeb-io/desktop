//! Photo previews for a host (the Settings pane Thumbnails and previews, and first-run step 4): the account's preview
//! setting ([`Engine::photo_previews_for`], [`Engine::set_photo_previews_for`]), the run that makes and uploads
//! previews on this PC ([`Engine::make_previews_for`]), and the sample a window compares ([`Engine::measure_sample_for`]).
//! The module doc of [`crate::photo_previews`] says what is made, from what, and what never leaves this PC.
//!
//! Session rule: everything is asked FOR a session (the one the host read at its press). The token and the vault key
//! are copied only while that session is the one in memory; every upload asks again, right before it is sent, whether
//! the session in memory is still the very one they were copied from ([`Engine::session_is_still`]): a lock, an
//! unlock, a sign-out or another sign-in ends the run, and nothing more is sent. An answer is handed over only while
//! its session is still the one in memory.
//!
//! BLOCKS: every call here is a worker's (the network, the state database, decoding a photo). Must not: log a name, a
//! path, an id, a key or a byte value (counts and kinds only); write a preview's plaintext anywhere.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use zeroize::Zeroizing;

use super::settings_api::AccountCallError;
use super::{CredentialStores, Engine, LocalStateRefused};
use crate::account::SessionGeneration;
use crate::api_client::{ApiClient, PreviewPutError};
use crate::photo_previews::runner::{RunEnd, Stopped};
use crate::photo_previews::{
    PX_CHOICES, PreviewFormat, PreviewSetting, PreviewState, PreviewsSummary, averages, encode, plan, seal,
};

#[cfg(test)]
#[path = "photo_previews_api_tests.rs"]
mod tests;

/// The largest original that is read into memory to make a preview of.
pub const MAX_ORIGINAL: u64 = 256 * 1024 * 1024;
/// How many of the newest photos are tried for the sample before the bundled one is said.
const SAMPLE_TRIES: usize = 5;

/// Why nothing was read or changed. Fixed words only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewsRefused {
    /// No session in memory, or not the one it was asked for (a lock, a sign-out, another sign-in).
    NotSignedIn,
    /// Beebeeb no longer accepts this session.
    SessionEnded,
    Offline,
    /// Beebeeb answered with an error, or its answer did not read.
    Failed,
}

impl PreviewsRefused {
    pub fn kind(self) -> &'static str {
        match self {
            Self::NotSignedIn => "not signed in",
            Self::SessionEnded => "session ended",
            Self::Offline => "offline",
            Self::Failed => "failed",
        }
    }
}

fn from_call(error: AccountCallError) -> PreviewsRefused {
    match error {
        AccountCallError::NotSignedIn => PreviewsRefused::NotSignedIn,
        AccountCallError::SessionEnded => PreviewsRefused::SessionEnded,
        AccountCallError::Offline => PreviewsRefused::Offline,
        AccountCallError::ServerError | AccountCallError::Unreadable => PreviewsRefused::Failed,
    }
}

/// What Thumbnails and previews shows, read for one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewsFacts {
    pub session: SessionGeneration,
    pub setting: PreviewSetting,
    pub summary: PreviewsSummary,
    /// The average stored preview at each (format, px), and how many it was measured on (only where some exist).
    pub averages: Vec<((PreviewFormat, u32), u64, u64)>,
}

/// The sample a window compares, measured on this PC. Holds pictures of the person's photo: its `Debug` prints
/// sizes only, and every picture is wiped when dropped.
pub struct SampleFacts {
    pub session: SessionGeneration,
    /// `None`: no photo of the account on this PC decodes here (the window shows the picture that ships inside it).
    pub photo: Option<SamplePhoto>,
}

pub struct SamplePhoto {
    /// The original's size on disk.
    pub original_bytes: u64,
    /// What a JPEG preview of it weighs at each of [`PX_CHOICES`], encoded on this PC.
    pub weights: Vec<(u32, u64)>,
    /// The original, as PNG, its long edge at most [`encode::SHOWN_LONG_EDGE`].
    pub original_png: Zeroizing<Vec<u8>>,
    /// The JPEG preview at each of [`PX_CHOICES`], as PNG at the same shown size.
    pub previews_png: Vec<(u32, Zeroizing<Vec<u8>>)>,
}

impl std::fmt::Debug for SampleFacts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SampleFacts")
            .field("session", &self.session)
            .field("photo", &self.photo.as_ref().map(|photo| photo.original_bytes))
            .finish()
    }
}

/// Read `path` into memory (wiped when dropped), at most [`MAX_ORIGINAL`] bytes. Only a file [`plan::ready_to_read`]
/// let through is ever opened. The buffer has room for every byte before the first is read, so it is never moved
/// (a move would free a copy that is not wiped); a file that is not the size it said while read is not read.
fn read_original(path: &Path) -> Option<Zeroizing<Vec<u8>>> {
    let mut file = std::fs::File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    if size > MAX_ORIGINAL {
        return None;
    }
    let mut bytes = Zeroizing::new(vec![0u8; usize::try_from(size).ok()?]);
    file.read_exact(&mut bytes).ok()?;
    let mut more = Zeroizing::new([0u8; 1]);
    (file.read(&mut *more).ok()? == 0).then_some(bytes)
}

/// What opens an original: the disk, or a test's counter.
pub(crate) type Reader<'a> = &'a dyn Fn(&Path) -> Option<Zeroizing<Vec<u8>>>;

impl<C: CredentialStores> Engine<C> {
    /// `Err` unless the session in memory is still `session`: an answer is handed over only then.
    fn previews_handed_over(&self, session: SessionGeneration) -> Result<(), PreviewsRefused> {
        if self.acct.session_unchanged_since(session) {
            Ok(())
        } else {
            Err(PreviewsRefused::NotSignedIn)
        }
    }

    /// The account's preview setting, for `session`. A failed read is an `Err`, never "off" or "on".
    pub fn photo_previews_setting_for(&self, session: SessionGeneration) -> Result<PreviewSetting, PreviewsRefused> {
        let setting = self
            .account_call_for(session, |api| async move { Ok(api.photo_previews_setting().await?) })
            .map_err(from_call)?;
        self.previews_handed_over(session)?;
        Ok(setting)
    }

    /// The setting, what the stored previews take, and the averages per setting, for `session`.
    pub fn photo_previews_for(&self, session: SessionGeneration) -> Result<PreviewsFacts, PreviewsRefused> {
        let (setting, summary, states) = self
            .account_call_for(session, |api| async move {
                let setting = api.photo_previews_setting().await?;
                let summary = api.previews_summary().await?;
                let states = api.preview_states().await?;
                Ok((setting, summary, states))
            })
            .map_err(from_call)?;
        self.previews_handed_over(session)?;
        Ok(PreviewsFacts {
            session,
            setting,
            summary,
            averages: averages(&states),
        })
    }

    /// What the stored previews take, alone, for `session` (a window reads it again while a run makes previews).
    pub fn photo_previews_summary_for(&self, session: SessionGeneration) -> Result<PreviewsSummary, PreviewsRefused> {
        let summary = self
            .account_call_for(session, |api| async move { Ok(api.previews_summary().await?) })
            .map_err(from_call)?;
        self.previews_handed_over(session)?;
        Ok(summary)
    }

    /// WRITES the account's setting (every device reads it), for `session` and no other. Nothing else: turning
    /// previews off deletes no stored preview.
    pub fn set_photo_previews_for(
        &self,
        session: SessionGeneration,
        setting: PreviewSetting,
    ) -> Result<(), PreviewsRefused> {
        self.account_call_for(session, move |api| async move {
            Ok(api.set_photo_previews_setting(&setting).await?)
        })
        .map_err(from_call)
    }

    /// Make and upload the previews the account's setting asks for, for `session`: one photo at a time, the newest
    /// first. `force`: a rebuild (a preview that matches is made again). `stop`: set to end the run after the photo
    /// in hand. `progress(done, total)` after every photo. Never reads a photo whose bytes are not all on this PC.
    pub fn make_previews_for(
        &self,
        session: SessionGeneration,
        force: bool,
        stop: &AtomicBool,
        progress: &dyn Fn(u32, u32),
    ) -> RunEnd {
        self.make_previews_reading(session, force, stop, progress, &read_original)
    }

    pub(crate) fn make_previews_reading(
        &self,
        session: SessionGeneration,
        force: bool,
        stop: &AtomicBool,
        progress: &dyn Fn(u32, u32),
        read: Reader<'_>,
    ) -> RunEnd {
        let moved = RunEnd {
            session_moved: true,
            ..RunEnd::default()
        };
        let stopped = |why| RunEnd {
            stopped: Some(why),
            ..RunEnd::default()
        };
        // The token and the key of the session asked for: copied under the session's lock, only while it is that one.
        if self.acct.session.is_poisoned() {
            return moved;
        }
        let Some((token, key)) = self.acct.while_session_is(&self.writes, session, |current| {
            (current.token.clone(), Zeroizing::new(current.master_key))
        }) else {
            return moved;
        };
        let api = ApiClient::new(self.config.api_base.clone(), token, [0; 32]);
        let read_server = self.runtime.block_on(async {
            let setting = api.photo_previews_setting().await?;
            if !setting.enabled || setting.format != PreviewFormat::Jpeg {
                return Ok::<_, anyhow::Error>((setting, Vec::new()));
            }
            Ok((setting, api.preview_states().await?))
        });
        let (setting, states) = match read_server {
            Ok(read) => read,
            Err(error) => {
                return match super::settings_api::kind_of(&error) {
                    AccountCallError::NotSignedIn | AccountCallError::SessionEnded => moved,
                    AccountCallError::Offline => stopped(Stopped::Offline),
                    _ => stopped(Stopped::Failed),
                };
            }
        };
        if !setting.enabled {
            tracing::info!("photo previews: the setting is off, nothing is made");
            return RunEnd::default();
        }
        if setting.format != PreviewFormat::Jpeg {
            // No HEIC encoder on this PC: nothing is made, nothing is sent.
            tracing::info!("photo previews: the setting is HEIC, nothing is made on this PC");
            return stopped(Stopped::HeicNotMadeHere);
        }
        let owned: HashMap<String, PreviewState> = states.into_iter().map(|state| (state.id.clone(), state)).collect();
        let photos = match self.local_state() {
            Ok(state) if state.session() == session => match state.with(plan::local_photos) {
                Ok(Some(photos)) => photos,
                _ => return stopped(Stopped::Failed),
            },
            Ok(_)
            | Err(
                LocalStateRefused::SessionChanged
                | LocalStateRefused::SignedOut
                | LocalStateRefused::Locked
                | LocalStateRefused::SessionEnded,
            ) => return moved,
            Err(_) => return stopped(Stopped::Failed),
        };
        let Some(sync_root) = crate::config::DesktopConfig::load_for(self.identity)
            .ok()
            .and_then(|config| config.sync_root)
        else {
            return stopped(Stopped::Failed);
        };
        let planned = plan::plan(&photos, &owned, setting, force);
        let total = u32::try_from(planned.len()).unwrap_or(u32::MAX);
        let mut end = RunEnd::default();
        progress(0, total);
        for (at, photo) in planned.into_iter().enumerate() {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            match self.make_one(session, &key, &api, &sync_root, photo, setting.px, read) {
                Made::Yes => end.made += 1,
                Made::NotHere => end.not_here += 1,
                Made::No => end.failed += 1,
                Made::SessionMoved => {
                    end.session_moved = true;
                    break;
                }
                Made::Stop(why) => {
                    end.stopped = Some(why);
                    break;
                }
            }
            progress(u32::try_from(at + 1).unwrap_or(u32::MAX), total);
        }
        tracing::info!(
            made = end.made,
            not_here = end.not_here,
            failed = end.failed,
            planned = total,
            session_moved = end.session_moved,
            "photo previews: run ended"
        );
        end
    }

    /// One photo: read (only when every byte is here), encode, seal, ask the session, upload.
    #[allow(clippy::too_many_arguments)]
    fn make_one(
        &self,
        session: SessionGeneration,
        key: &[u8; 32],
        api: &ApiClient,
        sync_root: &Path,
        photo: &plan::LocalPhoto,
        px: u32,
        read: Reader<'_>,
    ) -> Made {
        let Some(path) = plan::ready_to_read(sync_root, photo) else {
            return Made::NotHere;
        };
        let Some(original) = read(&path) else {
            return Made::No;
        };
        let Ok((jpeg, _)) = encode::encode_preview(&original, px) else {
            return Made::No;
        };
        drop(original);
        let Ok(sealed) = seal::seal_preview(key, &photo.id, &jpeg) else {
            return Made::No;
        };
        drop(jpeg);
        // Right before the upload: still the very session the key and the token were copied from.
        if !self.session_is_still(session, key) {
            return Made::SessionMoved;
        }
        match self.runtime.block_on(api.upload_preview(&photo.id, px, &sealed)) {
            Ok(()) => Made::Yes,
            Err(PreviewPutError::QuotaExceeded) => Made::Stop(Stopped::StorageFull),
            Err(PreviewPutError::Offline) => Made::Stop(Stopped::Offline),
            Err(PreviewPutError::Status(401)) => Made::SessionMoved,
            Err(PreviewPutError::TooLarge | PreviewPutError::Status(_)) => Made::No,
        }
    }

    /// The sample a window compares, for `session`: `picked` (a file the person chose on this PC), else the newest of
    /// the account's own photos here that decodes. Read and measured on this PC only; nothing is uploaded.
    pub fn measure_sample_for(
        &self,
        session: SessionGeneration,
        picked: Option<PathBuf>,
    ) -> Result<SampleFacts, PreviewsRefused> {
        self.previews_handed_over(session)?;
        let upright = match picked {
            Some(path) => {
                // A picked file in the sync folder can be a placeholder: never opened then.
                if !crate::cloud_attrs::file_bytes_here(&path) {
                    return Err(PreviewsRefused::Failed);
                }
                let bytes = read_original(&path).ok_or(PreviewsRefused::Failed)?;
                let upright = encode::decode(&bytes).map_err(|_| PreviewsRefused::Failed)?;
                Some((bytes.len() as u64, upright))
            }
            None => self.newest_sample(session)?,
        };
        let photo = match upright {
            Some((original_bytes, upright)) => Some(measure(original_bytes, &upright)?),
            None => None,
        };
        self.previews_handed_over(session)?;
        Ok(SampleFacts { session, photo })
    }

    /// The newest of the account's own photos on this PC that decodes, as its size and its upright picture.
    fn newest_sample(&self, session: SessionGeneration) -> Result<Option<(u64, encode::Upright)>, PreviewsRefused> {
        let states = self
            .account_call_for(session, |api| async move { Ok(api.preview_states().await?) })
            .map_err(from_call)?;
        let owned: HashMap<String, PreviewState> = states.into_iter().map(|state| (state.id.clone(), state)).collect();
        let state = self.local_state().map_err(|_| PreviewsRefused::Failed)?;
        if state.session() != session {
            return Err(PreviewsRefused::NotSignedIn);
        }
        let photos = state
            .with(plan::local_photos)
            .ok()
            .flatten()
            .ok_or(PreviewsRefused::Failed)?;
        let Some(sync_root) = crate::config::DesktopConfig::load_for(self.identity)
            .ok()
            .and_then(|config| config.sync_root)
        else {
            return Ok(None);
        };
        for photo in photos
            .iter()
            .filter(|photo| plan::may_be_source(photo, &owned))
            .take(SAMPLE_TRIES)
        {
            let Some(path) = plan::ready_to_read(&sync_root, photo) else {
                continue;
            };
            if let Some(bytes) = read_original(&path)
                && let Ok(upright) = encode::decode(&bytes)
            {
                return Ok(Some((bytes.len() as u64, upright)));
            }
        }
        Ok(None)
    }
}

/// The weights and pictures of one sample, encoded on this PC. The largest size is scaled from the original and each
/// smaller one from the size above it (one full-size scale, not four): a JPEG of the same photo at that size, not
/// byte for byte the one the maker would make from the original.
fn measure(original_bytes: u64, upright: &encode::Upright) -> Result<SamplePhoto, PreviewsRefused> {
    let original_png = encode::shown_png(upright).map_err(|_| PreviewsRefused::Failed)?;
    let mut weights = Vec::with_capacity(PX_CHOICES.len());
    let mut previews_png = Vec::with_capacity(PX_CHOICES.len());
    let mut largest_first = PX_CHOICES;
    largest_first.sort_unstable_by(|a, b| b.cmp(a));
    let mut above: Option<encode::Upright> = None;
    for px in largest_first {
        let picture = encode::scaled(above.as_ref().unwrap_or(upright), px);
        if let Ok((jpeg, _)) = encode::jpeg_fitting(&picture) {
            weights.push((px, jpeg.len() as u64));
            if let Ok(shown) = encode::decode(&jpeg).and_then(|preview| encode::shown_png(&preview)) {
                previews_png.push((px, shown));
            }
        }
        above = Some(picture);
    }
    weights.reverse();
    previews_png.reverse();
    Ok(SamplePhoto {
        original_bytes,
        weights,
        original_png,
        previews_png,
    })
}

/// How one photo ended.
enum Made {
    Yes,
    NotHere,
    No,
    SessionMoved,
    Stop(Stopped),
}
