//! The local log file: a warning reaches it, every forbidden shape never does, the cap holds.

use std::fs;
use std::path::Path;

use super::{AppLog, FILE_NAME, MAX_FILE_BYTES, OLD_FILE_NAME, redact};

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
    assert!(
        text.contains(file_id),
        "an opaque file id is the diagnostic and stays: {text}"
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
        tracing::error!("e-line");
        tracing::warn!("w-line");
        tracing::info!("i-line");
        tracing::debug!("d-line");
        tracing::trace!("t-line");
    });
    assert_eq!(text.lines().count(), 3, "{text}");
    assert!(text.contains("e-line") && text.contains("w-line") && text.contains("i-line"));
    assert!(!text.contains("d-line") && !text.contains("t-line"), "{text}");
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
            tracing::warn!(
                n = i,
                "a line of the cap test, long enough to fill a file in a few dozen events"
            );
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
    logged(dir.path(), MAX_FILE_BYTES, || tracing::warn!("first run"));
    let text = logged(dir.path(), MAX_FILE_BYTES, || tracing::warn!("second run"));
    assert!(text.contains("first run") && text.contains("second run"), "{text}");
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
    // The brackets and commas are separators and are not written; every word stays.
    assert_eq!(
        text,
        "sync tick failed status 503 retry in 30 s hresult 0x80070005 version 0.9.0 state.db busy"
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
fn a_uuid_survives_only_in_an_id_field_and_a_credential_word_beats_an_id_suffix() {
    let id = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let dir = tempfile::tempdir().unwrap();
    let text = logged(dir.path(), MAX_FILE_BYTES, || {
        tracing::warn!(file_id = %id, session_id = %id, key_id = %id, error = %id, "ids");
    });
    assert_eq!(text.matches(id).count(), 1, "only file_id keeps its value: {text}");
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

// ---- Delta round (review of #442): quoted names, HTTP bodies, a held file, `custom:`, cut-offs, the lock host ----

/// Feeds `message` through the file log as a message and as a field value; the file must never hold `secret`.
fn assert_never_written(what: &str, secret: &str, message: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let text = logged(dir.path(), MAX_FILE_BYTES, || {
        tracing::warn!(error = %message, "{message}");
    });
    assert!(!text.contains(secret), "{what}: `{secret}` reached the file: {text}");
    text
}

#[test]
fn a_quoted_name_is_one_name_and_never_reaches_the_file() {
    // Both sentences of `sync_root_owner::refusal`, as `runner.rs:194` logs them.
    assert_never_written(
        "sync folder owned by another provider",
        "Medical",
        "The folder \u{201c}Medical Records\u{201d} is already the sync folder of OneDrive. Choose another folder.",
    );
    let text = assert_never_written(
        "registration unreadable",
        "Medical",
        "Windows could not say whose sync folder \u{201c}Medical Records\u{201d} is.",
    );
    assert!(
        text.contains("<name>") && text.contains("sync folder"),
        "the sentence stays readable: {text}"
    );
    // A quoted path with spaces (the closing quote used to hide where it ended), and the other quote styles.
    assert_never_written(
        "quoted path",
        "Rome",
        "could not read \u{201c}C:\\Users\\a\\Beebeeb\\Medical Records 2026 Rome\\x\u{201d} now",
    );
    assert_never_written(
        "single quotes",
        "Holiday",
        "the folder \u{2018}Holiday Photos\u{2019} is busy",
    );
    assert_never_written("guillemets", "Holiday", "the folder \u{ab}Holiday Photos\u{bb} is busy");
    assert_never_written("backticks", "Holiday", "the folder `Holiday Photos` is busy");
    assert_never_written("unclosed quote", "Holiday", "the folder \u{201c}Holiday Photos is busy");
    // An apostrophe is not an opening quote.
    let text = assert_never_written(
        "macOS prose",
        "Holiday",
        "The file \u{201c}Holiday.pdf\u{201d} couldn\u{2019}t be opened. NSCocoaErrorDomain 260",
    );
    assert!(
        text.contains("couldn\u{2019}t be opened") || text.contains("couldn"),
        "{text}"
    );
}

#[test]
fn an_http_status_keeps_its_prefix_and_never_its_body() {
    let html = "HTTP 502 Bad Gateway: <html><head><title>502 Bad Gateway</title></head><body><center>nginx for acme-secret-host</center>";
    let plain =
        "HTTP 503 Service Unavailable: upstream connect error or disconnect/reset before headers for acme-secret-host";
    for (what, text) in [("html", html), ("plain text", plain)] {
        let dir = tempfile::tempdir().unwrap();
        let file = logged(dir.path(), MAX_FILE_BYTES, || {
            tracing::warn!(error = %text, "sync_tick: /sync/ops failed; retrying next tick");
            tracing::warn!("{text}");
        });
        assert!(
            !file.contains("acme-secret-host") && !file.contains("upstream") && !file.contains("nginx"),
            "{what}: {file}"
        );
        assert!(file.contains("<body omitted>"), "{what}: {file}");
    }
    let dir = tempfile::tempdir().unwrap();
    let file = logged(dir.path(), MAX_FILE_BYTES, || tracing::warn!(error = %plain, "failed"));
    assert!(
        file.contains("error=HTTP 503 Service Unavailable: <body omitted>"),
        "the status and its reason stay: {file}"
    );
    // A bare status, or "HTTP" in prose, is not a prefix.
    assert_eq!(redact::scrub("HTTP 404"), "HTTP 404");
    assert_eq!(redact::scrub("the HTTP client was built"), "the HTTP client was built");
}

fn refuse(_: &std::path::Path, _: &std::path::Path) -> std::io::Result<()> {
    Err(std::io::Error::other("held by another program"))
}
fn refuse_one(_: &std::path::Path) -> std::io::Result<()> {
    Err(std::io::Error::other("held by another program"))
}

#[test]
fn the_cap_holds_when_the_file_can_be_neither_renamed_nor_deleted() {
    // A seam stands for a log file another program holds open without delete sharing (on Windows both refuse); a real
    // share-mode hold is not possible on the Linux build VM, so this is the seam, not the OS.
    let dir = tempfile::tempdir().unwrap();
    let cap = 4 * 1024;
    let ops = super::FileOps {
        rename: refuse,
        remove: refuse_one,
    };
    let log = AppLog::open_with(dir.path(), cap, ops).unwrap();
    tracing::subscriber::with_default(log, || {
        for i in 0..600_u64 {
            tracing::warn!(
                n = i,
                "a line of the held-file test, long enough to pass the cap in a few dozen events"
            );
        }
    });
    let text = fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
    assert!(
        text.len() as u64 <= cap,
        "the file passed its cap: {} bytes",
        text.len()
    );
    assert!(
        text.contains("n=0\n") && !text.contains("n=599"),
        "lines are dropped, not the file grown: {} lines",
        text.lines().count()
    );
    assert!(!dir.path().join(OLD_FILE_NAME).exists());
}

#[test]
fn a_rotation_that_works_again_resumes_the_log() {
    let dir = tempfile::tempdir().unwrap();
    let cap = 4 * 1024;
    {
        let ops = super::FileOps {
            rename: refuse,
            remove: refuse_one,
        };
        let log = AppLog::open_with(dir.path(), cap, ops).unwrap();
        tracing::subscriber::with_default(log, || {
            for i in 0..300_u64 {
                tracing::warn!(
                    n = i,
                    "a line of the held-file test, long enough to pass the cap in a few dozen events"
                );
            }
        });
    }
    // The next run (the hold is gone): the full file rotates and the new line is kept.
    let text = logged(dir.path(), cap, || tracing::warn!("after the hold"));
    assert!(text.contains("after the hold"), "{text}");
    assert!(dir.path().join(OLD_FILE_NAME).exists());
}

#[test]
fn a_custom_backup_folder_key_never_prints_its_name() {
    for key in [
        "custom:Medical Records",
        "custom:Holiday",
        "Custom:Medical Records 2026",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let text = logged(dir.path(), MAX_FILE_BYTES, || {
            tracing::info!(category = %key, "backup: pass done");
        });
        assert!(
            !text.contains("Medical") && !text.contains("Holiday") && !text.contains("Records"),
            "{key}: {text}"
        );
        assert!(text.contains("custom:<name>"), "{key}: {text}");
    }
}

#[test]
fn the_path_cut_off_also_holds_after_a_redacted_label_or_key() {
    assert_never_written(
        "label then path",
        "Records",
        "path has no file name: C:\\Users\\a\\Beebeeb\\Medical Records 2026 Rome\\x",
    );
    assert_never_written(
        "key= then path",
        "Records",
        "failed error=C:\\Users\\a\\Beebeeb\\Medical Records 2026 Rome\\x",
    );
    assert_never_written(
        "key= then unix path",
        "Records",
        "failed error=/home/a/Medical Records 2026 Rome/x",
    );
}

#[test]
fn a_file_name_with_a_long_extension_is_a_name() {
    for name in ["Budget.numbers", "cache.sqlite3", "README.markdown", "Notes.mdownwrk"] {
        let text = redact::scrub(&format!("could not read {name} today"));
        assert!(!text.contains(name.split('.').next().unwrap()), "{name}: {text}");
    }
}

#[test]
fn the_host_of_the_lock_held_sentence_is_replaced() {
    let text = assert_never_written(
        "lock host",
        "Annas",
        "another Beebeeb agent is already syncing this folder: beebeeb-desktop (pid 4242 on Annas-PC, since 2026-11-15T07:00:00+00:00)",
    );
    assert!(
        text.contains("<host>") && text.contains("pid 4242") && text.contains("since"),
        "{text}"
    );
}
