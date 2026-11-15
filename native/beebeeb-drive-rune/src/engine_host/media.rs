//! Slice M7 (`media-bind-viewer`): the player's bytes from the engine. [`EngineMediaOpener`] opens a file with the
//! engine's range reader (`Engine::open_media_for`) for the session stamped at the press, and an answer that comes
//! under another session is dropped (the binding shape of `files_trash.rs` and `stamp.rs`). M7 also creates the
//! shell's one `PlayerHandle` with the real helper, sound and media-control factories and hands it to
//! `media::set_shell_player`.
//!
//! The press that opens the player stamps its session ([`stamp_press`]); the opener reads the stamp, never the
//! engine's current session, so a file is opened for the session that was pressed or not at all.
//!
//! **The late-open guarantee lives here, in the opener.** The player opens a file on its own worker thread, after
//! the press has returned. The opener, on that thread, opens only for the session stamped at the press
//! (`stamped()` -> `open_media_for(session, ..)`); the engine refuses a session that has ended, and [`refusal_of`]
//! turns that into `Refusal::NotSignedIn`, which the player shows. Nothing of a file opened late plays under
//! another session.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, PoisonError};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::bindings::media_stream::MediaReader;
use beebeeb_engine::app::viewer_api::OpenFileError;
use beebeeb_engine::app::{CredentialStores, Engine};
use zeroize::Zeroizing;

use crate::media::audio_out::DefaultAudio;
use crate::media::os_controls::{NowPlaying, OsButton, OsControls};
use crate::media::player::{MediaBytes, MediaOpener, NowPlayingSink, PlayerHandle, Refusal};
use crate::media::transport::SandboxedHelpers;
use crate::media::video::VideoSurface;

// The engine mirrors the protocol's longest read; one number, or the native crate (which sees both) does not build.
const _: () = assert!(beebeeb_engine::app::bindings::media_stream::MEDIA_MAX_READ == beebeeb_media_protocol::MAX_READ);

/// The session of the last press that opens the player. One player per process (`media::set_shell_player`), so one
/// stamp: the press writes it right before it asks the player to open, and the opener reads it when the player
/// asks for the bytes.
static PRESS: Mutex<Option<SessionGeneration>> = Mutex::new(None);

/// A press that opens a recording or a video: the session the engine must open its bytes for.
pub fn stamp_press(session: SessionGeneration) {
    *PRESS.lock().unwrap_or_else(PoisonError::into_inner) = Some(session);
}

fn stamped() -> Option<SessionGeneration> {
    *PRESS.lock().unwrap_or_else(PoisonError::into_inner)
}

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
    match error {
        OpenFileError::NotSignedIn | OpenFileError::SessionEnded | OpenFileError::NotYours => Refusal::NotSignedIn,
        _ => Refusal::CouldNotPlay,
    }
}

impl<C: CredentialStores + 'static> MediaOpener for EngineMediaOpener<C> {
    fn open(&self, file_id: &str) -> Result<Box<dyn MediaBytes>, Refusal> {
        // No press stamped it: nothing is opened (a session is never guessed from "now").
        let session = stamped().ok_or(Refusal::NotSignedIn)?;
        match self.engine.open_media_for(session, file_id) {
            Ok(reader) => Ok(Box::new(EngineBytes(reader))),
            Err(error) => Err(refusal_of(error)),
        }
    }
}

/// The OS's media controls of the main window, attached when the window exists (it does not yet when the player is
/// made). `show` runs on the player's thread; attaching is asked of the controls' own `attach`.
struct LazyControls {
    window: Arc<Mutex<Option<rune::WindowHandle>>>,
    controls: Option<OsControls>,
}

impl LazyControls {
    fn attached(&mut self) -> Option<&mut OsControls> {
        if self.controls.is_none() {
            let window = self.window.lock().unwrap_or_else(PoisonError::into_inner);
            self.controls = window.as_ref().and_then(OsControls::attach);
        }
        self.controls.as_mut()
    }
}

impl NowPlayingSink for LazyControls {
    fn show(&mut self, now: &NowPlaying) {
        if let Some(controls) = self.attached() {
            controls.show(now);
        }
    }

    fn clear(&mut self) {
        if let Some(controls) = self.controls.as_mut() {
            controls.clear();
        }
    }

    fn take_buttons(&mut self) -> Vec<OsButton> {
        self.controls.as_mut().map(OsControls::take_buttons).unwrap_or_default()
    }
}

/// Makes the shell's one player with the real helper, sound device and media controls, and registers it
/// (`media::set_shell_player`). Called once when the window is built on the engine (`shell::state::init_on_engine`).
pub fn install_shell_player<C: CredentialStores + 'static>(
    engine: Arc<Engine<C>>,
    window: Arc<Mutex<Option<rune::WindowHandle>>>,
) {
    let player = PlayerHandle::new(
        Arc::new(EngineMediaOpener::new(engine)),
        Arc::new(SandboxedHelpers),
        Arc::new(DefaultAudio),
        Some(Box::new(LazyControls { window, controls: None })),
        VideoSurface::new(),
    );
    crate::media::set_shell_player(player);
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
