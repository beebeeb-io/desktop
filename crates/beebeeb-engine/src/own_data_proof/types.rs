//! `types` items split out of `own_data_proof.rs`.

/// How many of the server-named rows must prove to be this account's own (all of them when fewer were checked).
pub const REQUIRED_PROOFS: usize = 2;

/// What the server answered for this account's tree.
#[derive(Debug, Clone)]
pub enum Listing {
    /// The nodes of `GET /api/v1/sync/snapshot`: this account's own, non-trashed files and folders.
    Nodes(Vec<serde_json::Value>),
    /// The server refused this session's token (401): nothing about ownership, and not a connection problem either.
    SignedOut,
    /// No answer that says anything about ownership: offline, a timeout, 429, a server error.
    Unavailable,
}

/// One server-named row, judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// In this account's tree, and its name decrypts under this session's master key: this account's own.
    Own,
    /// In this account's tree, but its name does not decrypt under this account's key.
    Unproven,
    /// Proves nothing either way: a plaintext (legacy) name, no name, or a name sealed under someone else's key by
    /// design (a file received through a file request, or one a collaborator uploaded into this account's folder).
    /// Set aside, never counted for or against.
    Neutral,
    /// Not in this account's tree, but its nearest ancestor that is, is this account's own: deleted since.
    Gone,
    /// Not in this account's tree, and nothing of this account's explains it (another account's file looks exactly
    /// like that).
    Missing,
}

/// The counts of one proof. Logged and reported; never a name or a path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    /// Server-named My Files rows judged: all of them.
    pub checked: usize,
    pub own: usize,
    pub neutral: usize,
    /// Rows deleted since from a folder proven this account's own.
    pub gone: usize,
    pub unproven: usize,
    pub missing: usize,
    /// Queued changes that act on proven data: they upload after the adoption.
    pub proven_changes: usize,
    /// Queued changes held for the person's review (`found_while_paused`).
    pub held_for_review: usize,
    /// 1 when the server refused this session's token for the listing.
    pub signed_out: usize,
    /// 1 when the listing did not answer, twice.
    pub unavailable: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Adopt,
    NotProven,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing a proof can settle: the data is owned by this session already, there is none, or the owner record
    /// names an account by its user id that this session cannot be compared with (R10's strongest signal).
    NotEligible,
    /// Proven, and this session is now the recorded owner: the next start proceeds and keeps everything. The changes
    /// that could not be tied to this account's data are held for review.
    Adopted(Tally),
    NotProven(Tally),
    /// No usable listing; nothing recorded. A later start asks again.
    Unavailable(Tally),
    /// The session or the owner record changed while the proof ran; nothing recorded.
    Changed(Tally),
}
