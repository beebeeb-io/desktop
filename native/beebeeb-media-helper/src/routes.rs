//! The route check before a stream's decoder opens ([`crate::ROUTES`], three kinds: [`crate::Route`]): `Rust` opens
//! FilmCraft's decoder, `System` asks [`crate::system`], `Unavailable` answers `Error(Unsupported(codec))`.
//!
//! One fallback, one way only: a `Rust` stream FilmCraft cannot decode (H.264 interlaced or 10-bit, the kinds its
//! decoder refuses) goes to [`crate::system`] when its codec is in [`crate::SYSTEM_FALLBACK`]; otherwise it is
//! `Error(NoDecoder)`. Never a fallback from `System` to FilmCraft's decoder (HEVC above all), and never to a decoder
//! this build does not contain: a codec whose Cargo feature is off is not in the binary.
//!
//! Slice M3 (`media-helper`) fills it.

use beebeeb_media_protocol::{Codec, ErrorCode};

use crate::{Route, route_in, system};

/// What a FilmCraft codec label says the stream is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    /// A codec with a row in the route table.
    Known(Codec),
    /// Uncompressed or Motion JPEG video: nothing to license, no row; reported to the app as `Codec::Other`.
    Raw,
    /// A label this table does not know.
    Unknown,
}

/// FilmCraft names streams with human labels ("H.264 (High)", "HE-AAC", "PCM float", "Apple ProRes 422", ...).
pub fn classify(label: &str) -> Stream {
    let l = label.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| l.contains(n));
    if has(&["uncompressed", "motion jpeg"]) {
        Stream::Raw
    } else if has(&["e-ac-3", "e-ac3", "eac3"]) {
        Stream::Known(Codec::Eac3)
    } else if has(&["ac-3", "ac3"]) {
        Stream::Known(Codec::Ac3)
    } else if has(&["aac"]) {
        Stream::Known(Codec::Aac)
    } else if has(&["h.264", "h264", "avc"]) {
        Stream::Known(Codec::H264)
    } else if has(&["hevc", "h.265", "h265"]) {
        Stream::Known(Codec::Hevc)
    } else if has(&["vp8"]) {
        Stream::Known(Codec::Vp8)
    } else if has(&["vp9"]) {
        Stream::Known(Codec::Vp9)
    } else if has(&["av1"]) {
        Stream::Known(Codec::Av1)
    } else if has(&["mpeg-2", "mpeg2", "mpeg-1", "mpeg1"]) {
        Stream::Known(Codec::Mpeg2)
    } else if has(&["mp3", "mp2", "mp1", "mpeg audio"]) {
        Stream::Known(Codec::Mp3)
    } else if has(&["prores"]) {
        Stream::Known(Codec::ProRes)
    } else if has(&["dnx"]) {
        Stream::Known(Codec::Dnx)
    } else if has(&["flac"]) {
        Stream::Known(Codec::Flac)
    } else if has(&["alac"]) {
        Stream::Known(Codec::Alac)
    } else if has(&["opus"]) {
        Stream::Known(Codec::Opus)
    } else if has(&["vorbis"]) {
        Stream::Known(Codec::Vorbis)
    } else if has(&["pcm"]) {
        Stream::Known(Codec::Pcm)
    } else {
        Stream::Unknown
    }
}

/// HE-AAC plays at its AAC-LC core here (FilmCraft decodes no SBR): the app says so.
pub fn is_reduced_quality(label: &str) -> bool {
    label.to_ascii_lowercase().contains("he-aac")
}

/// May a stream with this label open its decoder, under `table`? Its codec for the answer, or the error to send.
pub fn check(table: &[(Codec, Route)], label: &str) -> Result<Codec, ErrorCode> {
    match classify(label) {
        Stream::Raw => Ok(Codec::Other),
        Stream::Unknown => Err(ErrorCode::Unsupported(Codec::Other)),
        Stream::Known(codec) => match route_in(table, codec) {
            Route::Rust => Ok(codec),
            Route::System if system::available(codec) => {
                // A system module that says yes has no decoding path here yet (slice M10 builds it).
                Err(ErrorCode::Internal)
            }
            Route::System => Err(ErrorCode::NoSystemDecoder(codec)),
            Route::Unavailable => Err(ErrorCode::Unsupported(codec)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_classify_to_their_codec() {
        for (label, codec) in [
            ("H.264 (High)", Codec::H264),
            ("HEVC", Codec::Hevc),
            ("VP9 (Profile 0)", Codec::Vp9),
            ("AV1 (Main Profile)", Codec::Av1),
            ("Apple ProRes 422 HQ", Codec::ProRes),
            ("Avid DNxHD", Codec::Dnx),
            ("HE-AAC", Codec::Aac),
            ("AAC", Codec::Aac),
            ("MP3", Codec::Mp3),
            ("PCM float", Codec::Pcm),
            ("PCM 16-bit", Codec::Pcm),
            ("E-AC-3", Codec::Eac3),
            ("AC-3", Codec::Ac3),
            ("FLAC", Codec::Flac),
            ("Opus", Codec::Opus),
            ("Vorbis", Codec::Vorbis),
        ] {
            assert_eq!(classify(label), Stream::Known(codec), "{label}");
        }
        assert_eq!(classify("Uncompressed YUV"), Stream::Raw);
        assert_eq!(classify("something else"), Stream::Unknown);
        assert!(is_reduced_quality("HE-AAC") && !is_reduced_quality("AAC"));
    }

    #[test]
    fn a_system_route_is_never_answered_by_our_decoder() {
        let table = [
            (Codec::Hevc, Route::System),
            (Codec::Pcm, Route::Unavailable),
            (Codec::Flac, Route::Rust),
        ];
        assert_eq!(check(&table, "HEVC"), Err(ErrorCode::NoSystemDecoder(Codec::Hevc)));
        assert_eq!(check(&table, "PCM"), Err(ErrorCode::Unsupported(Codec::Pcm)));
        assert_eq!(check(&table, "FLAC"), Ok(Codec::Flac));
        assert_eq!(check(&table, "Vorbis"), Err(ErrorCode::Unsupported(Codec::Vorbis)));
        assert_eq!(check(&table, "Uncompressed YUV"), Ok(Codec::Other));
    }
}
