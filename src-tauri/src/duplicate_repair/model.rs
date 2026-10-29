//! Task 2274: what the tool records about each pair. It is written to the run file in the out dir on this PC only (spec
//! A3): names, paths and file ids never reach the screen.
use serde::{Deserialize, Serialize};

/// The spec's decision table (section 7), one value per row. The first matching row wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Class {
    /// L1: nothing at the path, unreadable, a plain file, or the identity names neither row.
    Unbound,
    /// L2: the file on this PC is bound to the row with no known content, while the other row has it.
    BoundToB,
    /// L3: no known content for A, A does not read in sync, or pending work names A.
    ABusy,
    /// L3b: pending work names B.
    BBusy,
    /// L4: the server has no live A.
    AGone,
    /// L5: the server has no B.
    BGone,
    /// L6: B is in the server's trash already.
    BInTrash,
    /// L7: A and B have different parents on the server.
    Moved,
    /// L8: B is shared, or has more than one version.
    YourCall,
    /// L9: B's size differs from A's.
    DiffersBySize,
    /// L10: equal size, the bytes not compared yet.
    Unknown,
    /// L11: compared, the bytes differ.
    Differs,
    /// L12: compared, the bytes are the same.
    Identical,
}

/// What this PC's state DB holds for one row of the pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RowFacts {
    pub file_id: String,
    pub status: String,
    pub size_bytes: i64,
    pub remote_updated_at: i64,
    pub local_hash: Option<String>,
    pub current_version: i64,
    /// The row sits in a shared folder (a share or shared root is recorded).
    pub shared: bool,
}

/// What the platform shows at the pair's path, read without fetching anything (spec D4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ViewFacts {
    /// `placeholder`, `plain`, `missing` or `unreadable`.
    pub kind: String,
    /// `nothing`, `partly` or `wholly`, for a placeholder.
    pub on_disk: Option<String>,
    pub in_sync: Option<bool>,
    /// The file id the placeholder's identity names.
    pub identity_names: Option<String>,
    pub size: Option<u64>,
    /// Last write, in 100 ns ticks since 1601 on Windows (as the platform reports it).
    pub modified: Option<u64>,
    pub read_only: Option<bool>,
}

/// One path that two rows share.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Pair {
    /// From 1, in path order: the number the person and the lane use for it.
    pub n: u32,
    /// The path relative to the sync root, as the state DB holds it ('/'-folded).
    pub path: String,
    /// The row the file on this PC is bound to (or, when it is bound to neither, the row with known content).
    pub a: RowFacts,
    pub b: RowFacts,
    pub view: ViewFacts,
    pub class: Class,
    /// Phase 1b: what the server says about A and B. `None` until read, and for a pair the local rows decided.
    pub server: Option<ServerRead>,
}

/// Phase 1b: one file as `GET /api/v1/files/{id}` answers (spec S3, S8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ServerFacts {
    pub trashed: bool,
    pub parent_id: Option<String>,
    pub size_bytes: i64,
    pub updated_at: String,
    /// `download_version.version_number`: the current version, 0 when the server names none.
    pub version_number: i64,
    /// `acted_via_share_id` is set.
    pub via_share: bool,
}

/// A file the server read: gone (404) or found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "state")]
pub(crate) enum ServerFile {
    Gone,
    Found(ServerFacts),
}

/// What phase 1b read for one pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ServerRead {
    pub a: ServerFile,
    /// `None` when B was not read (A gone).
    pub b: Option<ServerFile>,
    /// L8 amendment: B's live links and live invites, as counts. The responses themselves are never kept.
    pub live_links: Option<usize>,
    pub live_invites: Option<usize>,
}
