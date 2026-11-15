//! The local log file: a warning reaches it, every forbidden shape never does, the cap holds.

use std::fs;
use std::path::Path;

use super::{AppLog, FILE_NAME, MAX_FILE_BYTES, OLD_FILE_NAME, redact};

#[path = "allow_list_tests.rs"]
mod allow_list;
#[path = "review_rounds_tests.rs"]
mod review_rounds;
#[path = "vocabulary_sync.rs"]
mod vocabulary_sync;

/// Runs `body` with the file log (cap `max_bytes`) as the thread's subscriber, then returns the current file's text.
fn logged(dir: &Path, max_bytes: u64, body: impl FnOnce()) -> String {
    let log = AppLog::open(dir, max_bytes).expect("the log opens");
    tracing::subscriber::with_default(log, body);
    fs::read_to_string(dir.join(FILE_NAME)).unwrap_or_default()
}

#[test]
fn an_engine_warning_reaches_the_file_with_its_opaque_fields() {
    let dir = tempfile::tempdir().unwrap();
    let file_id = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let text = logged(dir.path(), MAX_FILE_BYTES, || {
        // The engine's own line shape (`engine_bridge/syncflow.rs`, `lockfile.rs`): a target, opaque ids, a count.
        tracing::warn!(target: "beebeeb_engine::runner", file_id = %file_id, attempt = 3_u64, reason = "attempts_used_up", "upload parked with its bytes kept in the queue");
    });
    assert_eq!(text.lines().count(), 1, "one event is one line: {text}");
    assert!(
        text.contains(" WARN beebeeb_engine::runner upload parked with its bytes kept in the queue"),
        "{text}"
    );
    // A UUID correlates even where it is not a name: it is `<id>` in every field (round 6).
    assert!(
        text.contains("file_id=<id>") && !text.contains(file_id),
        "an opaque file id is written as <id>: {text}"
    );
    assert!(
        text.contains("attempt=3") && text.contains("reason=attempts_used_up"),
        "{text}"
    );
}

#[test]
fn info_is_written_and_debug_and_trace_are_not() {
    let dir = tempfile::tempdir().unwrap();
    let text = logged(dir.path(), MAX_FILE_BYTES, || {
        tracing::error!("sync 1");
        tracing::warn!("sync 2");
        tracing::info!("sync 3");
        tracing::debug!("sync 4");
        tracing::trace!("sync 5");
    });
    assert_eq!(text.lines().count(), 3, "{text}");
    assert!(text.contains("sync 1") && text.contains("sync 2") && text.contains("sync 3"));
    assert!(!text.contains("sync 4") && !text.contains("sync 5"), "{text}");
}

/// Every shape the brief forbids, as (what it is, the secret part that must never be in the file, the text it is in).
fn forbidden_in_messages() -> Vec<(&'static str, &'static str, String)> {
    let base64 = "q83Jx0aZpL2mN8vB4cT7yU1wE5rHkD9sFgA6oPiXzQ3";
    vec![
        (
            "windows path",
            "Holiday2026",
            r"could not read C:\Users\anna\Documents\Holiday2026\x for sync".into(),
        ),
        (
            "unix path",
            "tax-return",
            "could not read /home/anna/Documents/tax-return for sync".into(),
        ),
        (
            "unc path",
            "fileserver",
            r"could not read \\fileserver\share\secret for sync".into(),
        ),
        ("home path", "diary", "could not read ~/diary for sync".into()),
        (
            "file name",
            "Passport-scan.pdf",
            "could not upload Passport-scan.pdf to the server".into(),
        ),
        (
            "file name in quotes",
            "Salary.xlsx",
            "could not upload 'Salary.xlsx' to the server".into(),
        ),
        ("email", "anna@example.eu", "signed in as anna@example.eu".into()),
        (
            "url",
            "example.eu/api/v1/files",
            "request to https://example.eu/api/v1/files failed".into(),
        ),
        ("base64 token", base64, format!("session refused for {base64}")),
        (
            "hex key",
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
            "key 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08 refused".into(),
        ),
        ("verification code", "493817", "the code 493817 was refused".into()),
        (
            "recovery phrase",
            "abandon",
            "phrase was abandon ability able about above absent absorb abstract absurd accident account accuse here"
                .into(),
        ),
        (
            "json server body",
            "hunter2",
            r#"server said {"error":"bad","detail":"hunter2"} and stopped"#.into(),
        ),
        (
            "html server body",
            "secretvalue",
            "<html><body>secretvalue</body></html>".into(),
        ),
        (
            "key=value inside a message",
            "Holiday",
            "retrying path=Holiday after the failure".into(),
        ),
    ]
}

#[test]
fn no_forbidden_shape_in_a_message_reaches_the_file() {
    for (what, secret, message) in forbidden_in_messages() {
        let dir = tempfile::tempdir().unwrap();
        let text = logged(dir.path(), MAX_FILE_BYTES, || {
            tracing::warn!("{message}");
        });
        assert!(
            text.contains(" WARN "),
            "{what}: the event itself is still logged: {text}"
        );
        assert!(!text.contains(secret), "{what}: `{secret}` reached the file: {text}");
        assert_eq!(text.lines().count(), 1, "{what}: {text}");
    }
}

#[test]
fn no_forbidden_shape_in_a_field_value_reaches_the_file() {
    // The same shapes, as the value of a field whose NAME is harmless (`error`, `detail`): the shape check catches them.
    for (what, secret, message) in forbidden_in_messages() {
        let dir = tempfile::tempdir().unwrap();
        let text = logged(dir.path(), MAX_FILE_BYTES, || {
            tracing::warn!(error = %message, detail = ?message, "step failed");
        });
        assert!(
            !text.contains(secret),
            "{what}: `{secret}` reached the file through a field value: {text}"
        );
    }
}

#[test]
fn a_field_named_like_a_secret_is_dropped_whatever_its_value() {
    let dir = tempfile::tempdir().unwrap();
    let text = logged(dir.path(), MAX_FILE_BYTES, || {
        tracing::warn!(
            path = "plain",
            rel_path = "plain2",
            name = "plain3",
            file_name = "plain4",
            email = "plain5",
            token = "plain6",
            api_key = "plain7",
            password = "plain8",
            phrase = "plain9",
            code = 123_u64,
            body = "plain10",
            url = "plain11",
            sync_root = "plain12",
            state_dir = "plain13",
            dest = "plain14",
            "dropped fields"
        );
    });
    for n in 2..=14 {
        assert!(
            !text.contains(&format!("plain{n}")),
            "plain{n} reached the file: {text}"
        );
    }
    assert!(!text.contains("path=plain "), "{text}");
    assert!(
        text.contains("path=<redacted>") && text.contains("code=<redacted>"),
        "the field is named, its value is not: {text}"
    );
}

#[test]
fn an_opaque_id_field_is_kept_even_when_a_denied_word_comes_first() {
    assert!(!redact::denied_name("root_id") && !redact::denied_name("dir_count") && !redact::denied_name("file_id"));
    assert!(redact::denied_name("session_id") && redact::denied_name("key_count") && redact::denied_name("key_id"));
    assert!(redact::denied_name("root") && redact::denied_name("request_key") && redact::denied_name("filename"));
}

#[test]
fn a_long_value_is_capped_and_one_event_stays_one_line() {
    let dir = tempfile::tempdir().unwrap();
    let long = "word ".repeat(400);
    let text = logged(dir.path(), MAX_FILE_BYTES, || {
        tracing::warn!(error = %format!("line one\nline two\r\n{long}"), "multi-line");
    });
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(text.len() < 400, "{} bytes", text.len());
}

#[test]
fn the_cap_holds_two_files_and_the_oldest_lines_go() {
    let dir = tempfile::tempdir().unwrap();
    let cap = 4 * 1024;
    let log = AppLog::open(dir.path(), cap).unwrap();
    tracing::subscriber::with_default(log, || {
        for i in 0..400_u64 {
            tracing::warn!(n = i, "upload parked with its bytes kept in the queue");
        }
    });
    let current = fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
    let old = fs::read_to_string(dir.path().join(OLD_FILE_NAME)).unwrap();
    let one_line = current.lines().next().unwrap().len() as u64 + 1;
    assert!(
        current.len() as u64 <= cap + one_line && old.len() as u64 <= cap + one_line,
        "{} / {}",
        current.len(),
        old.len()
    );
    let files: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(files.len(), 2, "exactly two files: {files:?}");
    assert!(current.contains("n=399"), "the newest line is kept");
    assert!(
        !old.contains("n=0\n") && !current.contains("n=0\n"),
        "the oldest line is dropped"
    );
}

#[test]
fn the_log_appends_to_what_an_earlier_run_left() {
    let dir = tempfile::tempdir().unwrap();
    logged(dir.path(), MAX_FILE_BYTES, || tracing::warn!("run 1"));
    let text = logged(dir.path(), MAX_FILE_BYTES, || tracing::warn!("run 2"));
    assert!(text.contains("run 1") && text.contains("run 2"), "{text}");
}

#[test]
fn a_log_folder_that_cannot_be_made_is_not_a_failure_to_start() {
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("a-file");
    fs::write(&blocker, b"x").unwrap();
    assert!(AppLog::open(&blocker.join("logs"), MAX_FILE_BYTES).is_none());
    assert!(!super::install(&blocker), "install reports false and does not panic");
}

#[test]
fn harmless_shapes_stay_readable() {
    // Over-redaction is accepted, but the lines a person reads in a support case must mostly survive.
    let text =
        redact::scrub("sync tick failed (status 503) retry in 30 s, hresult 0x80070005, version 0.9.0, state.db busy");
    // Every word is a log call's word or a number, so the line is written as it is, its punctuation too.
    assert_eq!(
        text,
        "sync tick failed (status 503) retry in 30 s, hresult 0x80070005, version 0.9.0, state.db busy"
    );
}

#[test]
fn credential_labels_and_shapes_the_first_round_missed_never_reach_the_file() {
    let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJhbm5hIn0.c2lnbmF0dXJl";
    let cases: Vec<(&str, &str, String)> = vec![
        ("jwt", "eyJhbGciOiJIUzI1NiJ9", format!("refused {jwt}")),
        ("authorization header", "s3cr3t-value", "sent Authorization: Bearer s3cr3t-value to the server".into()),
        ("bearer without a colon", "s3cr3t-value", "sent Bearer s3cr3t-value to the server".into()),
        ("password label", "hunter2", "login failed, password: hunter2 was refused".into()),
        ("token label", "abc", "token: abc was refused".into()),
        ("path with spaces", "Smith", r"could not read C:\Users\Anna Smith\Taxes 2026\file for sync".into()),
        ("unix path with spaces", "Smith", "could not read /home/anna/Anna Smith/Taxes for sync".into()),
        ("uuid-shaped secret in a message", "0f8fad5b-d9cb-469f-a165-70867728950e", "refused 0f8fad5b-d9cb-469f-a165-70867728950e today".into()),
        ("quoted phrase from a debug value", "abandon", r#"["abandon", "ability", "able", "about", "above", "absent", "absorb", "abstract", "absurd", "accident", "account", "accuse"]"#.into()),
        ("debug struct with a path", "Anna", r#"Os { code: 5, kind: NotFound, message: "C:\Users\Anna\x" }"#.into()),
    ];
    for (what, secret, message) in cases {
        let dir = tempfile::tempdir().unwrap();
        let text = logged(dir.path(), MAX_FILE_BYTES, || {
            tracing::warn!(error = ?message, detail = %message, "{message}");
        });
        assert!(!text.contains(secret), "{what}: `{secret}` reached the file: {text}");
    }
}

#[test]
fn a_uuid_is_an_id_in_every_field_and_a_credential_word_beats_an_id_suffix() {
    let id = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let dir = tempfile::tempdir().unwrap();
    let text = logged(dir.path(), MAX_FILE_BYTES, || {
        tracing::warn!(file_id = %id, session_id = %id, key_id = %id, error = %id, "ids");
    });
    // Rule 6 of round 6: a UUID is `<id>` everywhere, `file_id` too (it correlates).
    assert_eq!(text.matches(id).count(), 0, "no field keeps a UUID: {text}");
    assert!(text.contains("file_id=<id>") && text.contains("error=<id>"), "{text}");
    assert!(
        text.contains("session_id=<redacted>") && text.contains("key_id=<redacted>"),
        "{text}"
    );
}

#[test]
fn a_number_shaped_like_a_code_is_blanked_unless_it_is_a_plain_metric() {
    let dir = tempfile::tempdir().unwrap();
    let text = logged(dir.path(), MAX_FILE_BYTES, || {
        tracing::warn!(
            value = 493817_u64,
            bytes = 1234567_u64,
            otp = 123456_u64,
            count = 3_u64,
            "numbers"
        );
    });
    assert!(
        text.contains("value=<code>") && text.contains("otp=<redacted>"),
        "{text}"
    );
    assert!(text.contains("bytes=1234567") && text.contains("count=3"), "{text}");
}

#[test]
fn a_yes_or_no_and_a_count_named_like_a_word_are_kept() {
    let dir = tempfile::tempdir().unwrap();
    let text = logged(dir.path(), MAX_FILE_BYTES, || {
        // The held-edit warning's own fields (`watcher.rs`): counts named `content_unknown` and a bool `held_key_replaced`.
        tracing::warn!(held = 3_u64, content_unknown = 2_u64, held_key_replaced = true, "held");
    });
    assert!(
        text.contains("content_unknown=2") && text.contains("held_key_replaced=true"),
        "{text}"
    );
}

#[test]
fn pass_is_a_credential_word_in_a_field_name_and_as_a_label() {
    let dir = tempfile::tempdir().unwrap();
    let text = logged(dir.path(), MAX_FILE_BYTES, || {
        tracing::warn!(
            pass = "hunter2",
            db_pass = "hunter3",
            pass_count = 7_u64,
            "login pass: hunter4 refused"
        );
    });
    for secret in ["hunter2", "hunter3", "hunter4"] {
        assert!(!text.contains(secret), "{secret} reached the file: {text}");
    }
}
