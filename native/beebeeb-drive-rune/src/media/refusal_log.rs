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

/// The item a person opened does not play: the reason the window shows, in the log's words. A refusal that names
/// a format follows the helper's own line ([`helper_error`]), which has the format.
pub(crate) fn refused(why: &Refusal) {
    tracing::warn!(
        reason = match why {
            Refusal::NoDecoder(_) => "no decoder",
            Refusal::NoSystemDecoder(_) => "no system decoder",
            Refusal::NoSystemMedia(_) => "no system media",
            Refusal::CouldNotPlay => "failed",
            Refusal::NotSignedIn => "not signed in",
            Refusal::NeedsWindowsUpdate => "needs windows update",
            Refusal::NotInstalled => "no helper",
            Refusal::NoSandbox => "no sandbox",
        },
        "media refused"
    );
}

/// The helper's own answer, before it becomes a [`Refusal`] (which folds five codes into one), with the format
/// where the answer names one. Every word is written here, at the call: the log's word list is read from it.
pub(crate) fn helper_error(code: ErrorCode) {
    let codec = match code {
        ErrorCode::Unsupported(codec) | ErrorCode::NoSystemDecoder(codec) | ErrorCode::NoSystemMedia(codec) => {
            Some(codec)
        }
        _ => None,
    };
    tracing::warn!(
        answer = match code {
            ErrorCode::Unsupported(_) => "unsupported",
            ErrorCode::NoSystemDecoder(_) => "no system decoder",
            ErrorCode::NoSystemMedia(_) => "no system media",
            ErrorCode::NoDecoder => "no decoder",
            ErrorCode::Corrupt => "corrupt",
            ErrorCode::TooLarge => "too large",
            ErrorCode::ReadFailed => "read failed",
            ErrorCode::Timeout => "timeout",
            ErrorCode::Internal => "internal",
        },
        codec = match codec {
            None => "none",
            Some(Codec::H264) => "h264",
            Some(Codec::Hevc) => "hevc",
            Some(Codec::Vp8) => "vp8",
            Some(Codec::Vp9) => "vp9",
            Some(Codec::Av1) => "av1",
            Some(Codec::Mpeg2) => "mpeg2",
            Some(Codec::ProRes) => "prores",
            Some(Codec::Dnx) => "dnxhd",
            Some(Codec::Mp3) => "mp3",
            Some(Codec::Aac) => "aac",
            Some(Codec::Flac) => "flac",
            Some(Codec::Alac) => "alac",
            Some(Codec::Opus) => "opus",
            Some(Codec::Vorbis) => "vorbis",
            Some(Codec::Pcm) => "pcm",
            Some(Codec::Ac3) => "ac3",
            Some(Codec::Eac3) => "eac3",
            Some(Codec::Mp2) => "mp2",
            Some(Codec::Mpeg4Part2) => "mpeg4",
            Some(Codec::Vc1) => "vc1",
            Some(Codec::Wma) => "wma",
            Some(Codec::Other) => "other",
        },
        "media helper answered an error"
    );
}

/// The kind of rule the helper broke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Broke {
    /// An answer failed one of the worker's checks.
    Check,
    /// The helper's pipe ended.
    Gone,
    /// An answer was not one of the protocol's.
    Protocol,
    /// No answer in time (`Opened` within 10 s, a frame or sound by its deadline).
    Timeout,
    /// The app could not read the bytes the helper asked for.
    Read,
}

/// The helper was killed for breaking a rule of the player's: the kind of rule, and `line`, where the worker checked
/// it (a number of this build's source, nothing of the file).
pub(crate) fn broke(what: Broke, line: u32) {
    tracing::warn!(
        what = match what {
            Broke::Check => "check",
            Broke::Gone => "gone",
            Broke::Protocol => "protocol",
            Broke::Timeout => "timeout",
            Broke::Read => "read",
        },
        at = line,
        "media helper stopped by the player"
    );
}

/// A start that failed before or beside the sandbox: a step of ours, with no system code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NotStarted {
    /// The crash-report exclusion could not be written.
    Exclusion,
    /// The started helper had no pipes.
    Pipes,
    /// The pipes' threads did not start.
    Threads,
    /// The worker's thread did not start.
    WorkerThread,
    /// No helper beside the app's exe.
    NoHelper,
    /// `SandboxError::Unsupported`, `HelperPath`, `Limits` and `NotConfined`.
    Unsupported,
    HelperPath,
    Limits,
    NotConfined,
}

/// The (step, code) pairs already written this app start. At most [`SEEN_MAX`]: past it nothing more is written.
static SEEN: Mutex<Vec<(String, u32)>> = Mutex::new(Vec::new());
const SEEN_MAX: usize = 32;

/// Whether (step, code) is new to `seen` (and now in it).
fn first_time(seen: &Mutex<Vec<(String, u32)>>, step: &str, code: u32) -> bool {
    let mut seen = seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if seen.len() >= SEEN_MAX || seen.iter().any(|(s, c)| s == step && *c == code) {
        return false;
    }
    seen.push((step.to_owned(), code));
    true
}

/// The helper was not started, at a step of ours.
pub(crate) fn not_started(step: NotStarted) {
    not_started_in(&SEEN, step);
}

fn not_started_in(seen: &Mutex<Vec<(String, u32)>>, step: NotStarted) {
    if !first_time(seen, &format!("{step:?}"), 0) {
        return;
    }
    tracing::warn!(
        step = match step {
            NotStarted::Exclusion => "exclusion",
            NotStarted::Pipes => "pipes",
            NotStarted::Threads => "threads",
            NotStarted::WorkerThread => "worker thread",
            NotStarted::NoHelper => "no helper",
            NotStarted::Unsupported => "unsupported",
            NotStarted::HelperPath => "helper path",
            NotStarted::Limits => "limits",
            NotStarted::NotConfined => "not confined",
        },
        "media helper not started"
    );
}

/// The sandbox refused or failed: its step and the system's error code. `NeedsNewerWindows` has its own line
/// (`windows_floor`).
pub(crate) fn sandbox_failed(error: &SandboxError) {
    match error {
        SandboxError::Os { step, code } => os_failed(&SEEN, step, *code),
        SandboxError::Unsupported => not_started(NotStarted::Unsupported),
        SandboxError::HelperPath(_) => not_started(NotStarted::HelperPath),
        SandboxError::Limits(_) => not_started(NotStarted::Limits),
        SandboxError::NotConfined(_) => not_started(NotStarted::NotConfined),
        SandboxError::NeedsNewerWindows => {}
    }
}

/// A sandbox step that failed in the system. The step is the sandbox crate's fixed name (its words are in
/// `log_file/vocabulary_os.txt`); a `-` in it is written as a space (`read-back` has a code's shape to the log).
fn os_failed(seen: &Mutex<Vec<(String, u32)>>, step: &'static str, code: u32) {
    if first_time(seen, step, code) {
        let step = step.replace('-', " ");
        tracing::warn!(step = step.as_str(), os_error = code, "media helper not started");
    }
}

#[cfg(test)]
#[path = "refusal_log_tests.rs"]
pub(crate) mod tests;
