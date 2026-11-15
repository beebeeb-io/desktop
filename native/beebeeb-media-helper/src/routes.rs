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

use crate::mf::Unavailable;
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
    } else if has(&[
        "mpeg-4 part 2",
        "mpeg-4 visual",
        "mp4v",
        "xvid",
        "divx",
        "msmpeg4",
        "mp43",
    ]) {
        Stream::Known(Codec::Mpeg4Part2)
    } else if has(&["vc-1", "wvc1", "wmv"]) {
        // Not a bare "vc1": H.264's "avc1" holds it.
        Stream::Known(Codec::Vc1)
    } else if has(&["wma", "windows media audio"]) {
        Stream::Known(Codec::Wma)
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

/// Who decodes a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decoder {
    /// FilmCraft, in this process.
    Ours(Codec),
    /// The operating system's decoder, in this process ([`crate::system`]).
    System(Codec),
}

impl Decoder {
    /// The stream's codec.
    pub fn codec(self) -> Codec {
        match self {
            Decoder::Ours(codec) | Decoder::System(codec) => codec,
        }
    }
}

/// Who decodes a stream with this label under `table`, given whether the system's decoders start in this process
/// (`starts`); or the error to send. A `System` row never comes back as [`Decoder::Ours`].
pub fn decoder_with(
    table: &[(Codec, Route)],
    label: &str,
    starts: impl Fn(Codec) -> Result<(), Unavailable>,
) -> Result<Decoder, ErrorCode> {
    match classify(label) {
        Stream::Raw => Ok(Decoder::Ours(Codec::Other)),
        Stream::Unknown => Err(ErrorCode::Unsupported(Codec::Other)),
        Stream::Known(codec) => match route_in(table, codec) {
            Route::Rust => Ok(Decoder::Ours(codec)),
            Route::System => match starts(codec) {
                Ok(()) => Ok(Decoder::System(codec)),
                Err(why) => Err(system::refusal(why, codec)),
            },
            Route::Unavailable => Err(ErrorCode::Unsupported(codec)),
        },
    }
}

/// [`decoder_with`] with this process's own answer ([`system::availability`]).
pub fn decoder(table: &[(Codec, Route)], label: &str) -> Result<Decoder, ErrorCode> {
    decoder_with(table, label, system::availability)
}

/// Both streams of a file, the video first: its refusal is the one the app hears. So a file with HEVC video names
/// HEVC whatever its audio is, and the app picks the sandbox profile with the HEVC capabilities
/// (`beebeeb_media_protocol::profile_for`).
pub fn streams_with(
    table: &[(Codec, Route)],
    video: Option<&str>,
    audio: Option<&str>,
    starts: impl Fn(Codec) -> Result<(), Unavailable> + Copy,
) -> Result<(Option<Decoder>, Option<Decoder>), ErrorCode> {
    let video = video.map(|label| decoder_with(table, label, starts)).transpose()?;
    let audio = audio.map(|label| decoder_with(table, label, starts)).transpose()?;
    Ok((video, audio))
}

/// [`streams_with`] with this process's own answer.
pub fn streams(
    table: &[(Codec, Route)],
    video: Option<&str>,
    audio: Option<&str>,
) -> Result<(Option<Decoder>, Option<Decoder>), ErrorCode> {
    streams_with(table, video, audio, system::availability)
}

/// May a stream with this label open FilmCraft's decoder, under `table`? Its codec for the answer, or the error to
/// send. A stream for the system's decoder is opened by the session (`crate::system::SystemStreams`), never here.
pub fn check(table: &[(Codec, Route)], label: &str) -> Result<Codec, ErrorCode> {
    match decoder(table, label)? {
        Decoder::Ours(codec) => Ok(codec),
        Decoder::System(_) => Err(ErrorCode::Internal),
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
        // Never FilmCraft's decoder. Where the system's decoders do not start (any system but Windows; the sandbox's
        // default profile) the refusal names the codec; on a Windows test run they start, and the session opens them.
        assert_ne!(check(&table, "HEVC"), Ok(Codec::Hevc));
        #[cfg(not(windows))]
        assert_eq!(check(&table, "HEVC"), Err(ErrorCode::NoSystemDecoder(Codec::Hevc)));
        assert_eq!(check(&table, "PCM"), Err(ErrorCode::Unsupported(Codec::Pcm)));
        assert_eq!(check(&table, "FLAC"), Ok(Codec::Flac));
        assert_eq!(check(&table, "Vorbis"), Err(ErrorCode::Unsupported(Codec::Vorbis)));
        assert_eq!(check(&table, "Uncompressed YUV"), Ok(Codec::Other));
    }

    #[test]
    fn the_windows_formats_classify_to_their_codec_and_avc1_stays_h264() {
        for (label, codec) in [
            ("MPEG-4 Part 2 (Advanced Simple)", Codec::Mpeg4Part2),
            ("Xvid", Codec::Mpeg4Part2),
            ("VC-1 (Advanced)", Codec::Vc1),
            ("WMV3", Codec::Vc1),
            ("WMA Pro", Codec::Wma),
            ("H.264 (avc1)", Codec::H264),
        ] {
            assert_eq!(classify(label), Stream::Known(codec), "{label}");
        }
    }

    /// A `System` row goes to the system's decoder when it starts here, and is refused with the reason when it does
    /// not; it never comes back as ours. A `Rust` row never asks the system.
    #[test]
    fn a_system_row_goes_to_the_system_or_is_refused_with_the_reason() {
        let table = [(Codec::Hevc, Route::System), (Codec::Flac, Route::Rust)];
        let here = |_: Codec| -> Result<(), Unavailable> { Ok(()) };
        let not_here = |_: Codec| -> Result<(), Unavailable> { Err(Unavailable::NotHere) };
        let no_media = |_: Codec| -> Result<(), Unavailable> { Err(Unavailable::NoSystemMedia) };
        assert_eq!(decoder_with(&table, "HEVC", here), Ok(Decoder::System(Codec::Hevc)));
        assert_eq!(
            decoder_with(&table, "HEVC", not_here),
            Err(ErrorCode::NoSystemDecoder(Codec::Hevc))
        );
        assert_eq!(
            decoder_with(&table, "HEVC", no_media),
            Err(ErrorCode::NoSystemMedia(Codec::Hevc))
        );
        let asked = std::cell::Cell::new(0);
        let counting = |_: Codec| -> Result<(), Unavailable> {
            asked.set(asked.get() + 1);
            Ok(())
        };
        assert_eq!(decoder_with(&table, "FLAC", counting), Ok(Decoder::Ours(Codec::Flac)));
        assert_eq!(asked.get(), 0, "a Rust row does not ask for the system's decoder");
        assert_eq!(check(&table, "FLAC"), Ok(Codec::Flac));
    }

    /// The video stream is decided first, so its refusal is the one the app hears.
    #[test]
    fn the_video_stream_is_decided_first() {
        let table = [
            (Codec::Hevc, Route::System),
            (Codec::Aac, Route::System),
            (Codec::Opus, Route::Rust),
            (Codec::Ac3, Route::Unavailable),
        ];
        let not_here = |_: Codec| -> Result<(), Unavailable> { Err(Unavailable::NotHere) };
        let here = |_: Codec| -> Result<(), Unavailable> { Ok(()) };
        assert_eq!(
            streams_with(&table, Some("HEVC"), Some("AAC"), not_here),
            Err(ErrorCode::NoSystemDecoder(Codec::Hevc))
        );
        assert_eq!(
            streams_with(&table, Some("HEVC"), Some("AC-3"), here),
            Err(ErrorCode::Unsupported(Codec::Ac3))
        );
        assert_eq!(
            streams_with(&table, Some("HEVC"), Some("Opus"), here),
            Ok((Some(Decoder::System(Codec::Hevc)), Some(Decoder::Ours(Codec::Opus))))
        );
        assert_eq!(
            streams_with(&table, None, Some("AAC"), not_here),
            Err(ErrorCode::NoSystemDecoder(Codec::Aac))
        );
        assert_eq!(streams_with(&table, None, None, here), Ok((None, None)));
    }
}
