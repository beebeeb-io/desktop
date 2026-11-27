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
use std::sync::PoisonError;
use std::sync::atomic::{AtomicBool, Ordering};

use zeroize::Zeroizing;

use super::settings_api::AccountCallError;
use super::{CredentialStores, Engine, LocalStateRefused};
use crate::account::SessionGeneration;
use crate::api_client::{ApiClient, PreviewPutError, preview_retry_wait};
use crate::photo_previews::runner::{RunEnd, Stopped};
use crate::app::photos_api::ThumbPixels;
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

/// How long a read of the account's preview setting serves the thumbnail upload's looks.
const SETTING_LIFETIME: std::time::Duration = std::time::Duration::from_secs(600);

/// The setting last read for the thumbnail upload: which session of which account (and server) it was read for, when,
/// and what it said. Dropped when the setting is changed here ([`Engine::set_photo_previews_for`]).
type SettingRead = ((SessionGeneration, String, String), std::time::Instant, PreviewSetting);
static SETTING_READ: std::sync::Mutex<Option<SettingRead>> = std::sync::Mutex::new(None);

/// What [`Engine::upload_made_thumbnail_for`] came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MadeUpload {
    Sent,
    /// Not a thumbnail to send: none kept for this version, the server had one, or it was sent already.
    NothingToSend,
    /// The setting is off, or a metered-network or battery-saver hold stands: it stays kept and is tried at a later look.
    NotNow(crate::photo_previews::thumbs_upload::NotNow),
    Paused,
    SessionMoved,
    Offline,
    Failed,
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
        let set = self
            .account_call_for(session, move |api| async move {
                Ok(api.set_photo_previews_setting(&setting).await?)
            })
            .map_err(from_call);
        // Whatever was read before is not the setting now: the next look reads it again.
        let mine = (session, self.config.account_id.clone(), self.config.api_base.clone());
        let mut read = SETTING_READ.lock().unwrap_or_else(PoisonError::into_inner);
        if read.as_ref().is_some_and(|(at, ..)| *at == mine) {
            read.take();
        }
        drop(read);
        set
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
        // A session mutex a panic poisoned: nothing is copied, nothing is sent.
        if self.acct.session.is_poisoned() || !self.acct.session_unchanged_since(session) {
            return moved;
        }
        // Sync paused: nothing is read from the sync folder and nothing is sent.
        if self.is_paused() {
            tracing::info!("photo previews: sync is paused, nothing is made");
            return stopped(Stopped::Paused);
        }
        // This session's first reconcile pass first: before it committed, a local row can still hold a version the
        // server has replaced. Waited for before anything of the session is copied, watching the stop, a pause and
        // the session; never a timer of its own.
        if !self.first_pass_committed(session, stop) {
            return if !self.acct.session_unchanged_since(session) {
                moved
            } else if self.is_paused() {
                stopped(Stopped::Paused)
            } else {
                RunEnd::default()
            };
        }
        // The token and the key of the session asked for: copied through the one way a call reaches the session in
        // memory (the lock asked and the copy made in one hold), and only while it is that very session.
        let Ok(Some((token, key))) = self.person_session(|current, generation| {
            (generation == session).then(|| (current.token.clone(), Zeroizing::new(current.master_key)))
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
        let config = crate::config::DesktopConfig::load_for(self.identity).ok();
        let Some(sync_root) = config.as_ref().and_then(|config| config.sync_root.clone()) else {
            return stopped(Stopped::Failed);
        };
        // The uploads' speed limit (KiB/s, 0: none) paces the previews too.
        let kbps = config.map_or(0, |config| config.upload_kbps_limit);
        let planned = plan::plan(&photos, &owned, setting, force);
        let total = u32::try_from(planned.len()).unwrap_or(u32::MAX);
        let mut end = RunEnd::default();
        let mut server_errors = 0u32;
        progress(0, total);
        for (at, photo) in planned.into_iter().enumerate() {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            if self.is_paused() {
                end.stopped = Some(Stopped::Paused);
                break;
            }
            let made = self.make_one(session, &key, &api, &sync_root, photo, setting.px, read, stop, kbps);
            if !matches!(made, Made::ServerError) {
                server_errors = 0;
            }
            match made {
                Made::Yes => end.made += 1,
                Made::Halted => break,
                Made::NotHere => end.not_here += 1,
                Made::No => end.failed += 1,
                Made::ServerError => {
                    end.failed += 1;
                    server_errors += 1;
                    // Beebeeb answers with errors: wait a little longer each time, and stop after a few in a row
                    // (as when it cannot be reached), so an outage is not read, decoded and sent at every photo.
                    if server_errors >= SERVER_ERRORS_TO_STOP {
                        end.stopped = Some(Stopped::Offline);
                        break;
                    }
                    if !self.wait_while_running(session, stop, std::time::Duration::from_secs(1 << (server_errors - 1)))
                    {
                        end.session_moved = !self.acct.session_unchanged_since(session);
                        break;
                    }
                }
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

    /// A thumbnail this PC made (the local thumbnail cache, 079/095) of the owner's photo `file_id` at `version`, sent
    /// to the server once, sealed with core's thumbnail recipe, so web and phone show it and no device makes it again.
    /// `server_404`: the caller's fetch of the server's thumbnail answered "none" (noted in the cache entry). An entry
    /// is sent when so noted or when the row's stored `has_thumbnail` (dc-168) says the server has none at `version`,
    /// and never when that flag says it has one (unless the caller's 404 is fresh). Respects the account's preview setting (read now), the metered-network and
    /// battery-saver hold, and a paused sync; every send asks the session again. Called by the host right after it kept
    /// or looked up a thumbnail it made because the server had none. WORKER call: the network.
    pub fn upload_made_thumbnail_for(
        &self,
        session: SessionGeneration,
        file_id: &str,
        version: i64,
        server_404: bool,
    ) -> MadeUpload {
        use crate::photo_previews::thumbs_upload::{MadeThumb, NotNow, SendResult, Sent, send_all};
        if self.acct.session.is_poisoned() || !self.acct.session_unchanged_since(session) {
            return MadeUpload::SessionMoved;
        }
        let Some((entry, _)) = self.with_thumb_cache(session, |cache| {
            if server_404 {
                let _ = cache.mark_server_404(file_id, version);
            }
            cache.get_at(file_id, version)
        }) else {
            return MadeUpload::SessionMoved;
        };
        // dc-168: what the server's listing and ops said of this file's thumbnail at `version` (`files.has_thumbnail`).
        // A thumbnail the server has is never sent again; one it said it has not is sent without a 404 first. Not
        // known (an older row, another version): the cache entry's 404 decides, as before. A 404 the caller saw just
        // now is newer than the row and wins.
        let server_has = match self.local_state() {
            Ok(state) if state.session() == session => {
                state.with(|db| db.server_has_thumbnail(file_id, version).ok().flatten()).ok().flatten()
            }
            _ => None,
        };
        let Some(entry) = entry.filter(|entry| {
            !entry.uploaded && (server_404 || server_has.map_or(entry.server_404, |has| !has))
        }) else {
            return MadeUpload::NothingToSend;
        };
        if self.is_paused() {
            return MadeUpload::Paused;
        }
        let metered = crate::config::DesktopConfig::load_for(self.identity).ok().and_then(|c| c.metered);
        let hold = crate::power::hold_now(metered, &crate::power::SystemPower);
        if let Some(hold) = hold {
            return MadeUpload::NotNow(NotNow::Hold(hold));
        }
        let Ok(Some((token, key))) = self.person_session(|current, generation| {
            (generation == session).then(|| (current.token.clone(), Zeroizing::new(current.master_key)))
        }) else {
            return MadeUpload::SessionMoved;
        };
        // The row's own pin, as the upload path reads it.
        let pin = match self.local_state() {
            Ok(state) if state.session() == session => {
                state.with(|db| crate::engine_bridge::recorded_pin(db, &key, file_id))
            }
            _ => return MadeUpload::SessionMoved,
        };
        let Ok(Some(pin)) = pin else {
            return MadeUpload::Failed;
        };
        let api = ApiClient::new(self.config.api_base.clone(), token, [0; 32]);
        let read_key = (session, self.config.account_id.clone(), self.config.api_base.clone());
        let cached = SETTING_READ
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .filter(|(at, read, _)| *at == read_key && read.elapsed() < SETTING_LIFETIME)
            .map(|(_, _, setting)| *setting);
        let read = match cached {
            Some(setting) => Ok(setting),
            None => self.runtime.block_on(api.photo_previews_setting()),
        };
        let setting = match read {
            Ok(setting) => {
                *SETTING_READ.lock().unwrap_or_else(PoisonError::into_inner) =
                    Some((read_key, std::time::Instant::now(), setting));
                setting
            }
            Err(error) => {
                return match super::settings_api::kind_of(&anyhow::Error::from(error)) {
                    AccountCallError::NotSignedIn | AccountCallError::SessionEnded => MadeUpload::SessionMoved,
                    AccountCallError::Offline => MadeUpload::Offline,
                    _ => MadeUpload::Failed,
                };
            }
        };
        let thumb = MadeThumb {
            file_id: file_id.to_owned(),
            version,
            width: entry.width,
            height: entry.height,
            rgba: entry.rgba,
            pin,
        };
        let mut offline = false;
        let mut send = |id: &str, sealed: &[u8]| {
            // Right before the send: still the very session the key and the token were copied from.
            if !self.session_is_still(session, &key) {
                return SendResult::Stop(Stopped::Failed);
            }
            match self.runtime.block_on(api.upload_thumbnail(id, "medium", sealed, None)) {
                Ok(()) => SendResult::Sent,
                Err(error) if error.downcast_ref::<crate::api_client::DesktopApiError>().is_none() => {
                    offline = true;
                    SendResult::Stop(Stopped::Offline)
                }
                Err(_) => SendResult::Refused,
            }
        };
        let mut sent = Sent::default();
        let end = match send_all(&key, &setting, None, std::slice::from_ref(&thumb), &mut sent, &mut send) {
            Ok(end) => end,
            Err(why) => return MadeUpload::NotNow(why),
        };
        if end.sent == 1 {
            let _ = self.with_thumb_cache(session, |cache| cache.mark_uploaded(file_id, version));
            self.note_server_has_thumbnail(session, file_id);
            return MadeUpload::Sent;
        }
        if offline {
            MadeUpload::Offline
        } else if !self.acct.session_unchanged_since(session) {
            MadeUpload::SessionMoved
        } else {
            MadeUpload::Failed
        }
    }

    /// drive-complete-RS-d: what the local state says of the server's thumbnail of `file_id`, at the row's current
    /// version: `Some(false)` none (dc-168's `has_thumbnail`), `Some(true)` one is there, `None` not known (no flag,
    /// another version, an unknown id, or the session moved). A read of the state database only; never the network.
    pub fn server_thumbnail_known_for(&self, session: SessionGeneration, file_id: &str) -> Option<bool> {
        match self.local_state() {
            Ok(state) if state.session() == session => state
                .with(|db| {
                    let version = db.get_file_contract_state(file_id).ok().flatten()?.current_version;
                    db.server_has_thumbnail(file_id, version).ok().flatten()
                })
                .ok()
                .flatten(),
            _ => None,
        }
    }

    /// Why no thumbnail may be made and sent now, for `session`: a paused sync, a metered-network or battery-saver
    /// hold, the account's preview setting off, a session that moved, or Beebeeb out of reach. `None`: it may. A host
    /// asks this before it downloads a file to make its thumbnails (the repair), so nothing is downloaded for a
    /// thumbnail that would not be sent. WORKER call: the network (the setting, read at most every ten minutes).
    pub fn thumbnail_send_hold_for(&self, session: SessionGeneration) -> Option<MadeUpload> {
        match self.thumb_send_context(session) {
            Ok(_) => None,
            Err(why) => Some(why),
        }
    }

    /// drive-complete-RS-d: thumbnails made on this PC from a file's decoded pixels (the viewer's open, the repair),
    /// sealed with core's thumbnail recipe and sent: the medium one (what every grid shows) and, best effort, the
    /// large one and the photo preview the account's setting asks for. Sent only when the server has none:
    /// `server_404` (the caller's fetch said so just now) or the row's `has_thumbnail` says none at its current
    /// version. Once the medium one is on the server, the row's `has_thumbnail` is set. Respects the setting, the
    /// holds and the pause, as [`Engine::upload_made_thumbnail_for`] does; every send asks the session again.
    /// `pixels` is upright RGBA, its long edge at most a few thousand pixels. WORKER call: the network.
    pub fn upload_generated_thumbnails_for(
        &self,
        session: SessionGeneration,
        file_id: &str,
        pixels: &ThumbPixels,
        server_404: bool,
    ) -> MadeUpload {
        use crate::photo_previews::thumbs_upload::{MadeThumb, SendResult, Sent, send_all};
        if pixels.session != session
            || self.acct.session.is_poisoned()
            || !self.acct.session_unchanged_since(session)
        {
            return MadeUpload::SessionMoved;
        }
        let rgba_len = u64::from(pixels.width) * u64::from(pixels.height) * 4;
        if pixels.width == 0 || pixels.height == 0 || pixels.rgba.len() as u64 != rgba_len {
            return MadeUpload::Failed;
        }
        let version = match self.local_state() {
            Ok(state) if state.session() == session => state
                .with(|db| db.get_file_contract_state(file_id).ok().flatten().map(|c| c.current_version))
                .ok()
                .flatten(),
            _ => return MadeUpload::SessionMoved,
        };
        let Some(version) = version else {
            return MadeUpload::NothingToSend;
        };
        let server_has = self.server_thumbnail_known_for(session, file_id);
        if !(server_404 || server_has == Some(false)) {
            return MadeUpload::NothingToSend;
        }
        let (api, key, setting) = match self.thumb_send_context(session) {
            Ok(context) => context,
            Err(why) => return why,
        };
        let pin = match self.local_state() {
            Ok(state) if state.session() == session => {
                state.with(|db| crate::engine_bridge::recorded_pin(db, &key, file_id))
            }
            _ => return MadeUpload::SessionMoved,
        };
        let Ok(Some(pin)) = pin else {
            return MadeUpload::Failed;
        };
        let thumb = MadeThumb {
            file_id: file_id.to_owned(),
            version,
            width: pixels.width,
            height: pixels.height,
            rgba: pixels.rgba.clone(),
            pin,
        };
        let mut offline = false;
        let mut send = |id: &str, sealed: &[u8]| {
            // Right before the send: still the very session the key and the token were copied from.
            if !self.session_is_still(session, &key) {
                return SendResult::Stop(Stopped::Failed);
            }
            match self.runtime.block_on(api.upload_thumbnail(id, "medium", sealed, None)) {
                Ok(()) => SendResult::Sent,
                Err(error) if error.downcast_ref::<crate::api_client::DesktopApiError>().is_none() => {
                    offline = true;
                    SendResult::Stop(Stopped::Offline)
                }
                Err(_) => SendResult::Refused,
            }
        };
        let mut sent = Sent::default();
        let end = match send_all(&key, &setting, None, std::slice::from_ref(&thumb), &mut sent, &mut send) {
            Ok(end) => end,
            Err(why) => return MadeUpload::NotNow(why),
        };
        drop(thumb);
        if end.sent != 1 {
            return if offline {
                MadeUpload::Offline
            } else if !self.acct.session_unchanged_since(session) {
                MadeUpload::SessionMoved
            } else {
                MadeUpload::Failed
            };
        }
        self.note_server_has_thumbnail(session, file_id);
        // Best effort, after the one every grid shows: the large thumbnail, then the photo preview.
        if let Ok(sealed) = seal_large(&key, file_id, pixels, pin)
            && self.session_is_still(session, &key)
            && self.runtime.block_on(api.upload_thumbnail(file_id, "large", &sealed, None)).is_err()
        {
            tracing::info!("generated thumbnails: the large thumbnail was not sent");
        }
        if setting.format == PreviewFormat::Jpeg
            && let Some(jpeg) = preview_from_pixels(pixels, setting.px)
            && let Ok(sealed) = seal::seal_preview(&key, file_id, &jpeg)
        {
            drop(jpeg);
            if self.session_is_still(session, &key)
                && self.runtime.block_on(api.upload_preview(file_id, setting.px, &sealed)).is_err()
            {
                tracing::info!("generated thumbnails: the photo preview was not sent");
            }
        }
        MadeUpload::Sent
    }

    /// drive-complete-RS-d: the account's media files with no server thumbnail, for the repair: pictures by their
    /// name, not folders, not on their way out, whose `has_thumbnail` says none (`server_said_none`) or is not known
    /// at their current version. One already said to have one is left out. In path order, so a folder is done
    /// together.
    pub fn files_missing_thumbnails_for(&self, session: SessionGeneration) -> Result<Vec<MissingThumb>, PreviewsRefused> {
        use crate::drive_v2::model::{leaf_name, mime_for_name};
        use crate::state_db::FileStatus;
        let state = match self.local_state() {
            Ok(state) if state.session() == session => state,
            _ => return Err(PreviewsRefused::NotSignedIn),
        };
        let found = state
            .with(|db| -> Option<Vec<MissingThumb>> {
                let mut rows = db.list_files().ok()?;
                rows.sort_by(|a, b| a.path.cmp(&b.path));
                let mut missing = Vec::new();
                for row in rows {
                    if row.is_dir() || matches!(row.status, FileStatus::Trashing | FileStatus::Conflict) {
                        continue;
                    }
                    let name = leaf_name(&row);
                    if !mime_for_name(&name).is_some_and(|mime| mime.starts_with("image/")) {
                        continue;
                    }
                    let Some(version) =
                        db.get_file_contract_state(&row.file_id).ok().flatten().map(|c| c.current_version)
                    else {
                        continue;
                    };
                    let flag = db.server_has_thumbnail(&row.file_id, version).ok().flatten();
                    if flag == Some(true) {
                        continue;
                    }
                    missing.push(MissingThumb {
                        file_id: row.file_id,
                        name,
                        server_said_none: flag == Some(false),
                    });
                }
                Some(missing)
            })
            .map_err(|_| PreviewsRefused::NotSignedIn)?
            .ok_or(PreviewsRefused::Failed)?;
        self.previews_handed_over(session)?;
        Ok(found)
    }

    /// The row's `has_thumbnail` set: the server holds a thumbnail of `file_id` at the row's current version now.
    fn note_server_has_thumbnail(&self, session: SessionGeneration, file_id: &str) {
        if let Ok(state) = self.local_state_for_changes()
            && state.session() == session
        {
            let _ = state.with(|db| db.set_server_has_thumbnail(file_id, true));
        }
    }

    /// What every thumbnail send needs, asked in this order: not paused, no hold, the session's token and key (only
    /// while it is that very session), and the account's preview setting on (read at most every ten minutes).
    fn thumb_send_context(
        &self,
        session: SessionGeneration,
    ) -> Result<(ApiClient, Zeroizing<[u8; 32]>, PreviewSetting), MadeUpload> {
        use crate::photo_previews::thumbs_upload::NotNow;
        if self.acct.session.is_poisoned() || !self.acct.session_unchanged_since(session) {
            return Err(MadeUpload::SessionMoved);
        }
        if self.is_paused() {
            return Err(MadeUpload::Paused);
        }
        let metered = crate::config::DesktopConfig::load_for(self.identity).ok().and_then(|c| c.metered);
        if let Some(hold) = crate::power::hold_now(metered, &crate::power::SystemPower) {
            return Err(MadeUpload::NotNow(NotNow::Hold(hold)));
        }
        let Ok(Some((token, key))) = self.person_session(|current, generation| {
            (generation == session).then(|| (current.token.clone(), Zeroizing::new(current.master_key)))
        }) else {
            return Err(MadeUpload::SessionMoved);
        };
        let api = ApiClient::new(self.config.api_base.clone(), token, [0; 32]);
        let setting = self.thumb_setting(session, &api)?;
        if !setting.enabled {
            return Err(MadeUpload::NotNow(NotNow::SettingOff));
        }
        Ok((api, key, setting))
    }

    /// The account's preview setting for the thumbnail sends: the read of the last ten minutes for this session of
    /// this account, else read now (and kept).
    fn thumb_setting(&self, session: SessionGeneration, api: &ApiClient) -> Result<PreviewSetting, MadeUpload> {
        let read_key = (session, self.config.account_id.clone(), self.config.api_base.clone());
        let cached = SETTING_READ
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .filter(|(at, read, _)| *at == read_key && read.elapsed() < SETTING_LIFETIME)
            .map(|(_, _, setting)| *setting);
        if let Some(setting) = cached {
            return Ok(setting);
        }
        match self.runtime.block_on(api.photo_previews_setting()) {
            Ok(setting) => {
                *SETTING_READ.lock().unwrap_or_else(PoisonError::into_inner) =
                    Some((read_key, std::time::Instant::now(), setting));
                Ok(setting)
            }
            Err(error) => Err(match super::settings_api::kind_of(&anyhow::Error::from(error)) {
                AccountCallError::NotSignedIn | AccountCallError::SessionEnded => MadeUpload::SessionMoved,
                AccountCallError::Offline => MadeUpload::Offline,
                _ => MadeUpload::Failed,
            }),
        }
    }

    /// Whether this session's first reconcile pass has committed (the engine's queue sends nothing before it either):
    /// every session put in memory asks for one (`auth_resume_asked`), and the engine marks it served once its pass
    /// committed (`auth_resume_done`). Waits for it in short steps; `false` as soon as `stop` is set, sync is paused or
    /// the session moved. A Mac runs no reconcile pass (its engine serves a sign-in's resume as it starts), so nothing
    /// would ever mark one there: a Mac does not wait, and the version rule of the plan still holds.
    fn first_pass_committed(&self, session: SessionGeneration, stop: &AtomicBool) -> bool {
        if cfg!(target_os = "macos") {
            return true;
        }
        loop {
            if self.acct.auth_resume_done.load(Ordering::SeqCst) >= self.acct.auth_resume_asked.load(Ordering::SeqCst) {
                return true;
            }
            if stop.load(Ordering::SeqCst) || self.is_paused() || !self.acct.session_unchanged_since(session) {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    /// `photo`'s row read again for `session`: `None` while it is still as planned ([`plan::still_as_planned`]),
    /// else how this photo ends (`NotHere`: the row moved; `SessionMoved`: the session did).
    fn row_moved(&self, session: SessionGeneration, photo: &plan::LocalPhoto) -> Option<Made> {
        match self.local_state() {
            Ok(state) if state.session() == session => match state.with(|db| plan::still_as_planned(db, photo)) {
                Ok(true) => None,
                Ok(false) => Some(Made::NotHere),
                Err(_) => Some(Made::SessionMoved),
            },
            Ok(_)
            | Err(
                LocalStateRefused::SessionChanged
                | LocalStateRefused::SignedOut
                | LocalStateRefused::Locked
                | LocalStateRefused::SessionEnded,
            ) => Some(Made::SessionMoved),
            Err(_) => Some(Made::NotHere),
        }
    }

    /// Whether the entry at `photo`'s place is its own row's ([`plan::own_place`]), read for `session`: `false` once
    /// the session moved, or when it cannot be read.
    fn place_is_its_own(&self, session: SessionGeneration, sync_root: &Path, photo: &plan::LocalPhoto) -> bool {
        match self.local_state() {
            Ok(state) if state.session() == session => {
                state.with(|db| plan::own_place(db, sync_root, photo)).unwrap_or(false)
            }
            _ => false,
        }
    }

    /// Sleep `wait` in short steps; `false` as soon as `stop` is set or the session moved (nothing more is sent
    /// then, and the run's copy of the key goes with the run).
    fn wait_while_running(&self, session: SessionGeneration, stop: &AtomicBool, wait: std::time::Duration) -> bool {
        let began = std::time::Instant::now();
        let running = || !stop.load(Ordering::SeqCst) && self.acct.session_unchanged_since(session);
        while began.elapsed() < wait {
            if !running() {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(50).min(wait.saturating_sub(began.elapsed())));
        }
        running()
    }

    /// One photo: ask the session, check its row is still as planned, read (only when every byte is here), encode,
    /// ask the session again, seal, and upload: `stop`, the session and the row are asked right before every send. A
    /// 429 is waited out. A panic while reading or encoding ends this photo only. `kbps` (KiB/s, 0: none): after a
    /// send, the rest of its time at that speed is waited out. Every wait ends at once on `stop` or a session change.
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
        stop: &AtomicBool,
        kbps: u64,
    ) -> Made {
        // A lock, a vault close or a sign-out since the last photo: nothing more is opened.
        if !self.acct.session_unchanged_since(session) {
            return Made::SessionMoved;
        }
        // Its row again: what the engine changed since the plan (a pull, an edit here, a held write) waits for a
        // later run.
        if let Some(made) = self.row_moved(session, photo) {
            return made;
        }
        // Another item's entry at its place (one name, or one name but for case): never read as this photo.
        if !self.place_is_its_own(session, sync_root, photo) {
            return Made::NotHere;
        }
        let Some(path) = plan::ready_to_read(sync_root, photo) else {
            return Made::NotHere;
        };
        let encoded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let original = read(&path)?;
            encode::encode_preview(&original, px).ok().map(|(jpeg, _)| jpeg)
        }));
        let jpeg = match encoded {
            Ok(Some(jpeg)) => jpeg,
            Ok(None) => return Made::No,
            Err(_) => {
                tracing::warn!("photo previews: a photo could not be read or encoded (a panic), it is skipped");
                return Made::No;
            }
        };
        // Nothing is sealed for a session that moved while this photo was encoded.
        if !self.acct.session_unchanged_since(session) {
            return Made::SessionMoved;
        }
        #[cfg(test)]
        SEALED.with(|sealed| sealed.set(sealed.get() + 1));
        let Ok(sealed) = seal::seal_preview(key, &photo.id, &jpeg) else {
            return Made::No;
        };
        drop(jpeg);
        let (mut retries, mut waited) = (0, 0);
        loop {
            // Right before every send: not stopped (Off sends nothing more), and still the very session the key and
            // the token were copied from.
            if stop.load(Ordering::SeqCst) {
                return Made::Halted;
            }
            if !self.session_is_still(session, key) {
                return Made::SessionMoved;
            }
            // A version pulled since the photo was read: its bytes were the older version's, nothing is sent.
            if let Some(made) = self.row_moved(session, photo) {
                return made;
            }
            let sent_at = std::time::Instant::now();
            match self.runtime.block_on(api.upload_preview(&photo.id, px, &sealed)) {
                Ok(()) => {
                    if kbps > 0 {
                        let budget = std::time::Duration::from_secs_f64(sealed.len() as f64 / (kbps as f64 * 1024.0));
                        self.wait_while_running(session, stop, budget.saturating_sub(sent_at.elapsed()));
                    }
                    return Made::Yes;
                }
                Err(PreviewPutError::RateLimited { retry_after }) => {
                    let Some(wait) = preview_retry_wait(retries, waited, retry_after) else {
                        return Made::No;
                    };
                    if !self.wait_while_running(session, stop, std::time::Duration::from_secs(wait)) {
                        return if self.acct.session_unchanged_since(session) {
                            Made::Halted
                        } else {
                            Made::SessionMoved
                        };
                    }
                    retries += 1;
                    waited += wait;
                }
                Err(PreviewPutError::QuotaExceeded) => return Made::Stop(Stopped::StorageFull),
                Err(PreviewPutError::Offline) => return Made::Stop(Stopped::Offline),
                Err(PreviewPutError::Status(401)) => return Made::SessionMoved,
                Err(PreviewPutError::Status(500..=599)) => return Made::ServerError,
                Err(PreviewPutError::TooLarge | PreviewPutError::Status(_)) => return Made::No,
            }
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
            // Another item's entry at its place (one name, or one name but for case) is never its sample.
            if !state.with(|db| plan::own_place(db, &sync_root, photo)).unwrap_or(false) {
                continue;
            }
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

#[cfg(test)]
thread_local! {
    /// How many previews this thread sealed (tests: nothing is sealed for a session that moved).
    pub(crate) static SEALED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Server errors (5xx) in a row that stop the run.
const SERVER_ERRORS_TO_STOP: u32 = 3;

/// How one photo ended.
enum Made {
    Yes,
    /// Beebeeb answered with a server error (5xx).
    ServerError,
    /// The run was asked to stop while it waited to send again.
    Halted,
    NotHere,
    No,
    SessionMoved,
    Stop(Stopped),
}

/// A media file with no server thumbnail, as the repair walks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingThumb {
    pub file_id: String,
    /// Its name (for the kind of file it is; never logged).
    pub name: String,
    /// The row's `has_thumbnail` says the server has none. `false`: not known (a host asks the server first).
    pub server_said_none: bool,
}

/// The large thumbnail of `pixels`, sealed for `file_id` with core's thumbnail recipe (the one the upload path uses
/// for its large variant): owner-class ids only.
fn seal_large(
    master: &[u8; 32],
    file_id: &str,
    pixels: &ThumbPixels,
    pin: beebeeb_core::chunk_stream::ExpectedChunkFormat,
) -> Result<Vec<u8>, ()> {
    use beebeeb_core::item_id::{ItemClass, item_class};
    use beebeeb_core::kdf::{MasterKey, derive_file_key};
    use beebeeb_core::thumbnail::{ThumbnailConfig, generate_thumbnail};
    if !matches!(item_class(file_id), Ok(ItemClass::Owner)) {
        return Err(());
    }
    let made =
        generate_thumbnail(&pixels.rgba, pixels.width, pixels.height, &ThumbnailConfig::large()).map_err(|_| ())?;
    let webp = Zeroizing::new(made.data);
    let key = derive_file_key(&MasterKey::from_bytes(*master), file_id.as_bytes());
    #[allow(deprecated)]
    let sealed = beebeeb_core::encrypt::encrypt_thumbnail_for(&key, &webp, pin).map_err(|_| ())?;
    if sealed.len() > LARGE_SEALED_MAX {
        return Err(());
    }
    Ok(sealed)
}

/// The server stores a large thumbnail up to this many sealed bytes.
const LARGE_SEALED_MAX: usize = 512 * 1024;

/// A JPEG photo preview of upright RGBA `pixels`, its long edge at most `px`: quality 85, else 70, else none (over
/// the preview's cap). Every buffer is wiped when dropped.
fn preview_from_pixels(pixels: &ThumbPixels, px: u32) -> Option<Zeroizing<Vec<u8>>> {
    use zeroize::Zeroize;
    let mut rgb = Vec::with_capacity(pixels.rgba.len() / 4 * 3);
    for p in pixels.rgba.chunks_exact(4) {
        rgb.extend_from_slice(&p[..3]);
    }
    let mut whole = image::RgbImage::from_raw(pixels.width, pixels.height, rgb)?;
    let (w, h) = encode::fitted(pixels.width, pixels.height, px);
    let mut picture = if (w, h) == (pixels.width, pixels.height) {
        whole
    } else {
        let scaled = image::imageops::resize(&whole, w, h, image::imageops::FilterType::Lanczos3);
        let raw: &mut [u8] = &mut whole;
        raw.zeroize();
        scaled
    };
    let mut out = None;
    for quality in [85u8, 70] {
        // Room for the whole picture up front: the encoder's output never moves (a move leaves a copy not wiped).
        let mut jpeg = Zeroizing::new(Vec::with_capacity(w as usize * h as usize * 3 + 64 * 1024));
        let encoded = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut *jpeg, quality)
            .encode(picture.as_raw(), w, h, image::ExtendedColorType::Rgb8)
            .is_ok();
        if encoded && jpeg.len() <= seal::MAX_PLAINTEXT {
            out = Some(jpeg);
            break;
        }
    }
    let raw: &mut [u8] = &mut picture;
    raw.zeroize();
    out
}
