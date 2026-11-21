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
pub mod image_decode;
pub mod mf;
pub mod read;
pub mod routes;
pub mod session;
pub mod sniff;
pub mod system;
pub mod tags;

use beebeeb_media_protocol::Codec;

/// How the helper decodes a codec (`Rust`, `System`, `Unavailable`; decision D-MED-2). Defined in the protocol crate
/// so the app can name it too: it picks the sandbox profile by it (`beebeeb_media_protocol::profile_for`).
pub use beebeeb_media_protocol::Route;

/// The route each codec ships with: one row per codec (a test holds every codec to exactly one row). A row is a
/// `const`, settled when the helper is built, and follows the codec's Cargo feature: a codec whose feature is off has no
/// decoder in this binary, so its row can never be `Rust` (the legal brief's F1, 2026-11-15).
///
/// - H.264: FilmCraft's decoder (`h264`, shipped: the owner's ruling of 2026-11-15 18:04); the system's decoder takes
///   only the streams FilmCraft cannot decode ([`SYSTEM_FALLBACK`]).
/// - AAC, E-AC-3: compiled out until counsel clears them; the system's decoder.
/// - AC-3, MPEG-1/2 video, ProRes, DNxHD: compiled out until counsel clears them; not played here.
/// - HEVC: the system's decoder only, permanently (D-MED-2); FilmCraft's HEVC is never built into this helper.
/// - MPEG-4 Part 2, VC-1 / WMV, WMA: the system's decoder (FilmCraft has none).
/// - Royalty-free or expired: FilmCraft's decoders (VP8, VP9, AV1, MP3, MP1/MP2, FLAC, ALAC, Opus, Vorbis, PCM).
pub const ROUTES: &[(Codec, Route)] = &[
    (Codec::H264, ours_or(cfg!(feature = "h264"), Route::System)),
    (Codec::Aac, ours_or(cfg!(feature = "aac"), Route::System)),
    (Codec::Eac3, ours_or(cfg!(feature = "eac3"), Route::System)),
    (Codec::Ac3, ours_or(cfg!(feature = "ac3"), Route::Unavailable)),
    (Codec::Mpeg2, ours_or(cfg!(feature = "mpeg2"), Route::Unavailable)),
    (Codec::ProRes, ours_or(cfg!(feature = "prores"), Route::Unavailable)),
    (Codec::Dnx, ours_or(cfg!(feature = "dnx"), Route::Unavailable)),
    (Codec::Hevc, Route::System),
    (Codec::Mpeg4Part2, Route::System),
    (Codec::Vc1, Route::System),
    (Codec::Wma, Route::System),
    (Codec::Vp8, Route::Rust),
    (Codec::Vp9, Route::Rust),
    (Codec::Av1, Route::Rust),
    (Codec::Mp3, Route::Rust),
    (Codec::Mp2, Route::Rust),
    (Codec::Flac, Route::Rust),
    (Codec::Alac, Route::Rust),
    (Codec::Opus, Route::Rust),
    (Codec::Vorbis, Route::Rust),
    (Codec::Pcm, Route::Rust),
    (Codec::Other, Route::Unavailable),
];

/// `Rust` when the codec's decoder is built in, else `otherwise`.
const fn ours_or(built_in: bool, otherwise: Route) -> Route {
    if built_in { Route::Rust } else { otherwise }
}

/// The `Rust` rows whose streams FilmCraft cannot decode (H.264 interlaced or 10-bit, the kinds its decoder refuses) go
/// to the system's decoder instead of failing. Only these, only that way: never from `System` to FilmCraft.
pub const SYSTEM_FALLBACK: &[Codec] = &[Codec::H264];

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
        assert_eq!(route(Codec::Other), Route::Unavailable);
        assert_eq!(route_in(&[], Codec::Pcm), Route::Unavailable);
        // HEVC never has FilmCraft's decoder, whatever is built.
        assert_eq!(route(Codec::Hevc), Route::System);
        assert!(!SYSTEM_FALLBACK.contains(&Codec::Hevc));
        // M3: no row says `Rust` for a codec whose feature is off (the feature builds the decoder in; the row follows it).
        assert_eq!(route(Codec::H264) == Route::Rust, cfg!(feature = "h264"));
        assert_eq!(route(Codec::Aac) == Route::Rust, cfg!(feature = "aac"));
        assert_eq!(route(Codec::Ac3) == Route::Rust, cfg!(feature = "ac3"));
        assert_eq!(route(Codec::Eac3) == Route::Rust, cfg!(feature = "eac3"));
        assert_eq!(route(Codec::Mpeg2) == Route::Rust, cfg!(feature = "mpeg2"));
        assert_eq!(route(Codec::ProRes) == Route::Rust, cfg!(feature = "prores"));
        assert_eq!(route(Codec::Dnx) == Route::Rust, cfg!(feature = "dnx"));
    }

    /// The rulings as they ship (the default features): H.264 ours, the other pool codecs compiled out (AAC and E-AC-3
    /// on the system's decoder, the rest not played), HEVC and the Windows formats on the system's decoder.
    #[cfg(all(
        feature = "h264",
        not(any(
            feature = "aac",
            feature = "ac3",
            feature = "eac3",
            feature = "mpeg2",
            feature = "prores",
            feature = "dnx"
        ))
    ))]
    #[test]
    fn the_shipped_routes_are_the_rulings() {
        assert_eq!(route(Codec::H264), Route::Rust);
        for codec in [
            Codec::Aac,
            Codec::Eac3,
            Codec::Hevc,
            Codec::Mpeg4Part2,
            Codec::Vc1,
            Codec::Wma,
        ] {
            assert_eq!(route(codec), Route::System, "{codec:?}");
        }
        for codec in [Codec::Ac3, Codec::Mpeg2, Codec::ProRes, Codec::Dnx] {
            assert_eq!(route(codec), Route::Unavailable, "{codec:?}");
        }
        for codec in [
            Codec::Vp8,
            Codec::Vp9,
            Codec::Av1,
            Codec::Mp3,
            Codec::Mp2,
            Codec::Flac,
            Codec::Alac,
            Codec::Opus,
            Codec::Vorbis,
            Codec::Pcm,
        ] {
            assert_eq!(route(codec), Route::Rust, "{codec:?}");
        }
    }
}
