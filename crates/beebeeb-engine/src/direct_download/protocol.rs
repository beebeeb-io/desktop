//! The driver's contract: `Request` (one whole file, with its version pin), `Progress` (the two
//! numbers the caller sees), `Outcome`, the typed `DirectError`, and the internal event and task
//! types the download loop multiplexes.
//!
//! Data and names only: no I/O and no policy. `download` decides, this module names.
//! Tests: `crate::direct_download::tests`.

use super::*;

/// What the caller asked for: one whole file.
pub struct Request<'a> {
    pub file_id: &'a str,
    pub file_key: &'a beebeeb_core::kdf::FileKey,
    /// The metadata's chunk count; the grant manifest must agree.
    pub chunk_count: u32,
    /// The content record of the file's authenticated metadata: the pin and the version commitment.
    pub record: crate::content_record::ContentRecord,
    /// The user's download ceiling in KB/s (`0`: none), as on the API path.
    pub download_kbps_limit: u64,
    /// Sizes the plaintext buffer only.
    pub approx_size: usize,
    /// The version the metadata named, with its manifest (task 1643). The first window pins
    /// whatever is current, so landing on another version means the file was replaced after the
    /// metadata was read: a restart from fresh metadata, never the API path with the old geometry.
    /// `None`: the metadata names no version (a legacy file).
    pub pin: Option<&'a crate::download_pin::PinnedVersion>,
}

/// Where the driver reports progress: the same two numbers the API path reports.
pub trait Progress {
    /// Body bytes of a chunk arrived (storage or API): called per read as the body streams in, so a
    /// chunk that takes minutes still reports (LD-0105 S0). The calls for one chunk add up to its
    /// wire length; a chunk that fails part way has reported the bytes that did arrive.
    fn received(&self, wire_bytes: u64);
    /// Bytes reported by [`Progress::received`] that are not used after all: a body that broke off
    /// or failed its length check, or a chunk that did not authenticate. They kept the fetch alive;
    /// they are no longer progress, and no longer on the wire counter (lead decision 2026-11-06).
    fn discarded(&self, wire_bytes: u64);
    /// A chunk was decrypted and appended, strictly in order: the plaintext length so far. Called
    /// with 0 once, when the direct attempt starts (the API path's first report).
    fn decrypted(&self, plaintext_bytes: u64);
}

/// The result of an attempt to download over the direct transport.
pub enum Outcome {
    /// The file is complete and every chunk authenticated.
    Done {
        plaintext: Zeroizing<Vec<u8>>,
        transport: FileTransport,
    },
    /// Direct transport was not used, or was abandoned before the file completed: download this
    /// file through the API path. Nothing of the attempt is kept.
    UseApi,
}

/// A failure the API path would not fix. Nothing is returned on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectError {
    /// The pinned version went away mid-transfer (`version_unavailable`): restart from fresh
    /// metadata, never mix versions.
    VersionChanged,
    /// An authorization answer (401, 403, a JSON 404) on a grant or on the pinned route. Never
    /// routed around.
    Refused(u16),
    /// A chunk did not authenticate from storage, nor from the API.
    Corrupt,
}

impl std::fmt::Display for DirectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DirectError::VersionChanged => f.write_str("the file changed while it was downloading"),
            DirectError::Refused(status) => write!(f, "download refused by the server ({status})"),
            DirectError::Corrupt => f.write_str(
                "a chunk did not authenticate from storage or from the API: the file is damaged or was tampered with",
            ),
        }
    }
}

impl std::error::Error for DirectError {}

pub(super) enum Event {
    Window(GrantReply),
    Storage {
        index: u32,
        /// `Err(status)`: the HTTP status when there was one (0: none).
        result: Result<Vec<u8>, u16>,
    },
    Api {
        index: u32,
        result: Result<Vec<u8>, ChunkApiError>,
    },
}

pub(super) type Task<'a> = Pin<Box<dyn Future<Output = Event> + Send + 'a>>;
