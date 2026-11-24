//! What may reach the log file: the redaction rules, in one place, with no dependency.
//!
//! Three layers, all applied to every event before a byte is written:
//!
//! 1. **By field name** ([`denied_name`]): a field called `path`, `name`, `email`, `token`, `key`, `password`,
//!    `phrase`, `code`, `body`, `url` and the like is never written, whatever its value; the file shows
//!    `<redacted>`. A field that ends in `id`, `kind`, `count`, `reason` and the like is an opaque value by
//!    construction and goes to the next layers instead.
//! 2. **The word rules** ([`scrub`]'s first pass) replace what they recognise with a marker, to keep a line readable:
//!    a URL, an email address, a path (from a root to the last later word with a `/` or `\`, because a name can hold
//!    spaces), a file name, a UUID or long hex id (`<id>`), a token, a 6 to 8 digit code, a run of 12 plain lowercase
//!    words (a master key), the value after a credential label, a text between “…”, ‘…’, «…» or backticks (the
//!    engine quotes the names it prints: `<name>`), `custom:<name>` (a backup folder the person picked) and the host
//!    of the lock-held sentence. A server body (JSON or HTML) is replaced whole. From `HTTP <3 digits>` on, nothing
//!    of the text is read: the log writes `HTTP <status>` with the status's standard reason from a fixed table
//!    (`REASONS`), then `<body omitted>`. Control characters are blanked, so one event is one line.
//! 3. **The allow-list** ([`vocabulary`]) writes the result: a word is written as it is only when it is a word of the
//!    engine's log calls or the OS's error texts, a number or a marker; every other word becomes `<…>`. This is the
//!    layer that decides (fail closed): a person's folder or file name can take any shape, so a word the rules above
//!    do not recognise is masked, and a gap in them makes a line less readable, never leaks. A name is written only
//!    when it is identical to a word of the engine's own log texts (the residue: generic words such as "Photos").
//!
//! Decision (lead, 2026-11-15): the allow-list replaced a deny-list because five deny-list rounds of PR #442 each
//! leaked a new shape.

#[path = "vocabulary.rs"]
pub(crate) mod vocabulary;

use vocabulary::{Mark, allow_listed, is_private_use, status_mark};

/// Words of a field name (split on `_` and `.`) that make the whole field unwritable.
const DENIED_WORDS: &[&str] = &[
    "path",
    "paths",
    "dir",
    "dirs",
    "directory",
    "root",
    "src",
    "dest",
    "dst",
    "pass",
    "content",
    "name",
    "names",
    "email",
    "mail",
    "token",
    "key",
    "keys",
    "password",
    "passphrase",
    "phrase",
    "mnemonic",
    "secret",
    "code",
    "otp",
    "pin",
    "body",
    "url",
    "uri",
    "text",
    "fallback",
    "user",
    "username",
    "payload",
    "response",
    "title",
    "label",
    "host",
    "addr",
    "address",
    "cookie",
    "session",
    "signature",
    "nonce",
    "salt",
    "auth",
    "authorization",
    "bearer",
    "credential",
    "credentials",
    "jwt",
    "apikey",
    "cred",
    "private",
    "seed",
];

/// Words that name a credential: a field with one of them is denied even when it ends in `id` or `count`
/// (`session_id`, `key_id`), because the value of such a field can be the credential itself.
const CREDENTIAL_WORDS: &[&str] = &[
    "key",
    "keys",
    "token",
    "secret",
    "session",
    "cookie",
    "nonce",
    "salt",
    "pass",
    "password",
    "passphrase",
    "phrase",
    "mnemonic",
    "signature",
    "auth",
    "authorization",
    "bearer",
    "credential",
    "credentials",
    "jwt",
    "apikey",
    "seed",
    "otp",
    "pin",
    "code",
];

/// A word in a message that is followed by its credential, whether or not a colon comes between.
const ALWAYS_LABELS: &[&str] = &["bearer", "authorization", "basic"];

/// A last word that marks an opaque or numeric value: such a field is not denied by an earlier word
/// (`root_id`, `file_kind`, `key_count`), but its value still goes through [`scrub`].
const OPAQUE_LAST_WORDS: &[&str] = &[
    "id",
    "ids",
    "kind",
    "count",
    "len",
    "bytes",
    "secs",
    "ms",
    "version",
    "seq",
    "status",
    "category",
    "reason",
    "size",
    "offset",
    "length",
    "attempt",
    "generation",
    "step",
    "outcome",
];

/// Parts of a field name that deny it even inside a longer word (`syncpath`, `filename`).
const DENIED_PARTS: &[&str] = &[
    "path",
    "email",
    "token",
    "passw",
    "phrase",
    "mnemonic",
    "secret",
    "filename",
    "dirname",
    "auth",
    "bearer",
    "credential",
    "jwt",
    "apikey",
    "cookie",
];

/// File names the engine's own fixed messages print: not a person's name.
const KNOWN_NAMES: &[&str] = &["beebeeb-sync.lock", "desktop.toml", "state.db"];

pub const REDACTED: &str = "<redacted>";
const VALUE_MAX: usize = 240;
const MESSAGE_MAX: usize = 400;

/// True when a field of this name is never written.
/// True when a field of this name is never written.
pub fn denied_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if DENIED_PARTS.iter().any(|part| lower.contains(part)) {
        return true;
    }
    let words: Vec<&str> = lower.split(['_', '.']).filter(|w| !w.is_empty()).collect();
    if words.iter().any(|word| CREDENTIAL_WORDS.contains(word)) {
        return true;
    }
    if let Some(last) = words.last()
        && words.len() > 1
        && OPAQUE_LAST_WORDS.contains(last)
    {
        return false;
    }
    words.iter().any(|word| DENIED_WORDS.contains(word))
}

/// True when a NUMBER field of this name is never written: only a credential word or part denies it (a number holds
/// no name, path or body, so `content_unknown = 2` is a count and stays).
pub fn denied_numeric_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    DENIED_PARTS.iter().any(|part| lower.contains(part))
        || lower.split(['_', '.']).any(|word| CREDENTIAL_WORDS.contains(&word))
}

/// The message of an event, scrubbed and capped.
pub fn message(raw: &str) -> String {
    cap(scrub(raw), MESSAGE_MAX)
}

/// A field value, scrubbed and capped. A UUID becomes `<id>` in every field, `file_id` too: it is not a name, but it
/// correlates.
pub fn value(_name: &str, raw: &str) -> String {
    cap(scrub(raw), VALUE_MAX)
}

/// A number: kept, except a 6 to 8 digit value of a field that is not a plain metric (the shape of a one-time code).
/// A number: kept, except a 6 to 8 digit value of a field that is not a plain metric (the shape of a one-time code).
pub fn number(name: &str, shown: String) -> String {
    let lower = name.to_ascii_lowercase();
    let last = lower.rsplit(['_', '.']).next().unwrap_or("");
    let metric = [
        "count",
        "len",
        "bytes",
        "secs",
        "ms",
        "version",
        "seq",
        "size",
        "offset",
        "length",
        "attempt",
        "generation",
        "step",
        "id",
        "ids",
    ];
    if (6..=8).contains(&shown.len()) && shown.bytes().all(|b| b.is_ascii_digit()) && !metric.contains(&last) {
        return "<code>".to_string();
    }
    shown
}

fn cap(mut text: String, max: usize) -> String {
    if text.chars().count() > max {
        text = text.chars().take(max).collect::<String>() + "...";
    }
    text
}

/// What a text may write: the word rules mark what they recognise, then only known words, numbers and markers are
/// written; see the module docs.
pub fn scrub(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() || is_private_use(c) { ' ' } else { c })
        .collect();
    allow_listed(&first_pass(&cleaned))
}

fn first_pass(raw: &str) -> String {
    // `Custom("Medical Records")`: the Debug shape of a backup folder the person picked.
    if let Some(at) = raw.find("Custom(\"") {
        return format!("{} {}", first_pass(&raw[..at]), Mark::Custom.ch());
    }
    // `HTTP 502 Bad Gateway: <the first 300 characters of the answer>`: the status stays, the answer never does.
    if let Some((before, code)) = http_status_prefix(raw) {
        // `Records (HTTP 404`: no gap after an opening bracket or quote.
        let gap = if before.ends_with(['(', '[', '{', '"', '\'', '“', '‘', '«']) {
            ""
        } else {
            " "
        };
        return format!("{}{gap}{}: {}", first_pass(before), status_mark(code), Mark::Body.ch());
    }
    if looks_like_body(raw) {
        return Mark::Body.ch().to_string();
    }
    let marked = mark_quoted(&mark_host(raw));
    let tokens = tokens(&marked);
    let mut out = String::with_capacity(marked.len());
    let mut redact_next = false;
    let mut i = 0;
    while i < tokens.len() {
        let (token, gap) = tokens[i];
        let at = i;
        i += 1;
        if is_splitter(token) {
            put(&mut out, gap, token);
            continue;
        }
        let (run, run_end) = phrase_run(&tokens, at);
        if run >= 12 {
            put(&mut out, gap, Mark::Phrase.ch());
            i = run_end;
            redact_next = false;
            continue;
        }
        if token == Mark::Name.ch().to_string() {
            put(
                &mut out,
                gap,
                if redact_next { Mark::Redacted } else { Mark::Name }.ch(),
            );
            redact_next = false;
            continue;
        }
        if token == Mark::Host.ch().to_string() {
            put(&mut out, gap, token);
            continue;
        }
        if redact_next {
            // `Authorization: Bearer x`: the scheme word is shown, the credential after it is not.
            if ALWAYS_LABELS.contains(&token.to_ascii_lowercase().as_str()) {
                put(&mut out, gap, token);
            } else {
                redact_next = false;
                put(&mut out, gap, Mark::Redacted.ch());
                // A redacted value that starts a path: the whole path is the value.
                if absolute_start(token) {
                    i = path_end(&tokens, at) + 1;
                }
            }
            continue;
        }
        // `category=custom:<name>` inside a text: the same, after the `=`.
        if let Some((key, value)) = token.split_once('=')
            && value.get(..7).is_some_and(|head| head.eq_ignore_ascii_case("custom:"))
        {
            put(&mut out, gap, format!("{}={}", shown_key(key), Mark::Custom.ch()));
            break;
        }
        // `custom:<name>`: a backup folder the person picked; its name can hold spaces, so the rest goes too.
        if token.get(..7).is_some_and(|head| head.eq_ignore_ascii_case("custom:")) {
            put(&mut out, gap, Mark::Custom.ch());
            break;
        }
        // `password: x`, `Authorization: Bearer x`: the label decides, and the word after it is its value.
        let label = token.trim_end_matches(':').to_ascii_lowercase();
        if token.ends_with(':') && !token.contains('=') && denied_name(&label)
            || ALWAYS_LABELS.contains(&label.as_str())
        {
            redact_next = true;
            put(&mut out, gap, token);
            continue;
        }
        // A path that starts at a root can hold spaces (`C:\Users\Anna Smith\x`): it runs to its last word.
        let core = edges(token).1;
        let key_of_path = core
            .split_once('=')
            .filter(|(_, value)| absolute_start(value))
            .map(|(key, _)| key);
        if absolute_start(token) || key_of_path.is_some() {
            let end = path_end(&tokens, at);
            let key = key_of_path
                .map(|key| format!("{}=", shown_key(key)))
                .unwrap_or_default();
            put(
                &mut out,
                gap,
                format!("{key}{}{}", Mark::Path.ch(), edges(tokens[end].0).2),
            );
            i = end + 1;
            continue;
        }
        put(&mut out, gap, scrub_token(token));
    }
    out
}

/// Characters that split words for the word rules and are written as they are: a Debug value has them between words.
const SPLITTERS: &str = "\"'()[]{}<>,;";

fn is_splitter(token: &str) -> bool {
    token.len() == 1 && SPLITTERS.contains(token)
}

/// The words of `text` for the word rules, each with whether whitespace came before it; a splitter is a word of its own.
fn tokens(text: &str) -> Vec<(&str, bool)> {
    let mut out = Vec::new();
    let (mut start, mut gap) = (None, false);
    for (at, c) in text.char_indices() {
        if !c.is_whitespace() && !SPLITTERS.contains(c) {
            start.get_or_insert(at);
            continue;
        }
        if let Some(from) = start.take() {
            out.push((&text[from..at], gap));
            gap = false;
        }
        if c.is_whitespace() {
            gap = true;
        } else {
            out.push((&text[at..at + 1], gap));
            gap = false;
        }
    }
    out.extend(start.map(|from| (&text[from..], gap)));
    out
}

fn put(out: &mut String, gap: bool, piece: impl std::fmt::Display) {
    use std::fmt::Write as _;
    let space = if gap && !out.is_empty() { " " } else { "" };
    let _ = write!(out, "{space}{piece}");
}

/// The plain words of a master key from `tokens[at]` on (the splitters between them do not count): how many, and
/// the index after the last.
fn phrase_run(tokens: &[(&str, bool)], at: usize) -> (usize, usize) {
    let (mut run, mut end) = (0, at);
    for (j, (token, _)) in tokens.iter().enumerate().skip(at) {
        if is_plain_word(token) {
            run += 1;
            end = j + 1;
        } else if !is_splitter(token) {
            break;
        }
    }
    (run, end)
}

/// The last token of a path that starts at `tokens[at]`. A name in it can hold spaces, so the path runs past the last
/// later token with a `/` or `\` in it, to the first token after that which ends in `:` (`… Records 2026: Access is
/// denied`: the OS error after it stays readable). With no such `:`, the rest of the text is the path's (the leaf
/// `Medical Records 2026 exists but …` cannot be told apart from the words after it).
fn path_end(tokens: &[(&str, bool)], at: usize) -> usize {
    let last = (at..tokens.len())
        .rev()
        .find(|&j| tokens[j].0.contains(['/', '\\']))
        .unwrap_or(at);
    (last..tokens.len())
        .find(|&j| tokens[j].0.ends_with(':'))
        .unwrap_or(tokens.len() - 1)
}

/// A key before `=`: `<redacted>` when its name is denied, else a word like any other.
fn shown_key(key: &str) -> String {
    if denied_name(key) {
        Mark::Redacted.ch().to_string()
    } else {
        scrub_token(key)
    }
}

/// Finds the first `HTTP <3 ASCII digits>` (the start of `DesktopApiError::HttpStatus`'s text, which ends in the
/// first 300 characters of the server's answer): the text before it, and the status code. Nothing after the three
/// digits is read, so nothing of it can be written: the only input bytes the status writes are those digits, and its
/// reason comes from the fixed table (`vocabulary::REASONS`). `None` when there is no such prefix.
fn http_status_prefix(raw: &str) -> Option<(&str, u16)> {
    for (at, _) in raw.match_indices("HTTP") {
        let mut rest = &raw[at + 4..];
        // `HTTP/1.1 502 ...`: the protocol version is skipped (no byte of it is written either).
        if let Some(version) = rest.strip_prefix('/') {
            rest = &version[version.find(' ').unwrap_or(version.len())..];
        }
        let Some(rest) = rest.strip_prefix(' ') else { continue };
        let Some(digits) = rest.get(..3) else { continue };
        if !digits.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(code) = digits.parse::<u16>() else { continue };
        return Some((&raw[..at], code));
    }
    None
}

/// The host of the lock-held sentence (`... (pid 12 on Annas-PC, since ...)`): often a person's first name.
fn mark_host(raw: &str) -> String {
    if let Some(pid) = raw.find("(pid ")
        && let Some(on) = raw[pid..].find(" on ")
        && let Some(since) = raw[pid + on..].find(", since")
    {
        let from = pid + on + 4;
        return format!("{}{}{}", &raw[..from], Mark::Host.ch(), &raw[pid + on + since..]);
    }
    raw.to_string()
}

/// Text between “…”, ‘…’, «…» or backticks is a name (the engine quotes the names it prints): it becomes one mark. The
/// name runs from the first opening quote of ANY style to the last closing quote of ANY style: a name can hold a
/// closing quote of its own (`“Anna “Work” Files”`), `’` is also the apostrophe, and a quote of another style can sit
/// inside a name. Two quoted names become one mark, an over-redaction the module accepts. An opening quote with no
/// closing one after it marks the rest of the text.
fn mark_quoted(raw: &str) -> String {
    const OPENING: [char; 4] = ['\u{201c}', '\u{2018}', '\u{ab}', '`'];
    const CLOSING: [char; 4] = ['\u{201d}', '\u{2019}', '\u{bb}', '`'];
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < chars.len() {
        let Some(style) = OPENING.iter().position(|c| *c == chars[i]) else {
            out.push(chars[i]);
            i += 1;
            continue;
        };
        out.push(' ');
        out.push(Mark::Name.ch());
        out.push(' ');
        let rest = &chars[i + 1..];
        // Fails closed: the name runs to the last closing quote of ANY style only when it is well formed. With no closing
        // quote of the opener's OWN style after it (`’` alone is an apostrophe, not the end of `“…`), with a later
        // opener (`“`, `‘`, `«`) that has no closing quote of its own style after it, or with an odd number of backticks
        // from here on, the rest of the text is the name.
        let own_closer = rest.contains(&CLOSING[style]);
        let later_openers_closed = rest.iter().enumerate().all(|(at, c)| {
            OPENING[..3]
                .iter()
                .position(|o| o == c)
                .is_none_or(|later| rest[at + 1..].contains(&CLOSING[later]))
        });
        let backticks_even = chars[i..].iter().filter(|c| **c == '`').count() % 2 == 0;
        i = if own_closer && later_openers_closed && backticks_even {
            match rest.iter().rposition(|c| CLOSING.contains(c)) {
                Some(last) => i + 1 + last + 1,
                None => chars.len(),
            }
        } else {
            chars.len()
        };
    }
    out
}

fn absolute_start(token: &str) -> bool {
    let token = token.trim_start_matches(|c| "\"'([{<".contains(c));
    let b = token.as_bytes();
    // A path glued to the word before it (`at:/home/anna/...`, `at:C:\Users\...`, `at:~/...`) starts a path too.
    let glued = (token.contains(":/") && !token.contains("://")) || token.contains(":\\") || token.contains(":~");
    glued
        || token.starts_with('/')
        || token.starts_with('\\')
        || token.starts_with('~')
        || token.starts_with('%')
        || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
}

fn is_plain_word(token: &str) -> bool {
    (3..=8).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_lowercase())
}

/// JSON or HTML: what a server answers with. Never written, whatever it holds.
fn looks_like_body(raw: &str) -> bool {
    let trimmed = raw.trim_start();
    raw.contains("{\"")
        || raw.contains("\":")
        || raw.contains("\\\":")
        || trimmed.starts_with("<!")
        || trimmed.to_ascii_lowercase().starts_with("<html")
        || raw.to_ascii_lowercase().contains("</html")
}

/// A word's leading punctuation, its core and its trailing punctuation.
fn edges(token: &str) -> (&str, &str, &str) {
    let start = token.find(|c: char| !"\"'([{<".contains(c)).unwrap_or(token.len());
    let end = token
        .rfind(|c: char| !"\"')]}>,;.:!?".contains(c))
        .map_or(start, |i| i + token[i..].chars().next().map_or(1, char::len_utf8));
    if start >= end {
        return (token, "", "");
    }
    (&token[..start], &token[start..end], &token[end..])
}

fn scrub_token(token: &str) -> String {
    let (prefix, core, suffix) = edges(token);
    if core.is_empty() {
        return token.to_string();
    }
    let marked = |mark: Mark| format!("{prefix}{}{suffix}", mark.ch());
    // An email address is one word, even with an `=` in it (`anna=smith@example.eu`): never split at the `=`.
    if core.find('@').is_some_and(|at| core[at..].contains('.')) {
        return marked(Mark::Email);
    }
    // Trailing `=` padding (base64) belongs to the token, and the part before an `=` is a word like any other: a path,
    // an email or a file name there is replaced, not written.
    let unpadded = core.trim_end_matches('=');
    if unpadded.len() < core.len()
        && !unpadded.contains('=')
        && let Some(mark) = classify(unpadded)
    {
        return marked(mark);
    }
    if let Some((key, val)) = core.split_once('=')
        && !key.is_empty()
        && !key.contains("://")
    {
        let shown = if denied_name(key) {
            Mark::Redacted.ch().to_string()
        } else {
            scrub_token(val)
        };
        let key = classify(key).map_or(key.to_string(), |mark| mark.ch().to_string());
        return format!("{prefix}{key}={shown}{suffix}");
    }
    classify(core).map_or_else(|| token.to_string(), marked)
}

fn classify(core: &str) -> Option<Mark> {
    if core.contains("://") || core.starts_with("mailto:") {
        return Some(Mark::Url);
    }
    if let Some(at) = core.find('@')
        && core[at..].contains('.')
    {
        return Some(Mark::Email);
    }
    let bytes = core.as_bytes();
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    let env = core.len() > 2 && core.starts_with('%') && core.ends_with('%');
    if core.contains('/') || core.contains('\\') || drive || env || (core.starts_with('~') && core.len() > 1) {
        return Some(Mark::Path);
    }
    if is_uuid(core) || is_long_hex(core) {
        return Some(Mark::Id);
    }
    if is_jwt(core) {
        return Some(Mark::Secret);
    }
    if is_file_name(core) {
        return Some(Mark::Name);
    }
    if is_secret(core) {
        return Some(Mark::Secret);
    }
    if (6..=8).contains(&core.len()) && bytes.iter().all(u8::is_ascii_digit) {
        return Some(Mark::Code);
    }
    None
}

fn is_uuid(s: &str) -> bool {
    let groups: Vec<&str> = s.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(g, n)| g.len() == n && g.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn is_file_name(core: &str) -> bool {
    let Some(dot) = core.rfind('.') else { return false };
    let (stem, ext) = (&core[..dot], &core[dot + 1..]);
    if stem.is_empty() || ext.is_empty() || ext.len() > 8 {
        return false;
    }
    if KNOWN_NAMES.contains(&core.trim_start_matches('.')) {
        return false;
    }
    ext.bytes().all(|b| b.is_ascii_alphanumeric()) && ext.bytes().any(|b| b.is_ascii_alphabetic())
}

fn is_long_hex(core: &str) -> bool {
    !core.starts_with("0x") && core.len() >= 16 && core.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A long run of letters, digits and `+_-=` with both letters and digits in it: the shape of a token.
fn is_secret(core: &str) -> bool {
    core.len() >= 20
        && core.bytes().all(|b| b.is_ascii_alphanumeric() || b"+_-=".contains(&b))
        && core.bytes().any(|b| b.is_ascii_digit())
        && core.bytes().any(|b| b.is_ascii_alphabetic())
}

/// Three or more dot-separated base64url parts: the shape of a JWT.
fn is_jwt(core: &str) -> bool {
    core.len() >= 20
        && core.matches('.').count() >= 2
        && core.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-=".contains(&b))
        && core.split('.').all(|part| !part.is_empty())
}
