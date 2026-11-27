//! The sync drift check (folder digests, task 13): find a server change no op told this PC about.
//!
//! Sync follows the server's op log. A server write that records no op (an admin restore, a janitor, an erasure, a
//! pool delete) leaves the mirror wrong until a full re-snapshot. This check is started by the idle pull (the delta
//! came back empty at the head), at most once per [`CHECK_EVERY`], in the person's own files only. It runs beside
//! the tick, never inside it: the tick holds the pull lock a save waits for.
//!
//! 1. When the root's digest is stored and still holds, the check ends there: one request.
//! 2. Otherwise it descends top down from the root (a Merkle descent): one folder's digest page gives the digests of
//!    all its child folders in one read, and only a child whose digest differs from the one stored in
//!    `remote_folder_digests` (or was never stored) is entered, level by level, down to the folders that changed. A
//!    child the page gives no digest for is entered and listed. Children stored and now changed (or watched) go
//!    before children never stored, so a change since a verified state is found first. A folder is checked once every
//!    changed folder under it is (deepest first), and at most [`FOLDERS_PER_CHECK`] folders are checked per window: the
//!    next window goes on from what is stored. A change `d` levels down costs about `2 * d` requests.
//! 3. A checked folder is listed (the engine's listing, `prepare_owned_rows` and the stored or decrypted name) and
//!    its direct children compared with the mirror's rows under it: ids, kind, size, version
//!    (`download_version.version_number`, not against a mirror row at version 0) and object version id (only when the
//!    mirror has one: the snapshot never sends it), and the name against the mirror path's leaf.
//! 4. A child that is uploading (on the server or here), in error or conflict, listed while the mirror trashes it, or
//!    that has a local op queued, makes the folder in flux: not stored, not counted. A mirror row in `Trashing` the
//!    server does not list is the server's trash, not drift. A server folder the mirror lacks that is a system folder
//!    (`system_role` or `managed_by`) is skipped: sync may leave those out.
//! 5. A folder that agrees, whose changed subfolders all agreed, is stored at the end of the check by the after-read
//!    rule: one `digest_now` of the root, still the digest the descent read, holds for every folder read; when the root
//!    moved, each folder is asked on its own. A folder whose subtree holds
//!    a disagreeing or in-flux folder is not stored, so the next walk enters it again.
//! 6. A folder that disagrees is drift when the next check still finds it disagreeing and no op touched it in
//!    between: the sync cursor did not move, or the mirror's rows under it are what they were. Then a re-snapshot is
//!    asked for (`StateDb::request_resnapshot`) and the stored digests are cleared, so the check after it reads every
//!    folder again.
//!
//! A server without digests (`Unsupported`) is not asked again in this process; `NotReady` and failures wait for the
//! next window. One log line per check, counts only: never an id or a name.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::*;
use crate::api_client::{ApiClient, DigestsAnswer};
use crate::folder_digest::{digest_now, RequestCount, WalkError, MAX_RESTARTS, PAGE_LIMIT};
use crate::state_db::folder_digests::DriftChild;

/// How often the check runs at most.
pub(super) const CHECK_EVERY: Duration = Duration::from_secs(15 * 60);

/// How many folders one check lists at most (two requests each: its digest page and its listing). An idle tree costs
/// one request; this bounds the first check after a sign-in or a re-snapshot, which reads every folder once, so a tree
/// of a few thousand folders is verified in one check and a change after it is found by the next.
pub(super) const FOLDERS_PER_CHECK: usize = 5_000;

/// The pause between two checked folders, so a check never sends its requests back to back (none in tests).
const PACE: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(25)
};

/// More rows than this in one folder listing is refused (a cursor loop, not a folder).
const LISTING_CAP: usize = 200_000;

/// What one check did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DriftAction {
    /// Nothing changed since the stored digests.
    Unchanged,
    /// Every checked folder agreed (or was in flux); what agreed is stored.
    Agreed,
    /// A folder disagrees for the first time: the next check decides.
    Watching,
    /// A folder disagreed twice with no op on it in between: a re-snapshot is asked for.
    Resnapshot,
    /// The server has no digests: no check for the rest of this run.
    Unsupported,
    /// The account's digests are not computed yet: the next window.
    NotReady,
    /// A request or a read failed, or the engine is stopping: the next window.
    Failed,
}

impl DriftAction {
    fn as_str(self) -> &'static str {
        match self {
            DriftAction::Unchanged => "unchanged",
            DriftAction::Agreed => "agreed",
            DriftAction::Watching => "watching",
            DriftAction::Resnapshot => "resnapshot",
            DriftAction::Unsupported => "unsupported",
            DriftAction::NotReady => "not_ready",
            DriftAction::Failed => "failed",
        }
    }
}

/// One check's counts (what its log line says).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DriftReport {
    pub requests: u32,
    /// Folders checked (listed and compared).
    pub changed: usize,
    pub in_flux: usize,
    pub disagreeing: usize,
    pub stored: usize,
    /// The cap ended the check before the walk did: the next window goes on.
    pub capped: bool,
    pub action: DriftAction,
}

impl DriftReport {
    fn new(action: DriftAction) -> Self {
        DriftReport {
            requests: 0,
            changed: 0,
            in_flux: 0,
            disagreeing: 0,
            stored: 0,
            capped: false,
            action,
        }
    }
}

/// A folder that disagreed at a check: the sync cursor then, and what the mirror held under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Seen {
    cursor: Option<i64>,
    mirror: u64,
}

/// The folders that disagreed at the last check.
type Watching = HashMap<String, Seen>;

/// The check's memory for one sync root, for the life of this process.
#[derive(Default)]
struct RootState {
    last_started: Option<Instant>,
    running: bool,
    unsupported: bool,
    watching: Option<Watching>,
}

fn states() -> &'static Mutex<HashMap<PathBuf, RootState>> {
    static STATES: OnceLock<Mutex<HashMap<PathBuf, RootState>>> = OnceLock::new();
    STATES.get_or_init(Default::default)
}

fn with_state<R>(sync_root: &Path, f: impl FnOnce(&mut RootState) -> R) -> R {
    let mut all = states().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    f(all.entry(sync_root.to_path_buf()).or_default())
}

/// A check in progress for one root: the root is marked running while this lives, also when the check is dropped
/// half-way (the engine stopping, the runtime shutting down).
struct Running(PathBuf);

impl Running {
    fn start(sync_root: &Path) -> Self {
        with_state(sync_root, |state| {
            state.running = true;
            state.last_started = Some(Instant::now());
        });
        Running(sync_root.to_path_buf())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        with_state(&self.0, |state| state.running = false);
    }
}

/// Called from the idle branch of the pull: starts the check when its window is due, beside the tick (the tick
/// returns at once and its pull lock with it). Never fails the tick. The first idle tick of a run only starts the
/// window: the first check runs one window later, so idle ticks right after start stay one `/sync/ops` request each.
pub(super) async fn after_idle_pull(bridge: &EngineBridge, sync_root: &Path) {
    let due = with_state(sync_root, |state| {
        let Some(started) = state.last_started else {
            state.last_started = Some(Instant::now());
            return false;
        };
        !state.unsupported && !state.running && started.elapsed() >= CHECK_EVERY
    });
    if !due {
        return;
    }
    let running = Running::start(sync_root);
    match tokio::runtime::Handle::try_current() {
        Ok(runtime) => {
            // The same database, client and stop flag; its own caches.
            let beside = EngineBridge::new_with_stop_flag(
                bridge.identity,
                bridge.db.clone(),
                bridge.api.clone(),
                bridge.stopping.clone(),
            );
            let root = sync_root.to_path_buf();
            runtime.spawn(async move {
                run_check(&beside, &root, FOLDERS_PER_CHECK).await;
                drop(running);
            });
        }
        Err(_) => {
            run_check(bridge, sync_root, FOLDERS_PER_CHECK).await;
            drop(running);
        }
    }
}

/// One check now, whatever the window; it starts the next window. A server found without digests is not asked
/// again.
pub(crate) async fn check_now(bridge: &EngineBridge, sync_root: &Path) -> DriftReport {
    let _running = Running::start(sync_root);
    run_check(bridge, sync_root, FOLDERS_PER_CHECK).await
}

/// One check now that lists at most `cap` folders.
#[cfg(test)]
pub(crate) async fn check_capped(bridge: &EngineBridge, sync_root: &Path, cap: usize) -> DriftReport {
    let _running = Running::start(sync_root);
    run_check(bridge, sync_root, cap).await
}

async fn run_check(bridge: &EngineBridge, sync_root: &Path, cap: usize) -> DriftReport {
    let (unsupported, watching) = with_state(sync_root, |state| (state.unsupported, state.watching.take()));
    let (report, watching) = if unsupported {
        (DriftReport::new(DriftAction::Unsupported), None)
    } else {
        match check(bridge, watching.clone(), cap).await {
            Ok(done) => done,
            Err(error) => {
                tracing::warn!(error = %error, "sync drift check: a read failed; the next window tries again");
                (DriftReport::new(DriftAction::Failed), watching)
            }
        }
    };
    with_state(sync_root, |state| {
        state.watching = watching;
        if report.action == DriftAction::Unsupported {
            state.unsupported = true;
        }
    });
    if !unsupported {
        tracing::info!(
            requests = report.requests,
            changed = report.changed,
            in_flux = report.in_flux,
            disagreeing = report.disagreeing,
            stored = report.stored,
            capped = report.capped,
            action = report.action.as_str(),
            "sync drift check"
        );
    }
    report
}

/// How one listed folder compares with the mirror.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Agrees,
    InFlux,
    /// With what the mirror held under the folder (its rows' fingerprint).
    Disagrees(u64),
}

/// One folder's digest page, every page of it read at one digest: the folder's digest and each child folder with
/// the digest its parent's page gave it (`None`: the page carried none, so that child is read and listed as changed).
struct DigestRead {
    id: String,
    digest: String,
    child_folders: Vec<(String, Option<String>)>,
}

/// Read every page of `folder_id` (`""` = the root) at [`PAGE_LIMIT`], each counted. A digest that moves between pages
/// starts the folder again, at most [`MAX_RESTARTS`] times. A child folder without a digest is kept, as unknown: the
/// descent lists that one folder instead of failing the check.
async fn read_digests(api: &ApiClient, folder_id: &str, count: &RequestCount) -> Result<DigestRead, WalkError> {
    let param = (!folder_id.is_empty()).then_some(folder_id);
    let mut restarts = 0u32;
    'read: loop {
        let mut cursor: Option<String> = None;
        let mut out: Option<DigestRead> = None;
        loop {
            count.add(1);
            let page = match api.folder_digests(param, cursor.as_deref(), PAGE_LIMIT).await {
                Ok(DigestsAnswer::Page(page)) => page,
                Ok(DigestsAnswer::FolderGone) => return Err(WalkError::FolderGone(folder_id.to_string())),
                Ok(DigestsAnswer::NotReady { retry_after_secs }) => {
                    return Err(WalkError::NotReady { retry_after_secs })
                }
                Ok(DigestsAnswer::Unsupported) => return Err(WalkError::Unsupported),
                Err(error) => return Err(WalkError::Failed(error)),
            };
            let folder = out.get_or_insert_with(|| DigestRead {
                id: folder_id.to_string(),
                digest: page.digest.clone(),
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
                continue 'read;
            }
            folder.child_folders.extend(
                page.children
                    .into_iter()
                    .filter(|child| child.is_folder)
                    .map(|child| (child.id.to_string(), child.digest)),
            );
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => return Ok(out.expect("a first page was read")),
            }
        }
    }
}

/// The child folders of `folder` the descent enters, in order: first those this PC stored a digest for that now
/// differs, or that disagreed at the last check (a change since a verified state: drift is found there), then those
/// never stored. A child whose digest equals the stored one is not entered: nothing under it changed.
fn to_enter(folder: &DigestRead, known: &HashMap<String, String>, watched: &Watching) -> Vec<String> {
    let mut suspects = Vec::new();
    let mut fresh = Vec::new();
    for (child, digest) in &folder.child_folders {
        match (known.get(child), digest) {
            (Some(stored), Some(now)) if stored == now => {}
            (Some(_), _) => suspects.push(child.clone()),
            (None, _) if watched.contains_key(child) => suspects.push(child.clone()),
            (None, _) => fresh.push(child.clone()),
        }
    }
    suspects.extend(fresh);
    suspects
}

/// A folder of the descent whose changed child folders are not all checked yet.
struct Frame {
    folder: DigestRead,
    enter: Vec<String>,
    next_child: usize,
    /// Every changed child folder checked so far agreed, whole.
    whole: bool,
    /// The changed child folders checked (each waits in `pending`).
    checked: Vec<String>,
}

impl Frame {
    fn new(folder: DigestRead, known: &HashMap<String, String>, watched: &Watching) -> Self {
        let enter = to_enter(&folder, known, watched);
        Frame {
            folder,
            enter,
            next_child: 0,
            whole: true,
            checked: Vec::new(),
        }
    }
}

/// A folder that agrees with its whole changed subtree, waiting for the after-read rule before its digest is stored.
struct Pending {
    id: String,
    digest: String,
    checked: Vec<String>,
}

/// The top-down descent. The root's digest first: equal to the stored one, the check ends (one request). Otherwise the
/// root's page gives its child folders' digests in one read; only a child whose digest differs from the stored one
/// (or was never stored) is entered, level by level, so a change `d` levels down costs about `2 * d` requests (one
/// digest page and one listing per folder on its path). A folder is compared with the mirror once every changed
/// folder under it is (deepest first). What agrees is stored at the end, under one after-read of the root.
async fn check(
    bridge: &EngineBridge,
    watching: Option<Watching>,
    cap: usize,
) -> anyhow::Result<(DriftReport, Option<Watching>)> {
    let count = RequestCount::default();
    let cursor = bridge.db().get_sync_cursor()?;
    let known = bridge.db().remote_folder_digests()?;
    let watched = watching.clone().unwrap_or_default();
    let ended = |action: DriftAction, count: &RequestCount| {
        let mut report = DriftReport::new(action);
        report.requests = count.get();
        report
    };
    // What a walk error means for this check.
    let stop = |error: WalkError, count: &RequestCount, watching: Option<Watching>| match error {
        WalkError::Unsupported => (ended(DriftAction::Unsupported, count), None),
        WalkError::NotReady { .. } => (ended(DriftAction::NotReady, count), watching),
        error => {
            tracing::warn!(error = %error, "sync drift check: the walk failed; the next window tries again");
            (ended(DriftAction::Failed, count), watching)
        }
    };
    if let Some(trusted) = known.get("") {
        match digest_now(bridge.api(), "", &count).await {
            // Nothing moved since the digests stored: what disagreed then is not seen again, so nothing is watched.
            Ok(now) if now == *trusted => return Ok((ended(DriftAction::Unchanged, &count), None)),
            Ok(_) => {}
            Err(error) => return Ok(stop(error, &count, watching)),
        }
    }
    let root = match read_digests(bridge.api(), "", &count).await {
        Ok(root) => root,
        Err(error) => return Ok(stop(error, &count, watching)),
    };
    let root_digest = root.digest.clone();

    let mut report = DriftReport::new(DriftAction::Agreed);
    let mut disagreeing: Watching = HashMap::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut pending: Vec<Pending> = Vec::new();
    let mut forget: Vec<String> = Vec::new();
    let mut failure: Option<WalkError> = None;
    let mut stack = vec![Frame::new(root, &known, &watched)];
    while let Some(top) = stack.last_mut() {
        if bridge.is_stopping() {
            report.action = DriftAction::Failed;
            break;
        }
        // Depth first: the next changed child folder of the deepest open folder.
        if top.next_child < top.enter.len() {
            if report.changed >= cap {
                report.capped = true;
                break;
            }
            let child = top.enter[top.next_child].clone();
            top.next_child += 1;
            match read_digests(bridge.api(), &child, &count).await {
                Ok(folder) => stack.push(Frame::new(folder, &known, &watched)),
                // Gone since its parent was read: the parent's digest moved, so the parent is not stored.
                Err(WalkError::FolderGone(_)) => top.whole = false,
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
            continue;
        }
        // Every changed folder under it is checked: check this one.
        let frame = stack.pop().expect("the top frame");
        if report.changed >= cap {
            report.capped = true;
            break;
        }
        if report.changed > 0 {
            tokio::time::sleep(PACE).await;
        }
        let verdict = compare_folder(bridge, &frame.folder.id, &count).await?;
        report.changed += 1;
        visited.insert(frame.folder.id.clone());
        match verdict {
            Verdict::Agrees => {}
            Verdict::InFlux => report.in_flux += 1,
            Verdict::Disagrees(mirror) => {
                disagreeing.insert(frame.folder.id.clone(), Seen { cursor, mirror });
            }
        }
        let storable = frame.whole && verdict == Verdict::Agrees;
        if storable {
            pending.push(Pending {
                id: frame.folder.id.clone(),
                digest: frame.folder.digest.clone(),
                checked: frame.checked,
            });
        } else if known.contains_key(&frame.folder.id) {
            forget.push(frame.folder.id.clone());
        }
        if let Some(parent) = stack.last_mut() {
            parent.whole &= storable;
            if storable {
                parent.checked.push(frame.folder.id);
            }
        }
    }
    if !bridge.is_stopping() {
        report.stored = store_verified(bridge, &root_digest, pending, forget, &known, &count).await?;
    }
    report.disagreeing = disagreeing.len();
    report.requests = count.get();
    if let Some(error) = failure {
        let (mut ended, watching) = stop(error, &count, watching);
        ended.changed = report.changed;
        ended.in_flux = report.in_flux;
        ended.disagreeing = report.disagreeing;
        ended.stored = report.stored;
        return Ok((ended, watching));
    }

    let previous = watching.unwrap_or_default();
    // Drift: a folder that disagreed last time and again now, with no op on it in between.
    let drift = disagreeing.iter().any(|(folder, now)| {
        previous
            .get(folder)
            .is_some_and(|then| then.cursor == now.cursor || then.mirror == now.mirror)
    });
    if drift {
        report.action = DriftAction::Resnapshot;
        bridge.db().request_resnapshot()?;
        bridge.db().clear_remote_folder_digests()?;
        report.requests = count.get();
        return Ok((report, None));
    }
    let mut next = disagreeing;
    if report.capped || report.action == DriftAction::Failed {
        // Not every folder was reached: what was watched and not seen this time is still watched.
        for (folder, seen) in previous {
            if !visited.contains(&folder) {
                next.entry(folder).or_insert(seen);
            }
        }
    }
    if report.action != DriftAction::Failed && report.disagreeing > 0 {
        report.action = DriftAction::Watching;
    }
    Ok((report, (!next.is_empty()).then_some(next)))
}

/// Store the folders that agreed (deepest first in `pending`) by the after-read rule, and forget the stored digests of
/// those that did not. One read of the root's digest first: unmoved since the descent read it, nothing anywhere in
/// the tree changed while the check read it, so every folder holds. Moved: each folder is asked on its own, and one
/// is stored only when it holds and every changed child folder under it was stored.
async fn store_verified(
    bridge: &EngineBridge,
    root_digest: &str,
    pending: Vec<Pending>,
    mut forget: Vec<String>,
    known: &HashMap<String, String>,
    count: &RequestCount,
) -> anyhow::Result<usize> {
    let mut trusted: Vec<(String, String)> = Vec::new();
    if !pending.is_empty() {
        if after_read_holds(bridge, "", root_digest, count).await? {
            trusted = pending.into_iter().map(|p| (p.id, p.digest)).collect();
        } else {
            let mut held: HashSet<String> = HashSet::new();
            for p in pending {
                if p.checked.iter().all(|child| held.contains(child))
                    && after_read_holds(bridge, &p.id, &p.digest, count).await?
                {
                    held.insert(p.id.clone());
                    trusted.push((p.id, p.digest));
                } else if known.contains_key(&p.id) {
                    forget.push(p.id);
                }
            }
        }
    }
    if !trusted.is_empty() || !forget.is_empty() {
        bridge.db().update_remote_folder_digests(&trusted, &forget)?;
    }
    Ok(trusted.len())
}

async fn after_read_holds(
    bridge: &EngineBridge,
    folder_id: &str,
    digest: &str,
    count: &RequestCount,
) -> anyhow::Result<bool> {
    match digest_now(bridge.api(), folder_id, count).await {
        Ok(now) => Ok(now == digest),
        Err(WalkError::FolderGone(_)) => Ok(false),
        Err(WalkError::Failed(error)) => Err(error),
        Err(other) => Err(anyhow::anyhow!("{other}")),
    }
}

/// Every non-trashed child row of `folder_id` (`""` = the root), all pages, each page counted.
/// `None`: the folder is gone (its parent's digest moved, so it is not trusted either way).
async fn list_folder(
    bridge: &EngineBridge,
    folder_id: &str,
    count: &RequestCount,
) -> anyhow::Result<Option<Vec<serde_json::Value>>> {
    let mut rows = Vec::new();
    let mut cursor: Option<String> = None;
    let mut cursors = HashSet::new();
    loop {
        count.add(1);
        let page = if folder_id.is_empty() {
            bridge.api().list_root_page(cursor.as_deref()).await
        } else {
            bridge.api().list_owned_folder_page(folder_id, cursor.as_deref()).await
        };
        let page = match page {
            Ok(page) => page,
            Err(error) if !folder_id.is_empty() && error_http_status(&error) == Some(404) => return Ok(None),
            Err(error) => return Err(error),
        };
        let files = page["files"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("a folder listing without its rows"))?;
        rows.extend(
            files
                .iter()
                .filter(|row| row["is_trashed"].as_bool() != Some(true))
                .cloned(),
        );
        anyhow::ensure!(rows.len() <= LISTING_CAP, "a folder listing past {LISTING_CAP} rows");
        match &page["next_cursor"] {
            serde_json::Value::String(next) if !next.is_empty() && cursors.insert(next.clone()) => {
                cursor = Some(next.clone());
            }
            serde_json::Value::Null => return Ok(Some(rows)),
            _ => anyhow::bail!("a folder listing whose cursor does not move"),
        }
    }
}

/// The name of a listed row this PC reads: the stored owner-key record's, else core's reader over
/// `name_encrypted`. `None`: no name this PC can read (blocked, unreadable): the name is not compared.
fn server_name(bridge: &EngineBridge, id: &str, row: &serde_json::Value) -> Option<String> {
    let master = bridge.api().master_key();
    let name = match crate::owner_keys::stored_name(bridge.db(), id, master).ok()? {
        crate::owner_keys::StoredName::Name(name) => name,
        crate::owner_keys::StoredName::Blocked => return None,
        crate::owner_keys::StoredName::None => {
            let text = row["name_encrypted"].as_str().filter(|text| !text.trim().is_empty())?;
            let master = beebeeb_core::kdf::MasterKey::from_bytes(*master);
            beebeeb_core::encrypt::decrypt_name(&master, id, text).ok()?
        }
    };
    let name = name.trim().trim_matches('/').to_string();
    (!name.is_empty()).then_some(name)
}

/// The mirror path ends in `name` (its leaf; an old plaintext name may itself hold a `/`).
fn path_ends_in(path: &str, name: &str) -> bool {
    let path = path.trim_start_matches('/');
    path == name || path.strip_suffix(name).is_some_and(|parent| parent.ends_with('/'))
}

fn is_system_folder(row: &serde_json::Value) -> bool {
    !row["system_role"].is_null() || !row["managed_by"].is_null()
}

/// What the mirror holds under a folder, as one number: equal when no op changed a row under it.
fn mirror_fingerprint(mirror: &[DriftChild]) -> u64 {
    let mut rows: Vec<&DriftChild> = mirror.iter().collect();
    rows.sort_by(|a, b| a.file_id.cmp(&b.file_id));
    let mut hasher = DefaultHasher::new();
    for row in rows {
        row.file_id.hash(&mut hasher);
        row.path.hash(&mut hasher);
        row.status.as_str().hash(&mut hasher);
        row.is_folder.hash(&mut hasher);
        row.size_bytes.hash(&mut hasher);
        row.current_version.hash(&mut hasher);
        row.current_object_version_id.hash(&mut hasher);
    }
    hasher.finish()
}

/// Compare one folder's server children with the mirror's rows under it.
async fn compare_folder(bridge: &EngineBridge, folder_id: &str, count: &RequestCount) -> anyhow::Result<Verdict> {
    let Some(rows) = list_folder(bridge, folder_id, count).await? else {
        return Ok(Verdict::InFlux);
    };
    if rows.iter().any(|row| row["is_uploading"].as_bool() == Some(true)) {
        return Ok(Verdict::InFlux);
    }
    let mirror = bridge.db().drift_children(folder_id)?;
    // A `Trashing` row is not in flux by itself: the server's trash stays in the mirror as one. A local trash that
    // waits for the server has its op queued.
    if mirror.iter().any(|child| {
        matches!(
            child.status,
            FileStatus::Uploading | FileStatus::Error | FileStatus::Conflict
        )
    }) || bridge.db().local_op_queued_in_folder(folder_id)?
    {
        return Ok(Verdict::InFlux);
    }
    let disagrees = Verdict::Disagrees(mirror_fingerprint(&mirror));
    let rows: Vec<serde_json::Value> = bridge
        .prepare_owned_rows(&rows)
        .await?
        .into_iter()
        .map(|p| p.row)
        .collect();
    let mut by_id: HashMap<&str, &DriftChild> = mirror.iter().map(|c| (c.file_id.as_str(), c)).collect();
    for row in &rows {
        let Some(id) = row["id"].as_str().filter(|id| !id.is_empty()) else {
            continue;
        };
        let is_folder = row["is_folder"].as_bool().unwrap_or(false);
        let Some(child) = by_id.remove(id) else {
            if is_folder && is_system_folder(row) {
                continue;
            }
            return Ok(disagrees);
        };
        if child.status == FileStatus::Trashing {
            // Listed while the mirror trashes it: a local trash on its way (its op may have just landed).
            return Ok(Verdict::InFlux);
        }
        if child.is_folder != is_folder {
            return Ok(disagrees);
        }
        if !is_folder {
            // `GET /api/v1/files` carries the version under `download_version`; the snapshot never sends the object
            // id, so it is compared only when the mirror has one, and a mirror row at version 0 has no version yet.
            let size = row["size_bytes"].as_i64().or_else(|| row["size"].as_i64());
            let version = row["download_version"]["version_number"].as_i64();
            let object = row["download_version"]["object_version_id"].as_str();
            if size.is_some_and(|size| size != child.size_bytes)
                || version.is_some_and(|version| child.current_version != 0 && version != child.current_version)
                || object.is_some_and(|object| {
                    child
                        .current_object_version_id
                        .as_deref()
                        .is_some_and(|mine| mine != object)
                })
            {
                return Ok(disagrees);
            }
        }
        if !child.path.is_empty()
            && !child.name_unreadable
            && server_name(bridge, id, row).is_some_and(|name| !path_ends_in(&child.path, &name))
        {
            return Ok(disagrees);
        }
    }
    // A mirror row the server no longer lists; a `Trashing` one is in the server's trash.
    Ok(if by_id.values().all(|child| child.status == FileStatus::Trashing) {
        Verdict::Agrees
    } else {
        disagrees
    })
}

/// Against the fake server (`test_api`): the account's tree, its digests and its op log, through the real tick.
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;

    /// Move the window's start one window back, as if the engine had idled that long.
    fn expire_window(sync_root: &Path) {
        with_state(sync_root, |state| {
            state.last_started = Instant::now().checked_sub(CHECK_EVERY);
        });
    }
    use crate::test_api::fake_account::{account_handler, FakeAccount, TEST_PHRASE};
    use crate::test_api::{FakeApi, Handler, Reply, Request};

    fn uid(kind: u32, n: u32) -> String {
        format!("d{kind:07x}-0013-4000-8000-{n:012x}")
    }

    /// What a test turns on the fake: a version bump only the listing shows (no op), and slow digest answers.
    #[derive(Default)]
    struct Knobs {
        bumps: std::sync::Mutex<HashMap<String, i64>>,
        digest_delay_ms: std::sync::atomic::AtomicU64,
        /// Child folders whose digest the digest pages leave out (`null`).
        no_digest: std::sync::Mutex<HashSet<String>>,
    }

    /// A snapshot node as `GET /api/v1/files` carries it: the version under `download_version` (with an object id,
    /// which the snapshot never sends), none of it at the top.
    fn listing_row(node: &serde_json::Value, knobs: &Knobs) -> serde_json::Value {
        let mut row = node.clone();
        let fields = row.as_object_mut().unwrap();
        let version = fields.remove("version_number").and_then(|v| v.as_i64()).unwrap_or(0);
        let object = fields
            .remove("current_object_version_id")
            .and_then(|v| v.as_str().map(str::to_string));
        fields.remove("current_version");
        if row["is_folder"].as_bool() == Some(true) {
            row["download_version"] = serde_json::Value::Null;
        } else {
            let id = row["id"].as_str().unwrap().to_string();
            let version = version + knobs.bumps.lock().unwrap().get(&id).copied().unwrap_or(0);
            let object = object.unwrap_or_else(|| format!("ov-{id}-{version}"));
            row["download_version"] = json!({ "version_number": version, "object_version_id": object });
        }
        row
    }

    /// The fake's own handler, with `GET /api/v1/files` answered as the real server pages it (`files`,
    /// `next_cursor`, listing-shaped rows): the fake's listing says `items`, which the engine's listing does not read.
    fn handler(account: Arc<FakeAccount>, knobs: Arc<Knobs>) -> Handler {
        let inner = account_handler(account);
        Arc::new(move |req: &Request| {
            if req.path.contains("folder-digests") {
                let ms = knobs.digest_delay_ms.load(std::sync::atomic::Ordering::SeqCst);
                if ms > 0 {
                    std::thread::sleep(Duration::from_millis(ms));
                }
                let left_out = knobs.no_digest.lock().unwrap().clone();
                let reply = inner(req);
                if left_out.is_empty() || reply.status != 200 {
                    return reply;
                }
                let mut page: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
                for child in page["children"].as_array_mut().into_iter().flatten() {
                    if child["id"].as_str().is_some_and(|id| left_out.contains(id)) {
                        child["digest"] = serde_json::Value::Null;
                    }
                }
                return Reply::ok(page);
            }
            if req.method == "GET" && req.path.split('?').next() == Some("/api/v1/files") {
                let parent = req.query("parent_id").map(str::to_string);
                let mut snapshot = req.clone();
                snapshot.path = "/api/v1/sync/snapshot".into();
                let reply = inner(&snapshot);
                if reply.status != 200 {
                    return reply;
                }
                let body: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
                let files: Vec<serde_json::Value> = body["nodes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|node| node["parent_id"].as_str().map(str::to_string) == parent)
                    .map(|node| listing_row(node, &knobs))
                    .collect();
                return Reply::ok(json!({ "files": files, "next_cursor": null }));
            }
            inner(req)
        })
    }

    struct Fixture {
        account: Arc<FakeAccount>,
        knobs: Arc<Knobs>,
        fake: FakeApi,
        bridge: EngineBridge,
        root: PathBuf,
        _dir: tempfile::TempDir,
    }

    impl Fixture {
        /// Root: "Docs" (two files, "Deep" under it with one file) and a file at the top.
        fn new() -> Self {
            let (account, _) = FakeAccount::register("drift@example.invalid", "an invented password", TEST_PHRASE);
            let account = Arc::new(account);
            account.add_item(&uid(1, 1), None, "Docs", true, 0);
            account.add_item(&uid(2, 1), Some(&uid(1, 1)), "a.txt", false, 10);
            account.add_item(&uid(2, 2), Some(&uid(1, 1)), "b.txt", false, 20);
            account.add_item(&uid(1, 2), Some(&uid(1, 1)), "Deep", true, 0);
            account.add_item(&uid(2, 3), Some(&uid(1, 2)), "c.txt", false, 30);
            account.add_file_with_content(&uid(2, 4), None, "top.txt", b"top content");
            let knobs = Arc::new(Knobs::default());
            let fake = FakeApi::start(handler(account.clone(), knobs.clone()));
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("Beebeeb");
            std::fs::create_dir_all(&root).unwrap();
            let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
            let api = Arc::new(ApiClient::new(
                fake.base.clone(),
                account.session_token.clone(),
                account.master_key,
            ));
            let bridge = EngineBridge::new(&crate::identity::TEST, db, api);
            Fixture {
                account,
                knobs,
                fake,
                bridge,
                root,
                _dir: dir,
            }
        }

        async fn tick(&self) {
            sync_tick(&self.bridge, &self.root).await.unwrap();
        }

        /// Wait until no check runs for this root (a check the tick started runs beside it).
        async fn checks_done(&self) {
            let started = Instant::now();
            while with_state(&self.root, |state| state.running) {
                assert!(started.elapsed() < Duration::from_secs(30), "the check never ended");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }

        fn digest_requests(&self) -> usize {
            self.fake
                .routes()
                .iter()
                .filter(|route| route.contains("folder-digests"))
                .count()
        }

        /// A re-snapshot is asked for (read without taking it).
        fn resnapshot_asked(&self) -> bool {
            self.bridge.db().peek_resnapshot_request().unwrap().is_some()
        }

        /// Bootstrap, then the first check (the table is empty: every folder is read and stored).
        async fn synced_and_checked(&self) -> DriftReport {
            self.tick().await;
            assert!(self.bridge.db().get_sync_cursor().unwrap().is_some(), "bootstrapped");
            let first = check_now(&self.bridge, &self.root).await;
            println!("drift first check: {first:?}");
            assert_eq!(first.action, DriftAction::Agreed, "{first:?}");
            assert_eq!(first.disagreeing, 0);
            // Root, Docs, Deep.
            assert_eq!(first.stored, 3, "{first:?}");
            assert_eq!(self.bridge.db().remote_folder_digests().unwrap().len(), 3);
            assert!(!self.resnapshot_asked());
            first
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_three_idle_checks_after_the_first_ask_nothing() {
        let fx = Fixture::new();
        fx.synced_and_checked().await;
        for n in 0..3 {
            let report = check_now(&fx.bridge, &fx.root).await;
            println!("drift idle check {n}: {report:?}");
            assert_eq!(report.action, DriftAction::Unchanged);
            assert!(report.requests <= 1, "{report:?}");
            assert!(!fx.resnapshot_asked());
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_a_change_without_an_op_is_found_in_two_checks() {
        let fx = Fixture::new();
        fx.synced_and_checked().await;
        // The server changes c.txt (two levels down) and records no op.
        fx.account.add_item(&uid(2, 3), Some(&uid(1, 2)), "c.txt", false, 31);
        fx.tick().await;
        assert!(!fx.resnapshot_asked(), "the tick alone sees nothing");

        let first = check_now(&fx.bridge, &fx.root).await;
        println!("drift check 1 after the change: {first:?}");
        assert_eq!(first.action, DriftAction::Watching);
        assert_eq!(first.disagreeing, 1);
        assert!(!fx.resnapshot_asked());

        let second = check_now(&fx.bridge, &fx.root).await;
        println!("drift check 2 after the change: {second:?}");
        assert_eq!(second.action, DriftAction::Resnapshot);
        assert!(fx.resnapshot_asked());
        assert!(fx.bridge.db().remote_folder_digests().unwrap().is_empty());

        // The re-snapshot brings the mirror back; the next check reads every folder and agrees.
        fx.tick().await;
        assert!(!fx.resnapshot_asked());
        let after = check_now(&fx.bridge, &fx.root).await;
        println!("drift check after the re-snapshot: {after:?}");
        assert_eq!(after.action, DriftAction::Agreed);
        assert_eq!(after.disagreeing, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_a_change_with_its_op_is_applied_by_sync_and_asks_nothing() {
        let fx = Fixture::new();
        fx.synced_and_checked().await;
        fx.account.push_newer_version(&uid(2, 4), b"a newer version").unwrap();
        fx.tick().await;
        for n in 0..2 {
            let report = check_now(&fx.bridge, &fx.root).await;
            println!("drift check {n} after a change with its op: {report:?}");
            assert_eq!(report.disagreeing, 0, "{report:?}");
            assert!(!fx.resnapshot_asked());
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_an_uploading_row_is_in_flux() {
        let fx = Fixture::new();
        fx.synced_and_checked().await;
        fx.account.add_item(&uid(2, 1), Some(&uid(1, 1)), "a.txt", false, 11);
        fx.bridge.db().set_status(&uid(2, 1), FileStatus::Uploading).unwrap();
        for n in 0..3 {
            let report = check_now(&fx.bridge, &fx.root).await;
            println!("drift check {n} with an uploading row: {report:?}");
            assert_eq!(report.in_flux, 1, "{report:?}");
            assert_eq!(report.disagreeing, 0);
            assert!(!fx.resnapshot_asked());
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_old_server_is_not_asked_again() {
        let fx = Fixture::new();
        fx.account.digests_as_old_server(true);
        fx.tick().await;
        fx.tick().await;
        assert_eq!(fx.digest_requests(), 0, "the first idle tick only starts the window");
        // A window later, the idle tick runs the check through the tick itself.
        expire_window(&fx.root);
        fx.tick().await;
        fx.checks_done().await;
        assert_eq!(fx.digest_requests(), 1, "{:?}", fx.fake.routes());
        let before = fx.fake.routes().len();
        let report = check_now(&fx.bridge, &fx.root).await;
        assert_eq!(report.action, DriftAction::Unsupported);
        assert_eq!(report.requests, 0);
        fx.tick().await;
        assert_eq!(fx.digest_requests(), 1);
        assert!(!fx.fake.routes()[before..]
            .iter()
            .any(|route| route.contains("/api/v1/files")));
        assert!(!fx.resnapshot_asked());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_runs_from_the_idle_tick_once_per_window() {
        let fx = Fixture::new();
        fx.tick().await;
        assert_eq!(fx.digest_requests(), 0, "the bootstrap tick runs no check");
        fx.tick().await;
        fx.tick().await;
        assert_eq!(fx.digest_requests(), 0, "the first idle ticks only start the window");
        expire_window(&fx.root);
        fx.tick().await;
        fx.checks_done().await;
        let after_first = fx.digest_requests();
        assert!(after_first >= 3, "the first check, a window later, reads every folder");
        assert_eq!(fx.bridge.db().remote_folder_digests().unwrap().len(), 3);
        fx.tick().await;
        fx.checks_done().await;
        assert_eq!(fx.digest_requests(), after_first, "within the window: no check");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_not_ready_waits_for_the_next_window() {
        let fx = Fixture::new();
        fx.account.digests_not_ready(Some(30));
        fx.tick().await;
        let report = check_now(&fx.bridge, &fx.root).await;
        assert_eq!(report.action, DriftAction::NotReady);
        fx.account.digests_not_ready(None);
        let report = check_now(&fx.bridge, &fx.root).await;
        assert_eq!(report.action, DriftAction::Agreed, "{report:?}");
    }

    #[test]
    fn drift_check_window_is_fifteen_minutes() {
        assert_eq!(CHECK_EVERY, Duration::from_secs(15 * 60));
    }

    /// Review 11-26 (1): the listing carries the version under `download_version`; a version the server moved with
    /// no op (same size) is found.
    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_reads_the_version_of_listing_shaped_rows() {
        let fx = Fixture::new();
        fx.synced_and_checked().await;
        let mirror = fx.bridge.db().drift_children(&uid(1, 2)).unwrap();
        println!("drift mirror under Deep: {mirror:?}");
        assert!(mirror.iter().all(|child| child.current_version > 0), "{mirror:?}");
        // The server moves c.txt to a newer version, same size, and records no op; the account's digests move with it.
        fx.knobs.bumps.lock().unwrap().insert(uid(2, 3), 1);
        fx.account.add_item(&uid(2, 3), Some(&uid(1, 2)), "c.txt", false, 30);
        let first = check_now(&fx.bridge, &fx.root).await;
        println!("drift check after a version-only change: {first:?}");
        assert_eq!(first.disagreeing, 1, "{first:?}");
        assert_eq!(first.action, DriftAction::Watching);
        let second = check_now(&fx.bridge, &fx.root).await;
        assert_eq!(second.action, DriftAction::Resnapshot, "{second:?}");
    }

    /// Review 11-26 (1): the listing's object id is not compared when the mirror has none (the snapshot never sends
    /// it), so a synced tree agrees.
    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_listing_object_ids_agree_with_a_snapshot_mirror() {
        let fx = Fixture::new();
        let first = fx.synced_and_checked().await;
        assert_eq!(first.in_flux, 0);
    }

    /// Review 11-26 (2): an item in the server's trash (a `Trashing` row in the mirror, not listed by the server)
    /// leaves its folder and the folders above it storable.
    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_server_trash_is_not_in_flux() {
        let fx = Fixture::new();
        fx.tick().await;
        assert!(fx.account.trash_item(&uid(2, 1)));
        fx.bridge.db().set_status(&uid(2, 1), FileStatus::Trashing).unwrap();
        let report = check_now(&fx.bridge, &fx.root).await;
        println!("drift check with a server-trashed row: {report:?}");
        assert_eq!(report.in_flux, 0, "{report:?}");
        assert_eq!(report.disagreeing, 0, "{report:?}");
        assert_eq!(report.stored, 3, "{report:?}");
        let idle = check_now(&fx.bridge, &fx.root).await;
        assert_eq!(idle.action, DriftAction::Unchanged, "{idle:?}");
        assert_eq!(idle.requests, 1, "{idle:?}");
    }

    /// Review 11-26 (3): a check reads at most its cap of folders, deepest first, stores each as it is verified, and
    /// the next check goes on from there; an idle check then costs one request.
    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_is_capped_deepest_first_and_resumes() {
        let fx = Fixture::new();
        fx.tick().await;
        let stored = |fx: &Fixture| {
            let mut ids: Vec<String> = fx.bridge.db().remote_folder_digests().unwrap().into_keys().collect();
            ids.sort();
            ids
        };
        let first = check_capped(&fx.bridge, &fx.root, 1).await;
        println!("drift capped check 1: {first:?}");
        assert_eq!(first.changed, 1, "{first:?}");
        assert_eq!(stored(&fx), vec![uid(1, 2)], "Deep, the deepest, first");
        let second = check_capped(&fx.bridge, &fx.root, 1).await;
        println!("drift capped check 2: {second:?}");
        assert_eq!(stored(&fx), vec![uid(1, 1), uid(1, 2)]);
        let third = check_capped(&fx.bridge, &fx.root, 1).await;
        println!("drift capped check 3: {third:?}");
        assert_eq!(stored(&fx).len(), 3);
        let idle = check_capped(&fx.bridge, &fx.root, 1).await;
        assert_eq!(idle.action, DriftAction::Unchanged, "{idle:?}");
        assert_eq!(idle.requests, 1, "{idle:?}");
    }

    /// Review 11-26 (3): the check the idle tick starts runs beside the tick, not inside it (the tick holds the pull
    /// lock a save waits for).
    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_does_not_hold_the_tick() {
        let fx = Fixture::new();
        fx.tick().await;
        fx.tick().await;
        expire_window(&fx.root);
        fx.knobs
            .digest_delay_ms
            .store(1500, std::sync::atomic::Ordering::SeqCst);
        let started = Instant::now();
        fx.tick().await;
        let took = started.elapsed();
        assert!(took < Duration::from_secs(1), "the tick waited {took:?} for the check");
        fx.checks_done().await;
        assert_eq!(fx.bridge.db().remote_folder_digests().unwrap().len(), 3);
    }

    /// Review 11-26 (8): a folder that drifts at two checks in a row is drift even when the cursor moved, when no op
    /// touched that folder in between.
    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_drift_twice_with_ops_elsewhere_resnapshots() {
        let fx = Fixture::new();
        fx.synced_and_checked().await;
        fx.account.add_item(&uid(2, 3), Some(&uid(1, 2)), "c.txt", false, 31);
        let first = check_now(&fx.bridge, &fx.root).await;
        assert_eq!(first.action, DriftAction::Watching, "{first:?}");
        let cursor = fx.bridge.db().get_sync_cursor().unwrap();
        fx.account.push_newer_version(&uid(2, 4), b"a newer version").unwrap();
        fx.tick().await;
        assert_ne!(
            fx.bridge.db().get_sync_cursor().unwrap(),
            cursor,
            "an op elsewhere moved the cursor"
        );
        let second = check_now(&fx.bridge, &fx.root).await;
        println!("drift check 2 with an op elsewhere: {second:?}");
        assert_eq!(second.action, DriftAction::Resnapshot, "{second:?}");
        assert!(fx.resnapshot_asked());
    }

    /// Review 11-26 (9): a check dropped half-way (the engine stopping) does not leave the root marked running.
    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_dropped_clears_running() {
        let fx = Fixture::new();
        fx.tick().await;
        fx.knobs.digest_delay_ms.store(500, std::sync::atomic::Ordering::SeqCst);
        let cut = tokio::time::timeout(Duration::from_millis(50), check_now(&fx.bridge, &fx.root)).await;
        assert!(cut.is_err(), "the check was cut");
        assert!(!with_state(&fx.root, |state| state.running));
    }
    /// fd-24: a tree of 2,700 folders is verified by one check, and a file changed with no op 7 levels down is found
    /// by the next check with at most `2 * 7 + 2` requests (the root's digest, then one digest page and one listing
    /// per folder on its path); the check after that asks for the re-snapshot. Idle checks cost one request.
    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_a_deep_change_in_a_large_tree_is_found_by_the_next_check() {
        let fx = Fixture::new();
        // 30 folders at the top with 89 folders each, and a chain of 5 more under the first: 2,705 folders besides the
        // existing three. The file at /T0/L2/L3/L4/L5/L6/deep.txt is 7 levels deep.
        let mut n = 100;
        for t in 0..30u32 {
            let top = uid(3, t);
            fx.account.add_item(&top, None, &format!("T{t}"), true, 0);
            for _ in 0..89 {
                n += 1;
                fx.account.add_item(&uid(3, n), Some(&top), &format!("F{n}"), true, 0);
            }
        }
        let mut parent = uid(3, 0);
        for level in 2..=6u32 {
            let folder = uid(4, level);
            fx.account
                .add_item(&folder, Some(&parent), &format!("L{level}"), true, 0);
            parent = folder;
        }
        fx.account.add_item(&uid(5, 1), Some(&parent), "deep.txt", false, 70);
        fx.tick().await;

        let first = check_now(&fx.bridge, &fx.root).await;
        println!("drift first check of the large tree: {first:?}");
        assert_eq!(first.action, DriftAction::Agreed, "{first:?}");
        assert!(!first.capped, "the whole tree is verified in one check: {first:?}");
        assert_eq!(first.stored, 3 + 2_705, "{first:?}");
        assert!(
            first.requests as usize <= 2 * first.changed + 3,
            "two requests per folder: {first:?}"
        );

        for n in 0..3 {
            let idle = check_now(&fx.bridge, &fx.root).await;
            println!("drift idle check {n} of the large tree: {idle:?}");
            assert_eq!(idle.action, DriftAction::Unchanged, "{idle:?}");
            assert!(idle.requests <= 1, "{idle:?}");
            assert!(!fx.resnapshot_asked());
        }

        // The server changes deep.txt and records no op (direct SQL).
        fx.account.add_item(&uid(5, 1), Some(&parent), "deep.txt", false, 71);
        fx.tick().await;
        let found = check_now(&fx.bridge, &fx.root).await;
        println!("drift check after a change 7 levels down: {found:?}");
        assert_eq!(found.disagreeing, 1, "{found:?}");
        assert_eq!(found.action, DriftAction::Watching, "{found:?}");
        assert!(found.requests <= 2 * 7 + 2, "{found:?}");
        assert!(!fx.resnapshot_asked());

        let second = check_now(&fx.bridge, &fx.root).await;
        println!("drift check 2 after a change 7 levels down: {second:?}");
        assert_eq!(second.action, DriftAction::Resnapshot, "{second:?}");
        assert!(second.requests <= 2 * 7 + 2, "{second:?}");
        assert!(fx.resnapshot_asked());
    }

    /// fd-24: a change below a child folder whose digest the root's page leaves out is still found: that folder is
    /// entered and listed, the check does not fail.
    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_a_child_without_a_digest_is_listed() {
        let fx = Fixture::new();
        fx.synced_and_checked().await;
        fx.knobs.no_digest.lock().unwrap().insert(uid(1, 1));
        fx.account.add_item(&uid(2, 3), Some(&uid(1, 2)), "c.txt", false, 31);
        let first = check_now(&fx.bridge, &fx.root).await;
        println!("drift check with a child digest left out: {first:?}");
        assert_eq!(first.action, DriftAction::Watching, "{first:?}");
        assert_eq!(first.disagreeing, 1, "{first:?}");
        let second = check_now(&fx.bridge, &fx.root).await;
        assert_eq!(second.action, DriftAction::Resnapshot, "{second:?}");
    }

    /// fd-24: a root changed by an op sync applied, with a child folder's digest left out, agrees: the child is
    /// listed, nothing is watched and no re-snapshot is asked for.
    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_a_missing_child_digest_is_not_misread() {
        let fx = Fixture::new();
        fx.synced_and_checked().await;
        fx.knobs.no_digest.lock().unwrap().insert(uid(1, 1));
        fx.account.push_newer_version(&uid(2, 4), b"a newer version").unwrap();
        fx.tick().await;
        for n in 0..2 {
            let report = check_now(&fx.bridge, &fx.root).await;
            println!("drift check {n} with a child digest left out: {report:?}");
            assert_ne!(report.action, DriftAction::Failed, "{report:?}");
            assert_eq!(report.disagreeing, 0, "{report:?}");
            assert!(!fx.resnapshot_asked());
        }
    }
}
