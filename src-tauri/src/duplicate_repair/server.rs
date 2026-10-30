//! Task 2274, spec phase 1b: the server's facts about A and B (rows L4-L9 and the L8 amendment). Every call goes
//! through [`RepairApi`], the tool's one way to the server (spec I3, T14). No response is logged or written: the
//! link and invite lists are counted and dropped. An error names the pair and the HTTP status, never an id, the
//! address or the server's own text (I5).
use super::model::{Class, Pair, ServerFacts, ServerFile, ServerRead};
use crate::api_client::ApiClient;
use std::collections::HashMap;

/// An invite in one of these states still shares the file (the server's own set, `invites.rs` ~198).
const LIVE_INVITE: [&str; 3] = ["invited", "claimed", "approved"];

/// The tool's only way to the server: exactly the calls spec I3 allows.
pub(crate) struct RepairApi {
    api: ApiClient,
}

/// One op of the server's log: its seq, its type and the id its payload names (a file's or a folder's).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Op {
    pub(super) seq: i64,
    pub(super) kind: String,
    pub(super) id: Option<String>,
}

/// A failed call: the HTTP status when the server answered, nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Failure(pub(super) Option<u16>);

impl Failure {
    fn of(error: &anyhow::Error) -> Self {
        // `ApiClient::check_status` reads "HTTP {status}: {body}"; only the number is kept.
        let text = error.to_string();
        Self(text.strip_prefix("HTTP ").and_then(|rest| rest.get(..3)).and_then(|code| code.parse().ok()))
    }

    pub(super) fn at(self, pair: u32) -> anyhow::Error {
        match self.0 {
            Some(status) => anyhow::anyhow!("pair {pair}: the server answered HTTP {status}; nothing was changed"),
            None => anyhow::anyhow!("pair {pair}: no usable answer from the server; nothing was changed"),
        }
    }
}

impl RepairApi {
    pub(crate) fn new(api: ApiClient) -> Self {
        Self { api }
    }

    /// `GET /api/v1/files/{id}`: `Gone` on a 404.
    pub(super) async fn file(&self, file_id: &str) -> Result<ServerFile, Failure> {
        let json = match self.api.repair_file(file_id).await {
            Ok(json) => json,
            Err(error) if Failure::of(&error) == Failure(Some(404)) => return Ok(ServerFile::Gone),
            Err(error) => return Err(Failure::of(&error)),
        };
        let facts = (|| {
            Some(ServerFacts {
                trashed: json.get("is_trashed")?.as_bool()?,
                trashed_at: json.get("trashed_at").and_then(|at| at.as_str()).map(str::to_string),
                parent_id: json.get("parent_id").and_then(|id| id.as_str()).map(str::to_string),
                size_bytes: json.get("size_bytes")?.as_i64()?,
                updated_at: json.get("updated_at")?.as_str()?.to_string(),
                version_number: json.pointer("/download_version/version_number").and_then(|n| n.as_i64()).unwrap_or(0),
                via_share: json.get("acted_via_share_id").is_some_and(|id| !id.is_null()),
                chunk_count: json.get("chunk_count")?.as_i64()?,
                is_folder: json.get("is_folder")?.as_bool()?,
            })
        })();
        facts.map(ServerFile::Found).ok_or(Failure(None))
    }

    /// `GET /api/v1/files/{id}/chunks/{index}`: one encrypted chunk of the current version.
    pub(super) async fn chunk(&self, file_id: &str, index: u32) -> Result<Vec<u8>, Failure> {
        self.api.repair_chunk(file_id, index).await.map_err(|error| Failure::of(&error))
    }

    /// The key of a file this account owns, derived from the session's master key (no call). Only owned files
    /// reach the compare: a shared B is your call (L8).
    pub(super) fn file_key(&self, file_id: &str) -> beebeeb_core::kdf::FileKey {
        let master = beebeeb_core::kdf::MasterKey::from_bytes(*self.api.master_key());
        beebeeb_core::kdf::derive_file_key(&master, file_id.as_bytes())
    }

    /// `GET /api/v1/shares/by-file/{id}`: how many of the owner's links of the file have not expired.
    async fn live_links(&self, file_id: &str) -> Result<usize, Failure> {
        let json = self.api.repair_links(file_id).await.map_err(|error| Failure::of(&error))?;
        let rows = json.get("shares").and_then(|rows| rows.as_array()).ok_or(Failure(None))?;
        Ok(rows.iter().filter(|row| not_expired(row)).count())
    }

    /// `GET /api/v1/sync/ops?since=`: the first page of ops after `since` (the server's page holds 1,000, `sync.rs`
    /// ~292). `apply-one` asks only whether there are none, or B's trash alone (review F3), which one page answers.
    /// Seqs, types and ids are kept; the rest of the log is dropped.
    pub(super) async fn ops_after(&self, since: i64) -> Result<Vec<Op>, Failure> {
        let page = self.api.repair_ops(since).await.map_err(|error| Failure::of(&error))?;
        Ok(page
            .ops
            .iter()
            .map(|op| Op { seq: op.seq_id, kind: op.op_type.clone(), id: op.payload["id"].as_str().map(str::to_string) })
            .collect())
    }

    /// `DELETE /api/v1/files/{id}`: the soft trash (spec S1). The tool has no other write.
    pub(super) async fn trash(&self, file_id: &str) -> Result<(), Failure> {
        self.api.repair_trash(file_id).await.map_err(|error| Failure::of(&error))
    }

    /// `GET /api/v1/shares/invites/sent`: how many live invites name each of `file_ids`. Read once a run.
    async fn live_invites(&self, file_ids: &[&str]) -> Result<HashMap<String, usize>, Failure> {
        let json = self.api.repair_invites().await.map_err(|error| Failure::of(&error))?;
        let rows = json.get("invites").and_then(|rows| rows.as_array()).ok_or(Failure(None))?;
        let mut live = HashMap::new();
        for row in rows {
            let Some(file_id) = row.get("file_id").and_then(|id| id.as_str()) else { continue };
            let status = row.get("status").and_then(|status| status.as_str()).unwrap_or_default();
            if file_ids.contains(&file_id) && LIVE_INVITE.contains(&status) && not_expired(row) {
                *live.entry(file_id.to_string()).or_insert(0) += 1;
            }
        }
        Ok(live)
    }
}

/// `expires_at` null, missing, unreadable or in the future (an unreadable time counts as live: L8 then asks).
fn not_expired(row: &serde_json::Value) -> bool {
    match row.get("expires_at").and_then(|at| at.as_str()) {
        None => true,
        Some(at) => chrono::DateTime::parse_from_rfc3339(at).map_or(true, |at| at > chrono::Utc::now()),
    }
}

/// Rows L4-L9 for every pair the local rows left `Unknown`; the rest are not read. One call at a time.
pub(crate) async fn server_read(api: &RepairApi, pairs: &mut [Pair]) -> anyhow::Result<()> {
    let wanted: Vec<String> =
        pairs.iter().filter(|pair| pair.class == Class::Unknown).map(|pair| pair.b.file_id.clone()).collect();
    let wanted: Vec<&str> = wanted.iter().map(String::as_str).collect();
    let mut invites: Option<HashMap<String, usize>> = None;
    for pair in pairs.iter_mut().filter(|pair| pair.class == Class::Unknown) {
        let n = pair.n;
        let a = api.file(&pair.a.file_id).await.map_err(|failure| failure.at(n))?;
        let mut read = ServerRead { a: a.clone(), b: None, live_links: None, live_invites: None };
        let class = match a {
            // Review P2: a folder is never half of a file pair; B is not read.
            ServerFile::Found(a) if a.is_folder => Class::NotAFile,
            ServerFile::Gone => Class::AGone,
            ServerFile::Found(a) if a.trashed => Class::AGone,
            ServerFile::Found(a) => {
                let b = api.file(&pair.b.file_id).await.map_err(|failure| failure.at(n))?;
                read.b = Some(b.clone());
                match b {
                    ServerFile::Found(b) if b.is_folder => Class::NotAFile,
                    ServerFile::Gone => Class::BGone,
                    ServerFile::Found(b) if b.trashed => Class::BInTrash,
                    ServerFile::Found(b) if b.parent_id != a.parent_id => Class::Moved,
                    // Review P2: the links call lists only the owner's links and answers 404 for a B in a folder shared
                    // with this account, so what makes B your call without it is decided first, and it is not sent.
                    ServerFile::Found(b) if pair.b.shared || b.via_share || b.version_number > 1 => Class::YourCall,
                    ServerFile::Found(b) => {
                        let links = api.live_links(&pair.b.file_id).await.map_err(|failure| failure.at(n))?;
                        if invites.is_none() {
                            invites = Some(api.live_invites(&wanted).await.map_err(|failure| failure.at(n))?);
                        }
                        let invited = invites.as_ref().and_then(|live| live.get(&pair.b.file_id)).copied().unwrap_or(0);
                        read.live_links = Some(links);
                        read.live_invites = Some(invited);
                        if links > 0 || invited > 0 {
                            Class::YourCall
                        } else if b.size_bytes != a.size_bytes {
                            Class::DiffersBySize
                        } else {
                            Class::Unknown
                        }
                    }
                }
            }
        };
        pair.server = Some(read);
        pair.class = class;
    }
    Ok(())
}
