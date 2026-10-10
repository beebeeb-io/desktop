//! Structurally safe diagnostics export (task 1685).
//!
//! The Account page promises that the support bundle contains no secrets and
//! no plaintext names. A free-text filter that only strips token-shaped words
//! cannot keep that promise: the engine's error strings embed local paths
//! (`write {dest}: {e}`, `staged upload payload is missing: {path}`, ffmpeg
//! stderr naming its input) and file or folder names.
//!
//! The export therefore carries, in order of trust:
//!
//! 1. a typed [`DiagnosticErrorCode`] (a closed enum: it cannot hold a name),
//! 2. labels for queue groups, filtered through a fixed allow-list,
//! 3. the last error message, kept only as an ALLOW-LIST filter: every known
//!    file/folder name from the state DB is replaced first (multi-word names
//!    included, longest first), every path-like token becomes `[path]`, and
//!    every remaining word that is not a standard error word or a number
//!    becomes `[name]`. A name we have never seen therefore cannot survive
//!    unless it is itself an ordinary error word (a name like `letter` or
//!    `data` that the state DB does not know) or a bare number. UUIDs and the
//!    `scheme://host:port` of a URL are kept on purpose.
//!
//! [`RedactedError::redactions`] counts the placeholders written, so a bundle
//! reader can tell a clean message from a heavily filtered one.

use serde::Serialize;

/// Private-use marker written over a known name while the string is still being
/// scanned. Stripped from input first so a caller cannot forge one.
const NAME_MARK: char = '\u{E000}';
const MAX_INPUT_CHARS: usize = 2000;
const MAX_OUTPUT_TOKENS: usize = 60;

/// Closed set of failure classes the support bundle may report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticErrorCode {
    Auth,
    Quota,
    Permission,
    Locked,
    StaleVersion,
    PayloadMissing,
    LocalWriteFailed,
    PathRejected,
    Timeout,
    Network,
    ServerError,
    Crypto,
    Thumbnail,
    EngineStopping,
    WorkerNotAttached,
    Other,
}

/// Classify a raw engine error into a [`DiagnosticErrorCode`]. Reads the raw
/// message (names and all) but returns a value that cannot contain any of it.
pub fn classify_error_code(error: &str) -> DiagnosticErrorCode {
    use crate::engine_bridge::{classify_operation_error, OperationFailureClass};
    match classify_operation_error(error) {
        OperationFailureClass::Auth => return DiagnosticErrorCode::Auth,
        OperationFailureClass::Quota => return DiagnosticErrorCode::Quota,
        OperationFailureClass::Permission => return DiagnosticErrorCode::Permission,
        OperationFailureClass::Locked => return DiagnosticErrorCode::Locked,
        OperationFailureClass::Retryable => {}
    }
    // Same rule as `classify_operation_error`: never classify on the URL tail.
    let message = error.split(" for url (").next().unwrap_or(error);
    let m = message.to_ascii_lowercase();
    let has = |needle: &str| m.contains(needle);
    if has("engine is stopping") {
        DiagnosticErrorCode::EngineStopping
    } else if has("not yet attached") {
        DiagnosticErrorCode::WorkerNotAttached
    } else if has("stale") || has("base version") {
        DiagnosticErrorCode::StaleVersion
    } else if has("staged upload payload is missing") || has("staged upload ended") {
        DiagnosticErrorCode::PayloadMissing
    } else if has("must stay under")
        || has("unsafe")
        || has("not within an allowed root")
        || has("rel_path must")
        || has("non-normal path")
        || has("no parent directory")
    {
        DiagnosticErrorCode::PathRejected
    } else if m.starts_with("write ")
        || m.starts_with("rename ")
        || has("create dest dir")
        || has("hydrate destination")
    {
        DiagnosticErrorCode::LocalWriteFailed
    } else if has("thumbnail") || has("ffmpeg") {
        DiagnosticErrorCode::Thumbnail
    } else if has("encrypt") || has("decrypt") {
        DiagnosticErrorCode::Crypto
    } else if has("timed out") || has("timeout") {
        DiagnosticErrorCode::Timeout
    } else if has("server error")
        || has("internal server error")
        || has("bad gateway")
        || has("service unavailable")
        || has("gateway timeout")
    {
        DiagnosticErrorCode::ServerError
    } else if has("dns error") || has("connect") || has("connection") || has("error sending request") || has("network")
    {
        DiagnosticErrorCode::Network
    } else {
        DiagnosticErrorCode::Other
    }
}

/// Queue-group labels that may appear as keys in an export. Anything else is
/// folded into `other` so a stray DB value can never become a key.
pub const QUEUE_KIND_LABELS: &[&str] = &[
    "hydrate_file",
    "pin_tree",
    "upload_version",
    "upload_file",
    "create_folder",
    "rename_file",
    "move_file",
    "trash_file",
    "restore_file",
    "restore_version",
];
pub const PAUSE_REASON_LABELS: &[&str] = &["auth", "quota", "permission", "locked", "key_replaced"];

/// `label` if it is one of `allowed`, otherwise `other`.
pub fn allowed_label(label: &str, allowed: &[&str]) -> String {
    if allowed.contains(&label) {
        label.to_string()
    } else {
        "other".to_string()
    }
}

/// Plaintext names the local daemon knows about. Built from the state DB
/// (file paths, queued targets, activity rows) plus the sync-root path.
#[derive(Debug, Default, Clone)]
pub struct KnownNames {
    names: Vec<String>,
}

impl KnownNames {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a single name verbatim (also usable for a whole relative path).
    pub fn add_name(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() || name == "." || name == ".." {
            return;
        }
        self.names.push(name.to_ascii_lowercase());
    }

    /// Add a path as a whole plus each of its components, so both the full
    /// `Tax 2025/aangifte.pdf` and the bare `Tax 2025` are known.
    pub fn add_path(&mut self, path: &str) {
        self.add_name(path);
        for component in path.split(['/', '\\']) {
            self.add_name(component);
        }
    }

    /// Sort longest first (so `Tax 2025` is replaced before `Tax`) and dedupe.
    pub fn finish(mut self) -> Self {
        self.names.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        self.names.dedup();
        self
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedError {
    pub text: String,
    /// Number of placeholders (`[path]`, `[name]`, `[redacted]`) written.
    pub redactions: u32,
}

/// How much of the pipeline to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Credential tokens and oversize words only. Used when persisting.
    SecretsOnly,
    /// Everything: names, paths, and the standard-word allow-list.
    Export,
}

/// Secrets-only pass, for error text persisted locally in the state DB.
pub fn redact_secrets_only(error: &str) -> String {
    redact(error, &KnownNames::default(), Mode::SecretsOnly).text
}

/// Full export pass.
pub fn redact_for_export(error: &str, names: &KnownNames) -> RedactedError {
    redact(error, names, Mode::Export)
}

fn redact(error: &str, names: &KnownNames, mode: Mode) -> RedactedError {
    let mut redactions = 0u32;
    let bounded: String = error
        .chars()
        .filter(|c| *c != NAME_MARK)
        .take(MAX_INPUT_CHARS)
        .collect();
    let scanned = if mode == Mode::Export {
        replace_known_names(&bounded, names)
    } else {
        bounded
    };

    let mut out: Vec<String> = Vec::new();
    // Armed by a credential label; stays armed across further labels
    // (`Authorization: Bearer <tok>`) and bare punctuation until a VALUE token
    // has actually been redacted. The value is redacted whatever it looks like
    // (UUID, number, standard word).
    let mut armed = false;
    for part in scanned.split_whitespace() {
        if out.len() >= MAX_OUTPUT_TOKENS {
            out.push("[truncated]".into());
            redactions += 1;
            break;
        }
        let shape = credential_shape(part);
        if armed {
            match shape {
                // Another label (`Bearer` after `Authorization:`) or a run of
                // punctuation: not the value yet, keep the skip armed.
                CredentialShape::Label | CredentialShape::Punctuation => {
                    out.push(part.replace(NAME_MARK, ""));
                    continue;
                }
                // `Cookie: session=<tok>`: the pair's own value is the secret.
                CredentialShape::LabelledValue { key, delim, value_is_label } if !value_is_label => {
                    out.push(format!("{}{delim}[redacted]", key.replace(NAME_MARK, "")));
                    redactions += 1;
                    armed = false;
                    continue;
                }
                CredentialShape::LabelledValue { key, delim, .. } => {
                    out.push(format!("{}{delim}[redacted]", key.replace(NAME_MARK, "")));
                    redactions += 1;
                    continue; // `x=Bearer` then the real value: stay armed
                }
                CredentialShape::Other => {
                    out.push(redact_value(part));
                    redactions += 1;
                    armed = false;
                    continue;
                }
            }
        }
        match shape {
            CredentialShape::Label => {
                out.push(part.replace(NAME_MARK, ""));
                armed = true;
                continue;
            }
            CredentialShape::LabelledValue { key, delim, value_is_label } => {
                if value_is_label {
                    // `authorization=Bearer <tok>`: the label run continues.
                    out.push(part.replace(NAME_MARK, ""));
                    armed = true;
                } else {
                    out.push(format!("{}{delim}[redacted]", key.replace(NAME_MARK, "")));
                    redactions += 1;
                }
                continue;
            }
            CredentialShape::Punctuation | CredentialShape::Other => {}
        }
        if part.chars().count() > 96 {
            out.push("[redacted]".into());
            redactions += 1;
            continue;
        }
        if mode == Mode::SecretsOnly {
            out.push(part.into());
            continue;
        }
        match filter_token(part) {
            Filtered::Keep => out.push(part.into()),
            Filtered::Replace(text, counted) => {
                out.push(text);
                if counted {
                    redactions += 1;
                }
            }
        }
    }
    RedactedError {
        text: out.join(" "),
        redactions,
    }
}

/// Words that introduce a credential value (compared lowercase, punctuation and
/// the trailing `:`/`=` removed).
const CREDENTIAL_LABELS: &[&str] = &[
    "bearer",
    "token",
    "session",
    "authorization",
    "session_token",
    "session-token",
    "x-session-token",
    "access_token",
    "refresh_token",
    "x-auth-token",
    "api_key",
    "x-api-key",
    "cookie",
    "set-cookie",
];

enum CredentialShape<'a> {
    /// A bare label: `Bearer`, `Authorization:`, `"token":`, `Bearer,`.
    Label,
    /// Only punctuation (`-`, `:`, `"`): carries no value.
    Punctuation,
    /// `key=value` / `key:value` glued in one whitespace part, key a label.
    LabelledValue {
        key: &'a str,
        delim: char,
        /// The value is itself a label (`authorization=Bearer`), so the real
        /// secret is the NEXT part.
        value_is_label: bool,
    },
    Other,
}

fn label_core(text: &str) -> String {
    text.chars()
        .filter(|c| *c != NAME_MARK)
        .collect::<String>()
        .trim_matches(|c| EDGE_PUNCT.contains(&c) || c == '=')
        .to_ascii_lowercase()
}

fn is_credential_label(text: &str) -> bool {
    let core = label_core(text);
    CREDENTIAL_LABELS.contains(&core.as_str())
}

fn credential_shape(part: &str) -> CredentialShape<'_> {
    if is_credential_label(part) {
        return CredentialShape::Label;
    }
    if label_core(part).is_empty() {
        return CredentialShape::Punctuation;
    }
    if let Some(idx) = part.find(['=', ':']) {
        let (key, rest) = part.split_at(idx);
        let delim = rest.chars().next().unwrap_or('=');
        let value = &rest[delim.len_utf8()..];
        if is_credential_label(key) {
            if label_core(value).is_empty() {
                // `Authorization:` / `"token":` with the delimiter attached.
                return CredentialShape::Label;
            }
            return CredentialShape::LabelledValue {
                key,
                delim,
                value_is_label: is_credential_label(value),
            };
        }
    }
    CredentialShape::Other
}

/// Replacement for a credential value, keeping only edge punctuation so
/// `"Bearer <tok>"` stays well-formed.
fn redact_value(part: &str) -> String {
    let part = part.replace(NAME_MARK, "");
    // Already a placeholder (the persisted copy is redacted once at write and
    // again at export): stay idempotent rather than nesting `[[redacted]]`.
    if part.contains("[redacted]") {
        return part;
    }
    let trimmed_start = part.trim_start_matches(|c| EDGE_PUNCT.contains(&c));
    let lead = &part[..part.len() - trimmed_start.len()];
    let trail = &part[part.trim_end_matches(|c| EDGE_PUNCT.contains(&c)).len()..];
    format!("{lead}[redacted]{trail}")
}

enum Filtered {
    Keep,
    /// Replacement text and whether it counts as a redaction.
    Replace(String, bool),
}

const EDGE_PUNCT: &[char] = &[
    '(', ')', '"', '\'', '`', '[', ']', ',', ';', ':', '<', '>', '{', '}', '!', '?',
];

fn filter_token(token: &str) -> Filtered {
    let core = token.trim_matches(|c| EDGE_PUNCT.contains(&c));
    let lead = &token[..token.len() - token.trim_start_matches(|c| EDGE_PUNCT.contains(&c)).len()];
    let trail = &token[token.trim_end_matches(|c| EDGE_PUNCT.contains(&c)).len()..];

    if let Some(url) = summarise_url(core) {
        let changed = url.1;
        return Filtered::Replace(format!("{lead}{}{trail}", url.0), changed);
    }
    if is_path_like(core) {
        return Filtered::Replace("[path]".into(), true);
    }
    if core.contains(NAME_MARK) {
        return Filtered::Replace("[name]".into(), true);
    }
    if core.is_empty() || is_standard_word(core) {
        return Filtered::Keep;
    }
    Filtered::Replace("[name]".into(), true)
}

/// `http(s)://host[:port]/anything` becomes `http(s)://host/[path]`. Returns
/// the text plus whether anything was dropped. A host containing anything but
/// host characters (userinfo, odd bytes) is not a URL we trust: `None`, so the
/// caller treats it as a path.
fn summarise_url(core: &str) -> Option<(String, bool)> {
    let (scheme, rest) = if let Some(rest) = core.strip_prefix("https://") {
        ("https://", rest)
    } else if let Some(rest) = core.strip_prefix("http://") {
        ("http://", rest)
    } else {
        return None;
    };
    let host_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = &rest[..host_end];
    let host_ok = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
    if !host_ok {
        return None;
    }
    let tail = &rest[host_end..];
    if tail.is_empty() {
        Some((format!("{scheme}{host}"), false))
    } else {
        Some((format!("{scheme}{host}/[path]"), true))
    }
}

fn is_path_like(core: &str) -> bool {
    if core.contains('/') || core.contains('\\') || core.starts_with('~') {
        return true;
    }
    let bytes = core.as_bytes();
    if bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return true;
    }
    core.to_ascii_lowercase().contains("cloudstorage")
}

fn is_standard_word(core: &str) -> bool {
    if is_uuid(core) {
        return true;
    }
    for piece in core.split(|c: char| !c.is_alphanumeric()) {
        if piece.is_empty() {
            continue;
        }
        if piece.len() <= 12 && piece.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if !piece.is_ascii() || !STANDARD_WORDS.contains(&piece.to_ascii_lowercase().as_str()) {
            return false;
        }
    }
    // A token with no alphanumerics at all (`-`, `...`) carries nothing.
    true
}

fn is_uuid(s: &str) -> bool {
    let groups: Vec<&str> = s.split('-').collect();
    groups.len() == 5
        && [8usize, 4, 4, 4, 12]
            .iter()
            .zip(&groups)
            .all(|(len, g)| g.len() == *len && g.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Replace every occurrence of a known name (ASCII case-insensitive, at word
/// boundaries) with [`NAME_MARK`]. Names are pre-sorted longest first.
fn replace_known_names(input: &str, names: &KnownNames) -> String {
    let mut current = input.to_string();
    let mut hay = current.to_ascii_lowercase();
    for name in &names.names {
        if name.is_empty() || name.len() > hay.len() || !hay.contains(name.as_str()) {
            continue;
        }
        let mut out = String::with_capacity(current.len());
        let mut last = 0usize;
        let mut from = 0usize;
        let mut hits = 0usize;
        while let Some(rel) = hay[from..].find(name.as_str()) {
            let start = from + rel;
            let end = start + name.len();
            if boundary_ok(&current, start, end) {
                out.push_str(&current[last..start]);
                out.push(NAME_MARK);
                last = end;
                from = end;
                hits += 1;
            } else {
                from = start + hay[start..].chars().next().map_or(1, char::len_utf8);
            }
        }
        if hits > 0 {
            out.push_str(&current[last..]);
            current = out;
            hay = current.to_ascii_lowercase();
        }
    }
    current
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn boundary_ok(s: &str, start: usize, end: usize) -> bool {
    let before_ok = s[..start].chars().next_back().map_or(true, |c| !is_word_char(c));
    let after_ok = s[end..].chars().next().map_or(true, |c| !is_word_char(c));
    before_ok && after_ok
}

/// Standard error vocabulary: words our own engine messages, reqwest, HTTP
/// reason phrases and OS error text are built from. Kept lowercase and sorted
/// is not required, but every entry must be lowercase ASCII (unit-tested).
/// A word NOT in this list is replaced in the export, so adding a word is a
/// privacy decision. The list already contains ordinary words a user could
/// also have named a file or folder ("letter", "payment", "data", "vault",
/// "message", "drive", "storage", "trash", "share", "input"), because real
/// error text needs them ("402 Payment Required", "drive letter", "vault
/// locked"). That is safe ONLY because the known-name scan runs first: a name
/// the state DB knows is replaced before this list is consulted. A name the DB
/// does not know and that is also on this list, or is digits only, survives.
/// Extend the list only with words our own messages, reqwest, HTTP reason
/// phrases or OS errors actually produce; never to make a test string pass.
const STANDARD_WORDS: &[&str] = &[
    // grammar
    "a",
    "an",
    "and",
    "are",
    "as",
    "at",
    "be",
    "been",
    "before",
    "but",
    "by",
    "can",
    "cannot",
    "could",
    "did",
    "do",
    "does",
    "for",
    "from",
    "had",
    "has",
    "have",
    "if",
    "in",
    "into",
    "is",
    "it",
    "its",
    "must",
    "no",
    "not",
    "of",
    "on",
    "only",
    "or",
    "out",
    "over",
    "should",
    "than",
    "that",
    "the",
    "then",
    "this",
    "to",
    "too",
    "under",
    "was",
    "were",
    "when",
    "which",
    "while",
    "with",
    "without",
    "would",
    "yet",
    "during",
    "after",
    "again",
    "any",
    "all",
    "new",
    "first",
    "next",
    "same",
    "both",
    "either",
    "instead",
    "because",
    "via",
    // failure vocabulary
    "error",
    "errors",
    "failed",
    "fail",
    "failure",
    "unable",
    "refused",
    "refusing",
    "denied",
    "forbidden",
    "unauthorized",
    "unauthorised",
    "invalid",
    "missing",
    "unexpected",
    "unsupported",
    "unavailable",
    "rejected",
    "exceeded",
    "exceeds",
    "expired",
    "locked",
    "unlock",
    "stopped",
    "stopping",
    "aborted",
    "interrupted",
    "closed",
    "reset",
    "broken",
    "corrupt",
    "short",
    "bad",
    "stale",
    "incomplete",
    "failed",
    "cancelled",
    "canceled",
    "timed",
    "timeout",
    "wait",
    "waiting",
    "retry",
    "attempt",
    // http / network
    "http",
    "https",
    "status",
    "client",
    "server",
    "request",
    "requests",
    "response",
    "body",
    "url",
    "connect",
    "connection",
    "dns",
    "tcp",
    "tls",
    "ssl",
    "certificate",
    "handshake",
    "host",
    "address",
    "lookup",
    "network",
    "unreachable",
    "route",
    "resolve",
    "resolution",
    "temporary",
    "service",
    "gateway",
    "redirect",
    "redirects",
    "proxy",
    "sending",
    "decoding",
    "decode",
    "builder",
    "hyper",
    "message",
    "completed",
    "header",
    "headers",
    "payment",
    "required",
    "method",
    "allowed",
    "conflict",
    "gone",
    "payload",
    "large",
    "many",
    "internal",
    "insufficient",
    "storage",
    "unprocessable",
    "entity",
    "precondition",
    "found",
    "ok",
    "continue",
    // os / io
    "os",
    "io",
    "no",
    "such",
    "directory",
    "exists",
    "space",
    "left",
    "device",
    "busy",
    "pipe",
    "file",
    "files",
    "folder",
    "read",
    "write",
    "written",
    "open",
    "create",
    "created",
    "rename",
    "move",
    "delete",
    "deleted",
    "remove",
    "removed",
    "access",
    "operation",
    "permitted",
    "permission",
    "read-only",
    "readonly",
    "system",
    "temporarily",
    "resource",
    "argument",
    "input",
    "data",
    "end",
    "eof",
    "symlink",
    "link",
    "parent",
    "component",
    "components",
    "prefix",
    "drive",
    "letter",
    "absolute",
    "relative",
    "empty",
    "unc",
    "normal",
    "root",
    "allowed",
    "stay",
    "unsafe",
    "path",
    "local",
    "dest",
    "destination",
    "dir",
    "contain",
    "contains",
    "interior",
    "nul",
    "byte",
    "bytes",
    "size",
    "zero",
    "dimensions",
    // engine vocabulary
    "upload",
    "uploads",
    "uploaded",
    "init",
    "chunk",
    "chunks",
    "range",
    "covering",
    "staged",
    "expected",
    "ended",
    "truncated",
    "encrypt",
    "encrypted",
    "encryption",
    "decrypt",
    "decrypted",
    "decryption",
    "cipher",
    "key",
    "keys",
    "hydrate",
    "hydrated",
    "hydration",
    "queue",
    "queued",
    "worker",
    "attached",
    "enqueue",
    "sync",
    "engine",
    "state",
    "db",
    "row",
    "id",
    "ids",
    "uuid",
    "item",
    "items",
    "share",
    "shared",
    "malformed",
    "returned",
    "metadata",
    "name",
    "blob",
    "version",
    "versions",
    "restore",
    "restored",
    "trash",
    "accept",
    "changed",
    "change",
    "during",
    "finder",
    "callback",
    "include",
    "included",
    "contents",
    "modify",
    "modified",
    "resolved",
    "base",
    "current",
    "readable",
    "vault",
    "sign",
    "session",
    "token",
    "bearer",
    "authorization",
    "quota",
    "limit",
    "storage",
    "shared",
    "base64",
    "standard",
    "valid",
    "bad",
    "32",
    "hex",
    "u32",
    "usize",
    "u64",
    "i64",
    "utf8",
    "utf",
    "range",
    "line",
    "column",
    "value",
    "type",
    "field",
    "while",
    "ffmpeg",
    "spawn",
    "failed",
    "lookup",
    "mismatch",
    "lock",
    "locks",
    "unlocked",
    "panic",
    "panicked",
    "join",
    "channel",
    "send",
    "receive",
    "received",
    "closed",
    "dropped",
    "full",
    "capacity",
    "database",
    "sqlite",
    "busy",
    "constraint",
    "unique",
    "foreign",
    "violation",
    "locked",
    "disk",
    "full",
    "i",
    "o",
    "worker",
    "transfer",
    "sent",
    "expected",
    "actual",
    "got",
    "want",
    "wanted",
    "need",
    "needs",
    "needed",
    "require",
    "requires",
    "requested",
    "open",
    "opened",
    "supported",
    "enabled",
    "disabled",
    "yet",
    "now",
    "later",
    "soon",
    "try",
    "trying",
    "please",
    "already",
    "still",
    "also",
    "null",
    "none",
    "some",
    "unknown",
    "other",
    "reading",
    "writing",
    "creating",
    "deleting",
    "moving",
    "renaming",
    "uploading",
    "hydrating",
    "syncing",
    "checking",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn names(paths: &[&str]) -> KnownNames {
        let mut k = KnownNames::new();
        for p in paths {
            k.add_path(p);
        }
        k.finish()
    }

    const ISSUE_ERROR: &str = "/Users/guus/Library/CloudStorage/Beebeeb-Drive/Tax 2025/aangifte.pdf failed";

    #[test]
    fn standard_words_are_lowercase_ascii() {
        for w in STANDARD_WORDS {
            assert!(
                w.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                "vocabulary entry {w:?} must be lowercase ascii"
            );
        }
    }

    #[test]
    fn issue_example_path_and_known_multiword_name_are_gone() {
        let out = redact_for_export(ISSUE_ERROR, &names(&["Tax 2025/aangifte.pdf"]));
        for leaked in [
            "/Users/guus",
            "CloudStorage",
            "Beebeeb-Drive",
            "Tax 2025",
            "Tax",
            "2025",
            "aangifte",
            ".pdf",
        ] {
            assert!(!out.text.contains(leaked), "{leaked:?} leaked: {}", out.text);
        }
        assert!(
            out.text.contains("failed"),
            "the failure word must survive: {}",
            out.text
        );
        assert!(out.redactions >= 1, "redactions must be counted: {out:?}");
    }

    #[test]
    fn multiword_name_is_replaced_even_when_the_path_is_unknown() {
        // The name is known (from the DB) but the error embeds it with spaces
        // and no slash: substring scan must catch it before tokenising.
        let out = redact_for_export("upload of Tax 2025 folder failed", &names(&["Tax 2025"]));
        assert!(!out.text.contains("Tax"), "{}", out.text);
        assert!(!out.text.contains("2025"), "{}", out.text);
        assert!(out.text.contains("failed"));
        assert_eq!(out.redactions, 1, "{out:?}");
    }

    #[test]
    fn longest_known_name_wins_over_its_prefix() {
        let out = redact_for_export("x Tax 2025 y Tax z", &names(&["Tax", "Tax 2025"]));
        assert!(!out.text.contains("2025"), "{}", out.text);
        assert!(!out.text.contains("Tax"), "{}", out.text);
    }

    #[test]
    fn names_match_case_insensitively_at_word_boundaries_only() {
        let out = redact_for_export("upload TAX 2025 failed, taxi failed", &names(&["tax 2025"]));
        assert!(!out.text.to_lowercase().contains("tax 2025"), "{}", out.text);
        // `taxi` is not the known name `tax`/`tax 2025`; it is also not a
        // standard word, so the allow-list replaces it rather than the scan.
        assert!(!out.text.contains("taxi"), "{}", out.text);
    }

    #[test]
    fn unknown_names_are_removed_by_the_allow_list() {
        // No DB knowledge at all: a never-seen name cannot survive.
        let out = redact_for_export(
            "cannot read Holiday photos Zoë.heic: Permission denied (os error 13)",
            &KnownNames::new(),
        );
        for leaked in ["Holiday", "photos", "Zoë", "heic"] {
            assert!(!out.text.contains(leaked), "{leaked} leaked: {}", out.text);
        }
        assert!(out.text.contains("Permission denied (os error 13)"), "{}", out.text);
    }

    #[test]
    fn path_shapes_are_all_redacted() {
        for p in [
            "/Users/guus/Documents/x.txt",
            "~/Documents/x.txt",
            "C:\\Users\\guus\\Documents\\x.txt",
            "D:\\x.txt",
            "\\\\server\\share\\x.txt",
            "Library/CloudStorage/Beebeeb-Drive",
            "relative/dir/file",
        ] {
            let out = redact_for_export(&format!("write {p}: denied"), &KnownNames::new());
            assert!(out.text.contains("[path]"), "no [path] for {p}: {}", out.text);
            assert!(!out.text.contains("Users"), "{p}: {}", out.text);
            assert!(!out.text.contains("Documents"), "{p}: {}", out.text);
            assert!(!out.text.contains("Beebeeb-Drive"), "{p}: {}", out.text);
            assert!(!out.text.contains("x.txt"), "{p}: {}", out.text);
        }
    }

    #[test]
    fn url_keeps_host_and_drops_path_and_query() {
        let out = redact_for_export(
            "HTTP status client error (401 Unauthorized) for url (http://127.0.0.1:8080/api/v1/files/Tax%202025?cursor=abc)",
            &KnownNames::new(),
        );
        assert!(out.text.contains("401 Unauthorized"), "{}", out.text);
        assert!(out.text.contains("http://127.0.0.1:8080/[path]"), "{}", out.text);
        assert!(!out.text.contains("Tax"), "{}", out.text);
        assert!(!out.text.contains("cursor"), "{}", out.text);
    }

    #[test]
    fn url_with_userinfo_is_not_trusted() {
        let out = redact_for_export("request to https://guus:pw@evil.example/x failed", &KnownNames::new());
        assert!(!out.text.contains("guus"), "{}", out.text);
        assert!(!out.text.contains("evil"), "{}", out.text);
    }

    #[test]
    fn credential_tokens_are_still_redacted_and_counted() {
        let out = redact_for_export(
            "401 unauthorized Bearer abc.def.ghi session_token=super-secret",
            &KnownNames::new(),
        );
        assert!(out.text.contains("Bearer [redacted]"), "{}", out.text);
        assert!(out.text.contains("session_token=[redacted]"), "{}", out.text);
        assert!(!out.text.contains("abc.def.ghi"));
        assert!(!out.text.contains("super-secret"));
        assert_eq!(out.redactions, 2, "{out:?}");
    }

    // ---- task 1685 P1: stacked / variant credential label shapes ----------
    //
    // A credential label must keep the skip armed until an actual VALUE has
    // been redacted, and that value is redacted even when it looks like a UUID,
    // a number or a standard word.

    const UUID_TOKEN: &str = "123e4567-e89b-12d3-a456-426614174000";

    /// Runs both passes. The token must not appear in either output, the export
    /// pass must equal `expected` exactly and count `redactions` placeholders.
    fn assert_credential_shape(input: &str, token: &str, expected: &str, redactions: u32) {
        let export = redact_for_export(input, &KnownNames::new());
        assert!(!export.text.contains(token), "export leaked {token:?}: {}", export.text);
        assert_eq!(export.text, expected, "export text for {input:?}");
        assert_eq!(export.redactions, redactions, "export count for {input:?}: {export:?}");
        let secrets = redact_secrets_only(input);
        assert!(!secrets.contains(token), "secrets-only leaked {token:?}: {secrets}");
        assert_eq!(secrets, expected, "secrets-only text for {input:?}");
    }

    #[test]
    fn credential_authorization_colon_bearer_uuid() {
        let input = format!("Authorization: Bearer {UUID_TOKEN}");
        assert_credential_shape(&input, UUID_TOKEN, "Authorization: Bearer [redacted]", 1);
    }

    #[test]
    fn credential_authorization_bearer_uuid_keeps_following_words() {
        let input = format!("401 Authorization: Bearer {UUID_TOKEN} retry failed");
        assert_credential_shape(&input, UUID_TOKEN, "401 Authorization: Bearer [redacted] retry failed", 1);
    }

    #[test]
    fn credential_authorization_equals_bearer() {
        let input = format!("authorization=Bearer {UUID_TOKEN}");
        assert_credential_shape(&input, UUID_TOKEN, "authorization=Bearer [redacted]", 1);
    }

    #[test]
    fn credential_bearer_colon() {
        let input = format!("Bearer: {UUID_TOKEN}");
        assert_credential_shape(&input, UUID_TOKEN, "Bearer: [redacted]", 1);
    }

    #[test]
    fn credential_token_colon() {
        let input = format!("token: {UUID_TOKEN}");
        assert_credential_shape(&input, UUID_TOKEN, "token: [redacted]", 1);
    }

    #[test]
    fn credential_session_token_space() {
        let input = format!("session_token {UUID_TOKEN}");
        assert_credential_shape(&input, UUID_TOKEN, "session_token [redacted]", 1);
    }

    #[test]
    fn credential_x_session_token_header() {
        let input = format!("X-Session-Token: {UUID_TOKEN}");
        assert_credential_shape(&input, UUID_TOKEN, "X-Session-Token: [redacted]", 1);
    }

    #[test]
    fn credential_cookie_session_equals() {
        let input = format!("Cookie: session={UUID_TOKEN}");
        assert_credential_shape(&input, UUID_TOKEN, "Cookie: session=[redacted]", 1);
    }

    #[test]
    fn credential_bearer_comma_then_value() {
        let input = format!("Bearer, {UUID_TOKEN}");
        assert_credential_shape(&input, UUID_TOKEN, "Bearer, [redacted]", 1);
    }

    #[test]
    fn credential_quoted_bearer_value() {
        let input = format!("\"Bearer {UUID_TOKEN}\"");
        assert_credential_shape(&input, UUID_TOKEN, "\"Bearer [redacted]\"", 1);
    }

    #[test]
    fn credential_json_shaped_token() {
        let input = format!("{{\"token\": \"{UUID_TOKEN}\"}}");
        assert_credential_shape(&input, UUID_TOKEN, "{\"token\": \"[redacted]\"}", 1);
    }

    #[test]
    fn credential_value_that_looks_like_a_number_or_standard_word() {
        assert_credential_shape("Authorization: Bearer 4815162342 ok", "4815162342", "Authorization: Bearer [redacted] ok", 1);
        assert_credential_shape("token: failed ok", "failed", "token: [redacted] ok", 1);
    }

    #[test]
    fn credential_label_at_end_of_string_is_kept_and_nothing_else_is_touched() {
        for input in ["failed with Authorization:", "failed with Authorization: Bearer", "failed with token"] {
            let export = redact_for_export(input, &KnownNames::new());
            assert_eq!(export.text, input);
            assert_eq!(export.redactions, 0, "{export:?}");
            assert_eq!(redact_secrets_only(input), input);
        }
    }

    #[test]
    fn credential_redaction_is_idempotent_across_the_persist_and_export_passes() {
        let input = format!("Authorization: Bearer {UUID_TOKEN} \"token\": \"{UUID_TOKEN}\"");
        let once = redact_secrets_only(&input);
        assert_eq!(redact_secrets_only(&once), once);
        let export = redact_for_export(&once, &KnownNames::new());
        assert!(!export.text.contains("[["), "{}", export.text);
        assert!(!export.text.contains(UUID_TOKEN), "{}", export.text);
    }

    #[test]
    fn credential_labels_do_not_arm_beyond_the_value() {
        // Only the value after the label run is consumed; later words survive.
        let input = format!("token: {UUID_TOKEN} token: other failed");
        let export = redact_for_export(&input, &KnownNames::new());
        assert_eq!(export.text, "token: [redacted] token: [redacted] failed");
        assert_eq!(export.redactions, 2);
    }

    #[test]
    fn secrets_only_mode_does_not_touch_paths() {
        // The persisted copy keeps only credential stripping; the export pass
        // is the privacy boundary.
        let s = redact_secrets_only("write /a/b/c failed Bearer zzz");
        assert_eq!(s, "write /a/b/c failed Bearer [redacted]");
    }

    #[test]
    fn a_forged_name_marker_in_input_is_stripped() {
        let out = redact_for_export("failed \u{E000}\u{E000} ok", &KnownNames::new());
        assert!(!out.text.contains('\u{E000}'), "{:?}", out.text);
    }

    /// Pins the documented residual (docs/CAPABILITIES.md, the Account copy): a
    /// name the state DB does not know survives when it is a standard word, and
    /// is removed the moment the DB does know it.
    #[test]
    fn residual_unknown_standard_word_name_survives_but_a_known_one_does_not() {
        let unknown = redact_for_export("rename Payment to Letter failed", &KnownNames::new().finish());
        assert!(unknown.text.contains("Letter"), "documented residual: {}", unknown.text);
        let known = redact_for_export("rename Payment to Letter failed", &names(&["Payment", "Letter"]));
        assert!(!known.text.contains("Payment") && !known.text.contains("Letter"), "{}", known.text);
        assert!(known.redactions >= 2, "{known:?}");
        // Digits-only names and opaque ids are kept on purpose.
        let id = "0b9d5a52-7f0e-4c9e-8a6f-1f2f3a4b5c6d";
        let kept = redact_for_export(&format!("op {id} at http://127.0.0.1:8080/x/y failed"), &KnownNames::new().finish());
        assert!(kept.text.contains(id) && kept.text.contains("http://127.0.0.1:8080"), "{}", kept.text);
        assert!(!kept.text.contains("/x/y"), "{}", kept.text);
    }

    #[test]
    fn uuids_and_numbers_survive() {
        let out = redact_for_export(
            "no state.db row for 123e4567-e89b-12d3-a456-426614174000 (chunk 3 of 12)",
            &KnownNames::new(),
        );
        assert_eq!(
            out.text,
            "no state.db row for 123e4567-e89b-12d3-a456-426614174000 (chunk 3 of 12)"
        );
        assert_eq!(out.redactions, 0);
    }

    #[test]
    fn oversized_input_is_bounded() {
        let long = "failed ".repeat(5000);
        let out = redact_for_export(&long, &KnownNames::new());
        assert!(
            out.text.ends_with("[truncated]"),
            "{}",
            &out.text[out.text.len().saturating_sub(40)..]
        );
        assert!(out.text.split_whitespace().count() <= MAX_OUTPUT_TOKENS + 1);
    }

    #[test]
    fn error_code_is_a_closed_value_that_cannot_carry_a_name() {
        let json = serde_json::to_string(&classify_error_code(ISSUE_ERROR)).unwrap();
        assert_eq!(json, "\"other\"");
        assert_eq!(
            classify_error_code("write /Users/guus/Tax 2025/aangifte.pdf: Permission denied (os error 13)"),
            DiagnosticErrorCode::Permission
        );
        assert_eq!(
            classify_error_code("staged upload payload is missing: /Users/guus/x.pdf"),
            DiagnosticErrorCode::PayloadMissing
        );
        assert_eq!(
            classify_error_code("create dest dir /Users/guus/Tax 2025: No such file or directory (os error 2)"),
            DiagnosticErrorCode::LocalWriteFailed
        );
        assert_eq!(
            classify_error_code("local path must stay under the sync root: rel_path must not be absolute"),
            DiagnosticErrorCode::PathRejected
        );
        assert_eq!(
            classify_error_code("ffmpeg video thumbnail failed: /Users/guus/movie.mov: Invalid data"),
            DiagnosticErrorCode::Thumbnail
        );
        assert_eq!(
            classify_error_code("error sending request for url (http://x/): connection refused"),
            DiagnosticErrorCode::Network
        );
        assert_eq!(
            classify_error_code("500 Internal Server Error"),
            DiagnosticErrorCode::ServerError
        );
        assert_eq!(
            classify_error_code("engine is stopping; refusing to enqueue a new local write"),
            DiagnosticErrorCode::EngineStopping
        );
        assert_eq!(
            classify_error_code("stale base version rejected"),
            DiagnosticErrorCode::StaleVersion
        );
    }

    #[test]
    fn group_labels_outside_the_allow_list_fold_into_other() {
        assert_eq!(allowed_label("upload_version", QUEUE_KIND_LABELS), "upload_version");
        assert_eq!(allowed_label("Tax 2025", QUEUE_KIND_LABELS), "other");
        assert_eq!(allowed_label("auth", PAUSE_REASON_LABELS), "auth");
        assert_eq!(allowed_label("key_replaced", PAUSE_REASON_LABELS), "key_replaced");
        assert_eq!(allowed_label("/Users/guus", PAUSE_REASON_LABELS), "other");
    }
}
