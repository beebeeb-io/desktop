//! Windows' own decoders (Media Foundation) inside this confined process: the `System` route (decision D-MED-2;
//! decision 2541, option (a)).
//!
//! A stream whose route is `System` is decoded by an `IMFSourceReader` over an `IMFByteStream` of ours whose reads
//! are the protocol's `ReadAt` asks ([`crate::read::PipeReader`]): never a path, never a temporary file, never the
//! whole file in memory. Out come the same messages as FilmCraft's path: frames scaled and letterboxed to the asked
//! size, PCM at the asked rate and channels, `Seek`, `Ended`.
//!
//! **Where it runs.** Only in the sandbox's system-decoder profile do Windows' decoders start (COM is refused in the
//! default profile: measured, 49 of 49 runs). The app starts the helper in that profile only for a `System` stream,
//! and only once the switch `beebeeb_media_protocol::MEDIA_SYSTEM_DECODER_ON` is on (it is: the owner's ruling of 2026-11-20).
//!
//! **Nothing of Media Foundation is loaded at the helper's start.** Its libraries are absent on Windows N and KN
//! editions without the Media Feature Pack; a load-time import would stop the helper from starting at all there, for
//! FilmCraft's formats too. [`api`] loads them from the system folder the first time a `System` stream needs them,
//! and a missing library is the typed [`Unavailable::NoSystemMedia`].
//!
//! **Reads from Windows' own threads.** Media Foundation reads on its threads, also between the app's messages. A read
//! then must not take the app's next message off the pipe, so every read of the byte stream passes a [`gate::Gate`]
//! that the session opens while it answers a message and closes before it replies.
//!
//! Pure pieces ([`convert`], [`pcm`], [`gate`]) build on every system, so their tests run on Linux; the parts that call
//! Windows are `cfg(windows)`. No logging of content or names, anywhere.

pub mod convert;
pub mod gate;
pub mod pcm;

#[cfg(windows)]
mod api;
#[cfg(windows)]
mod reader;
#[cfg(windows)]
mod stream;

#[cfg(windows)]
pub use reader::{SystemAudio, SystemVideo};

/// Why Windows' decoders cannot run in this process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// This process may not run them: the sandbox's default profile refuses COM (or this is not Windows). The app
    /// may restart the helper in the system-decoder profile; it is not a statement about the PC.
    NotHere,
    /// This Windows has no media codecs at all: an N or KN edition without the Media Feature Pack.
    NoSystemMedia,
}

/// Whether Windows' decoders start in this process: loads and starts Media Foundation once, the first time it is
/// asked, and remembers the answer.
pub fn start() -> Result<(), Unavailable> {
    #[cfg(windows)]
    {
        api::api().map(|_| ())
    }
    #[cfg(not(windows))]
    {
        Err(Unavailable::NotHere)
    }
}
