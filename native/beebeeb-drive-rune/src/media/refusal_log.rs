//! What the log says when nothing plays (ux-02): every refusal, every sandbox failure and every rule the helper
//! broke, in the REAL build too. A line carries fixed words of ours, the format's name, the helper's error code, the
//! sandbox's step and the system's error code. It never carries a file's name, id or path: none of these calls is
//! given one.
//!
//! A sandbox failure is written once per (step, code) per app start: thumbnails start the same helper, and a folder
//! of clips must not fill the log. A refusal is written once per item a person opens.

use std::sync::Mutex;

use beebeeb_media_protocol::{Codec, ErrorCode};
use beebeeb_office_sandbox::SandboxError;

use super::player::Refusal;

/// The format's word in the log: one word, lower case (the list's words: `log_file/vocabulary.txt`).
pub(crate) fn codec_word(codec: Codec) -> &'static str {
    match codec {
        Codec::H264 => "h264",
        Codec::Hevc => "hevc",
        Codec::Vp8 => "vp8",
        Codec::Vp9 => "vp9",
        Codec::Av1 => "av1",
        Codec::Mpeg2 => "mpeg2",
        Codec::ProRes => "prores",
        Codec::Dnx => "dnxhd",
        Codec::Mp3 => "mp3",
        Codec::Aac => "aac",
        Codec::Flac => "flac",
        Codec::Alac => "alac",
        Codec::Opus => "opus",
        Codec::Vorbis => "vorbis",
        Codec::Pcm => "pcm",
        Codec::Ac3 => "ac3",
        Codec::Eac3 => "eac3",
        Codec::Mp2 => "mp2",
        Codec::Mpeg4Part2 => "mpeg4",
        Codec::Vc1 => "vc1",
        Codec::Wma => "wma",
        Codec::Other => "other",
    }
}

/// The refusal's words in the log, and the format where the refusal knows it.
pub(crate) fn reason_of(why: &Refusal) -> (&'static str, Option<Codec>) {
    match why {
        Refusal::NoDecoder(_) => ("no decoder", None),
        Refusal::NoSystemDecoder(codec) => ("system decoder off", Some(*codec)),
        Refusal::NoSystemMedia(codec) => ("no system media", Some(*codec)),
        Refusal::CouldNotPlay => ("failed", None),
        Refusal::NotSignedIn => ("not signed in", None),
        Refusal::NeedsWindowsUpdate => ("needs windows update", None),
        Refusal::NotInstalled => ("no helper", None),
        Refusal::NoSandbox => ("no sandbox", None),
    }
}

/// The item a person opened does not play: the reason the window shows, in the log's words.
pub(crate) fn refused(why: &Refusal) {
    let (reason, codec) = reason_of(why);
    let codec = codec.map_or("none", codec_word);
    tracing::warn!(reason, codec, "media refused");
}

/// The helper's own answer, before it becomes a [`Refusal`] (which folds five codes into "could not play").
pub(crate) fn helper_error(code: ErrorCode) {
    let (code, codec) = match code {
        ErrorCode::Unsupported(codec) => ("unsupported", Some(codec)),
        ErrorCode::NoSystemDecoder(codec) => ("no system decoder", Some(codec)),
        ErrorCode::NoSystemMedia(codec) => ("no system media", Some(codec)),
        ErrorCode::NoDecoder => ("no decoder", None),
        ErrorCode::Corrupt => ("corrupt", None),
        ErrorCode::TooLarge => ("too large", None),
        ErrorCode::ReadFailed => ("read failed", None),
        ErrorCode::Timeout => ("timeout", None),
        ErrorCode::Internal => ("internal", None),
    };
    let codec = codec.map_or("none", codec_word);
    tracing::warn!(code, codec, "media helper answered an error");
}

/// The helper was killed for breaking a rule of the player's: `what` is the kind of rule, `line` where the worker
/// checked it (a number of this build's source, nothing of the file).
pub(crate) fn broke(what: &'static str, line: u32) {
    tracing::warn!(what, at = line, "media helper stopped by the player");
}

/// The (step, code) pairs already written this app start. At most [`SEEN_MAX`]: past it nothing more is written.
static SEEN: Mutex<Vec<(&'static str, u32)>> = Mutex::new(Vec::new());
const SEEN_MAX: usize = 32;

/// Whether (step, code) is new to `seen` (and now in it).
fn first_time(seen: &Mutex<Vec<(&'static str, u32)>>, step: &'static str, code: u32) -> bool {
    let mut seen = seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if seen.len() >= SEEN_MAX || seen.contains(&(step, code)) {
        return false;
    }
    seen.push((step, code));
    true
}

/// The helper was not started: a step of ours with no system code (`exclusion`, `pipes`, `threads`).
pub(crate) fn not_started(step: &'static str) {
    start_failed(&SEEN, step, 0);
}

/// The sandbox refused or failed: its step and the system's error code. `NeedsNewerWindows` has its own line
/// (`windows_floor`).
pub(crate) fn sandbox_failed(error: &SandboxError) {
    let (step, code) = match error {
        SandboxError::Os { step, code } => (*step, *code),
        SandboxError::Unsupported => ("unsupported", 0),
        SandboxError::HelperPath(_) => ("helper path", 0),
        SandboxError::Limits(_) => ("limits", 0),
        SandboxError::NotConfined(_) => ("not confined", 0),
        SandboxError::NeedsNewerWindows => return,
    };
    start_failed(&SEEN, step, code);
}

fn start_failed(seen: &Mutex<Vec<(&'static str, u32)>>, step: &'static str, code: u32) {
    if first_time(seen, step, code) {
        tracing::warn!(step, code, "media helper not started");
    }
}

#[cfg(test)]
#[path = "refusal_log_tests.rs"]
pub(crate) mod tests;
