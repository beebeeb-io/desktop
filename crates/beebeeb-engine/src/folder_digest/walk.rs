//! The top-down walker over `GET /api/v1/folder-digests` (see the module doc of `folder_digest` for the root and
//! the after-read rule).
//!
//! Owns: paging a folder (every page at [`PAGE_LIMIT`]), starting it again when its digest moves between pages
//! (at most [`MAX_RESTARTS`] times), and the walk order (parents before children, a child folder whose digest the
//! caller knows is never entered). Must not: store a digest (the caller does, after its own check) or log an id.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::api_client::{ApiClient, DigestsAnswer};

/// The page size of every full read: the endpoint's maximum.
pub const PAGE_LIMIT: u32 = 1000;

/// How many times one folder read starts again when the digest moves between its pages.
pub const MAX_RESTARTS: u32 = 3;

/// How many digest requests a walk made. Every request the walker sends adds 1; callers add their own listing
/// requests to it (`add`), so a pass can log its whole request count.
#[derive(Default, Debug)]
pub struct RequestCount(AtomicU32);

impl RequestCount {
    pub fn get(&self) -> u32 {
        self.0.load(Ordering::Relaxed)
    }

    /// Count `n` more requests (a caller's own listing requests).
    pub fn add(&self, n: u32) {
        self.0.fetch_add(n, Ordering::Relaxed);
    }
}

/// Folder id -> the digest hex the caller trusts for that folder's subtree (`""` = the account root).
pub type Known = HashMap<String, String>;

/// One folder as one consistent read of all its pages gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderDigests {
    /// The folder's id (`""` = the account root).
    pub id: String,
    /// Its digest, 64 lowercase hex.
    pub digest: String,
    pub child_count: i64,
    /// Every child folder: (its id, its digest), in the server's order (id ascending).
    pub child_folders: Vec<(String, String)>,
}

/// Why a read or walk stopped.
#[derive(Debug)]
pub enum WalkError {
    /// The server has no digests: list the old way.
    Unsupported,
    /// The account's digests are not computed yet: try again after this many seconds.
    NotReady { retry_after_secs: u64 },
    /// The folder with this id is gone (unknown, trashed, a file, or no longer readable).
    FolderGone(String),
    /// Anything else: transport, a refusal, an answer that does not read, a digest that kept moving.
    Failed(anyhow::Error),
}

impl std::fmt::Display for WalkError {
    // No id here: this text may reach a log line.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WalkError::Unsupported => f.write_str("the server has no folder digests"),
            WalkError::NotReady { retry_after_secs } => {
                write!(f, "folder digests not ready (retry after {retry_after_secs} s)")
            }
            WalkError::FolderGone(_) => f.write_str("a folder is gone"),
            WalkError::Failed(error) => write!(f, "folder digests failed: {error:#}"),
        }
    }
}

impl std::error::Error for WalkError {}

fn as_param(folder_id: &str) -> Option<&str> {
    (!folder_id.is_empty()).then_some(folder_id)
}

/// One request, counted, with the non-page answers as errors.
async fn fetch_page(
    api: &ApiClient,
    folder_id: &str,
    cursor: Option<&str>,
    limit: u32,
    count: &RequestCount,
) -> Result<beebeeb_types::folder_digest::FolderDigestPage, WalkError> {
    count.add(1);
    match api.folder_digests(as_param(folder_id), cursor, limit).await {
        Ok(DigestsAnswer::Page(page)) => Ok(page),
        Ok(DigestsAnswer::FolderGone) => Err(WalkError::FolderGone(folder_id.to_string())),
        Ok(DigestsAnswer::NotReady { retry_after_secs }) => Err(WalkError::NotReady { retry_after_secs }),
        Ok(DigestsAnswer::Unsupported) => Err(WalkError::Unsupported),
        Err(error) => Err(WalkError::Failed(error)),
    }
}

/// Read every page of `folder_id` at [`PAGE_LIMIT`]. When a page carries another digest than the first page of the
/// same read, the folder changed under the read: start it again, at most [`MAX_RESTARTS`] times, then
/// `Failed`.
pub async fn read_folder(api: &ApiClient, folder_id: &str, count: &RequestCount) -> Result<FolderDigests, WalkError> {
    let mut restarts = 0u32;
    'read: loop {
        let mut cursor: Option<String> = None;
        let mut out: Option<FolderDigests> = None;
        loop {
            let page = fetch_page(api, folder_id, cursor.as_deref(), PAGE_LIMIT, count).await?;
            let folder = out.get_or_insert_with(|| FolderDigests {
                id: folder_id.to_string(),
                digest: page.digest.clone(),
                child_count: page.child_count,
                child_folders: Vec::new(),
            });
            if page.digest != folder.digest {
                if restarts >= MAX_RESTARTS {
                    return Err(WalkError::Failed(anyhow::anyhow!(
                        "a folder's digest moved between pages {} times in a row",
                        restarts + 1
                    )));
                }
                restarts += 1;
                tracing::debug!(restarts, "folder digest moved between pages; reading the folder again");
                continue 'read;
            }
            for child in page.children.into_iter().filter(|c| c.is_folder) {
                let Some(digest) = child.digest else {
                    return Err(WalkError::Failed(anyhow::anyhow!(
                        "a child folder came without a digest"
                    )));
                };
                folder.child_folders.push((child.id.to_string(), digest));
            }
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => return Ok(out.expect("a first page was read")),
            }
        }
    }
}

/// The folder's digest now (`limit=0`, one request).
pub async fn digest_now(api: &ApiClient, folder_id: &str, count: &RequestCount) -> Result<String, WalkError> {
    Ok(fetch_page(api, folder_id, None, 0, count).await?.digest)
}

/// Every folder under `root` (itself included) whose digest differs from `known` or is not in it, read whole,
/// parents before children. A child folder whose digest (as its parent's read gave it) equals `known` is not
/// entered. When `root` is in `known`, its digest is asked first (`limit=0`): unchanged costs 1 request and
/// returns an empty list. A child folder that is gone by the time it is read is skipped (its parent's digest moved,
/// so the after-read rule keeps the parent untrusted); `root` gone is `FolderGone`.
pub async fn changed_folders(
    api: &ApiClient,
    root: &str,
    known: &Known,
    count: &RequestCount,
) -> Result<Vec<FolderDigests>, WalkError> {
    if let Some(trusted) = known.get(root)
        && digest_now(api, root, count).await? == *trusted
    {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut queue = VecDeque::from([read_folder(api, root, count).await?]);
    let mut skipped_gone = 0u32;
    while let Some(folder) = queue.pop_front() {
        for (child, digest) in &folder.child_folders {
            if known.get(child) == Some(digest) {
                continue;
            }
            match read_folder(api, child, count).await {
                Ok(read) => queue.push_back(read),
                Err(WalkError::FolderGone(_)) => skipped_gone += 1,
                Err(error) => return Err(error),
            }
        }
        out.push(folder);
    }
    tracing::debug!(
        changed = out.len(),
        skipped_gone,
        requests = count.get(),
        "folder digest walk finished"
    );
    Ok(out)
}
