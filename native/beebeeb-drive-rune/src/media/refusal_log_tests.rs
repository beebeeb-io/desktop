//! The refusal lines through the REAL log filter (`log_file::AppLog`): the words stay readable, and nothing of a
//! file reaches the log.

use std::sync::Mutex;

use beebeeb_media_protocol::{Codec, ErrorCode};
use beebeeb_office_sandbox::SandboxError;

use super::*;
use crate::log_file::{AppLog, FILE_NAME, MAX_FILE_BYTES};

/// Runs `body` with the file log as the thread's subscriber and returns what it wrote.
pub(crate) fn logged(body: impl FnOnce()) -> String {
    let dir = tempfile::tempdir().unwrap();
    let log = AppLog::open(dir.path(), MAX_FILE_BYTES).expect("the log opens");
    tracing::subscriber::with_default(log, body);
    std::fs::read_to_string(dir.path().join(FILE_NAME)).unwrap_or_default()
}

#[test]
fn every_refusal_is_a_readable_line_with_its_codec() {
    let all = [
        (Refusal::NoDecoder("AC-3".into()), "reason=no decoder"),
        (Refusal::NoSystemDecoder(Codec::Hevc), "reason=system decoder off codec=hevc"),
        (Refusal::NoSystemMedia(Codec::H264), "reason=no system media codec=h264"),
        (Refusal::CouldNotPlay, "reason=failed"),
        (Refusal::NotSignedIn, "reason=not signed in"),
        (Refusal::NeedsWindowsUpdate, "reason=needs windows update"),
        (Refusal::NotInstalled, "reason=no helper"),
        (Refusal::NoSandbox, "reason=no sandbox"),
    ];
    for (why, want) in all {
        let text = logged(|| refused(&why));
        assert!(text.contains("media refused"), "{why:?}: {text}");
        assert!(text.contains(want), "{why:?}: {text}");
        assert!(!text.contains("<\u{2026}>"), "no word of the line is masked: {text}");
    }
}

#[test]
fn every_codec_word_and_helper_code_passes_the_filter() {
    let codecs = [
        Codec::H264,
        Codec::Hevc,
        Codec::Vp8,
        Codec::Vp9,
        Codec::Av1,
        Codec::Mpeg2,
        Codec::ProRes,
        Codec::Dnx,
        Codec::Mp3,
        Codec::Aac,
        Codec::Flac,
        Codec::Alac,
        Codec::Opus,
        Codec::Vorbis,
        Codec::Pcm,
        Codec::Ac3,
        Codec::Eac3,
        Codec::Mp2,
        Codec::Mpeg4Part2,
        Codec::Vc1,
        Codec::Wma,
        Codec::Other,
    ];
    for codec in codecs {
        let text = logged(|| helper_error(ErrorCode::Unsupported(codec)));
        let want = format!("code=unsupported codec={}", codec_word(codec));
        assert!(text.contains(&want), "{codec:?}: {text}");
    }
    let codes = [
        (ErrorCode::NoSystemDecoder(Codec::Hevc), "code=no system decoder codec=hevc"),
        (ErrorCode::NoSystemMedia(Codec::Hevc), "code=no system media codec=hevc"),
        (ErrorCode::NoDecoder, "code=no decoder codec=none"),
        (ErrorCode::Corrupt, "code=corrupt"),
        (ErrorCode::TooLarge, "code=too large"),
        (ErrorCode::ReadFailed, "code=read failed"),
        (ErrorCode::Timeout, "code=timeout"),
        (ErrorCode::Internal, "code=internal"),
    ];
    for (code, want) in codes {
        let text = logged(|| helper_error(code));
        assert!(text.contains(want), "{code:?}: {text}");
        assert!(!text.contains("<\u{2026}>"), "{text}");
    }
}

#[test]
fn a_sandbox_failure_says_its_step_and_code_once() {
    let seen = Mutex::new(Vec::new());
    let text = logged(|| {
        start_failed(&seen, "create process", 5);
        start_failed(&seen, "create process", 5);
        start_failed(&seen, "job limits", 87);
    });
    assert_eq!(text.matches("media helper not started").count(), 2, "{text}");
    assert!(text.contains("step=create process code=5"), "{text}");
    assert!(text.contains("step=job limits code=87"), "{text}");
}

#[test]
fn a_broken_rule_says_what_and_where() {
    let text = logged(|| broke("timeout", 731));
    assert!(text.contains("media helper stopped by the player"), "{text}");
    assert!(text.contains("what=timeout at=731"), "{text}");
}

#[test]
fn every_sandbox_error_has_a_step_but_the_windows_floor() {
    // `sandbox_failed` writes to the process's own list: a step no other test uses.
    let text = logged(|| {
        sandbox_failed(&SandboxError::Os { step: "resume", code: 1450 });
        sandbox_failed(&SandboxError::NeedsNewerWindows);
    });
    assert!(text.contains("step=resume code=1450"), "{text}");
    assert_eq!(text.lines().count(), 1, "{text}");
}

/// Every step name of the sandbox crate (`crates/beebeeb-office-sandbox`, `SandboxError::Os`), as of ux-02: each
/// passes the filter whole, so a failure on a person's PC names its step.
#[test]
fn every_sandbox_step_name_is_readable_in_the_log() {
    let steps = [
        "administrators identifier",
        "attribute",
        "attribute list",
        "attribute list size",
        "capabilities read-back",
        "capability identifier",
        "container profile",
        "container profile removal",
        "create process",
        "exit code",
        "grant",
        "handle count",
        "helper token",
        "inheritable pipe end",
        "integrity read-back",
        "job",
        "job UI read-back",
        "job UI restrictions",
        "job limits",
        "job limits read-back",
        "job membership",
        "kill",
        "lpac read-back",
        "map",
        "memory read",
        "own token",
        "pipe",
        "random",
        "restricted token",
        "resume",
        "section",
        "section view",
        "system directory",
        "wait",
        "watch handle",
        "watch thread",
    ];
    for step in steps {
        let seen = Mutex::new(Vec::new());
        let text = logged(|| start_failed(&seen, step, 5));
        assert!(text.contains(&format!("step={step} code=5")), "{step}: {text}");
    }
}
