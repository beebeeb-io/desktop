//! What an `ApiClient` call fails with, and the one place each failure is
//! named.
//!
//! Owns: the failure kinds and the text each one shows. That text is a
//! contract: `link_health` reads the `HTTP <status>` prefix, and callers
//! outside this module inspect the chain (`engine_bridge`'s status and conflict
//! readers, `owner_keys`' custody states), which is why every wrapper names its
//! cause as its source. Must not: build a request or know what an endpoint's
//! refusal means (see `types`). Tests: `api_client/tests/errors.rs`.

use super::*;

/// What a call on [`ApiClient`] fails with.
///
/// One variant per kind of failure this client can tell apart, so a caller
/// matches instead of reading text. The two transport-shaped kinds still show
/// exactly what `reqwest` showed before, so no log line or view text changes.
#[derive(Debug)]
pub enum DesktopApiError {
    /// The request never got an answer, or the answer did not read as the wire
    /// type. `reqwest`'s own error, unchanged (status, URL, never the body).
    Transport(reqwest::Error),
    /// The server answered outside 2xx. `snippet` is the head of the body (300
    /// chars: these bodies are error objects, and a large body must not reach a
    /// log or a view).
    HttpStatus {
        status: reqwest::StatusCode,
        snippet: String,
    },
    /// An upload init refused with 409: the class, and the plan reason when the
    /// server named one.
    InitConflict(InitConflict),
    /// An upload request refused with a non-2xx, carrying the server's code.
    UploadRefused(UploadRefused),
    /// A custody write refused until the folder's owner upgrades it (L11).
    UpgradeRequired(UpgradeRequired),
    /// A search shard was offered empty; the server stores it as absent.
    EmptyShardBlob,
    /// A search shard blob is over the per-shard cap the server enforces.
    ShardBlobTooLarge {
        bucket: u32,
        page: u32,
        bytes: usize,
    },
    /// A listing ran past `cap` rows: a cursor loop, not a folder.
    ListingTooLarge { listing: Listing, cap: usize },
    /// The owner-key adoption answer carried no `adopted` boolean.
    InvalidAdoptionResponse,
}

/// Which listing outgrew its cap. The text names the listing, because a failure
/// to list the trash is a different report than a failure on a shared folder.
#[derive(Debug, Clone, Copy)]
pub enum Listing {
    SharedFolder,
    Trashed,
}

impl Listing {
    fn as_str(self) -> &'static str {
        match self {
            Listing::SharedFolder => "shared folder",
            Listing::Trashed => "trashed file",
        }
    }
}

impl From<reqwest::Error> for DesktopApiError {
    fn from(error: reqwest::Error) -> Self {
        DesktopApiError::Transport(error)
    }
}

impl std::fmt::Display for DesktopApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DesktopApiError::Transport(error) => error.fmt(f),
            DesktopApiError::HttpStatus { status, snippet } => write!(f, "HTTP {status}: {snippet}"),
            DesktopApiError::InitConflict(error) => error.fmt(f),
            DesktopApiError::UploadRefused(error) => error.fmt(f),
            DesktopApiError::UpgradeRequired(error) => error.fmt(f),
            DesktopApiError::EmptyShardBlob => f.write_str("search shard blob must not be empty"),
            DesktopApiError::ShardBlobTooLarge { bucket, page, bytes } => {
                write!(f, "search shard {bucket}/{page} blob {bytes}B exceeds cap")
            }
            DesktopApiError::ListingTooLarge { listing, cap } => {
                write!(f, "{} listing exceeded hard cap of {cap}", listing.as_str())
            }
            DesktopApiError::InvalidAdoptionResponse => f.write_str("invalid adoption response"),
        }
    }
}

impl std::error::Error for DesktopApiError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DesktopApiError::Transport(error) => Some(error),
            DesktopApiError::InitConflict(error) => Some(error),
            DesktopApiError::UploadRefused(error) => Some(error),
            DesktopApiError::UpgradeRequired(error) => Some(error),
            _ => None,
        }
    }
}

/// The error a call in this module returns. Named, so a signature says which
/// failure type it hands back instead of `Result<_, DesktopApiError>` at every
/// step.
pub(crate) type ApiResult<T> = Result<T, DesktopApiError>;

/// Reading an error's causes, for the two shapes one can arrive in: the typed
/// [`DesktopApiError`] this module hands out, and the `anyhow::Error` older
/// call sites still carry (a `?` in an `anyhow` function converts, it does not
/// unwrap). Both must answer the same questions, so both walk the same way.
pub(crate) trait ErrorChain {
    /// This error and every cause it names, outermost first.
    fn causes(&self) -> Vec<&(dyn std::error::Error + 'static)>;
}

impl ErrorChain for DesktopApiError {
    fn causes(&self) -> Vec<&(dyn std::error::Error + 'static)> {
        std::iter::successors(Some(self as &(dyn std::error::Error + 'static)), |cause| cause.source())
            .collect()
    }
}

impl ErrorChain for anyhow::Error {
    fn causes(&self) -> Vec<&(dyn std::error::Error + 'static)> {
        self.chain().collect()
    }
}
