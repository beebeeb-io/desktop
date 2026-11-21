//! Slice M7 (`media-bind-viewer`): the player's bytes from the engine. [`EngineMediaOpener`] opens a file with the
//! engine's range reader (`Engine::open_media_for`) for the press that came WITH the item (`MediaItem::press`: the
//! session and the account the press was made under), and refuses an item of another account or with no press (the
//! binding shape of `files_trash.rs` and `stamp.rs`). M7 also creates the shell's one `PlayerHandle` with the real
//! helper, sound and media-control factories and hands it to `media::set_shell_player`.
//!
//! **The late-open guarantee lives here, in the opener.** The player opens a file on its own worker thread, after
//! the press has returned. The opener, on that thread, opens only for the item's own press
//! (`open_for(press, ..)` -> `open_media_for(press.session, ..)`), and only while the signed-in account is the
//! press's: the engine refuses a session that has ended, and [`refusal_of`] turns that into `Refusal::NotSignedIn`,
//! which the player shows. There is no process-wide stamp: a press made in Photos opens its video as one made in
//! Files does, and a stale session number of another account (each account counts its sessions from its own start)
//! never opens anything, because the account is compared too. Nothing of a file opened late plays under another
//! session.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use beebeeb_engine::app::bindings::media_stream::MediaReader;
use beebeeb_engine::app::viewer_api::OpenFileError;
use beebeeb_engine::app::{CredentialStores, Engine};
use zeroize::Zeroizing;

use super::stamp::account_of;
use crate::media::audio_out::DefaultAudio;
use crate::media::player::{MediaBytes, MediaOpener, PlayerHandle, Press, Refusal};
use crate::media::transport::SandboxedHelpers;
use crate::media::video::VideoSurface;

// The engine mirrors the protocol's longest read; one number, or the native crate (which sees both) does not build.
const _: () = assert!(beebeeb_engine::app::bindings::media_stream::MEDIA_MAX_READ == beebeeb_media_protocol::MAX_READ);

/// The engine's range reader as the player's [`MediaOpener`].
pub struct EngineMediaOpener<C: CredentialStores> {
    engine: Arc<Engine<C>>,
}

impl<C: CredentialStores> EngineMediaOpener<C> {
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        Self { engine }
    }
}

/// What the player says when the engine refuses: not signed in for a session that is gone or locked, else the file
/// could not be played. Never the engine's own words.
pub(super) fn refusal_of(error: OpenFileError) -> Refusal {
    // ux-02: the kind is written (fixed words, no size and nothing of the file); the window says less.
    tracing::warn!(
        kind = match error {
            OpenFileError::NotSignedIn => "not signed in",
            OpenFileError::SessionEnded => "session ended",
            OpenFileError::NotYours => "not yours",
            OpenFileError::NothingSynced => "nothing synced",
            OpenFileError::NotFound => "not found",
            OpenFileError::TooLarge { .. } => "too large",
            OpenFileError::OverLimit => "over limit",
            OpenFileError::CopyNotReadable => "copy not readable",
            OpenFileError::Cancelled => "cancelled",
            OpenFileError::Offline => "offline",
            OpenFileError::NoAccess => "no access",
            OpenFileError::ChangedWhileOpening => "changed while opening",
            OpenFileError::Failed => "failed",
        },
        "media file not opened"
    );
    match error {
        OpenFileError::NotSignedIn | OpenFileError::SessionEnded | OpenFileError::NotYours => Refusal::NotSignedIn,
        _ => Refusal::CouldNotPlay,
    }
}

impl<C: CredentialStores> EngineMediaOpener<C> {
    /// Whether the signed-in account is the one `press` was made under (in memory only: never waits). A press that
    /// names no account opens nothing.
    fn is_the_press_account(&self, press: &Press) -> bool {
        press.account.is_some() && account_of(&self.engine) == press.account
    }
}

impl<C: CredentialStores + 'static> MediaOpener for EngineMediaOpener<C> {
    /// No press came with it: nothing is opened (a session is never guessed from "now").
    fn open(&self, _file_id: &str) -> Result<Box<dyn MediaBytes>, Refusal> {
        Err(Refusal::NotSignedIn)
    }

    fn open_for(&self, press: &Press, file_id: &str) -> Result<Box<dyn MediaBytes>, Refusal> {
        if !self.is_the_press_account(press) {
            return Err(Refusal::NotSignedIn);
        }
        let reader = self.engine.open_media_for(press.session, file_id).map_err(refusal_of)?;
        // The account is asked again once the reader is there: another account that came in while the bytes were
        // being opened gets nothing of them (the reader is dropped, and with it its chunk cache).
        if !self.is_the_press_account(press) {
            return Err(Refusal::NotSignedIn);
        }
        Ok(Box::new(EngineBytes(reader)))
    }
}

/// Makes the shell's one player with the real helper, sound device and media controls, and registers it
/// (`media::set_shell_player`). Called once when the window is built on the engine (`shell::state::init_on_engine`).
pub fn install_shell_player<C: CredentialStores + 'static>(engine: Arc<Engine<C>>) {
    let host = Arc::clone(&engine);
    let player = PlayerHandle::new(
        Arc::new(EngineMediaOpener::new(engine)),
        Arc::new(SandboxedHelpers),
        Arc::new(DefaultAudio),
        Some(Box::new(crate::media::os_controls::shell_controls())),
        VideoSurface::new(),
    );
    // What the player's worker changes reaches the screen: the window's loop is woken for it (the first frame of a
    // video is in no scene yet, so no frame alone would).
    player.set_waker(Arc::new(move || host.wake_host()));
    crate::media::set_shell_player(player);
    // The engine's video thumbnails come from the same confined helper, never from a program found by name.
    crate::media::thumb_frame::install();
}

/// One opened file's bytes, read by the engine for the session it was opened for.
struct EngineBytes(MediaReader);

impl MediaBytes for EngineBytes {
    fn len(&self) -> u64 {
        self.0.len()
    }

    fn read_at(&mut self, offset: u64, len: u32, cancel: &AtomicBool) -> Result<Zeroizing<Vec<u8>>, ()> {
        self.0.read_at(offset, len, cancel).map_err(|_| ())
    }
}
