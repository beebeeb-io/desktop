//! The review rounds of #442 (delta round, rounds 3 to 5): quoted names, HTTP bodies, a held file, `custom:`,
//! cut-offs, the lock host, equals signs and the HTTP reason table.

use super::*;

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
        "Records",
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
    // `\u{2019}` is also a closing quote, so the name runs to the apostrophe of `couldn\u{2019}t` (over-redaction, accepted); the
    // error stays readable: its domain is in the OS phrase list, so its code is not a number beside a masked word.
    assert!(
        text.contains("<name>") && text.contains("NSCocoaErrorDomain 260"),
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
            !file.contains("acme-secret-host")
                && !file.contains("upstream")
                && !file.contains("nginx")
                && !file.contains("head title"),
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
    // A status is always written with its standard reason and `<body omitted>`: nothing after the digits is read.
    assert_eq!(redact::scrub("HTTP 404"), "HTTP 404 Not Found: <body omitted>");
    assert_eq!(
        redact::scrub("the HTTP client was created"),
        "the HTTP client was created"
    );
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
    let ops = super::super::FileOps {
        rename: refuse,
        remove: refuse_one,
    };
    let log = AppLog::open_with(dir.path(), cap, ops).unwrap();
    tracing::subscriber::with_default(log, || {
        for i in 0..600_u64 {
            tracing::warn!(n = i, "upload parked with its bytes kept in the queue");
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
        let ops = super::super::FileOps {
            rename: refuse,
            remove: refuse_one,
        };
        let log = AppLog::open_with(dir.path(), cap, ops).unwrap();
        tracing::subscriber::with_default(log, || {
            for i in 0..300_u64 {
                tracing::warn!(n = i, "upload parked with its bytes kept in the queue");
            }
        });
    }
    // The next run (the hold is gone): the full file rotates and the new line is kept.
    // A line as long as the ones that filled the file: it cannot fit in what is left, so the file must rotate.
    let text = logged(dir.path(), cap, || {
        tracing::warn!(n = 999_u64, "upload parked with its bytes kept in the queue")
    });
    assert!(text.contains("n=999"), "{text}");
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
    // `pid` is in no log call, so the process id beside it is masked with it (a number next to a masked word).
    assert!(
        text.contains("<…> on <host>, since") && !text.contains("4242"),
        "{text}"
    );
}

// ---- Round 3 (delta review of #442) ----

#[test]
fn an_http_status_with_no_plain_reason_still_drops_its_body() {
    for (what, text, kept) in [
        (
            "unknown status code",
            "HTTP 520 <unknown status code>: <html><head><title>Medical Records origin error</title></head><body>",
            "HTTP 520: <body omitted>",
        ),
        (
            "unknown status code, plain text",
            "HTTP 509 <unknown status code>: Bandwidth exceeded for anna smith account",
            "HTTP 509: <body omitted>",
        ),
        (
            "apostrophe in the reason",
            "HTTP 418 I'm a teapot: Medical Records body text here",
            "HTTP 418 I'm a teapot: <body omitted>",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let file = logged(dir.path(), MAX_FILE_BYTES, || tracing::warn!(error = %text, "{text}"));
        for secret in ["Medical", "Records", "anna", "Bandwidth", "head title"] {
            assert!(!file.contains(secret), "{what}: `{secret}` reached the file: {file}");
        }
        assert!(
            file.contains(kept),
            "{what}: the status stays, the body does not: {file}"
        );
    }
}

#[test]
fn a_quote_inside_a_quoted_name_does_not_end_it() {
    // The two sentences of `sync_root_owner::refusal`, for a folder whose own name holds a closing quote.
    assert_never_written(
        "closing quote in the name",
        "Reports",
        "The folder \u{201c}Q3 \u{201d}Final\u{201d} Reports\u{201d} is already the sync folder of OneDrive. Nothing was changed. Choose another folder.",
    );
    assert_never_written(
        "opening quote in the name",
        "Files",
        "Windows could not say whose sync folder \u{201c}Anna \u{201c}Work\u{201d} Files\u{201d} is. Nothing was changed. Try again, or choose another folder.",
    );
    // `\u{2019}` is also the apostrophe.
    assert_never_written(
        "apostrophe in single quotes",
        "Notes",
        "could not open \u{2018}Anna\u{2019}s Notes\u{2019} today",
    );
    let text = assert_never_written(
        "apostrophe inside double quotes",
        "Files",
        "The folder \u{201c}Anna\u{2019}s Files\u{201d} is busy.",
    );
    assert!(text.contains("<name>") && text.contains("is busy"), "{text}");
}

#[test]
fn a_custom_key_is_caught_after_an_equals_sign_and_in_a_debug_value() {
    assert_never_written(
        "category= in a text",
        "Medical",
        "backup pass done category=custom:Medical Records",
    );
    assert_never_written(
        "category= in a text, 2",
        "Records",
        "backup pass done category=custom:Medical Records",
    );
    assert_never_written(
        "Debug Custom",
        "Medical",
        "category Custom(\"Medical Records\") was refused",
    );
    assert_never_written(
        "Debug Custom, 2",
        "Records",
        "category Custom(\"Medical Records\") was refused",
    );
    // The key before the `=` is read like any other word.
    assert_never_written(
        "url before =custom:",
        "example.eu",
        "see https://example.eu/Medical?category=custom:X today",
    );
    assert_never_written(
        "path before =custom:",
        "Medical",
        "see /home/anna/Medical?category=custom:X today",
    );
    assert_never_written("email before =custom:", "anna@", "see anna@example.eu=custom:X today");
    // The word "custom" in prose is not the key.
    assert_eq!(
        redact::scrub("a custom folder was created"),
        "a custom folder was created"
    );
}

static HELD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn held_rename(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    if HELD.load(std::sync::atomic::Ordering::SeqCst) {
        Err(std::io::Error::other("held"))
    } else {
        fs::rename(from, to)
    }
}
fn held_remove(path: &std::path::Path) -> std::io::Result<()> {
    if HELD.load(std::sync::atomic::Ordering::SeqCst) {
        Err(std::io::Error::other("held"))
    } else {
        fs::remove_file(path)
    }
}

#[test]
fn the_cap_holds_while_the_file_is_held_and_the_log_recovers_in_the_same_run() {
    // One run: the file is held (rename and delete refused) for 300 lines, then released. The seam is a switch the test
    // flips; a real share-mode hold is not possible on the Linux build VM.
    let dir = tempfile::tempdir().unwrap();
    let cap = 4 * 1024;
    let ops = super::super::FileOps {
        rename: held_rename,
        remove: held_remove,
    };
    let log = AppLog::open_with(dir.path(), cap, ops).unwrap();
    tracing::subscriber::with_default(log, || {
        for i in 0..40_u64 {
            tracing::warn!(n = i, "upload parked before the lock, with its bytes kept in the queue");
        }
        HELD.store(true, std::sync::atomic::Ordering::SeqCst);
        for i in 40..340_u64 {
            tracing::warn!(n = i, "upload parked during the lock, with its bytes kept in the queue");
        }
        let held = fs::metadata(dir.path().join(FILE_NAME)).unwrap().len();
        assert!(held <= cap, "the file passed its cap while held: {held} bytes");
        HELD.store(false, std::sync::atomic::Ordering::SeqCst);
        for i in 0..20_u64 {
            tracing::warn!(n = i, "sync started after the lock, with its bytes kept in the queue");
        }
    });
    let current = fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
    assert!(
        current.contains("sync started after the lock") && current.contains("n=19\n"),
        "the log recovers when the hold ends: {current}"
    );
    // Only the rotation after the hold can move a line written during it into `.1`: the 40 lines before the hold already
    // rotated once, so `.1` existing says nothing.
    let old = fs::read_to_string(dir.path().join(OLD_FILE_NAME)).unwrap_or_default();
    assert!(
        old.contains("during the lock"),
        "the full file was rotated once it could be: {old}"
    );
    assert!(current.len() as u64 <= cap);
}

#[test]
fn a_reopen_that_finds_the_file_over_the_cap_drops_the_line() {
    // `rotate`'s reopen failed once (size 0, no handle) and the file is over the cap: the next line must not be written.
    let dir = tempfile::tempdir().unwrap();
    let cap = 1024;
    fs::write(dir.path().join(FILE_NAME), vec![b'x'; 4096]).unwrap();
    let mut sink = super::super::Sink {
        dir: dir.path().to_path_buf(),
        max_bytes: cap,
        ops: super::super::FileOps {
            rename: refuse,
            remove: refuse_one,
        },
        file: None,
        size: 0,
    };
    sink.write_line("one more line\n");
    assert_eq!(
        fs::metadata(dir.path().join(FILE_NAME)).unwrap().len(),
        4096,
        "a line was written past the cap"
    );
}

// ---- Round 4 (delta review 2 of #442) ----

#[test]
fn an_http_looking_folder_name_never_writes_what_follows_it() {
    // A folder called `HTTP 404 Medical Records` under the engine's path-printing texts (Windows and Unix).
    assert_never_written(
        "windows path",
        "Medical",
        "write lock C:\\Users\\a\\Projects\\HTTP 404 Medical Records\\Rome: Access is denied. (os error 5)",
    );
    assert_never_written(
        "unix path",
        "Medical",
        "write lock /home/a/Projects/HTTP 404 Medical Records/Rome: Access is denied. (os error 5)",
    );
    assert_never_written(
        "quoted name",
        "Smith",
        "upload of \u{201c}Anna HTTP 404 Smith Files\u{201d} failed: HTTP 500 Internal Server Error: body",
    );
    assert_never_written(
        "quoted name alone",
        "Anna",
        "could not read \u{201c}HTTP 200 Anna Smith\u{201d}: The system cannot find the file specified.",
    );
    // A status inside a text (after a `: `, a quote or a bracket) is still a status.
    let text = assert_never_written(
        "status after a colon",
        "Medical",
        "sync ops: HTTP 502 Bad Gateway: Medical Records page",
    );
    assert!(
        text.contains("sync ops: HTTP 502 Bad Gateway: <body omitted>"),
        "{text}"
    );
    // A prefix that is not at a boundary is dropped with the rest, and a later real one is still found (the `€` case).
    assert_never_written(
        "multibyte after HTTP",
        "Anna",
        "x HTTP 5\u{20ac} then HTTP 500 Internal Server Error: Anna Smith body",
    );
}

#[test]
fn a_path_or_token_with_an_equals_sign_is_read_like_any_other_word() {
    assert_never_written(
        "windows path with =",
        "Anna",
        "could not read C:\\Users\\Anna\\Downloads\\export=1\\Medical Records\\scan.pdf: Access is denied. (os error 5)",
    );
    assert_never_written(
        "windows path with =, folder",
        "Medical",
        "could not read C:\\Users\\Anna\\Downloads\\export=1\\Medical Records\\scan.pdf: Access is denied. (os error 5)",
    );
    assert_never_written(
        "unix path with =",
        "Medical",
        "could not read /home/anna/a=b/Medical Records/scan.pdf: Permission denied (os error 13)",
    );
    assert_never_written(
        "file name before =",
        "scan",
        "could not read C:\\Users\\Anna\\file=abc.pdf: Access is denied.",
    );
    assert_never_written(
        "padded base64",
        "dGhpcyBpcyBh",
        "got dGhpcyBpcyBhIHNlY3JldCB0b2tlbiBmb3IgdGVzdGluZw== from the server",
    );
    assert_never_written("email before =", "anna@", "sign in as anna@example.eu=1 failed");
    assert_never_written("file name before = 2", "Medical", "open Medical.pdf=1 failed");
}

#[test]
fn a_credential_keeps_its_label_even_when_it_holds_custom() {
    assert_never_written("password label", "hunter2", "password: hunter2=custom:abc");
    assert_never_written("bearer", "eyJhbGciOi", "Authorization: Bearer eyJhbGciOi=custom:x");
}

#[test]
fn quotes_of_mixed_styles_mark_from_the_first_opening_to_the_last_closing() {
    assert_never_written(
        "backtick then double",
        "Files",
        "`sync` failed for \u{201c}Anna`s Files\u{201d} today",
    );
    assert_never_written(
        "single then double",
        "Files",
        "\u{2018}Q3\u{2019} moved to \u{201c}Anna\u{2019}s Files\u{201d} today",
    );
    assert_never_written(
        "guillemet then double",
        "Files",
        "\u{ab}a\u{bb} moved to \u{201c}Anna\u{bb}s Files\u{201d} today",
    );
    // No closing quote of its own style: an apostrophe is not the end of `\u{201c}`, so the rest of the text is the name.
    assert_never_written(
        "unclosed double quote, then an apostrophe",
        "Photos",
        "could not open \u{201c}Anna Smith\u{2019}s Holiday Photos today",
    );
}

// ---- Round 5 (delta review 3 of #442): the HTTP reason comes from a fixed table ----
// Decision (lead, 2026-11-15 13:16): the HTTP reason comes from a fixed table, never from the text, because three
// input-reading designs leaked (delta reviews 1-3).

#[test]
fn an_http_prefix_in_a_folder_name_never_writes_a_byte_after_its_digits() {
    for text in [
        "write lock C:\\Users\\Anna\\Projects\\Site (HTTP 404 Medical Records)\\Rome: Access is denied. (os error 5)",
        "write lock /home/anna/Projects/Site (HTTP 404 Medical Records)/Rome: Permission denied (os error 13)",
        "write lock C:\\Users\\Anna\\Projects\\(HTTP 500) Medical Records\\Rome: Access is denied. (os error 5)",
        "could not read /Users/anna/Site \"HTTP 404 Medical Records\"/Rome: Permission denied (os error 13)",
        "could not read C:\\Users\\Anna\\Site (HTTP 404 x=1)\\Medical Records\\Rome: Access is denied.",
        "could not read /home/anna/Bug: HTTP 500 Medical Records/Rome: Permission denied (os error 13)",
    ] {
        for secret in ["Medical", "Records", "Smith", "x=1"] {
            assert_never_written("HTTP in a folder name", secret, text);
        }
    }
}

#[test]
fn a_real_status_is_written_with_the_tables_reason_and_never_the_inputs() {
    for (input, written) in [
        (
            "sync ops: HTTP 502 Bad Gateway: <html><head>",
            "sync ops: HTTP 502 Bad Gateway: <body omitted>",
        ),
        (
            "HTTP 520 <unknown status code>: <html>Medical Records",
            "HTTP 520: <body omitted>",
        ),
        (
            "HTTP 418 I'm a teapot: Medical Records",
            "HTTP 418 I'm a teapot: <body omitted>",
        ),
        (
            "HTTP 503 Service Unavailable: upstream Anna Smith",
            "HTTP 503 Service Unavailable: <body omitted>",
        ),
        // The input's reason is never copied, even when it is a plain phrase that differs from the table's.
        ("HTTP 404 Medical Records: body", "HTTP 404 Not Found: <body omitted>"),
        ("HTTP 599 anything at all", "HTTP 599: <body omitted>"),
        (
            "HTTP/1.1 502 Bad Gateway: Medical Records body",
            "HTTP 502 Bad Gateway: <body omitted>",
        ),
        ("HTTP/2 404 Medical Records", "HTTP 404 Not Found: <body omitted>"),
    ] {
        assert_eq!(redact::scrub(input), written, "input: {input}");
    }
}

#[test]
fn quoted_names_stay_closed_when_a_later_quote_is_open() {
    assert_never_written(
        "second name unclosed",
        "Files",
        "\u{201c}a\u{201d} and \u{201c}Anna\u{2019}s Files",
    );
    assert_never_written(
        "single then unclosed double",
        "Files",
        "\u{2018}a\u{2019} moved to \u{201c}Anna\u{2019}s Files today",
    );
}

#[test]
fn the_equals_cases_can_go_red_and_the_path_cut_off_is_pinned() {
    // Each secret is a word the input holds outside the part after the `=` (the key before it is what leaked).
    assert_never_written(
        "key is a path",
        "Downloads",
        "could not read C:\\Users\\Anna\\Downloads\\export=1\\Medical Records\\scan.pdf: Access is denied. (os error 5)",
    );
    assert_never_written(
        "unix key is a path",
        "anna",
        "could not read /home/anna/a=b/Medical Records/scan.pdf: Permission denied (os error 13)",
    );
    assert_never_written(
        "key is a path with a file name",
        "Anna",
        "could not read C:\\Users\\Anna\\file=abc.pdf: Access is denied.",
    );
    // The cut-off fires on `<path>=<path>` too: a later folder with two spaces must not be written.
    assert_never_written(
        "cut-off after <path>=",
        "Medical",
        "could not read C:\\Users\\Anna\\x=1\\My Medical Records\\scan.pdf: Access is denied.",
    );
}

#[test]
fn an_equals_sign_inside_an_email_does_not_split_it() {
    assert_never_written("email with =", "anna", "sign in as anna=smith@example.eu failed");
    assert_never_written(
        "email with = and a dot",
        "anna",
        "sign in as anna.smith=work@example.eu failed",
    );
}

#[test]
fn a_path_glued_to_the_word_before_it_is_cut_off() {
    assert_never_written(
        "glued unix path",
        "Medical",
        "at:/home/anna/My Medical Records/x: Permission denied",
    );
    assert_never_written(
        "glued windows path",
        "Medical",
        "at:C:\\Users\\Anna\\My Medical Records\\x: Access is denied.",
    );
}
