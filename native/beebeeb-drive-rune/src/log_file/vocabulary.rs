//! The final writer of every message and field value: a word reaches the file only when it is KNOWN to be safe, every
//! other word becomes `<…>` (fail closed). The word rules in [`super`] run first and replace what they recognise
//! with a marker (they make a line readable); whatever they miss is masked here, so a gap in them can make a line
//! less readable, never leak.
//!
//! A word is a run of letters, digits, `-`, `_` and any character that is not whitespace or punctuation; the
//! punctuation (ASCII, and `“”‘’«»`) between words is kept. A word is written as it is only when it is
//! 1. in the vocabulary: every word of the engine's and this crate's log calls (`vocabulary.txt`, kept in step by a
//!    test that scans the sources) and the OS error phrases and units (`vocabulary_os.txt`), case-insensitive;
//! 2. a number: at most 5 digits, at most one `.` part, an optional `-`, an optional unit (`ms`, `s`, `KB`, `MB`,
//!    `GB`), a version of three such parts (`0.9.0`; four parts, an IPv4 address, are masked), or a `0x` value of at
//!    most 8 hex digits (a Windows error code);
//! 3. one of the markers the first pass writes ([`Mark`], the HTTP status from the fixed table).
//!
//! Consecutive masked words, with the punctuation between them, become one `<…>`.

use std::collections::HashSet;
use std::sync::OnceLock;

/// The standard reason phrase of each HTTP status the `http` crate knows (RFC 9110 and its registered extensions). The
/// log writes THIS text for a status and never the text of the input: three designs that read the reason out of the input
/// each leaked a name (delta reviews 1 to 3 of #442). An unknown status has no reason.
const REASONS: &[(u16, &str)] = &[
    (100, "Continue"),
    (101, "Switching Protocols"),
    (102, "Processing"),
    (200, "OK"),
    (201, "Created"),
    (202, "Accepted"),
    (203, "Non-Authoritative Information"),
    (204, "No Content"),
    (205, "Reset Content"),
    (206, "Partial Content"),
    (207, "Multi-Status"),
    (208, "Already Reported"),
    (226, "IM Used"),
    (300, "Multiple Choices"),
    (301, "Moved Permanently"),
    (302, "Found"),
    (303, "See Other"),
    (304, "Not Modified"),
    (305, "Use Proxy"),
    (307, "Temporary Redirect"),
    (308, "Permanent Redirect"),
    (400, "Bad Request"),
    (401, "Unauthorized"),
    (402, "Payment Required"),
    (403, "Forbidden"),
    (404, "Not Found"),
    (405, "Method Not Allowed"),
    (406, "Not Acceptable"),
    (407, "Proxy Authentication Required"),
    (408, "Request Timeout"),
    (409, "Conflict"),
    (410, "Gone"),
    (411, "Length Required"),
    (412, "Precondition Failed"),
    (413, "Payload Too Large"),
    (414, "URI Too Long"),
    (415, "Unsupported Media Type"),
    (416, "Range Not Satisfiable"),
    (417, "Expectation Failed"),
    (418, "I'm a teapot"),
    (421, "Misdirected Request"),
    (422, "Unprocessable Entity"),
    (423, "Locked"),
    (424, "Failed Dependency"),
    (426, "Upgrade Required"),
    (428, "Precondition Required"),
    (429, "Too Many Requests"),
    (431, "Request Header Fields Too Large"),
    (451, "Unavailable For Legal Reasons"),
    (500, "Internal Server Error"),
    (501, "Not Implemented"),
    (502, "Bad Gateway"),
    (503, "Service Unavailable"),
    (504, "Gateway Timeout"),
    (505, "HTTP Version Not Supported"),
    (506, "Variant Also Negotiates"),
    (507, "Insufficient Storage"),
    (508, "Loop Detected"),
    (510, "Not Extended"),
    (511, "Network Authentication Required"),
];

/// The words of the log calls, one per line, sorted (`vocabulary_sync.rs` holds the scan that checks it).
pub(crate) const VOCABULARY: &str = include_str!("vocabulary.txt");
/// The OS error phrases (`std::io::Error`, Windows, Linux, macOS) and `io::ErrorKind` names the engine logs, and units.
const OS_PHRASES: &str = include_str!("vocabulary_os.txt");
pub(crate) const MASKED: &str = "<…>";
const UNITS: [&str; 5] = ["ms", "s", "kb", "mb", "gb"];

/// A marker the first pass writes in place of a word. While the text is put together each is one private-use
/// character (the input's private-use characters are blanked first), so the final writer can tell the module's own
/// markers from the input's text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Mark {
    Redacted,
    Name,
    Host,
    Phrase,
    Custom,
    Body,
    Url,
    Email,
    Path,
    Secret,
    Code,
    Id,
}

const MARK_TEXT: [&str; 12] = [
    "<redacted>",
    "<name>",
    "<host>",
    "<phrase>",
    "custom:<name>",
    "<body omitted>",
    "<url>",
    "<email>",
    "<path>",
    "<secret>",
    "<code>",
    "<id>",
];
const MARK_BASE: u32 = 0xE100;
/// `HTTP <code> <reason>` of the `REASONS` row at `STATUS_BASE + row`; `HTTP <code>` at `UNKNOWN_STATUS_BASE + code`.
const STATUS_BASE: u32 = 0xE200;
const UNKNOWN_STATUS_BASE: u32 = 0xE300;

impl Mark {
    pub(crate) fn ch(self) -> char {
        char::from_u32(MARK_BASE + self as u32).expect("a private-use character")
    }
}

/// The status line of `code` as one marker: its standard reason comes from the fixed table, never from the input.
pub(crate) fn status_mark(code: u16) -> char {
    let at = match REASONS.iter().position(|(known, _)| *known == code) {
        Some(row) => STATUS_BASE + row as u32,
        None => UNKNOWN_STATUS_BASE + u32::from(code),
    };
    char::from_u32(at).expect("a private-use character")
}

/// True for the characters the first pass uses as markers; the input's own are blanked before it runs.
pub(crate) fn is_private_use(c: char) -> bool {
    ('\u{E000}'..='\u{F8FF}').contains(&c)
}

fn marker_text(c: char) -> Option<String> {
    let n = c as u32;
    let row = n.wrapping_sub(STATUS_BASE) as usize;
    if let Some(text) = MARK_TEXT.get(n.wrapping_sub(MARK_BASE) as usize) {
        Some((*text).to_string())
    } else if let Some((code, reason)) = REASONS.get(row) {
        Some(format!("HTTP {code} {reason}"))
    } else if (UNKNOWN_STATUS_BASE..UNKNOWN_STATUS_BASE + 1000).contains(&n) {
        Some(format!("HTTP {:03}", n - UNKNOWN_STATUS_BASE))
    } else {
        None
    }
}

fn is_separator(c: char) -> bool {
    c.is_whitespace() || (c.is_ascii_punctuation() && c != '-' && c != '_') || "“”‘’«»".contains(c) || is_private_use(c)
}

/// The words of `text`, in order (see the module docs for what a word is).
pub(crate) fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(is_separator).filter(|w| !w.is_empty())
}

fn vocabulary() -> &'static HashSet<String> {
    static WORDS: OnceLock<HashSet<String>> = OnceLock::new();
    WORDS.get_or_init(|| {
        VOCABULARY
            .lines()
            .chain(OS_PHRASES.lines())
            .filter(|line| !line.starts_with('#'))
            .flat_map(words)
            .map(str::to_lowercase)
            .collect()
    })
}

fn is_number(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    if let Some(hex) = lower.strip_prefix("0x") {
        return (1..=8).contains(&hex.len()) && hex.bytes().all(|b| b.is_ascii_hexdigit());
    }
    let unsigned = lower.strip_prefix('-').unwrap_or(&lower);
    let digits_end = unsigned
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(unsigned.len());
    let (number, unit) = unsigned.split_at(digits_end);
    let mut parts = number.split('.');
    let whole = parts.next().unwrap_or("");
    let fraction = parts.next();
    (1..=5).contains(&whole.len())
        && whole.bytes().all(|b| b.is_ascii_digit())
        && fraction.is_none_or(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()))
        && parts.next().is_none()
        && (unit.is_empty() || UNITS.contains(&unit))
}

fn is_known(word: &str) -> bool {
    word.chars().all(|c| c == '-' || c == '_') || is_number(word) || vocabulary().contains(&word.to_lowercase())
}

/// Writes `text` (the first pass's output) with only known words, markers and punctuation; see the module docs.
pub(crate) fn allow_listed(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    // Punctuation and spaces after the last word, not yet written: dropped when they sit between two masked words.
    let mut held = String::new();
    let mut masked = false;
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if let Some(marker) = marker_text(c) {
            out.push_str(&held);
            out.push_str(&marker);
            held.clear();
            masked = false;
            rest = &rest[c.len_utf8()..];
            continue;
        }
        if is_separator(c) {
            held.push(if c.is_whitespace() || is_private_use(c) { ' ' } else { c });
            rest = &rest[c.len_utf8()..];
            continue;
        }
        let mut end = rest.find(is_separator).unwrap_or(rest.len());
        // `12.5`, `0.9.0`: a dot between digits belongs to the number.
        while rest[..end].bytes().last().is_some_and(|b| b.is_ascii_digit())
            && rest[end..].starts_with('.')
            && rest[end + 1..].bytes().next().is_some_and(|b| b.is_ascii_digit())
        {
            end = rest[end + 1..].find(is_separator).map_or(rest.len(), |at| end + 1 + at);
        }
        let word = &rest[..end];
        rest = &rest[end..];
        // `0.9.0` is a version; four parts (`10.0.5.161`) are an address, which says where a person is: masked.
        let known = is_known(word) || (word.matches('.').count() == 2 && word.split('.').all(is_number));
        if known || !masked {
            out.push_str(&held);
            out.push_str(if known { word } else { MASKED });
        }
        held.clear();
        masked = !known;
    }
    out.push_str(&held);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}
