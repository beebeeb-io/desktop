//! An open file on the real engine: [`EngineViewerSource`], the `ViewerSource` of the running app. One source per
//! opened file (`frames::viewers::RouteSource`).
//!
//! What it shows, and from where:
//! - the name, the folders above it and the size: the signed-in account's local state, through the engine's
//!   account-bound handle ([`Engine::open_file_facts`]); read once, when the file opens.
//! - the content, on a worker, from one of two places:
//!   - a file whose bytes are wholly ON THIS PC is read from there ([`Engine::open_local_copy_in_memory`]): it
//!     opens without a connection, and a change made here that is not sent yet is what is shown, never the older
//!     version Beebeeb has. The view says where the content came from ([`ViewerSource::origin`]), and never
//!     more than is known: a copy read while sync is off is not called the version Beebeeb has; a copy with an
//!     unsent change that cannot be read is a state of its own, never answered with a download; and Beebeeb's
//!     version of a file whose copy here is not settled with it (a conflict, a failed sync) is said to be that;
//!   - any other file is downloaded and decrypted IN MEMORY by the engine's own download path
//!     ([`Engine::open_file_in_memory`]: the path every hydrate uses, no crypto here), with the engine's
//!     progress.
//!     Text, Markdown, CSV / TSV (as text), PNG / JPEG / GIF / WebP pictures and PDFs are shown. A PDF's pages are
//!     drawn by the confined helper, asked from the same worker page by page as the view shows them (`viewer_pdf`).
//! - which app opens a file: a list's Open of a file that is kept on this PC while sync runs goes to the file's
//!   own app ([`ViewerSource::kept_on_this_pc`]) when that app shows it (a document, a picture); the viewer is
//!   for a file that is not on this PC (no download to disk), for a route that names a file directly, and for
//!   source code, scripts and web pages wherever they are (`kinds::its_app_shows_it`: Enter reads one, it
//!   never runs it).
//! - not shown here: a PDF that asks for a password or cannot be read, audio, archives, RAW and HEIC pictures, Office
//!   documents. A list's Open leaves those to their own app, as before ([`ViewerSource::previews`]); a route that
//!   names one anyway (Search, a link) gets the honest state, `Body::NotPreviewable`, and nothing is downloaded.
//!   The same state, with its own words, for any file over [`IN_MEMORY_MAX`], and for a picture that declares
//!   more than the viewer decodes (`viewer_picture`, asked of the header before anything is decoded).
//! - a file that cannot be opened at all says why ([`OpenError`]): gone, the session ended, or "not right now"
//!   (asked again on Try again), never one catch-all.
//! - "Open in another app…": the system opens the file from the account's folder on this PC, where the engine
//!   already puts opened files. No copy is made here.
//! - a newer version: while a file is open the local state's version is read (on a worker, about every 1.5 s,
//!   `viewer_change::check_for_newer`); a version past the one the view holds raises "A newer version arrived, 09:14."
//!   (no device name in 0.9.0, decision 2505). Show it opens the file again (`ViewerSource::newer`).
//! - audio and video play through the shell's player (`viewer_media`); not bound: a photo's strip.
//!
//! Where the decrypted bytes are: in the engine's `Zeroizing` buffer until the worker made of them what the view
//! draws (lines of text, or pixels), then wiped. The text is overwritten when the view lets go of it
//! (`OpenFile::wipe`, `viewers::Ui`'s drop) or when an answer is never taken ([`Content`]'s drop). Decoded pixels
//! are the toolkit's memory: released with the file, not overwritten (the toolkit has no call for it). Nothing
//! is written to disk, here or in the engine.
//!
//! Session rule: the file is opened for one session (the engine's session generation) of one account. Every
//! answer carries the generation it was made for; [`ViewerSource::poll`] (the window's poll, every frame) drops an
//! answer of another generation, and the moment the session is not that one any more (a lock, a sign-out, another
//! sign-in) it cancels the download, releases the picture and hands the view a body that holds nothing: on the
//! window's side, without asking a worker. Each accessor asks the same question before it hands anything out.
//!
//! Thread rule: the download runs on a worker of its own and sends ONE answer down a channel; the worker wakes
//! the window ([`Engine::wake_host`]) as progress moves and when it is done. The facts are a local read, as a
//! listing is. Opening in another app asks the system on the window's thread, as the Files window's open does.
//!
//! Must not: log a name, an id, a path or a byte; write a decrypted byte anywhere; show a sample.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::viewer_api::{LocalCopy, OnThisPc, OpenFileError, OpenFileFacts, OpenedFile};
use beebeeb_engine::app::viewer_edit_api::{EditFacts, EditRefused, NewCopy, SaveEnd};
use beebeeb_engine::app::{CredentialStores, Engine};
use beebeeb_engine::file_ops;
use zeroize::Zeroize;

use super::log;
use beebeeb_office_protocol::office::{EditKind, EditOp, HitMode, Template};
use crate::frames::viewers::kinds::{self, PdfDoc, PhotoDoc, Picture, Viewer};
#[allow(unused_imports)] // the text readers moved to `viewer_text.rs`; tests reach them through `super::*`.
use crate::frames::viewers::source::{
    Body, Cut, Details, EditAnswer, Elsewhere, Keep, MdDoc, NoPreview, OpenError, OpenFile, Origin, PageState, SaveOutcome,
    Settled, Stopped, TextDoc, Truncated, ViewerSource, plain_lines,
};

#[path = "viewer_pages.rs"]
mod pages;
#[path = "viewer_find.rs"]
mod viewer_find;
#[path = "viewer_media.rs"]
pub(crate) mod viewer_media;
#[path = "viewer_office.rs"]
pub(crate) mod viewer_office;
#[path = "viewer_text.rs"]
mod viewer_text;
use pages::{Pages, PdfEnds};
use viewer_text::text_body;
#[path = "viewer_pdf.rs"]
pub(crate) mod viewer_pdf;
#[path = "viewer_picture.rs"]
pub(crate) mod viewer_picture;

/// How many rendered pages of the open PDF are kept (the page on screen and the ones just left).
pub const PAGES_KEPT: usize = 6;

#[path = "viewer_change.rs"]
mod change;
// Task 2530: This PC's Keep both names this PC as the viewer's does.
pub(super) use change::this_pc;
#[path = "viewer_read.rs"]
mod where_from;
use where_from::{Recorded, read};

#[cfg(test)]
#[path = "viewer_office_tests.rs"]
mod office_tests;
#[cfg(test)]
#[path = "viewer_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "viewer_window_tests.rs"]
mod window_tests;

/// The largest file a preview decrypts: it is held in memory, whole, while it is open. Larger files are not
/// downloaded for a preview at all.
pub const IN_MEMORY_MAX: u64 = 64 * 1024 * 1024;

/// A text over this is shown read-only, its first lines only (the board's DW-1527).
pub const READ_WHOLE_MAX: usize = 2_000_000;
/// A text under [`READ_WHOLE_MAX`] with more lines than this, or a line longer than [`LINE_BYTES_MAX`], is shown
/// the same way: the reader draws every line it holds.
pub const WHOLE_LINES_MAX: usize = 5_000;
pub const LINE_BYTES_MAX: usize = 20_000;
/// What a text shown in part holds: its first lines, at most this many and at most [`SHOWN_BYTES`] of them.
pub const SHOWN_LINES: usize = 500;
pub const SHOWN_BYTES: usize = 64_000;
/// A Markdown note over this is shown as its text, not rendered (the note is parsed whenever it is drawn).
pub const MARKDOWN_RENDER_MAX: usize = 256_000;

/// What the adapter asks of the engine. The running app's is [`LiveViewerEngine`]; the tests' is a double with
/// hooks, so each rule here is tested without a network.
pub trait ViewerEngine: Send + Sync + 'static {
    /// The session's generation now ([`Engine::session_turn`]). Never waits.
    fn session_turn(&self) -> SessionGeneration;
    /// A session is in memory (signed in and unlocked). Never waits.
    fn has_session(&self) -> bool;
    /// Whose session it is (the email, else the user id), as `engine_host::settings` names an account.
    fn account(&self) -> Option<String>;
    /// [`Engine::open_file_facts`].
    fn facts(&self, file_id: &str) -> Result<OpenFileFacts, OpenFileError>;
    /// [`Engine::open_file_in_memory`]. BLOCKS: a worker's.
    fn fetch(
        &self,
        file_id: &str,
        max_bytes: u64,
        progress: &(dyn Fn(u64, u64) + Send + Sync + 'static),
        cancel: &AtomicBool,
    ) -> Result<OpenedFile, OpenFileError>;
    /// [`Engine::open_local_copy_in_memory`]: the copy on this PC, read into memory. No network. A worker's.
    fn local(&self, file_id: &str, max_bytes: u64, cancel: &AtomicBool) -> Result<LocalCopy, OpenFileError>;
    /// Sync runs for the account's folder on this PC ([`Engine::is_syncing`]): the system can open a file kept
    /// there.
    fn is_syncing(&self) -> bool;
    /// Ask the system to open the file with its own app, from the account's folder on this PC. Never
    /// `Elsewhere::NotBound`.
    fn open_elsewhere(&self, file_id: &str) -> Elsewhere;
    /// The app chooser, then the file in the app picked (macOS). Default: not here.
    fn choose_elsewhere(&self, _file_id: &str) -> Elsewhere {
        Elsewhere::NotBound
    }
    /// The file shown in the file manager (macOS). Default: not here.
    fn reveal_elsewhere(&self, _file_id: &str) -> Elsewhere {
        Elsewhere::NotBound
    }
    /// Wake the window's loop: an answer or progress is waiting.
    fn wake(&self);
    /// Beebeeb is open: signed in and not locked ([`Engine::lock_state`] is `Open`). Never waits. A screen lock keeps
    /// the session in memory, so [`ViewerEngine::has_session`] alone does not say it.
    fn screen_open(&self) -> bool {
        self.has_session()
    }
    /// [`Engine::edit_facts_for`], for the engine's opening `opening` ([`ViewerEngine::begin_opening`]). `None`: not
    /// read (the switch and the editor are not offered).
    /// Playback is available here (the helper is installed and the platform has a sandbox). Default: no.
    fn media_ready(&self) -> bool {
        false
    }
    /// The shell's player. Default: none.
    fn media_player(&self) -> Option<crate::media::player::PlayerHandle> {
        None
    }
    /// The audio files of the folder the file is in, for the session. Default: not read.
    fn media_queue(&self, _session: SessionGeneration, _file_id: &str) -> Option<viewer_media::MediaQueue> {
        None
    }
    /// macOS (dm-74): whether the file's copy is Finder's to show: the account's own file (never one shared with it,
    /// ruling 1701) while Beebeeb is set up in Finder. Reads the local state: a worker's. Default: no.
    fn in_finder(&self, _file_id: &str) -> bool {
        false
    }
    /// How far a download of the whole file to this computer has come (done, total; a total of 0 is not known yet),
    /// from the transfer board ([`Engine::downloads_for`]). A worker's. Default: not known.
    fn download_progress(&self, _session: SessionGeneration, _file_id: &str) -> Option<(u64, u64)> {
        None
    }
    fn edit_facts(&self, _session: SessionGeneration, _file_id: &str, _opening: u64) -> Option<EditFacts> {
        None
    }
    /// [`Engine::set_kept_for`]. A worker's.
    fn set_kept(&self, _session: SessionGeneration, _file_id: &str, _kept: bool) -> Result<(), ()> {
        Err(())
    }
    /// [`Engine::save_text_for`], under the engine's opening `opening`. A worker's.
    fn save_text(
        &self,
        _session: SessionGeneration,
        _file_id: &str,
        _text: &str,
        _base: i64,
        _opening: u64,
    ) -> Result<SaveEnd, EditRefused> {
        Err(EditRefused::Failed)
    }
    /// [`Engine::viewer_opening`]: a new opening of `file_id`, which holds none of the viewer's earlier saves of it.
    /// Its number is handed to every read and save of this opening; a save of an earlier one is never its own.
    fn begin_opening(&self, _session: SessionGeneration, _file_id: &str) -> u64 {
        0
    }
    /// [`Engine::save_office_for`], under the engine's opening `opening`. A worker's.
    fn save_office(
        &self,
        _session: SessionGeneration,
        _file_id: &str,
        _bytes: &zeroize::Zeroizing<Vec<u8>>,
        _base: i64,
        _opening: u64,
    ) -> Result<SaveEnd, EditRefused> {
        Err(EditRefused::Failed)
    }
    /// [`Engine::save_office_copy_for`]. A worker's.
    fn save_office_copy(
        &self,
        _session: SessionGeneration,
        _file_id: &str,
        _bytes: &zeroize::Zeroizing<Vec<u8>>,
        _device: &str,
        _hhmm: &str,
    ) -> Result<NewCopy, EditRefused> {
        Err(EditRefused::Failed)
    }
    /// [`Engine::create_office_for`]: the new file's id. A worker's.
    fn create_office(
        &self,
        _session: SessionGeneration,
        _folder: Option<&str>,
        _name: &str,
        _bytes: &zeroize::Zeroizing<Vec<u8>>,
    ) -> Result<String, EditRefused> {
        Err(EditRefused::Failed)
    }
    /// [`Engine::scratch_for`]: the session's sealed scratch for the editor's large working copies. `None`: copies
    /// stay in memory.
    fn office_scratch(
        &self,
        _session: SessionGeneration,
    ) -> Option<beebeeb_engine::app::office_scratch::ScratchStore> {
        None
    }
    /// [`Engine::set_office_session_end_hook`]: a lock seals the open editors' unsaved work, a sign-out ends them,
    /// before the session does. Asked again for every editor; the same hook each time.
    fn office_install_session_end_hook(&self) {}
    /// [`Engine::put_office_snapshot_for`]: keep an editing session's snapshot of `file_id` as its one recovery blob.
    fn recovery_put(
        &self,
        _session: SessionGeneration,
        _file_id: &str,
        _base: i64,
        _bytes: &zeroize::Zeroizing<Vec<u8>>,
    ) -> Result<(), beebeeb_engine::app::office_recovery::RecoveryRefused> {
        Err(beebeeb_engine::app::office_recovery::RecoveryRefused::NotSignedIn)
    }
    /// [`Engine::office_snapshots_for`]: the account's recovery blobs.
    fn recovery_list(
        &self,
        _session: SessionGeneration,
    ) -> Result<
        Vec<beebeeb_engine::app::office_recovery::RecoveryEntry>,
        beebeeb_engine::app::office_recovery::RecoveryRefused,
    > {
        Err(beebeeb_engine::app::office_recovery::RecoveryRefused::NotSignedIn)
    }
    /// [`Engine::take_office_snapshot_for`]: the blob's bytes (the blob stays).
    fn recovery_take(
        &self,
        _session: SessionGeneration,
        _file_id: &str,
    ) -> Result<zeroize::Zeroizing<Vec<u8>>, beebeeb_engine::app::office_recovery::RecoveryRefused> {
        Err(beebeeb_engine::app::office_recovery::RecoveryRefused::NotSignedIn)
    }
    /// [`Engine::discard_office_snapshot_for`].
    fn recovery_discard(
        &self,
        _session: SessionGeneration,
        _file_id: &str,
    ) -> Result<(), beebeeb_engine::app::office_recovery::RecoveryRefused> {
        Err(beebeeb_engine::app::office_recovery::RecoveryRefused::NotSignedIn)
    }
    /// [`Engine::save_text_copy_for`]. A worker's.
    fn save_copy(
        &self,
        _session: SessionGeneration,
        _file_id: &str,
        _text: &str,
        _device: &str,
        _hhmm: &str,
    ) -> Result<NewCopy, EditRefused> {
        Err(EditRefused::Failed)
    }
}

/// The engine handle, as the viewer asks it.
pub struct LiveViewerEngine<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    /// "Open in another app" starts here, never on the window's thread.
    launcher: super::launcher::Launcher,
    /// What opens a file (the system's own; a test's stand-in).
    open: super::files_launch::Start,
}

impl<C: CredentialStores> LiveViewerEngine<C> {
    pub(super) fn new(engine: Arc<Engine<C>>) -> Self {
        Self {
            engine,
            launcher: super::launcher::Launcher::default(),
            open: super::open_path,
        }
    }
}

impl<C: CredentialStores + 'static> LiveViewerEngine<C> {
    /// The path the system opens the file from, or why not.
    fn elsewhere_path(&self, file_id: &str) -> Result<std::path::PathBuf, Elsewhere> {
        if !self.engine.is_syncing() {
            return Err(Elsewhere::NeedsSync);
        }
        let Ok(sync_root) = self.engine.sync_root() else {
            return Err(Elsewhere::NeedsSync);
        };
        let path = self.engine.local_state_for_changes().and_then(|state| {
            state.with(|db| {
                // A folder is not a file to open from a viewer.
                file_ops::resolve(db, file_id).ok().filter(|entry| !entry.is_dir())?;
                super::path_the_system_opens(db, &sync_root, file_id).ok()
            })
        });
        match path {
            Ok(Some(path)) => Ok(path),
            _ => Err(Elsewhere::Refused),
        }
    }

    /// macOS: the chooser and the app picked (`reveal` false), or Finder with the file selected (`reveal` true), on a
    /// thread of its own (the chooser waits for the person). Elsewhere: not here.
    fn mac_elsewhere(&self, file_id: &str, reveal: bool) -> Elsewhere {
        let path = match self.elsewhere_path(file_id) {
            Ok(path) => path,
            Err(why) => return why,
        };
        #[cfg(target_os = "macos")]
        {
            use super::open_with_apps::mac;
            let (engine, turn) = (self.engine.clone(), self.engine.session_turn());
            let started = std::thread::Builder::new().name("viewer-elsewhere".into()).spawn(move || {
                let done = if reveal {
                    mac::reveal(&path)
                } else if let Some(app) = mac::choose_app() {
                    if engine.session_turn() == turn {
                        mac::open_with(&app, &path)
                    } else {
                        Ok(())
                    }
                } else {
                    Ok(())
                };
                if done.is_err() {
                    log("viewer: the file did not open in another app");
                }
                engine.wake_host();
            });
            if started.is_ok() { Elsewhere::Opened } else { Elsewhere::Refused }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (path, reveal);
            Elsewhere::NotBound
        }
    }
}

impl<C: CredentialStores + 'static> ViewerEngine for LiveViewerEngine<C> {
    fn session_turn(&self) -> SessionGeneration {
        self.engine.session_turn()
    }

    fn has_session(&self) -> bool {
        self.engine.has_session()
    }

    fn account(&self) -> Option<String> {
        self.engine.account_email().or_else(|| self.engine.user_id())
    }

    fn facts(&self, file_id: &str) -> Result<OpenFileFacts, OpenFileError> {
        self.engine.open_file_facts(file_id)
    }

    fn fetch(
        &self,
        file_id: &str,
        max_bytes: u64,
        progress: &(dyn Fn(u64, u64) + Send + Sync + 'static),
        cancel: &AtomicBool,
    ) -> Result<OpenedFile, OpenFileError> {
        self.engine.open_file_in_memory(file_id, max_bytes, progress, cancel)
    }

    fn local(&self, file_id: &str, max_bytes: u64, cancel: &AtomicBool) -> Result<LocalCopy, OpenFileError> {
        self.engine.open_local_copy_in_memory(file_id, max_bytes, cancel)
    }

    fn is_syncing(&self) -> bool {
        self.engine.is_syncing()
    }

    /// What the Files window's own open does, for this one file by its id, through the same check
    /// (`engine_host::path_the_system_opens`): the path is resolved under the account-bound handle, a kind that
    /// is not opened for safety is refused, and the system is asked without a shell, after the handle answered.
    /// "Sync is off" and "refused" are two answers.
    fn open_elsewhere(&self, file_id: &str) -> Elsewhere {
        let path = match self.elsewhere_path(file_id) {
            Ok(path) => path,
            Err(why) => return why,
        };
        let asked = self.launcher.open_for(
            &self.engine,
            self.open,
            path,
            "viewer: the file did not open in another app",
        );
        match asked {
            Ok(()) => Elsewhere::Opened,
            Err(()) => Elsewhere::Refused,
        }
    }

    fn choose_elsewhere(&self, file_id: &str) -> Elsewhere {
        self.mac_elsewhere(file_id, false)
    }

    fn reveal_elsewhere(&self, file_id: &str) -> Elsewhere {
        self.mac_elsewhere(file_id, true)
    }

    fn screen_open(&self) -> bool {
        super::away::screen_open(&self.engine)
    }

    fn wake(&self) {
        self.engine.wake_host();
    }

    fn media_ready(&self) -> bool {
        viewer_media::playback_ready()
    }

    fn media_player(&self) -> Option<crate::media::player::PlayerHandle> {
        crate::media::shell_player()
    }

    fn media_queue(&self, session: SessionGeneration, file_id: &str) -> Option<viewer_media::MediaQueue> {
        viewer_media::queue_of(&self.engine, session, file_id)
    }

    #[cfg(target_os = "macos")]
    fn in_finder(&self, file_id: &str) -> bool {
        use crate::shell::platform::macos::macos_fp::open as finder;
        if finder::not_ready_line(&super::finder_setup::view()).is_some() {
            return false;
        }
        let own = self
            .engine
            .local_state_for_changes()
            .and_then(|state| state.with(|db| finder::refuse_shared(db, file_id).is_ok()));
        matches!(own, Ok(true))
    }

    fn download_progress(&self, session: SessionGeneration, file_id: &str) -> Option<(u64, u64)> {
        let rows = self.engine.downloads_for(session).ok()?;
        rows.into_iter()
            .find(|row| row.file_id == file_id)
            .map(|row| (row.done, row.total))
    }

    fn edit_facts(&self, session: SessionGeneration, file_id: &str, opening: u64) -> Option<EditFacts> {
        self.engine.edit_facts_for(session, file_id, opening).ok()
    }

    fn set_kept(&self, session: SessionGeneration, file_id: &str, kept: bool) -> Result<(), ()> {
        self.engine.set_kept_for(session, file_id, kept).map_err(|_| ())
    }

    fn save_text(
        &self,
        session: SessionGeneration,
        file_id: &str,
        text: &str,
        base: i64,
        opening: u64,
    ) -> Result<SaveEnd, EditRefused> {
        self.engine.save_text_for(session, file_id, text, base, opening)
    }

    fn save_copy(
        &self,
        session: SessionGeneration,
        file_id: &str,
        text: &str,
        device: &str,
        hhmm: &str,
    ) -> Result<NewCopy, EditRefused> {
        self.engine.save_text_copy_for(session, file_id, text, device, hhmm)
    }

    fn begin_opening(&self, session: SessionGeneration, file_id: &str) -> u64 {
        self.engine.viewer_opening(session, file_id)
    }

    fn save_office(
        &self,
        session: SessionGeneration,
        file_id: &str,
        bytes: &zeroize::Zeroizing<Vec<u8>>,
        base: i64,
        opening: u64,
    ) -> Result<SaveEnd, EditRefused> {
        self.engine.save_office_for(session, file_id, bytes, base, opening)
    }

    fn save_office_copy(
        &self,
        session: SessionGeneration,
        file_id: &str,
        bytes: &zeroize::Zeroizing<Vec<u8>>,
        device: &str,
        hhmm: &str,
    ) -> Result<NewCopy, EditRefused> {
        self.engine.save_office_copy_for(session, file_id, bytes, device, hhmm)
    }

    fn create_office(
        &self,
        session: SessionGeneration,
        folder: Option<&str>,
        name: &str,
        bytes: &zeroize::Zeroizing<Vec<u8>>,
    ) -> Result<String, EditRefused> {
        self.engine.create_office_for(session, folder, name, bytes)
    }

    fn office_scratch(
        &self,
        session: SessionGeneration,
    ) -> Option<beebeeb_engine::app::office_scratch::ScratchStore> {
        self.engine.scratch_for(session)
    }

    fn office_install_session_end_hook(&self) {
        self.engine
            .set_office_session_end_hook(std::sync::Arc::new(crate::engine_host::office_host::on_session_end));
        self.engine
            .set_office_unsaved_probe(std::sync::Arc::new(crate::engine_host::office_host::unsaved_count));
    }

    fn recovery_put(
        &self,
        session: SessionGeneration,
        file_id: &str,
        base: i64,
        bytes: &zeroize::Zeroizing<Vec<u8>>,
    ) -> Result<(), beebeeb_engine::app::office_recovery::RecoveryRefused> {
        self.engine.put_office_snapshot_for(session, file_id, base, bytes)
    }

    fn recovery_list(
        &self,
        session: SessionGeneration,
    ) -> Result<
        Vec<beebeeb_engine::app::office_recovery::RecoveryEntry>,
        beebeeb_engine::app::office_recovery::RecoveryRefused,
    > {
        self.engine.office_snapshots_for(session)
    }

    fn recovery_take(
        &self,
        session: SessionGeneration,
        file_id: &str,
    ) -> Result<zeroize::Zeroizing<Vec<u8>>, beebeeb_engine::app::office_recovery::RecoveryRefused> {
        self.engine.take_office_snapshot_for(session, file_id)
    }

    fn recovery_discard(
        &self,
        session: SessionGeneration,
        file_id: &str,
    ) -> Result<(), beebeeb_engine::app::office_recovery::RecoveryRefused> {
        self.engine.discard_office_snapshot_for(session, file_id)
    }
}

/// What the worker made of the decrypted file, for the view. Its text is overwritten if nobody takes it (the
/// file was closed while the answer was on its way).
struct Content {
    body: Option<Body>,
    picture: Option<rune::Pixels>,
    /// Where the bytes came from.
    origin: Origin,
}

impl Drop for Content {
    fn drop(&mut self) {
        if let Some(body) = self.body.as_mut() {
            body.wipe();
        }
    }
}

/// The worker's one answer: what to show and the session it was decrypted for, or why not.
type Answer = Result<(SessionGeneration, Content), OpenFileError>;

/// What the download says while it runs.
#[derive(Default)]
struct Progress {
    done: AtomicU64,
    total: AtomicU64,
    /// The whole percent the window was last woken for.
    woken_at: AtomicU64,
}

/// The file this source has open, and the session it was opened for.
struct Opening {
    file_id: String,
    session: SessionGeneration,
    account: Option<String>,
    cancel: Arc<AtomicBool>,
    progress: Arc<Progress>,
    inbox: mpsc::Receiver<Answer>,
    /// The worker has not answered yet.
    fetching: bool,
    /// The progress the view was last handed.
    drawn: (u64, u64),
    /// The view holds decrypted content of this file (text, or the picture below).
    holds: bool,
    picture: Option<rune::Pixels>,
    /// The open PDF's pages, while the file is a PDF.
    pages: Option<Pages>,
    /// Where the content the view holds came from (Beebeeb, until an answer says the copy on this PC).
    origin: Origin,
    /// The session moved on and the view was told: nothing more comes from this opening.
    ended: bool,
    /// It ended at a lock of the screen, the same account still signed in: an edit kept open across the lock is saved
    /// under the file opened again for the session after it ([`EngineViewerSource::resume`]). Another account, a
    /// sign-out: never.
    resumable: bool,
    /// What the local state said at the open: the version, whether the file is kept, whether it may be changed.
    /// `None`: not read.
    edit: Option<EditFacts>,
    /// The version the view holds, or was told of by a banner: a read past it is a version this opening did not make
    /// (another device's, or an earlier opening's on this PC). Only the window's thread writes it
    /// (`viewer_change::weigh`); it never goes down.
    seen: i64,
    /// Which opening this is (`Changes::openings`): an answer begun under another one is not this one's.
    serial: u64,
    /// The engine's number for this opening ([`ViewerEngine::begin_opening`]): only saves made under it are its own.
    epoch: u64,
    /// How the file is shown, and its name (a Keep both names the copy after it).
    shows: Shows,
    name: String,
}

/// An open file on the real engine. See the module's doc.
pub struct EngineViewerSource<E: ViewerEngine> {
    engine: Arc<E>,
    open: Option<Opening>,
    /// A change handed to a worker (Available offline, a save) and the answers that come back.
    changes: change::Changes,
    /// The recording or video on screen and its queue (`viewer_media`).
    media: viewer_media::Media,
    /// The Office file on screen: its switches, its read and its renderer (`viewer_office`).
    office: viewer_office::Office,
}

impl<C: CredentialStores + 'static> EngineViewerSource<LiveViewerEngine<C>> {
    /// Starts no thread and reads nothing until a file is opened.
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        Self::on(Arc::new(LiveViewerEngine::new(engine)))
    }
}

/// How a file is shown here, by its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Shows {
    Text,
    Markdown,
    Picture,
    Pdf,
    /// No viewer here for this kind of file: nothing is downloaded.
    Nothing,
}

/// Why a file with no viewer is not shown: a HEIC is said as that, any other kind has no viewer.
fn heic_or_no_viewer(name: &str) -> NoPreview {
    let heic = name
        .rsplit_once('.')
        .is_some_and(|(_, ext)| matches!(ext.to_ascii_lowercase().as_str(), "heic" | "heif"));
    if heic { NoPreview::Heic } else { NoPreview::NoViewer }
}

pub(super) fn shows(name: &str) -> Shows {
    let ext = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    match kinds::viewer_of(name) {
        // A CSV or TSV is shown as the text it is (the table view is not bound).
        Some(Viewer::Text | Viewer::Table) => Shows::Text,
        Some(Viewer::Markdown) => Shows::Markdown,
        // The formats the toolkit decodes. A RAW or a HEIC has no decoder here.
        Some(Viewer::Photo) if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp") => Shows::Picture,
        // 095: a HEIC or a RAW is a picture too (decoded by the confined helper) unless this PC is known to have
        // no decoder for it: then nothing is downloaded and the notice says so.
        Some(Viewer::Photo)
            if super::media::HelperPicture::of_name(name).is_some_and(|kind| !kind.decoder_missing()) =>
        {
            Shows::Picture
        }
        Some(Viewer::Pdf) => Shows::Pdf,
        _ => Shows::Nothing,
    }
}

/// Whether the Office renderer's sandbox exists on this platform: `beebeeb-office-sandbox` has a module only on
/// Windows today. Elsewhere a list's Open of an Office file goes to its own app.
pub(super) const fn office_sandbox_here() -> bool {
    cfg!(windows)
}

/// "Markdown" of "Markdown · 1.4 KB": the kind in a word, by the name.
pub(super) fn kind_word(name: &str) -> &'static str {
    // The board's words (DW-1507, DW-1508, DW-1509, DW-1511), once the Office route is on.
    if let Some(kinds::OfficeRoute::Shows(kind) | kinds::OfficeRoute::NoPreview(kind)) = kinds::office_route(name) {
        return match kind {
            kinds::OfficeKind::Docx => "Word document",
            kinds::OfficeKind::Doc => "Word 97\u{2013}2004 document",
            kinds::OfficeKind::Xlsx => "Excel workbook",
            kinds::OfficeKind::Xls => "Excel 97\u{2013}2004 workbook",
            kinds::OfficeKind::Pptx => "PowerPoint presentation",
            kinds::OfficeKind::Ppt => "PowerPoint 97\u{2013}2004 presentation",
        };
    }
    match kinds::viewer_of(name) {
        Some(Viewer::Markdown) => "Markdown",
        Some(Viewer::Text) => "Text",
        Some(Viewer::Table) => "Table",
        Some(Viewer::Photo) => "Picture",
        Some(Viewer::Pdf) => "PDF",
        Some(Viewer::Audio) => "Audio",
        Some(Viewer::Archive) => "Archive",
        Some(Viewer::Video) => "Video",
        None => "File",
    }
}

/// The toolkit's decoders, one set for the app (started with the first picture). Its waker does nothing: the
/// worker waits for its own picture.
fn decoders() -> &'static rune::ImageCache {
    static DECODERS: std::sync::OnceLock<rune::ImageCache> = std::sync::OnceLock::new();
    DECODERS.get_or_init(|| rune::ImageCache::new(Arc::new(|| {})))
}

/// The decrypted bytes as a picture, or why not. What the file declares is held to the viewer's own limits FIRST
/// (`viewer_picture::check`, from the header): a file that is not one of the four formats by its content, or
/// that declares more than the viewer decodes, never reaches a decoder. `cancel` is asked before the decode
/// starts; the decode itself cannot be stopped, and is bounded by those limits.
fn picture_body(bytes: &[u8], cancel: &AtomicBool) -> Content {
    let nothing = |why| Content {
        body: Some(Body::NotPreviewable(why)),
        picture: None,
        origin: Origin::Beebeeb,
    };
    // 095: a HEIC or a RAW never reaches a decoder of this process: the confined helper decodes it. No decoder for
    // it on this PC: the notice a HEIC always had, and Open in another app.
    if let Some(kind) = super::media::HelperPicture::of_bytes(bytes) {
        if cancel.load(Ordering::SeqCst) {
            return nothing(NoPreview::Unreadable);
        }
        return match super::media::helper_pixels(bytes) {
            Ok(pixels) => Content {
                body: Some(Body::Photo(PhotoDoc {
                    pill: None,
                    note: String::new(),
                    strip: Vec::new(),
                })),
                picture: Some(pixels),
                origin: Origin::Beebeeb,
            },
            Err(super::media::ImageRefusal::NoDecoder) => nothing(match kind {
                super::media::HelperPicture::Heif => NoPreview::Heic,
                super::media::HelperPicture::Raw => NoPreview::NoViewer,
            }),
            Err(_) => nothing(NoPreview::Unreadable),
        };
    }
    match viewer_picture::check(bytes) {
        Err(viewer_picture::Refused::NotAPicture) => return nothing(NoPreview::Unreadable),
        Err(viewer_picture::Refused::TooBig(declared)) => {
            return nothing(NoPreview::PictureTooBig {
                width: declared.width,
                height: declared.height,
            });
        }
        Ok(_) => {}
    }
    if cancel.load(Ordering::SeqCst) {
        return nothing(NoPreview::Unreadable);
    }
    match picture(bytes) {
        Some(pixels) => Content {
            body: Some(Body::Photo(PhotoDoc {
                pill: None,
                note: String::new(),
                strip: Vec::new(),
            })),
            picture: Some(pixels),
            origin: Origin::Beebeeb,
        },
        None => nothing(NoPreview::Unreadable),
    }
}

#[cfg(test)]
thread_local! {
    /// How many times this thread handed bytes to a picture decoder: a test asserts that a refusal handed none.
    static DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The bytes decoded by the toolkit's decoders, waited for on this (the worker's) thread. `None`: it does not
/// decode. The decoder reads a copy of the bytes; that copy is overwritten here once the decoder let go of it,
/// and a line is logged if it never did. Only for bytes [`viewer_picture::check`] passed.
fn picture(bytes: &[u8]) -> Option<rune::Pixels> {
    #[cfg(test)]
    DECODES.with(|count| count.set(count.get() + 1));
    let mut encoded: Arc<[u8]> = Arc::from(bytes);
    let image = decoders().load(encoded.clone());
    let decoded = settled(&image);
    // The decoder's job holds the other handle for a moment after it answered.
    let mut wiped = false;
    for _ in 0..2_000 {
        if let Some(copy) = Arc::get_mut(&mut encoded) {
            copy.zeroize();
            wiped = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    if !wiped {
        log("viewer: the picture decoder kept its copy of an open file; it was released, not overwritten");
    }
    decoded
}

/// What the view draws of the file's bytes.
fn content_of(shows: Shows, bytes: &[u8], cancel: &AtomicBool) -> Content {
    let unreadable = || Body::NotPreviewable(NoPreview::Unreadable);
    match shows {
        Shows::Text | Shows::Markdown => Content {
            body: Some(text_body(bytes, shows == Shows::Markdown).unwrap_or_else(unreadable)),
            picture: None,
            origin: Origin::Beebeeb,
        },
        Shows::Picture => picture_body(bytes, cancel),
        // A PDF is opened by the worker itself (`open_pdf`): it keeps the document for its pages.
        Shows::Pdf | Shows::Nothing => Content {
            body: Some(Body::NotPreviewable(NoPreview::NoViewer)),
            picture: None,
            origin: Origin::Beebeeb,
        },
    }
}

/// Open the PDF in `bytes` in the confined helper (a copy stays with the session, for find; both are wiped when
/// they are let go). The view's body, and the session for its pages when it opened.
fn open_pdf(bytes: zeroize::Zeroizing<Vec<u8>>, size: &str, cancel: &AtomicBool) -> (Body, Option<viewer_pdf::Session>) {
    match viewer_pdf::Session::open(viewer_pdf::start(bytes.clone()), bytes, cancel) {
        Ok(session) => (
            Body::Pdf(PdfDoc {
                pages: session.pages(),
                page: 1,
                size: size.to_string(),
            }),
            Some(session),
        ),
        // The helper, not the file and not its decryption: said as that (RC2-3).
        Err(viewer_pdf::NotOpened::Stopped) => (Body::Stopped(Stopped::HelperStopped), None),
        Err(why) => (
            Body::NotPreviewable(match why {
                viewer_pdf::NotOpened::Locked => NoPreview::PdfLocked,
                viewer_pdf::NotOpened::TooLong { pages } => NoPreview::PdfTooLong { pages },
                viewer_pdf::NotOpened::NoRenderer => NoPreview::NoViewer,
                _ => NoPreview::PdfUnreadable,
            }),
            None,
        ),
    }
}

/// A rendered page as the toolkit draws it (with the smaller copies it scales down from). `None`: the toolkit
/// did not take it.
fn page_pixels(page: viewer_pdf::Rendered) -> Option<rune::Pixels> {
    settled(&decoders().rgba(page.width, page.height, page.rgba))
}

/// The pixels of `image` once the toolkit decoded it, waited for on this (a worker's) thread. `None`: it does
/// not decode. The toolkit marks an image decoded a moment BEFORE its pixel handle holds the picture, so the
/// handle is waited for too: handed out earlier it has no size yet, and a page or a photo drawn from it in that
/// moment has no height.
fn settled(image: &rune::Image) -> Option<rune::Pixels> {
    let [width, height] = image.wait().ok()?;
    let pixels = image.pixels().clone();
    for _ in 0..2_000 {
        if pixels.size() == (width, height) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    Some(pixels)
}

/// What the view shows for a download that did not hand over content. `None`: nothing (it was cancelled).
fn body_of(error: OpenFileError) -> Option<Body> {
    Some(match error {
        OpenFileError::Cancelled => return None,
        OpenFileError::TooLarge { size } => Body::NotPreviewable(NoPreview::TooBig {
            size,
            limit: IN_MEMORY_MAX,
        }),
        // It passed the limit while it was read: no size is known, and none is said.
        OpenFileError::OverLimit => Body::NotPreviewable(NoPreview::OverLimit { limit: IN_MEMORY_MAX }),
        // The copy on this PC holds a change that is not sent, and could not be read: asked again on Try
        // again, never answered with the older version Beebeeb has.
        OpenFileError::CopyNotReadable => Body::Stopped(Stopped::CopyNotReadable),
        OpenFileError::Offline => Body::Stopped(Stopped::Offline),
        OpenFileError::SessionEnded => Body::Stopped(Stopped::SessionEnded),
        // The session, or whose local state this is, changed under the download.
        OpenFileError::NotSignedIn | OpenFileError::NotYours => Body::Stopped(Stopped::SessionChanged),
        OpenFileError::NotFound => Body::Stopped(Stopped::Gone),
        OpenFileError::NothingSynced => Body::Stopped(Stopped::NothingSynced),
        // Refused to this account (403), and a version that kept changing: named, never "not decrypted".
        OpenFileError::NoAccess => Body::Stopped(Stopped::NoAccess),
        OpenFileError::ChangedWhileOpening => Body::Stopped(Stopped::Changed),
        OpenFileError::Failed => {
            // The kind only: never the name or the file's id.
            tracing::warn!("viewer open failed: the file could not be read or decrypted");
            Body::Failed
        }
    })
}

impl<E: ViewerEngine> EngineViewerSource<E> {
    pub(crate) fn on(engine: Arc<E>) -> Self {
        Self {
            engine,
            open: None,
            changes: change::Changes::default(),
            media: viewer_media::Media::default(),
            office: viewer_office::Office::default(),
        }
    }

    /// Whether the session in memory is the one `opening` was made for: the same generation, the same account,
    /// and a session at all.
    fn is_current(engine: &E, opening: &Opening) -> bool {
        engine.has_session() && engine.session_turn() == opening.session && engine.account() == opening.account
    }

    /// The opening of `file_id`, while its session is the session.
    fn current(&self, file_id: &str) -> Option<&Opening> {
        self.open
            .as_ref()
            .filter(|opening| opening.file_id == file_id && !opening.ended && Self::is_current(&self.engine, opening))
    }

    /// Open `file_id` for the session in memory: its facts now, its content on a worker.
    fn begin(&mut self, file_id: &str) -> Result<OpenFile, OpenError> {
        // Its facts first: a refusal leaves what is open as it is (Show it keeps the version on screen open, and the
        // window says why). Then what was open before is over (a retry, Show it, or the source asked again). Never
        // while a change is on its way: its answer belongs to the opening that asked (delta check of #432).
        if self.changes.busy {
            return Err(OpenError::NotNow);
        }
        let (session, account, facts) = self.facts_now(file_id)?;
        // The bytes of a document just made are for that file's own open, no other.
        self.office_forget_made_but(file_id);
        // The new opening holds none of the viewer's earlier saves: none of them is its own, however late it lands.
        let epoch = self.engine.begin_opening(session, file_id);
        if let Some(mut before) = self.open.take() {
            before.end();
            self.office_left();
        }
        let recorded = Recorded {
            here: facts.on_this_pc.is_some(),
            unsettled: facts.unsettled_here,
        };
        let shows = shows(&facts.name);
        let (answers, inbox) = mpsc::channel();
        // A PDF: the worker keeps the document and renders the pages the view asks for.
        let (pages, pdf) = if shows == Shows::Pdf && facts.size <= IN_MEMORY_MAX {
            let size = crate::source::format_size(i64::try_from(facts.size).unwrap_or(i64::MAX));
            let (pages, ends) = Pages::open(size);
            (Some(pages), Some(ends))
        } else {
            (None, None)
        };
        self.changes.openings += 1;
        let mut opening = Opening {
            file_id: file_id.to_string(),
            session,
            account,
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(Progress::default()),
            inbox,
            fetching: false,
            drawn: (0, facts.size),
            holds: false,
            picture: None,
            pages,
            origin: Origin::Beebeeb,
            ended: false,
            resumable: false,
            // Read for the session of this open, from the local state alone, BEFORE the content: the version the
            // view is labelled with is never newer than its bytes.
            edit: self.engine.edit_facts(session, file_id, epoch),
            seen: 0,
            serial: self.changes.openings,
            epoch,
            shows,
            name: facts.name.clone(),
        };
        // The first read of the version is one interval after the open, not at it.
        self.changes.checked = Some(std::time::Instant::now());
        if let Some(edit) = opening.edit {
            opening.seen = edit.version;
        }
        let media = self.media_body(session, opening.account.clone(), file_id, &facts.name, &facts.folders);
        // An Office file goes by its own route, before the viewer's kinds (`viewer_office`).
        let office = self.office_route(&facts.name);
        let body = if let Some(route) = office {
            self.office_begin(route, &opening, &facts, recorded)
        } else if let Some(media) = media {
            media
        } else if shows == Shows::Nothing {
            Body::NotPreviewable(heic_or_no_viewer(&facts.name))
        } else if facts.size > IN_MEMORY_MAX {
            Body::NotPreviewable(NoPreview::TooBig {
                size: facts.size,
                limit: IN_MEMORY_MAX,
            })
        } else if self.fetch_on_a_worker(&opening, shows, recorded, answers, pdf) {
            opening.fetching = true;
            Body::Decrypting {
                done: 0,
                total: facts.size,
            }
        } else {
            log("viewer: could not start a thread for an open file");
            Body::Failed
        };
        let edit = opening.edit;
        self.open = Some(opening);
        Ok(file_of(file_id, facts, body, edit))
    }

    /// The facts of `file_id` for the session in memory, with the session they are for. Each refusal keeps its
    /// kind: no session, a session Beebeeb ended, a file that is not there, a PC where nothing has synced yet,
    /// a local state that is not this account's, and "not right now" (the local state could not be read, or the
    /// session changed during the read; the read is tried once more first, whether the change shows as facts
    /// of another session or as a refused read).
    fn facts_now(&self, file_id: &str) -> Result<(SessionGeneration, Option<String>, OpenFileFacts), OpenError> {
        let mut again = true;
        loop {
            if !self.engine.has_session() {
                return Err(OpenError::SignedOut);
            }
            let (session, account) = (self.engine.session_turn(), self.engine.account());
            match self.engine.facts(file_id) {
                // Facts read across a session change are not this session's.
                Ok(facts) if facts.session == session => return Ok((session, account, facts)),
                Ok(_) => {}
                Err(OpenFileError::NotFound) => return Err(OpenError::NotFound),
                // Neither says anything about the file: it is not "gone".
                Err(OpenFileError::NothingSynced) => return Err(OpenError::NothingSynced),
                Err(OpenFileError::NotYours) => return Err(OpenError::NotThisAccounts),
                Err(OpenFileError::SessionEnded) => return Err(OpenError::SessionEnded),
                // The read was refused for the session: there is none any more (the next turn of this loop says
                // signed out), or it changed during the read (asked once more).
                Err(OpenFileError::NotSignedIn) => {}
                Err(_) => return Err(OpenError::NotNow),
            }
            if !std::mem::take(&mut again) {
                return Err(OpenError::NotNow);
            }
        }
    }

    /// Read (the copy on this PC, else download and decrypt) on a worker; its one answer goes down `answers`.
    /// `false`: no worker started.
    fn fetch_on_a_worker(
        &self,
        opening: &Opening,
        shows: Shows,
        recorded: Recorded,
        answers: mpsc::Sender<Answer>,
        pdf: Option<PdfEnds>,
    ) -> bool {
        let (engine, file_id) = (self.engine.clone(), opening.file_id.clone());
        let (cancel, progress) = (opening.cancel.clone(), opening.progress.clone());
        std::thread::Builder::new()
            .name("viewer-open".into())
            .spawn(move || {
                let (waker, moving) = (engine.clone(), progress.clone());
                let report = move |done: u64, total: u64| {
                    moving.done.store(done, Ordering::SeqCst);
                    moving.total.store(total, Ordering::SeqCst);
                    // The window is woken once per whole percent, not once per chunk.
                    let percent = (done.min(total) * 100).checked_div(total).unwrap_or(100);
                    if moving.woken_at.swap(percent, Ordering::SeqCst) != percent {
                        waker.wake();
                    }
                };
                // A panic in this worker must not leave the view on "Decrypting" for good: it answers Failed.
                let (answer, document): (Answer, Option<viewer_pdf::Session>) =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let (session, bytes, origin) =
                            match read(&*engine, &file_id, recorded, IN_MEMORY_MAX, &report, &cancel) {
                                Ok(read) => read,
                                Err(error) => return (Err(error), None),
                            };
                        let (mut content, document) = match (&pdf, shows) {
                            (Some(ends), Shows::Pdf) => {
                                let (body, document) = open_pdf(bytes, &ends.size, &cancel);
                                let content = Content {
                                    body: Some(body),
                                    picture: None,
                                    origin,
                                };
                                (content, document)
                            }
                            // `bytes` is dropped at the end of this arm: the engine's buffer is wiped there.
                            _ => (content_of(shows, &bytes, &cancel), None),
                        };
                        content.origin = origin;
                        (Ok((session, content)), document)
                    }))
                    .unwrap_or_else(|_| {
                        tracing::warn!("viewer open failed: worker panic");
                        (Err(OpenFileError::Failed), None)
                    });
                // Nobody waits for the content any more (the file was closed, its session moved on, Cancel): it is
                // dropped here, its text overwritten, and never queued. What is sent says only "cancelled".
                let answer = if cancel.load(Ordering::SeqCst) {
                    drop(answer);
                    Err(OpenFileError::Cancelled)
                } else {
                    answer
                };
                let cancelled = answer.is_err();
                let _ = answers.send(answer);
                engine.wake();
                // An open PDF: this thread keeps its helper and asks it for the pages the view asks for, until the
                // file is left (the asker goes), its session moves on (`cancel`) or the helper stops. Then the
                // helper is ended, and the bytes kept for find are overwritten.
                if let (Some(document), Some(ends), false) = (document, pdf, cancelled) {
                    let page = |page, rendered: Option<viewer_pdf::Rendered>| {
                        let lacks_text = rendered.as_ref().is_some_and(|rendered| rendered.lacks_text);
                        let _ = ends.answers.send((page, rendered.and_then(page_pixels), lacks_text));
                        engine.wake();
                    };
                    // A find's answer comes the same way, under the serial it was asked with; nobody reading: dropped.
                    let found = |serial, matches| {
                        let _ = ends.found.send((serial, matches));
                        engine.wake();
                    };
                    let stopped = || {
                        ends.stopped.store(true, Ordering::SeqCst);
                        engine.wake();
                    };
                    document.serve(&ends.asks, &cancel, &page, &found, &stopped);
                }
            })
            .is_ok()
    }
}

/// The file as the view draws it. The path starts at "Files", as the window's own crumbs do.
fn file_of(file_id: &str, facts: OpenFileFacts, body: Body, edit: Option<EditFacts>) -> OpenFile {
    // The size as the Files window writes it.
    let size = crate::source::format_size(i64::try_from(facts.size).unwrap_or(i64::MAX));
    let kind = kind_word(&facts.name);
    let mut path = vec!["Files".to_string()];
    path.extend(facts.folders);
    OpenFile {
        id: file_id.to_string(),
        facts: format!("{kind} \u{00B7} {size}"),
        // The version the local state holds (0: not read, and the editor is not offered).
        version: edit.map_or(0, |edit| u32::try_from(edit.version).unwrap_or(0)),
        body,
        details: Details {
            kind: kind.to_string(),
            size,
            facts: vec![("Where", path.join(" \u{203A} "), false)],
            // Read from the local state; not drawn when it was not read (`knows_offline`).
            offline: edit.is_some_and(|edit| edit.kept),
        },
        newer: None,
        path,
        name: facts.name,
    }
}

impl<E: ViewerEngine> ViewerSource for EngineViewerSource<E> {
    fn open(&mut self, file_id: &str) -> Result<OpenFile, OpenError> {
        self.begin(file_id)
    }

    fn poll(&mut self, file_id: &str) -> Option<Body> {
        if let Some(body) = self.media_poll(file_id) {
            return Some(body);
        }
        if let Some(answer) = self.office_poll(file_id) {
            return answer;
        }
        let opening = self.open.as_mut().filter(|opening| opening.file_id == file_id)?;
        let same_account = self.engine.has_session() && self.engine.account() == opening.account;
        // Whose session it is, also while a lock keeps the key closed away (the account stays signed in).
        let same_person = opening.account.is_some() && self.engine.account() == opening.account;
        if opening.ended {
            // An answer that slipped past the worker's own check is dropped here (its text overwritten).
            while opening.inbox.try_recv().is_ok() {}
            // Another account came in, or none is signed in: this opening is over for a save too.
            opening.resumable &= same_person;
            return None;
        }
        // The session is not the one the file was opened for: everything of it goes NOW, on the window's side.
        if !Self::is_current(&self.engine, opening) {
            let had = opening.fetching || opening.holds;
            // Nothing of the earlier session is held or on its way, and the account is the same (a lock and an
            // unlock): what is on screen says nothing a session decrypted, so the opening is this session's now.
            if !had && same_account {
                opening.session = self.engine.session_turn();
                return None;
            }
            opening.end();
            // A lock (the same account): an edit the window keeps open across it can still be saved.
            opening.resumable = same_person;
            return had.then_some(Body::Stopped(Stopped::SessionChanged));
        }
        match opening.inbox.try_recv() {
            Ok(answer) => {
                opening.fetching = false;
                match answer {
                    Ok((session, mut content)) if session == opening.session => {
                        opening.picture = content.picture.take();
                        opening.origin = content.origin;
                        opening.holds = true;
                        let body = content.body.take();
                        if let (Some(pages), Some(Body::Pdf(doc))) = (&opening.pages, &body) {
                            *pages.doc.borrow_mut() = Some(doc.clone());
                        }
                        body
                    }
                    // Decrypted for another session than the one this file was opened for: never shown.
                    Ok(_) => Some(Body::Stopped(Stopped::SessionChanged)),
                    Err(error) => {
                        if !matches!(
                            error,
                            OpenFileError::Cancelled | OpenFileError::TooLarge { .. } | OpenFileError::OverLimit
                        ) {
                            log(format!("viewer: a file did not open ({})", error.kind()));
                        }
                        body_of(error)
                    }
                }
            }
            Err(mpsc::TryRecvError::Empty) if opening.fetching => {
                let now = (
                    opening.progress.done.load(Ordering::SeqCst),
                    opening.progress.total.load(Ordering::SeqCst),
                );
                // Before the first report the total is the size the local state named.
                let now = if now.1 == 0 { (now.0, opening.drawn.1) } else { now };
                (now != opening.drawn).then(|| {
                    opening.drawn = now;
                    Body::Decrypting {
                        done: now.0,
                        total: now.1,
                    }
                })
            }
            // The helper that draws the PDF's pages stopped: nothing more comes of this opening. Try again opens anew.
            Err(_) if opening.pages.as_ref().is_some_and(Pages::take_stopped) => {
                tracing::warn!("viewer: the helper stopped after the file was opened");
                opening.pages = None;
                Some(Body::Stopped(Stopped::HelperStopped))
            }
            // A page of the open PDF came back: the same document again, so the view is drawn with the page.
            Err(_) if opening.pages.as_ref().is_some_and(Pages::take_in) => opening
                .pages
                .as_ref()
                .and_then(|pages| pages.doc.borrow().clone())
                .map(Body::Pdf),
            Err(mpsc::TryRecvError::Empty) => None,
            // The worker ended without an answer.
            Err(mpsc::TryRecvError::Disconnected) => std::mem::take(&mut opening.fetching).then_some(Body::Failed),
        }
    }

    fn cancel(&mut self, file_id: &str) {
        if let Some(opening) = self.open.as_ref().filter(|opening| opening.file_id == file_id) {
            opening.cancel.store(true, Ordering::SeqCst);
        }
    }

    fn retry(&mut self, file_id: &str) -> Result<OpenFile, OpenError> {
        if let Some(answer) = self.office_retry(file_id) {
            return answer;
        }
        self.begin(file_id)
    }

    /// Show it: the file is read again as Beebeeb has it now (its content comes on the poll, as at an open). A
    /// refusal leaves the open file as it is (its reads, its editor), and the window says why.
    fn newer(&mut self, file_id: &str) -> Result<OpenFile, OpenError> {
        self.begin(file_id)
    }

    fn picture(&self, file_id: &str, which: Picture) -> Option<rune::Pixels> {
        match which {
            Picture::Photo => self.current(file_id)?.picture.clone(),
            // A page of the open PDF: the one kept, else it is asked of the worker (once) and comes later.
            Picture::Page(page) => {
                let pages = self.current(file_id)?.pages.as_ref()?;
                pages
                    .doc
                    .borrow()
                    .as_ref()
                    .filter(|doc| (1..=doc.pages).contains(&page))?;
                if let Some(held) = pages.held(page) {
                    return held;
                }
                pages.ask_page(page);
                None
            }
            // A strip and artwork are not bound: the frame draws its plain tile.
            _ => None,
        }
    }

    fn open_elsewhere(&mut self, file_id: &str) -> Elsewhere {
        if self.current(file_id).is_none() {
            return Elsewhere::Refused;
        }
        self.engine.open_elsewhere(file_id)
    }

    fn choose_elsewhere(&mut self, file_id: &str) -> Elsewhere {
        if self.current(file_id).is_none() {
            return Elsewhere::Refused;
        }
        self.engine.choose_elsewhere(file_id)
    }

    fn reveal_elsewhere(&mut self, file_id: &str) -> Elsewhere {
        if self.current(file_id).is_none() {
            return Elsewhere::Refused;
        }
        self.engine.reveal_elsewhere(file_id)
    }

    fn previews(&self, name: &str) -> bool {
        shows(name) != Shows::Nothing
            || heic_or_no_viewer(name) == NoPreview::Heic
            || self.media_previews(name) || self.office_route(name).is_some()
    }

    /// An audio or a video file kept on this PC still opens in the player when playback is available (D-MED-13); an
    /// Office file opens in the viewer while its route is on ("Open in another app…" stays one press away).
    fn opens_here_when_kept(&self, name: &str) -> bool {
        // A PDF too (W20): a kept one went to the browser; the viewer shows it, the other app stays one press away.
        self.media_previews(name)
            || shows(name) == Shows::Pdf
            || matches!(self.office_route(name), Some(kinds::OfficeRoute::Shows(_)))
    }

    fn office_live(&self) -> bool {
        office_sandbox_here()
    }

    fn office_picture(&self, file_id: &str, which: kinds::OfficePicture) -> Option<rune::Pixels> {
        self.office_picture_of(file_id, which)
    }

    fn office_cells(&self, file_id: &str, sheet: u32, first_row: u32, first_col: u32, rows: u32, cols: u32) {
        self.office_cells_of(file_id, sheet, first_row, first_col, rows, cols);
    }

    fn can_play(&self) -> bool {
        self.media_can_play()
    }

    fn play(&mut self, file_id: &str, on: bool) -> bool {
        self.media_play(file_id, on)
    }

    fn player(&self) -> Option<crate::media::player::PlayerHandle> {
        self.media_handle()
    }

    /// The local state says the file's bytes are on this PC, and sync runs for the folder they are in (so the
    /// system can open the file there). Otherwise the viewer shows it.
    fn kept_on_this_pc(&self, file_id: &str) -> bool {
        self.engine.has_session()
            && self.engine.is_syncing()
            && self
                .engine
                .facts(file_id)
                .is_ok_and(|facts| facts.session == self.engine.session_turn() && facts.on_this_pc.is_some())
    }

    fn origin(&self, file_id: &str) -> Origin {
        self.current(file_id)
            .filter(|opening| opening.holds)
            .map_or(Origin::Beebeeb, |opening| opening.origin)
    }

    fn page_state(&self, file_id: &str, page: u32) -> PageState {
        let Some(pages) = self.current(file_id).and_then(|opening| opening.pages.as_ref()) else {
            return PageState::Ready;
        };
        match pages.held(page) {
            Some(Some(_)) if pages.lacks_text(page) => PageState::LacksText,
            Some(Some(_)) => PageState::Ready,
            Some(None) => PageState::Failed,
            None => PageState::Rendering,
        }
    }

    fn finds(&self) -> bool {
        self.viewer_find_finds()
    }
    #[rustfmt::skip]
    fn find(&mut self, file_id: &str, words: &str) -> Vec<kinds::Match> { self.viewer_find_find(file_id, words) }

    /// "Save a copy…" has nowhere to go yet: the button is not drawn.
    fn saves_copies(&self) -> bool {
        false
    }

    /// The editor opens for a text or a note the account may change, whose version was read.
    fn can_save(&self) -> bool {
        self.change_facts()
            .is_some_and(|(_, edit)| edit.can_write && edit.version > 0)
            && self
                .open
                .as_ref()
                .is_some_and(|opening| matches!(opening.shows, Shows::Text | Shows::Markdown))
    }

    /// Whether the file is kept on this PC was read from the local state.
    fn knows_offline(&self) -> bool {
        self.change_facts().is_some()
    }

    fn set_offline(&mut self, file_id: &str, on: bool) -> bool {
        self.start_offline(file_id, on)
    }

    fn save(&mut self, file_id: &str, text: &str, base: u32) -> SaveOutcome {
        self.start_save(file_id, text, base)
    }

    fn resolve(&mut self, file_id: &str, text: &str, keep: Keep) -> SaveOutcome {
        self.start_resolve(file_id, text, keep)
    }

    fn settled(&mut self, file_id: &str) -> Option<Settled> {
        self.office_settled(file_id)
    }

    fn can_edit(&self, file_id: &str) -> bool {
        self.office_can_edit(file_id)
    }

    fn edit_guarded(&self, file_id: &str) -> Vec<beebeeb_office_protocol::office::DroppedPart> {
        self.office_guarded(file_id)
    }

    fn default_app(&self, file_id: &str) -> Option<String> {
        self.office_default_app(file_id)
    }

    fn edit(&mut self, file_id: &str, op: EditOp) -> bool {
        self.office_edit(file_id, op)
    }

    fn hit(&mut self, file_id: &str, page: u32, x_pt: f32, y_pt: f32, mode: HitMode) -> bool {
        self.office_hit(file_id, page, x_pt, y_pt, mode)
    }

    fn copy_selection(&mut self, file_id: &str) -> bool {
        self.office_copy(file_id)
    }

    fn layouts(&mut self, file_id: &str) -> bool {
        self.office_layouts(file_id)
    }

    fn insert_image(&mut self, file_id: &str, bytes: zeroize::Zeroizing<Vec<u8>>, png: bool) -> bool {
        self.office_insert_image(file_id, bytes, png)
    }

    fn poll_edit(&mut self, file_id: &str) -> Option<EditAnswer> {
        self.office_poll_edit(file_id)
    }

    fn create_document(&mut self, folder: Option<&str>, name: &str, kind: EditKind, template: Template) -> bool {
        self.office_create(folder, name, kind, template)
    }

    fn recovery(&self, file_id: &str) -> Option<crate::frames::viewers::source::RecoveryOffer> {
        self.office_recovery(file_id)
    }

    fn restore(&mut self, file_id: &str) -> bool {
        self.office_restore(file_id)
    }

    fn discard_recovery(&mut self, file_id: &str) {
        self.office_discard_recovery(file_id)
    }

    fn recovery_pending(&self) -> Vec<(String, String)> {
        let pending = self.office_recovery_pending();
        pending.into_iter().map(|kept| (kept.file_id, kept.at)).collect()
    }

    fn save_document(&mut self, file_id: &str, base: u32) -> SaveOutcome {
        self.office_save_document(file_id, base)
    }

    fn resolve_document(&mut self, file_id: &str, keep: Keep) -> SaveOutcome {
        self.office_resolve_document(file_id, keep)
    }

    fn answers_later(&self) -> bool {
        true
    }

    fn next_check(&self) -> Option<std::time::Duration> {
        self.next_check_due()
    }

    fn saving(&self) -> bool {
        self.changes.busy
    }
}

/// The file is left (closed, the route moved on, the account left): the download stops, the picture goes.
impl<E: ViewerEngine> Drop for EngineViewerSource<E> {
    fn drop(&mut self) {
        if let Some(opening) = self.open.as_mut() {
            opening.end();
        }
        self.media_closed();
        self.office_left();
    }
}

#[cfg(test)]
mod heic_open_tests {
    use super::*;

    #[test]
    fn a_heic_has_its_own_words_and_opens_in_the_viewer() {
        assert_eq!(heic_or_no_viewer("n.HEIC"), NoPreview::Heic);
        assert_eq!(heic_or_no_viewer("n.heif"), NoPreview::Heic);
        assert_eq!(heic_or_no_viewer("n.xyz"), NoPreview::NoViewer);
    }
}
