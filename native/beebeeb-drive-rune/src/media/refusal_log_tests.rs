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
        (Refusal::NoSystemDecoder(Codec::Hevc), "reason=no system decoder"),
        (Refusal::NoSystemMedia(Codec::H264), "reason=no system media"),
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
    let mut words = std::collections::BTreeSet::new();
    for codec in codecs {
        let text = logged(|| helper_error(ErrorCode::Unsupported(codec)));
        let word = text.trim_end().rsplit("answer=unsupported codec=").next().unwrap().to_owned();
        assert!(text.contains("answer=unsupported codec="), "{codec:?}: {text}");
        assert!(!word.contains('<') && !word.contains(' '), "{codec:?} is one readable word: {text}");
        words.insert(word);
    }
    assert_eq!(words.len(), codecs.len(), "each format has its own word: {words:?}");
    let codes = [
        (ErrorCode::NoSystemDecoder(Codec::Hevc), "answer=no system decoder codec=hevc"),
        (ErrorCode::NoSystemMedia(Codec::Hevc), "answer=no system media codec=hevc"),
        (ErrorCode::NoDecoder, "answer=no decoder codec=none"),
        (ErrorCode::Corrupt, "answer=corrupt"),
        (ErrorCode::TooLarge, "answer=too large"),
        (ErrorCode::ReadFailed, "answer=read failed"),
        (ErrorCode::Timeout, "answer=timeout"),
        (ErrorCode::Internal, "answer=internal"),
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
        os_failed(&seen, "create process", 5);
        os_failed(&seen, "create process", 5);
        os_failed(&seen, "job limits", 87);
        not_started_in(&seen, NotStarted::Pipes);
        not_started_in(&seen, NotStarted::Pipes);
    });
    assert_eq!(text.matches("media helper not started").count(), 3, "{text}");
    assert!(text.contains("step=pipes"), "{text}");
    assert!(text.contains("step=create process os_error=5"), "{text}");
    assert!(text.contains("step=job limits os_error=87"), "{text}");
}

#[test]
fn a_broken_rule_says_what_and_where() {
    let text = logged(|| broke(Broke::Timeout, 731));
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
    assert!(text.contains("step=resume os_error=1450"), "{text}");
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
        let text = logged(|| os_failed(&seen, step, 5));
        let want = format!("step={} os_error=5", step.replace('-', " "));
        assert!(text.contains(&want), "{step}: {text}");
    }
}

#[test]
fn every_step_of_ours_and_every_broken_rule_is_readable() {
    let steps = [
        (NotStarted::Exclusion, "step=exclusion"),
        (NotStarted::Pipes, "step=pipes"),
        (NotStarted::Threads, "step=threads"),
        (NotStarted::WorkerThread, "step=worker thread"),
        (NotStarted::NoHelper, "step=no helper"),
        (NotStarted::Unsupported, "step=unsupported"),
        (NotStarted::HelperPath, "step=helper path"),
        (NotStarted::Limits, "step=limits"),
        (NotStarted::NotConfined, "step=not confined"),
    ];
    for (step, want) in steps {
        let seen = Mutex::new(Vec::new());
        let text = logged(|| not_started_in(&seen, step));
        assert!(text.trim_end().ends_with(want), "{step:?}: {text}");
    }
    let rules = [
        (Broke::Check, "what=check"),
        (Broke::Gone, "what=gone"),
        (Broke::Protocol, "what=protocol"),
        (Broke::Timeout, "what=timeout"),
        (Broke::Read, "what=read"),
    ];
    for (rule, want) in rules {
        let text = logged(|| broke(rule, 7));
        assert!(text.contains(&format!("{want} at=7")), "{rule:?}: {text}");
    }
}
