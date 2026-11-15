//! The media helper: decodes ONE audio or video file inside the sandbox, and nothing else.
//!
//! The app starts it confined (`crates/beebeeb-office-sandbox`: an AppContainer with no capabilities, in a job object,
//! gone with the app; decision D-MED-3). It holds no key, no name, no path and has no network. The file's decrypted
//! bytes come from the app range by range over the protocol (`beebeeb-media-protocol`), only the ranges asked for; out
//! go frames of the asked size, PCM, short tags and codes. Every parser and decoder of the file runs here and only
//! here (FilmCraft's demuxers and decoders, and the symphonia it brings for audio files): a decoder bug ends this
//! process, it does not reach the memory of the app that holds the keys.
//!
//! Slice M3 (`media-helper`) fills [`read`], [`decode`], [`routes`] and [`session`]. [`ROUTES`] is the table that
//! ships; a row flips by its own pull request.

#![deny(unsafe_code)]

pub mod decode;
pub mod read;
pub mod routes;
pub mod session;
pub mod system;

use beebeeb_media_protocol::Codec;

/// How the helper decodes a codec: three kinds (decision D-MED-2, the owner's ruling of 2026-11-15 17:30).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// FilmCraft's Rust decoder, in this process.
    Rust,
    /// The operating system's decoder, inside this (confined) process: Media Foundation on Windows, Apple's on
    /// macOS ([`system`]). A PC without one answers `ErrorCode::NoSystemDecoder(codec)` ("This PC can't play HEVC
    /// video without Microsoft's HEVC extension", with "Open in another app…"). NEVER this crate's own decoder in
    /// its place: not as a fallback, not silently.
    System,
    /// Not played here: `ErrorCode::Unsupported(codec)`, and the app offers "Open in another app…".
    Unavailable,
}

/// The route each codec SHIPS with: one row per codec (a test holds every codec to exactly one row).
///
/// - Patent-pool codecs (D-MED-2 (c)): FilmCraft's decoders, ON until counsel's answer flips a row: H.264, AAC,
///   AC-3, E-AC-3, MPEG-2, ProRes, DNxHD.
/// - HEVC: FilmCraft's decoder is built, and its route ships OFF: the shipped route is the system's decoder.
/// - Royalty-free (or expired) codecs: FilmCraft's decoders.
pub const ROUTES: &[(Codec, Route)] = &[
    (Codec::H264, Route::Rust),
    (Codec::Aac, Route::Rust),
    (Codec::Ac3, Route::Rust),
    (Codec::Eac3, Route::Rust),
    (Codec::Mpeg2, Route::Rust),
    (Codec::ProRes, Route::Rust),
    (Codec::Dnx, Route::Rust),
    (Codec::Hevc, Route::System),
    (Codec::Vp8, Route::Rust),
    (Codec::Vp9, Route::Rust),
    (Codec::Av1, Route::Rust),
    (Codec::Mp3, Route::Rust),
    (Codec::Flac, Route::Rust),
    (Codec::Alac, Route::Rust),
    (Codec::Opus, Route::Rust),
    (Codec::Vorbis, Route::Rust),
    (Codec::Pcm, Route::Rust),
    (Codec::Other, Route::Unavailable),
];

/// The route of `codec` in `table` ([`ROUTES`] when shipping; a test hands its own). A codec without a row is
/// `Unavailable`.
pub fn route_in(table: &[(Codec, Route)], codec: Codec) -> Route {
    table
        .iter()
        .find(|(row, _)| *row == codec)
        .map_or(Route::Unavailable, |(_, route)| *route)
}

/// The route `codec` ships with.
pub fn route(codec: Codec) -> Route {
    route_in(ROUTES, codec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_codec_has_exactly_one_route() {
        for codec in Codec::ALL {
            let rows = ROUTES.iter().filter(|(row, _)| *row == codec).count();
            assert_eq!(rows, 1, "{codec:?}");
        }
        assert_eq!(ROUTES.len(), Codec::ALL.len());
        // The owner's ruling, as it ships: HEVC on the system's decoder, the other pool codecs on FilmCraft's.
        assert_eq!(route(Codec::Hevc), Route::System);
        for codec in [
            Codec::H264,
            Codec::Aac,
            Codec::Ac3,
            Codec::Eac3,
            Codec::Mpeg2,
            Codec::ProRes,
            Codec::Dnx,
        ] {
            assert_eq!(route(codec), Route::Rust, "{codec:?}");
        }
        assert_eq!(route(Codec::Other), Route::Unavailable);
        assert_eq!(route_in(&[], Codec::Pcm), Route::Unavailable);
    }
}
