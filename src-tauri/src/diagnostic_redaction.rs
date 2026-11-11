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

mod classify;
pub use classify::{DiagnosticErrorCode, PAUSE_REASON_LABELS, QUEUE_KIND_LABELS, allowed_label, classify_error_code};

mod known_names;
pub use known_names::KnownNames;
use known_names::*;

mod export;
pub use export::{redact_for_export, redact_secrets_only};

mod credential_shapes;
use credential_shapes::*;

mod tokens;
use tokens::*;

mod standard_words;
use standard_words::*;

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

    mod bounds;
    mod codes;
    mod credentials;
    mod names;
    mod urls;
}
