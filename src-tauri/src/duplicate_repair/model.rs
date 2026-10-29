//! Task 2274: what the tool records about each pair. It is written to the run file in the out dir on this PC only (spec
//! A3): names, paths and file ids never reach the screen.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    /// What the person marked (spec G2): the tool proposes one from the class, the person can change it.
    #[serde(default)]
    pub action: Action,
    /// What `apply-one` did (spec L2: with B's id above, on this PC only, for a restore within retention).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied: Option<Applied>,
    /// SHA-256 of everything the tool wrote for the pair, the action left out: set when the run file is saved, checked
    /// when it is loaded, so a hand edit of anything but the action is refused (review F1). It catches a mistake, not
    /// someone who rewrites the file and the seal together.
    #[serde(default)]
    pub seal: String,
}

/// The actions a pair can be marked with (spec section 7, "Proposed action").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Action {
    #[default]
    None,
    /// Forget B's row on this PC, then move B to Beebeeb's trash (L12, or L8/L9/L11 when the person marks it).
    TrashB,
    /// Forget B's row on this PC only: B is gone or in the trash already (L5, L6).
    ForgetB,
}

impl Pair {
    /// The pair's seal: SHA-256 of the pair as the tool wrote it, with the action and the seal left out.
    fn sealed(&self) -> String {
        let mut bare = self.clone();
        bare.action = Action::None;
        bare.seal = String::new();
        format!("{:x}", Sha256::digest(serde_json::to_vec(&bare).expect("a pair serializes")))
    }
}

impl Class {
    /// The action the tool proposes for a pair of this class.
    pub(crate) fn proposed(self) -> Action {
        match self {
            Class::Identical => Action::TrashB,
            Class::BGone | Class::BInTrash => Action::ForgetB,
            _ => Action::None,
        }
    }

    /// Whether the person may mark a pair of this class with `action`: "trash B" only where the table proposes it or
    /// leaves it to the person (L8, L9, L11, L12), "forget B" only where B is gone or in the trash (L5, L6).
    pub(crate) fn allows(self, action: Action) -> bool {
        match action {
            Action::None => true,
            Action::TrashB => matches!(self, Class::Identical | Class::Differs | Class::DiffersBySize | Class::YourCall),
            Action::ForgetB => matches!(self, Class::BGone | Class::BInTrash),
        }
    }
}

/// What `apply-one` did to a pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Applied {
    pub outcome: Outcome,
    /// The app's op cursor when the step ran.
    pub cursor: i64,
    /// The copy of the state DB taken before the step, in the out dir.
    pub backup: String,
    /// When the step ran, in Unix seconds.
    pub at: i64,
}

/// The result of `apply-one` for one pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "result")]
pub(crate) enum Outcome {
    /// B's row forgotten and B in the server's trash, A live: `seq` is B's `file_trash` op, the echo the app applies.
    Trashed { seq: i64, b_trashed_at: Option<String> },
    /// The trash went through, but the server does not show A live and B in the trash, or the ops after the app's
    /// cursor are not B's trash alone (`seq` is B's trash op when found, `other_ops` counts the rest): the run stops
    /// and the lane reports.
    Unexpected {
        a_live: bool,
        b_trashed: bool,
        seq: Option<i64>,
        #[serde(default)]
        other_ops: usize,
    },
    /// B's row forgotten, no call: B is gone or in the trash already.
    Forgotten,
    /// B's row forgotten, the trash failed (or its answer was lost). `apply-one` on the pair again re-checks it, and
    /// sends the trash again only while B is still live (review F2).
    TrashFailed { status: Option<u16> },
    /// B's row forgotten and the trash sent, but A, B or the op log could not be read back (`status`, when the server
    /// answered): `apply-one` on the pair again reads them back, without a second trash.
    Unconfirmed { status: Option<u16> },
    /// Nothing was changed.
    Skipped { reason: Reason },
}

/// Why a pair was skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Reason {
    /// The pair is marked "none".
    NotMarked,
    /// The pair was applied already.
    AlreadyDone,
    /// The rows or the file at the path differ from the list (spec A5).
    ChangedOnThisPc,
    /// What the server says about A or B differs from the server read (spec A5).
    ChangedOnServer,
    /// The server holds ops after the app's cursor that the app has not applied: start the app, let it sync, stop it,
    /// and run again (review F3).
    PendingOps,
    /// The app's cursor moved since the try that failed: stop and report (review F3).
    AppRanSinceTheTry,
}

/// The run file in the out dir on this PC: every pair, what was marked and what was done.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RunFile {
    pub pairs: Vec<Pair>,
    /// L0: paths with more than two rows, counted, never listed.
    pub more_than_two: usize,
    /// OP3: the server's trash retention in days, as the person read it in the admin panel (`retention <days>`).
    /// `apply-one` refuses until it is recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_days: Option<u32>,
}

impl RunFile {
    /// Read the run file. A mistake in it is reported by line and column only, never with what it holds (I5). A pair
    /// changed in more than its action does not load (review F1); the error names the pair only.
    pub(crate) fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|_| anyhow::anyhow!("The run file could not be read."))?;
        let run: Self = serde_json::from_str(&text).map_err(|error| {
            anyhow::anyhow!(
                "The run file is not readable as written (line {}, column {}). An action is none, trash-b or forget-b; \
                 nothing was changed.",
                error.line(),
                error.column()
            )
        })?;
        if let Some(pair) = run.pairs.iter().find(|pair| pair.seal != pair.sealed()) {
            anyhow::bail!(
                "pair {}: the run file was changed in more than the action, the only field marked by hand; nothing was \
                 changed. Undo that edit.",
                pair.n
            );
        }
        Ok(run)
    }

    /// Write the run file, each pair sealed: a new file next to it, then a rename, so a crash never leaves half a file.
    pub(crate) fn save(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let mut sealed = self.clone();
        for pair in &mut sealed.pairs {
            pair.seal = pair.sealed();
        }
        let next = path.with_extension("json.next");
        std::fs::write(&next, serde_json::to_vec_pretty(&sealed)?)?;
        std::fs::rename(&next, path)?;
        Ok(())
    }

    /// Spec T11: the pairs are numbered 1 to N, each once, and every mark is one its class allows. Checked before any
    /// write; a run file that fails is refused whole.
    pub(crate) fn check(&self) -> anyhow::Result<()> {
        for (index, pair) in self.pairs.iter().enumerate() {
            if usize::try_from(pair.n).ok() != Some(index + 1) {
                anyhow::bail!(
                    "The run file's pairs are not numbered 1 to {} in order (entry {} holds pair {}); nothing was changed.",
                    self.pairs.len(),
                    index + 1,
                    pair.n
                );
            }
            if !pair.class.allows(pair.action) {
                anyhow::bail!(
                    "pair {}: {} is not an action for a pair of class {}; nothing was changed.",
                    pair.n,
                    kebab(&pair.action),
                    kebab(&pair.class)
                );
            }
        }
        Ok(())
    }
}

/// Phase 1b: one file as `GET /api/v1/files/{id}` answers (spec S3, S8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ServerFacts {
    pub trashed: bool,
    /// `trashed_at`, for a file in the trash.
    #[serde(default)]
    pub trashed_at: Option<String>,
    pub parent_id: Option<String>,
    pub size_bytes: i64,
    pub updated_at: String,
    /// `download_version.version_number`: the current version, 0 when the server names none.
    pub version_number: i64,
    /// `acted_via_share_id` is set.
    pub via_share: bool,
    /// `chunk_count`: how many chunks the compare downloads.
    pub chunk_count: i64,
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

/// A value's name as the run file spells it.
pub(crate) fn kebab(value: &impl Serialize) -> String {
    serde_json::to_value(value).ok().and_then(|value| value.as_str().map(str::to_string)).unwrap_or_default()
}
