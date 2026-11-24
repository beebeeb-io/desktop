//! The operating system's decoders, for the codecs whose route is [`crate::Route::System`] (decision D-MED-2, the
//! owner's ruling of 2026-11-15: HEVC). On Windows, Media Foundation inside this confined process ([`crate::mf`],
//! decision 2541 option (a)); on macOS, Apple's decoders (the macOS phase).
//!
//! Serves the `System` rows (H.264's fallback, AAC, E-AC-3, HEVC, MPEG-4 Part 2, VC-1 / WMV, WMA: the legal brief's F1
//! of 2026-11-15). Where Windows' decoders do not start in this process (the sandbox's default profile, or any system
//! but Windows), such a stream is `ErrorCode::NoSystemDecoder(codec)`; the app may restart the helper in the
//! system-decoder profile (`beebeeb_media_protocol::profile_for`) and otherwise offers "Open in another app…". A Windows
//! with no media codecs at all (an N or KN edition without the Media Feature Pack) answers
//! `ErrorCode::NoSystemMedia(codec)`, the same honest state with the reason. This crate's own decoder is never used in
//! their place.

use std::sync::Arc;

use beebeeb_media_protocol::{Codec, ErrorCode, FromHelper};

use crate::mf::Unavailable;
use crate::mf::gate::Gate;
use crate::read::{PipeReader, ReadFlags};

/// Whether this process can decode `codec` with the operating system's decoder: `Ok` once Windows' decoders have
/// started here, else why not. Whether the PC has the decoder for this very stream is only known when it opens.
pub fn availability(_codec: Codec) -> Result<(), Unavailable> {
    crate::mf::start()
}

/// Whether this process can decode `codec` with the operating system's decoder.
pub fn available(codec: Codec) -> bool {
    availability(codec).is_ok()
}

/// The protocol's answer for a `System` stream of `codec` that cannot play here.
pub fn refusal(why: Unavailable, codec: Codec) -> ErrorCode {
    match why {
        Unavailable::NotHere => ErrorCode::NoSystemDecoder(codec),
        Unavailable::NoSystemMedia => ErrorCode::NoSystemMedia(codec),
    }
}

/// What the operating system's decoder opens for one file.
pub struct SystemOpen<'a> {
    pub pipe: &'a Arc<PipeReader>,
    pub gate: &'a Arc<Gate>,
    pub flags: &'a Arc<ReadFlags>,
    /// FilmCraft's name for the container (a hint for Windows' reader; the helper has no file name).
    pub container: &'a str,
    /// The video stream's codec, when the file has a video that plays.
    pub video: Option<Codec>,
    /// The audio stream's codec, when the file has audio that plays.
    pub audio: Option<Codec>,
}

/// The streams of one file that the operating system decodes: every track that plays, once the file is on the
/// system's route (one route per file, [`crate::routes::decide`]).
pub struct SystemStreams {
    #[cfg(windows)]
    video: Option<crate::mf::SystemVideo>,
    #[cfg(windows)]
    audio: Option<crate::mf::SystemAudio>,
}

impl SystemStreams {
    /// Opens each stream (the video first). Off Windows nothing opens: `NoSystemDecoder`.
    pub fn open(what: SystemOpen<'_>) -> Result<SystemStreams, ErrorCode> {
        #[cfg(windows)]
        {
            let open_video = |codec| {
                crate::mf::SystemVideo::open(
                    what.pipe.clone(),
                    what.gate.clone(),
                    what.flags.clone(),
                    what.container,
                    codec,
                )
            };
            let open_audio = |codec| {
                crate::mf::SystemAudio::open(
                    what.pipe.clone(),
                    what.gate.clone(),
                    what.flags.clone(),
                    what.container,
                    codec,
                )
            };
            Ok(SystemStreams {
                video: what.video.map(open_video).transpose()?,
                audio: what.audio.map(open_audio).transpose()?,
            })
        }
        #[cfg(not(windows))]
        {
            let codec = what.video.or(what.audio).unwrap_or(Codec::Other);
            Err(ErrorCode::NoSystemDecoder(codec))
        }
    }

    /// Whether the video stream is the system's.
    pub fn has_video(&self) -> bool {
        #[cfg(windows)]
        {
            self.video.is_some()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    /// Whether the audio stream is the system's.
    pub fn has_audio(&self) -> bool {
        #[cfg(windows)]
        {
            self.audio.is_some()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    /// The audio stream's own rate and channels, as the system's decoder gives them.
    pub fn audio_format(&self) -> Option<(u32, u16)> {
        #[cfg(windows)]
        {
            self.audio.as_ref().map(crate::mf::SystemAudio::format)
        }
        #[cfg(not(windows))]
        {
            None
        }
    }

    /// The file's duration as the system's reader knows it (from the audio stream's reader), when it does.
    pub fn duration_ms(&self) -> Option<u64> {
        #[cfg(windows)]
        {
            self.audio.as_ref().and_then(crate::mf::SystemAudio::duration_ms)
        }
        #[cfg(not(windows))]
        {
            None
        }
    }

    /// The next frame at exactly `w × h`.
    pub fn video_frame(&mut self, w: u32, h: u32) -> Result<FromHelper, ErrorCode> {
        #[cfg(windows)]
        if let Some(video) = self.video.as_mut() {
            return video.frame(w, h);
        }
        let _ = (w, h);
        Err(ErrorCode::Internal)
    }

    /// Up to `frames` frames of PCM at `rate` with `channels` channels.
    pub fn audio(&mut self, rate: u32, channels: u16, frames: u32) -> Result<FromHelper, ErrorCode> {
        #[cfg(windows)]
        if let Some(audio) = self.audio.as_mut() {
            return audio.audio(rate, channels, frames);
        }
        let _ = (rate, channels, frames);
        Err(ErrorCode::Internal)
    }

    /// The next answers start at `at_ms`.
    pub fn seek(&mut self, at_ms: u64) {
        #[cfg(windows)]
        {
            if let Some(video) = self.video.as_mut() {
                video.seek(at_ms);
            }
            if let Some(audio) = self.audio.as_mut() {
                audio.seek(at_ms);
            }
        }
        let _ = at_ms;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "Not here" is about this process's sandbox, not about the PC; only a missing Media Feature Pack says the PC
    /// has no media codecs.
    #[test]
    fn a_refusal_names_the_codec_and_the_reason() {
        assert_eq!(
            refusal(Unavailable::NotHere, Codec::Hevc),
            ErrorCode::NoSystemDecoder(Codec::Hevc)
        );
        assert_eq!(
            refusal(Unavailable::NoSystemMedia, Codec::Aac),
            ErrorCode::NoSystemMedia(Codec::Aac)
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn off_windows_the_system_decoder_never_starts() {
        assert_eq!(availability(Codec::Aac), Err(Unavailable::NotHere));
        assert!(!available(Codec::Hevc));
    }
}
