//! Round 6 and after: the allow-list (fail closed), the word split, numbers next to masked words, grouped digit
//! runs and the pinned OS phrase list.

use super::*;

// ---- Round 6: the allow-list (fail closed) ----

/// Words of the reviewers' corpora that are a person's (no log call has them): each must be masked when alone.
const NAME_WORDS: [&str; 14] = [
    "Anna",
    "Smith",
    "Medical",
    "Holiday",
    "Rome",
    "Trip",
    "corp",
    "Hotel",
    "MacBook",
    "hunter2",
    "Notes",
    "Budget",
    "Projects",
    "Downloads",
];

/// The engine texts that print a path or a name (`state_paths.rs`, `sync_root_owner.rs`, the write lock, a read),
/// with the shapes delta reviews 1 to 4 found leaking, as message and as field value.
fn round6_corpus() -> Vec<String> {
    let names = [
        r"C:\Users\Anna\Beebeeb\Medical Records Anna 2026\Holiday Photos Rome\scan.pdf",
        r"C:\Users\Anna\x=1\anna@corp.eu\My Medical Records\Rome",
        r"C:\Users\Anna\Backups\anna@corp.eu\My Medical Records\Rome",
        r"C:\Users\Anna\Pictures\Trip@Rome.2024\My Medical Records\scan.pdf",
        "/home/anna@corp.example.eu/My Medical Records/Rome",
        "/home/anna/a://b/My Medical Records/x",
        r"C:\Users\Anna\Projects\Site (HTTP 404 Medical Records)\Rome",
        r"\\fileserver\Anna\export=1\HTTP 404 Medical Records\Rome",
        r"C:\Users\Anna\Medical Records Anna 2026",
        "at:~/My Medical Records/x",
        "x/home/anna/My Medical Records/x",
    ];
    let templates = [
        "could not read {}: Access is denied. (os error 5)",
        "legacy state path {} exists but is not a directory",
        "create state dir {}: Permission denied (os error 13)",
        "write lock {}: Access is denied. (os error 5)",
    ];
    let folders = [
        "The \u{201c}Final\u{201d} Medical Records (HTTP 404)",
        "Q3 \u{201c}Final\u{201d} Medical Records HTTP 404 notes",
        "Anna\u{2019}s Medical Records (HTTP 404)",
        "Medical Records",
    ];
    let sentences = [
        "The folder \u{201c}{}\u{201d} is already the sync folder of OneDrive. Nothing was changed. Choose another folder.",
        "Windows could not say whose sync folder \u{201c}{}\u{201d} is. Nothing was changed. Try again, or choose another folder.",
    ];
    let mut corpus: Vec<String> = templates
        .iter()
        .flat_map(|t| names.iter().map(move |n| t.replace("{}", n)))
        .chain(
            sentences
                .iter()
                .flat_map(|t| folders.iter().map(move |f| t.replace("{}", f))),
        )
        .collect();
    corpus.extend(
        [
            "Anna Smith Medical Records",
            "lock held: desktop (pid 1 on Anna HTTP 404 Mac, since x)",
            "Medical Records: HTTP 404 {\"a\":1}",
            "\u{201c}a\u{201d} Medical Records HTTP 404 x\u{201d}",
            "`a` and `Anna Files today",
            "sign in as Anna=work@example.eu failed",
            "upload of Holiday Rome failed: Hotel MacBook hunter2",
            "BackupFolder { category: Custom( \"Medical Records\", ) }",
            "filter=category=custom:Medical Records",
            "ignoring non-absolute sync_root in desktop.toml: Medical Records",
        ]
        .map(str::to_string),
    );
    corpus
}

#[test]
fn rule_1_no_word_of_a_persons_name_reaches_the_file_whatever_its_shape() {
    for word in NAME_WORDS {
        assert_eq!(
            redact::scrub(word),
            "<…>",
            "`{word}` is no log call's word, so it is masked"
        );
    }
    let corpus = round6_corpus();
    assert!(corpus.len() >= 60, "{}", corpus.len());
    let mut written = 0;
    for text in &corpus {
        for out in [redact::message(text), redact::value("error", text)] {
            written += 1;
            for word in NAME_WORDS {
                let found = out
                    .split(|c: char| !c.is_alphanumeric())
                    .any(|w| w.eq_ignore_ascii_case(word));
                assert!(!found, "`{word}` reached the file: {text} => {out}");
            }
        }
    }
    assert_eq!(written, corpus.len() * 2);
}

#[test]
fn rule_2_a_real_status_line_keeps_its_status_and_drops_its_body() {
    assert_eq!(
        redact::scrub("sync ops: HTTP 502 Bad Gateway: <html><title>Anna</title></html>"),
        "sync ops: HTTP 502 Bad Gateway: <body omitted>"
    );
}

#[test]
fn rule_3_an_os_error_stays_readable_after_a_path() {
    assert_eq!(
        redact::scrub(r"could not read C:\Users\Anna\Medical Records\scan.pdf: Access is denied. (os error 5)"),
        "could not read <path>: Access is denied. (os error 5)"
    );
    assert_eq!(
        redact::scrub(
            "could not read /home/anna/x=1/anna@corp.eu/My Medical Records/scan.pdf: Permission denied (os error 13)"
        ),
        "could not read <path>: Permission denied (os error 13)"
    );
    // A leaf with spaces and no `:` after it: the rest of the text is the path's (its words cannot be told apart).
    assert_eq!(
        redact::scrub(r"legacy state path C:\Users\Anna\Medical Records 2026 exists but is not a directory"),
        "legacy state path <path>"
    );
}

#[test]
fn rule_5_numbers_and_units_pass() {
    assert_eq!(
        redact::scrub("uploaded 3 files, 12.5 MB in 840 ms"),
        "uploaded 3 files, 12.5 MB in 840 ms"
    );
    // A long digit run can be a phone, account or citizen number: 6 to 8 digits stay `<code>`, longer is masked.
    assert_eq!(redact::scrub("uploaded 123456789 files"), "uploaded <…> files");
    // A version stays; an IPv4 address says where a person is and is masked.
    assert_eq!(
        redact::scrub("version 0.9.0, connect 10.0.5.161 timed out"),
        "version 0.9.0, connect <…> timed out"
    );
    // A dot does not let a long digit run through: the part after it has at most 3 digits, a version part too.
    assert_eq!(redact::scrub("took 1.123456789 s"), "took <…> s");
    assert_eq!(redact::scrub("version 12345.12345.12345"), "version <…>");
}

#[test]
fn rule_6_a_uuid_is_an_id() {
    let id = "0f8fad5b-d9cb-469f-a165-70867728950e";
    assert_eq!(redact::scrub(&format!("upload {id} parked")), "upload <id> parked");
    assert_eq!(redact::value("file_id", id), "<id>");
}

#[test]
fn the_word_rules_see_words_through_debug_punctuation() {
    // A label, a phrase and `custom:` inside brackets and quotes: the punctuation is written, the value is not.
    assert_eq!(redact::scrub("(code: 4821)"), "(code: <redacted>)");
    assert_eq!(redact::scrub("Some(\"pin: 1234\")"), "Some(\"pin: <redacted>\")");
    let phrase = "[\"abandon\",\"ability\",\"able\",\"about\",\"above\",\"absent\",\"absorb\",\"abstract\",\"absurd\",\"abuse\",\"access\",\"accident\"]";
    assert_eq!(redact::scrub(phrase), "[\"<phrase>\"]");
    assert_eq!(redact::scrub("(custom:Medical Records)"), "(custom:<name>");
}

#[test]
fn a_name_joined_with_an_ampersand_or_a_plus_is_one_masked_word() {
    for joined in ["Mark&Grant", "Will+Max"] {
        let text = redact::scrub(&format!("shared with {joined} today"));
        assert!(
            !text.contains("Mark") && !text.contains("Grant") && !text.contains("Will") && !text.contains("Max"),
            "{joined} reached the file: {text}"
        );
        assert!(text.contains("<…>"), "{joined} is masked: {text}");
    }
}

#[test]
fn a_percentage_is_still_a_number() {
    assert_eq!(redact::scrub("disk 50% full"), "disk 50% full");
}

#[test]
fn a_number_next_to_a_masked_word_is_masked() {
    let text = redact::scrub("room Annastraat 12");
    assert!(!text.contains("Annastraat") && !text.contains("12"), "{text}");
    assert!(!redact::scrub("room Annastraat 12 13").contains("13"));
    let text = redact::scrub("room 12 Annastraat");
    assert!(!text.contains("Annastraat") && !text.contains("12"), "{text}");
    assert_eq!(redact::scrub("HTTP 404"), "HTTP 404 Not Found: <body omitted>");
}

#[test]
fn a_grouped_phone_or_card_number_is_masked() {
    for number in ["06 1234 5678", "4111 1111 1111 1111", "+31 6 1234 5678"] {
        let text = redact::scrub(&format!("call {number} now"));
        let digits = text.bytes().filter(u8::is_ascii_digit).count();
        assert_eq!(digits, 0, "{number} left digits in the file: {text}");
        assert!(text.contains("<…>"), "{text}");
    }
}

#[test]
fn two_small_numbers_in_a_row_are_kept() {
    assert_eq!(redact::scrub("retry 2 of 5"), "retry 2 of 5");
    assert_eq!(redact::scrub("12 345 bytes"), "12 345 bytes");
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, b| {
        (hash ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[test]
fn vocabulary_os_is_pinned() {
    let file = include_str!("vocabulary_os.txt");
    assert_eq!(
        fnv1a(file.as_bytes()),
        0x9963_bb01_5124_9861,
        "vocabulary_os.txt changed: update the pin on purpose"
    );
    let phrases: Vec<&str> = file.lines().filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
    assert!(phrases.len() <= 150, "{} phrase lines", phrases.len());
    for phrase in phrases {
        assert!(
            phrase
                .chars()
                .all(|c| c.is_ascii_alphabetic() || matches!(c, ' ' | '\'' | ',' | '-' | '.' | '/')),
            "an OS phrase holds only letters, spaces and ' , - . / (a digit would let a number through): {phrase:?}"
        );
    }
}
