//! Slice M7 (`media-bind-viewer`): the player's bytes from the engine. [`EngineMediaOpener`] opens a file with the
//! engine's range reader (`Engine::open_media_for`) for the session stamped at the press, and an answer that comes
//! under another session is dropped (the binding shape of `files_trash.rs` and `stamp.rs`). M7 also creates the
//! shell's one `PlayerHandle` with the real helper, sound and media-control factories and hands it to
//! `media::set_shell_player`.
//!
//! The scaffold opens nothing: [`Refusal::CouldNotPlay`].

use std::sync::atomic::AtomicBool;

use beebeeb_engine::app::bindings::media_stream::MediaReader;
use zeroize::Zeroizing;

use crate::media::player::{MediaBytes, MediaOpener, Refusal};

/// The engine's range reader as the player's [`MediaOpener`].
#[allow(dead_code)] // M7 creates it with the shell's player.
pub struct EngineMediaOpener;

impl MediaOpener for EngineMediaOpener {
    fn open(&self, file_id: &str) -> Result<Box<dyn MediaBytes>, Refusal> {
        let _ = file_id;
        Err(Refusal::CouldNotPlay)
    }
}

/// One opened file's bytes, read by the engine for the session it was opened for.
#[allow(dead_code)] // M7 makes one in `EngineMediaOpener::open`.
struct EngineBytes(MediaReader);

impl MediaBytes for EngineBytes {
    fn len(&self) -> u64 {
        self.0.len()
    }

    fn read_at(&mut self, offset: u64, len: u32, cancel: &AtomicBool) -> Result<Zeroizing<Vec<u8>>, ()> {
        self.0.read_at(offset, len, cancel).map_err(|_| ())
    }
}
