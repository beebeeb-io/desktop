//! What may reach the log file: the redaction rules, in one place, with no dependency.
//!
//! Two layers, both applied to every event before a byte is written:
//!
//! 1. **By field name** ([`denied_name`]): a field called `path`, `name`, `email`, `token`, `key`, `password`,
//!    `phrase`, `code`, `body`, `url` and the like is never written, whatever its value; the file shows
//!    `<redacted>`. A field that ends in `id`, `kind`, `count`, `reason` and the like is an opaque value by
//!    construction and goes to the shape check instead.
//! 2. **By shape** ([`scrub`]): every remaining string (the message too) is read word by word (quotes, commas and
//!    brackets count as spaces and are not written); a word that is a
//!    URL, an email address, a path, a file name, a long token or hex string, a 6 to 8 digit code, or part of a run of
//!    12 plain lowercase words (the shape of a recovery phrase) is replaced by a marker. A value that looks like a
//!    server body (JSON or HTML) is replaced whole. Control characters are blanked, so one event is one line.
//!
//! Over-redaction is the accepted failure: a harmless word that looks like a path becomes `<path>`. A name that has
//! no shape (a folder called "Holiday" inside a free-text message) cannot be found by shape; no engine call formats
//! one into its message (audited in the PR that added this file), and a new one is a review finding.

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
    "name",
    "names",
    "email",
    "mail",
    "token",
    "key",
    "keys",
    "pass",
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
    "content",
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

/// The message of an event, scrubbed and capped.
pub fn message(raw: &str) -> String {
    cap(scrub(raw), MESSAGE_MAX)
}

/// A field value, scrubbed and capped. A UUID-shaped word survives only in a field whose name ends in `id`/`ids`
/// (the engine's opaque `file_id`, `op_id`): anywhere else it could be a credential.
pub fn value(name: &str, raw: &str) -> String {
    cap(scrub_with(raw, is_id_name(name)), VALUE_MAX)
}

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

fn is_id_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(lower.rsplit(['_', '.']).next(), Some("id" | "ids"))
}

fn cap(mut text: String, max: usize) -> String {
    if text.chars().count() > max {
        text = text.chars().take(max).collect::<String>() + "...";
    }
    text
}

/// Replaces every word of `raw` that has a forbidden shape; see the module docs.
pub fn scrub(raw: &str) -> String {
    scrub_with(raw, false)
}

fn scrub_with(raw: &str, allow_uuid: bool) -> String {
    if looks_like_body(raw) {
        return "<body omitted>".to_string();
    }
    // Quotes, commas and brackets are word separators (a Debug value has them between every word); control characters
    // too, so one event is one line.
    let blanked: String = raw
        .chars()
        .map(|c| {
            if c.is_control() || "\"'()[]{}<>,;".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let tokens: Vec<&str> = blanked.split_whitespace().collect();
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut redact_next = false;
    let mut i = 0;
    while i < tokens.len() {
        let run = tokens[i..].iter().take_while(|t| is_plain_word(t)).count();
        if run >= 12 {
            out.push("<phrase>".to_string());
            i += run;
            redact_next = false;
            continue;
        }
        let token = tokens[i];
        i += 1;
        if redact_next {
            // `Authorization: Bearer x`: the scheme word is shown, the credential after it is not.
            if ALWAYS_LABELS.contains(&token.to_ascii_lowercase().as_str()) {
                out.push(token.to_string());
            } else {
                redact_next = false;
                out.push(REDACTED.to_string());
            }
            continue;
        }
        // `password: x`, `Authorization: Bearer x`: the label decides, and the word after it is its value.
        let label = token.trim_end_matches(':').to_ascii_lowercase();
        if token.ends_with(':') && !token.contains('=') && denied_name(&label)
            || ALWAYS_LABELS.contains(&label.as_str())
        {
            redact_next = true;
            out.push(token.to_string());
            continue;
        }
        let shown = scrub_token(token, allow_uuid);
        // A path that starts at a root can hold spaces (`C:\Users\Anna Smith\x`): nothing after it is trusted.
        if shown == "<path>" && absolute_start(token) {
            out.push(shown);
            break;
        }
        out.push(shown);
    }
    out.join(" ")
}

fn absolute_start(token: &str) -> bool {
    let b = token.as_bytes();
    token.starts_with('/')
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

fn scrub_token(token: &str, allow_uuid: bool) -> String {
    let start = token.find(|c: char| !"\"'([{<".contains(c)).unwrap_or(token.len());
    let end = token
        .rfind(|c: char| !"\"')]}>,;.:!?".contains(c))
        .map_or(start, |i| i + token[i..].chars().next().map_or(1, char::len_utf8));
    if start >= end {
        return token.to_string();
    }
    let (prefix, core, suffix) = (&token[..start], &token[start..end], &token[end..]);
    // `name=value`: the name decides first, then the value is read as a token of its own.
    if let Some((key, val)) = core.split_once('=')
        && !key.is_empty()
        && !key.contains("://")
    {
        let shown = if denied_name(key) {
            REDACTED.to_string()
        } else {
            scrub_token(val, allow_uuid)
        };
        return format!("{prefix}{key}={shown}{suffix}");
    }
    match classify(core, allow_uuid) {
        Some(marker) => format!("{prefix}{marker}{suffix}"),
        None => token.to_string(),
    }
}

fn classify(core: &str, allow_uuid: bool) -> Option<&'static str> {
    if core.contains("://") || core.starts_with("mailto:") {
        return Some("<url>");
    }
    if let Some(at) = core.find('@')
        && core[at..].contains('.')
    {
        return Some("<email>");
    }
    let bytes = core.as_bytes();
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    let env = core.len() > 2 && core.starts_with('%') && core.ends_with('%');
    if core.contains('/') || core.contains('\\') || drive || env || (core.starts_with('~') && core.len() > 1) {
        return Some("<path>");
    }
    if is_uuid(core) {
        return if allow_uuid { None } else { Some("<secret>") };
    }
    if is_jwt(core) {
        return Some("<secret>");
    }
    if is_file_name(core) {
        return Some("<name>");
    }
    if is_secret(core) {
        return Some("<secret>");
    }
    if (6..=8).contains(&core.len()) && bytes.iter().all(u8::is_ascii_digit) {
        return Some("<code>");
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
    if stem.is_empty() || ext.is_empty() || ext.len() > 5 {
        return false;
    }
    if KNOWN_NAMES.contains(&core.trim_start_matches('.')) {
        return false;
    }
    ext.bytes().all(|b| b.is_ascii_alphanumeric()) && ext.bytes().any(|b| b.is_ascii_alphabetic())
}

fn is_secret(core: &str) -> bool {
    if core.starts_with("0x") {
        return false;
    }
    let hex = core.len() >= 16 && core.bytes().all(|b| b.is_ascii_hexdigit());
    let tokenish = core.len() >= 20
        && core.bytes().all(|b| b.is_ascii_alphanumeric() || b"+_-=".contains(&b))
        && core.bytes().any(|b| b.is_ascii_digit())
        && core.bytes().any(|b| b.is_ascii_alphabetic());
    hex || tokenish
}

/// Three or more dot-separated base64url parts: the shape of a JWT.
fn is_jwt(core: &str) -> bool {
    core.len() >= 20
        && core.matches('.').count() >= 2
        && core.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-=".contains(&b))
        && core.split('.').all(|part| !part.is_empty())
}
