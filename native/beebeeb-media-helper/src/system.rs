//! The operating system's decoders, for the codecs whose route is [`crate::Route::System`] (decision D-MED-2, the
//! owner's ruling of 2026-11-15: HEVC). On Windows, Media Foundation inside this confined process (slice M10 proves
//! it works in the AppContainer, then a slice builds it here); on macOS, Apple's decoders (the macOS phase).
//!
//! Serves the `System` rows (H.264's fallback, AAC, E-AC-3, HEVC, MPEG-4 Part 2, VC-1 / WMV, WMA: the legal brief's F1
//! of 2026-11-15). Until a module here answers, [`available`] is `false` for every codec, and such a stream is
//! `ErrorCode::NoSystemDecoder(codec)`: the app says this PC can't play it and offers "Open in another app…". A
//! Windows with no media codecs at all (an N or KN edition without the Media Feature Pack) answers
//! `ErrorCode::NoSystemMedia(codec)`, the same honest state with the reason. This crate's own decoder is never used in
//! their place.

use beebeeb_media_protocol::Codec;

/// Whether this PC has a decoder of its own for `codec` that runs inside this process. `false` until a system
/// module is built.
pub fn available(_codec: Codec) -> bool {
    false
}
