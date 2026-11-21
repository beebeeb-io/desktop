//! The route check before a file's decoders open ([`crate::ROUTES`], three kinds: [`crate::Route`]): `Rust` is
//! FilmCraft's decoder, `System` the operating system's ([`crate::system`]), `Unavailable` is not played here.
//!
//! **One route per file** (the owner's ruling of 2026-11-15 19:50; [`decide`]). If a track needs the system's decoder
//! and it is available in this process, the WHOLE file goes there and Media Foundation decodes every track that
//! plays. If it is not available, the tracks FilmCraft can play play, and the audio track that cannot is named
//! (`Opened::no_sound`), so the app shows one line for it; a video nothing here can show is refused. A track whose
//! route is `Unavailable` (compiled out, e.g. AC-3) never needs the system's decoder: it never moves a file there.
//!
//! One fallback, one way only: an H.264 stream FilmCraft's decoder refuses (interlaced or 10-bit; it checks only
//! when it decodes) needs the system's decoder, because H.264 is in [`crate::SYSTEM_FALLBACK`]; a refused stream of
//! any other codec is `Error(NoDecoder)`. Never a fallback from `System` to FilmCraft's decoder (HEVC above all), and
//! never to a decoder this build does not contain: a codec whose Cargo feature is off is not in the binary.

use beebeeb_media_protocol::{Codec, ErrorCode};

use crate::mf::Unavailable;
use crate::{Route, SYSTEM_FALLBACK, route_in};

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
    } else if has(&["mp3"]) {
        // Layer III: "MP3" (Matroska, AVI, a bare stream), "mp3" (MP4).
        Stream::Known(Codec::Mp3)
    } else if has(&["mp1", "mp2", "mpeg audio"]) {
        // Layers I and II: "MP1" / "MP2" (a bare stream), "MPEG Audio" (Matroska, MP4, AVI). MPEG-TS/PS name every
        // layer "MPEG Audio", so an MP3 track in a transport stream reads as MP2 here (both are FilmCraft's).
        Stream::Known(Codec::Mp2)
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

/// One track of a file, as the decision sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Track {
    /// Its codec (`Codec::Other` for raw video and for a label nothing here knows).
    pub codec: Codec,
    /// Its codec's row in the route table.
    pub route: Route,
    /// FilmCraft's decoder refused the stream when the file opened (H.264 interlaced or 10-bit): it needs the
    /// system's decoder when its codec is in [`SYSTEM_FALLBACK`], and nothing here decodes it otherwise.
    pub refused: bool,
    /// With `refused`: the refusal was the stream's 10-bit depth (FilmCraft's "bit depth" words), not interlacing.
    pub ten_bit: bool,
}

/// What a track takes, given its route and whether FilmCraft refused it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Need {
    /// FilmCraft's decoder plays it.
    Ours,
    /// It needs the operating system's decoder.
    System,
    /// Not played here (its route is `Unavailable`): never a reason to go to the system's decoder.
    Unavailable,
    /// FilmCraft refused it and no fallback takes it.
    NoDecoder,
}

impl Track {
    /// A track of `codec` under `table`.
    pub fn of(table: &[(Codec, Route)], codec: Codec) -> Track {
        Track {
            codec,
            route: route_in(table, codec),
            refused: false,
            ten_bit: false,
        }
    }

    /// The track FilmCraft's label names, under `table`. Raw video plays (nothing to license, no row); a label nothing
    /// here knows is not played.
    pub fn of_label(table: &[(Codec, Route)], label: &str) -> Track {
        let (codec, route) = match classify(label) {
            Stream::Known(codec) => (codec, route_in(table, codec)),
            Stream::Raw => (Codec::Other, Route::Rust),
            Stream::Unknown => (Codec::Other, Route::Unavailable),
        };
        Track {
            codec,
            route,
            refused: false,
            ten_bit: false,
        }
    }

    fn need(self) -> Need {
        match self.route {
            Route::Rust if !self.refused => Need::Ours,
            Route::Rust if SYSTEM_FALLBACK.contains(&self.codec) => Need::System,
            Route::Rust => Need::NoDecoder,
            Route::System => Need::System,
            Route::Unavailable => Need::Unavailable,
        }
    }

    /// Whether this track needs the operating system's decoder: its route is `System`, or FilmCraft refused it and its
    /// codec falls back there.
    pub fn needs_system(self) -> bool {
        self.need() == Need::System
    }
}

/// The operating system's decoders, as one file's decision sees them (contract C6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemState {
    /// The switch (`beebeeb_media_protocol::MEDIA_SYSTEM_DECODER_ON`, on as shipped) is held off: never Media
    /// Foundation.
    Off,
    /// The switch is on, but Media Foundation does not start in this process (the sandbox's default profile, or not
    /// Windows): the app may restart the helper once in the system-decoder profile.
    NotHere,
    /// The switch is on, and this Windows has no media codecs at all (an N or KN edition).
    NoSystemMedia,
    /// Media Foundation runs in this process.
    Available,
}

/// The system state from the switch and this process's answer ([`crate::system::availability`]). The switch wins:
/// off is `Off`, whatever Media Foundation says.
pub fn system_state(switch_on: bool, available: Result<(), Unavailable>) -> SystemState {
    if !switch_on {
        return SystemState::Off;
    }
    match available {
        Ok(()) => SystemState::Available,
        Err(Unavailable::NotHere) => SystemState::NotHere,
        Err(Unavailable::NoSystemMedia) => SystemState::NoSystemMedia,
    }
}

/// The system state for a file with these tracks. `ask` (this process's Media Foundation) is asked only when the
/// switch is on and a track needs the system's decoder, so a file FilmCraft plays alone never loads it. With no such
/// track the state does not change [`decide`]'s answer: `Off`, and nothing is asked.
pub fn state_for(
    video: Option<Track>,
    audio: Option<Track>,
    switch_on: bool,
    ask: impl FnOnce() -> Result<(), Unavailable>,
) -> SystemState {
    if !(video.is_some_and(Track::needs_system) || audio.is_some_and(Track::needs_system)) {
        return SystemState::Off;
    }
    let available = if switch_on { ask() } else { Err(Unavailable::NotHere) };
    system_state(switch_on, available)
}

/// The tracks that play, by codec, and the audio track that does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub video: Option<Codec>,
    pub audio: Option<Codec>,
    /// The file's audio track that does not play here (`Opened::no_sound`): only beside a video that plays, and only
    /// when `audio` is `None`.
    pub no_sound: Option<Codec>,
}

/// How one file plays: one route for the whole file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// FilmCraft decodes the plan's tracks.
    Ours(Plan),
    /// The operating system's decoder decodes the plan's tracks: the whole file, both tracks, even one FilmCraft
    /// could play.
    System(Plan),
    /// Nothing plays: the code the app hears.
    Error(ErrorCode),
}

/// The refusal for a track that needs the system's decoder where it is not available.
fn refusal(system: SystemState, codec: Codec) -> ErrorCode {
    match system {
        SystemState::NoSystemMedia => ErrorCode::NoSystemMedia(codec),
        SystemState::Off | SystemState::NotHere | SystemState::Available => ErrorCode::NoSystemDecoder(codec),
    }
}

/// One route for the file with these tracks (PLAN.md, "The row-34 decision"; the owner's ruling of 19:50).
///
/// The video decides first: a video nothing here can show is the answer whatever the audio, and the codec it names
/// is the one the app picks the sandbox profile by. Then:
/// - a track that needs the system's decoder, where it is `Available`: the WHOLE file on the system's decoder;
/// - `NotHere`: that track's `NoSystemDecoder` (the host restarts the helper once in the system profile);
/// - `Off` / `NoSystemMedia`: a video plays on FilmCraft with `no_sound` naming the audio track; audio alone is
///   refused;
/// - an `Unavailable` (or refused) audio track beside a video that plays: the video plays, `no_sound` names it, in
///   every state (never a restart, never the system's decoder for it).
pub fn decide(video: Option<Track>, audio: Option<Track>, system: SystemState) -> Decision {
    let available = system == SystemState::Available;
    let Some(video) = video else {
        return match audio {
            None => Decision::Error(ErrorCode::NoDecoder),
            Some(audio) => decide_audio_only(audio, system),
        };
    };
    match video.need() {
        Need::Unavailable if video.codec == Codec::Other => return Decision::Error(ErrorCode::UnknownCodec),
        Need::Unavailable => return Decision::Error(ErrorCode::Unsupported(video.codec)),
        Need::NoDecoder => return Decision::Error(ErrorCode::NoDecoder),
        Need::System if !available && video.refused && video.ten_bit && system != SystemState::NoSystemMedia => {
            return Decision::Error(ErrorCode::TenBit);
        }
        Need::System if !available => return Decision::Error(refusal(system, video.codec)),
        Need::System | Need::Ours => {}
    }
    // From here the video plays; when it needs the system's decoder, that decoder is available.
    let video_on_system = video.needs_system();
    let plan = |audio: Option<Codec>, no_sound: Option<Codec>| Plan {
        video: Some(video.codec),
        audio,
        no_sound,
    };
    let on = |plan: Plan| {
        if video_on_system {
            Decision::System(plan)
        } else {
            Decision::Ours(plan)
        }
    };
    let Some(audio) = audio else {
        return on(plan(None, None));
    };
    match audio.need() {
        Need::Ours => on(plan(Some(audio.codec), None)),
        // One route per file: the system's decoder takes the whole file, the video included.
        Need::System if available => Decision::System(plan(Some(audio.codec), None)),
        // Not in this process: the host restarts the helper once in the system profile.
        Need::System if system == SystemState::NotHere => Decision::Error(ErrorCode::NoSystemDecoder(audio.codec)),
        // Off, or no media codecs on this Windows: the video plays and the app says the sound does not.
        Need::System => Decision::Ours(plan(None, Some(audio.codec))),
        // A track nothing here decodes never moves the file: the video plays where it would, without that sound.
        Need::Unavailable | Need::NoDecoder => on(plan(None, Some(audio.codec))),
    }
}

/// An audio-only file: the track plays, or its refusal is the answer.
fn decide_audio_only(audio: Track, system: SystemState) -> Decision {
    let plan = Plan {
        video: None,
        audio: Some(audio.codec),
        no_sound: None,
    };
    match audio.need() {
        Need::Ours => Decision::Ours(plan),
        Need::System if system == SystemState::Available => Decision::System(plan),
        Need::System => Decision::Error(refusal(system, audio.codec)),
        Need::Unavailable => Decision::Error(ErrorCode::Unsupported(audio.codec)),
        Need::NoDecoder => Decision::Error(ErrorCode::NoDecoder),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_STATES: [SystemState; 4] = [
        SystemState::Off,
        SystemState::NotHere,
        SystemState::NoSystemMedia,
        SystemState::Available,
    ];

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
            ("AAC (ADTS)", Codec::Aac),
            ("MP3", Codec::Mp3),
            ("mp3", Codec::Mp3),
            // MPEG audio Layers I and II: FilmCraft's whole-file opener says "MP1" / "MP2", the containers
            // "MPEG Audio" (Matroska `A_MPEG/L1`, `/L2`; MP4 with a Layer I/II first frame; AVI tag 0x50).
            ("MP1", Codec::Mp2),
            ("MP2", Codec::Mp2),
            ("MPEG Audio", Codec::Mp2),
            ("PCM float", Codec::Pcm),
            ("PCM 16-bit", Codec::Pcm),
            ("E-AC-3", Codec::Eac3),
            ("AC-3", Codec::Ac3),
            ("FLAC", Codec::Flac),
            ("flac", Codec::Flac),
            ("Opus", Codec::Opus),
            ("Vorbis", Codec::Vorbis),
            ("MPEG-2 Video", Codec::Mpeg2),
            ("MPEG-1 Video", Codec::Mpeg2),
        ] {
            assert_eq!(classify(label), Stream::Known(codec), "{label}");
        }
        assert_eq!(classify("Uncompressed YUV"), Stream::Raw);
        assert_eq!(classify("something else"), Stream::Unknown);
        assert!(is_reduced_quality("HE-AAC") && !is_reduced_quality("AAC"));
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

    /// The shipped table's rows for the codecs the table test names (the routes at the shipped features).
    const TABLE: &[(Codec, Route)] = &[
        (Codec::H264, Route::Rust),
        (Codec::Hevc, Route::System),
        (Codec::Vp9, Route::Rust),
        (Codec::Mpeg2, Route::Unavailable),
        (Codec::Aac, Route::System),
        (Codec::Eac3, Route::System),
        (Codec::Ac3, Route::Unavailable),
        (Codec::Mp3, Route::Rust),
        (Codec::Mp2, Route::Rust),
        (Codec::Opus, Route::Rust),
    ];

    fn t(codec: Codec) -> Option<Track> {
        Some(Track::of(TABLE, codec))
    }

    /// H.264 that FilmCraft's decoder refused when the file opened (interlaced, 10-bit).
    fn h264_fallback() -> Option<Track> {
        Some(Track {
            refused: true,
            ..Track::of(TABLE, Codec::H264)
        })
    }

    fn ours(video: Option<Codec>, audio: Option<Codec>, no_sound: Option<Codec>) -> Decision {
        Decision::Ours(Plan { video, audio, no_sound })
    }

    fn system(video: Option<Codec>, audio: Option<Codec>, no_sound: Option<Codec>) -> Decision {
        Decision::System(Plan { video, audio, no_sound })
    }

    /// PLAN.md's table, row by row: (video track, audio track, system state) → the route and the line the app shows.
    #[test]
    fn a_file_takes_one_route() {
        use Codec::*;
        use SystemState::*;
        let rows: [(Option<Track>, Option<Track>, SystemState, Decision); 16] = [
            // A System audio track beside FilmCraft's video: the video plays with the line, a restart, or Media
            // Foundation takes the whole file.
            (t(H264), t(Aac), Off, ours(Some(H264), None, Some(Aac))),
            (t(H264), t(Aac), NoSystemMedia, ours(Some(H264), None, Some(Aac))),
            (
                t(H264),
                t(Aac),
                NotHere,
                Decision::Error(ErrorCode::NoSystemDecoder(Aac)),
            ),
            (t(H264), t(Aac), Available, system(Some(H264), Some(Aac), None)),
            // A System video: nothing to show without it.
            (t(Hevc), t(Aac), Off, Decision::Error(ErrorCode::NoSystemDecoder(Hevc))),
            (t(Hevc), t(Aac), Available, system(Some(Hevc), Some(Aac), None)),
            // An Unavailable audio track: the video plays without it in every state; never a restart, never Media
            // Foundation for it.
            (t(H264), t(Ac3), Off, ours(Some(H264), None, Some(Ac3))),
            (t(H264), t(Ac3), NotHere, ours(Some(H264), None, Some(Ac3))),
            (t(H264), t(Ac3), Available, ours(Some(H264), None, Some(Ac3))),
            // Every track FilmCraft's.
            (t(H264), t(Mp3), Off, ours(Some(H264), Some(Mp3), None)),
            (t(Vp9), t(Opus), Off, ours(Some(Vp9), Some(Opus), None)),
            // An Unavailable video: refused, whatever the audio.
            (t(Mpeg2), t(Mp2), Off, Decision::Error(ErrorCode::Unsupported(Mpeg2))),
            // Audio only, on the system's route: its refusal.
            (None, t(Aac), Off, Decision::Error(ErrorCode::NoSystemDecoder(Aac))),
            (None, t(Eac3), Off, Decision::Error(ErrorCode::NoSystemDecoder(Eac3))),
            // H.264 that FilmCraft refused is a System video (SYSTEM_FALLBACK).
            (
                h264_fallback(),
                t(Aac),
                Off,
                Decision::Error(ErrorCode::NoSystemDecoder(H264)),
            ),
            (
                h264_fallback(),
                None,
                NoSystemMedia,
                Decision::Error(ErrorCode::NoSystemMedia(H264)),
            ),
        ];
        for (video, audio, state, want) in rows {
            assert_eq!(decide(video, audio, state), want, "{video:?} {audio:?} {state:?}");
        }

        // The rest of the table, by its columns.
        // A System video where Media Foundation runs: the whole file, also with no audio or audio FilmCraft plays.
        assert_eq!(decide(t(Hevc), None, Available), system(Some(Hevc), None, None));
        assert_eq!(
            decide(t(Hevc), t(Opus), Available),
            system(Some(Hevc), Some(Opus), None)
        );
        assert_eq!(
            decide(h264_fallback(), t(Mp3), Available),
            system(Some(H264), Some(Mp3), None)
        );
        // ... and an Unavailable audio track then plays on neither: the video on Media Foundation, without it.
        assert_eq!(decide(t(Hevc), t(Ac3), Available), system(Some(Hevc), None, Some(Ac3)));
        // A System video elsewhere: the refusal names the video (the host picks the profile by it).
        assert_eq!(
            decide(t(Hevc), t(Aac), NotHere),
            Decision::Error(ErrorCode::NoSystemDecoder(Hevc))
        );
        assert_eq!(
            decide(t(Hevc), t(Aac), NoSystemMedia),
            Decision::Error(ErrorCode::NoSystemMedia(Hevc))
        );
        // Audio only: FilmCraft's plays everywhere; a System one plays where Media Foundation runs and says why not
        // elsewhere; an Unavailable one is Unsupported everywhere.
        for state in ALL_STATES {
            assert_eq!(decide(None, t(Mp3), state), ours(None, Some(Mp3), None), "{state:?}");
            assert_eq!(
                decide(None, t(Ac3), state),
                Decision::Error(ErrorCode::Unsupported(Ac3)),
                "{state:?}"
            );
            assert_eq!(
                decide(t(Mpeg2), t(Mp3), state),
                Decision::Error(ErrorCode::Unsupported(Mpeg2)),
                "{state:?}"
            );
            assert_eq!(
                decide(None, None, state),
                Decision::Error(ErrorCode::NoDecoder),
                "{state:?}"
            );
        }
        assert_eq!(decide(None, t(Aac), Available), system(None, Some(Aac), None));
        assert_eq!(
            decide(None, t(Aac), NotHere),
            Decision::Error(ErrorCode::NoSystemDecoder(Aac))
        );
        assert_eq!(
            decide(None, t(Aac), NoSystemMedia),
            Decision::Error(ErrorCode::NoSystemMedia(Aac))
        );
        // A refused stream of a codec with no fallback: nothing here decodes it.
        let vp9_refused = Some(Track {
            refused: true,
            ..Track::of(TABLE, Vp9)
        });
        assert_eq!(
            decide(vp9_refused, t(Opus), Available),
            Decision::Error(ErrorCode::NoDecoder)
        );

        // Every `no_sound` the decision gives sits beside a video that plays and without audio (the wire refuses any
        // other shape).
        for state in ALL_STATES {
            for video in [None, t(H264), t(Hevc), t(Vp9), t(Mpeg2), h264_fallback()] {
                for audio in [None, t(Aac), t(Eac3), t(Ac3), t(Mp3), t(Opus)] {
                    if let Decision::Ours(plan) | Decision::System(plan) = decide(video, audio, state)
                        && plan.no_sound.is_some()
                    {
                        assert!(
                            plan.video.is_some() && plan.audio.is_none(),
                            "{video:?} {audio:?} {state:?}"
                        );
                    }
                }
            }
        }
    }

    /// C6: the switch wins over whatever Media Foundation says.
    #[test]
    fn the_system_route_is_off_while_the_switch_is_off() {
        assert_eq!(system_state(false, Ok(())), SystemState::Off);
        assert_eq!(system_state(false, Err(Unavailable::NotHere)), SystemState::Off);
        assert_eq!(system_state(false, Err(Unavailable::NoSystemMedia)), SystemState::Off);
        assert_eq!(system_state(true, Ok(())), SystemState::Available);
        assert_eq!(system_state(true, Err(Unavailable::NotHere)), SystemState::NotHere);
        assert_eq!(
            system_state(true, Err(Unavailable::NoSystemMedia)),
            SystemState::NoSystemMedia
        );
        // So with the switch off an H.264 + AAC video plays without its sound, wherever the helper runs.
        let state = state_for(t(Codec::H264), t(Codec::Aac), false, || Ok(()));
        assert_eq!(state, SystemState::Off);
        assert_eq!(
            decide(t(Codec::H264), t(Codec::Aac), state),
            ours(Some(Codec::H264), None, Some(Codec::Aac))
        );
    }

    /// A `System` row goes to the system's decoder when it starts here and is refused with the reason when it does
    /// not; it never comes back as FilmCraft's. A file FilmCraft plays alone never asks for the system's decoder, and
    /// neither does any file while the switch is off.
    #[test]
    fn a_system_row_goes_to_the_system_or_is_refused_with_the_reason() {
        let hevc = t(Codec::Hevc);
        let state = |answer: Result<(), Unavailable>| state_for(hevc, None, true, || answer);
        assert_eq!(decide(hevc, None, state(Ok(()))), system(Some(Codec::Hevc), None, None));
        assert_eq!(
            decide(hevc, None, state(Err(Unavailable::NotHere))),
            Decision::Error(ErrorCode::NoSystemDecoder(Codec::Hevc))
        );
        assert_eq!(
            decide(hevc, None, state(Err(Unavailable::NoSystemMedia))),
            Decision::Error(ErrorCode::NoSystemMedia(Codec::Hevc))
        );

        let asked = std::cell::Cell::new(0);
        let counting = || -> Result<(), Unavailable> {
            asked.set(asked.get() + 1);
            Ok(())
        };
        assert_eq!(
            state_for(t(Codec::H264), t(Codec::Mp3), true, counting),
            SystemState::Off
        );
        assert_eq!(asked.get(), 0, "a Rust row does not ask for the system's decoder");
        assert_eq!(state_for(hevc, t(Codec::Aac), false, counting), SystemState::Off);
        assert_eq!(asked.get(), 0, "nothing asks while the switch is off");
        assert_eq!(
            state_for(t(Codec::H264), t(Codec::Ac3), true, counting),
            SystemState::Off
        );
        assert_eq!(asked.get(), 0, "an Unavailable row does not ask either");
        assert_eq!(state_for(h264_fallback(), None, true, counting), SystemState::Available);
        assert_eq!(asked.get(), 1, "a refused H.264 stream asks once");
    }

    /// Never FilmCraft's decoder for a `System` row, in any state; `Unavailable` is Unsupported; a label nothing here
    /// knows is `Unsupported(Other)`; raw video plays on FilmCraft.
    #[test]
    fn a_system_route_is_never_answered_by_our_decoder() {
        let table = [
            (Codec::Hevc, Route::System),
            (Codec::Pcm, Route::Unavailable),
            (Codec::Flac, Route::Rust),
        ];
        for state in ALL_STATES {
            let hevc = Some(Track::of_label(&table, "HEVC"));
            assert!(!matches!(decide(hevc, None, state), Decision::Ours(_)), "{state:?}");
            let pcm = Some(Track::of_label(&table, "PCM"));
            assert_eq!(
                decide(None, pcm, state),
                Decision::Error(ErrorCode::Unsupported(Codec::Pcm))
            );
            let flac = Some(Track::of_label(&table, "FLAC"));
            assert_eq!(decide(None, flac, state), ours(None, Some(Codec::Flac), None));
            let vorbis = Some(Track::of_label(&table, "Vorbis"));
            assert_eq!(
                decide(None, vorbis, state),
                Decision::Error(ErrorCode::Unsupported(Codec::Vorbis))
            );
            let unknown = Some(Track::of_label(&table, "something else"));
            assert_eq!(
                decide(unknown, None, state),
                Decision::Error(ErrorCode::Unsupported(Codec::Other))
            );
            let raw = Some(Track::of_label(&table, "Uncompressed YUV"));
            assert_eq!(decide(raw, None, state), ours(Some(Codec::Other), None, None));
        }
    }

    /// The video decides first: its refusal is the one the app hears, whatever the audio's.
    #[test]
    fn the_video_stream_is_decided_first() {
        use Codec::*;
        assert_eq!(
            decide(t(Hevc), t(Aac), SystemState::NotHere),
            Decision::Error(ErrorCode::NoSystemDecoder(Hevc))
        );
        assert_eq!(
            decide(t(Mpeg2), t(Aac), SystemState::Off),
            Decision::Error(ErrorCode::Unsupported(Mpeg2))
        );
        // The ruling of 19:50 replaced the split streams of batch 1: HEVC + AC-3 where Media Foundation runs was
        // `Unsupported(Ac3)`, HEVC + Opus was HEVC on the system and Opus on FilmCraft. Now one route each.
        assert_eq!(
            decide(t(Hevc), t(Ac3), SystemState::Available),
            system(Some(Hevc), None, Some(Ac3))
        );
        assert_eq!(
            decide(t(Hevc), t(Opus), SystemState::Available),
            system(Some(Hevc), Some(Opus), None)
        );
    }
}
