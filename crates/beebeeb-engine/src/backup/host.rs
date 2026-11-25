//! LD-0113 Task D4: `DesktopBackupHost`, desktop's side of core's backup pass (`beebeeb_backup::BackupHost`).
//!
//! It works in one resolved layout (the Backups root and this device's folder, cached in the state DB): it lists
//! this device's backup of a category, creates and moves folders and files there through the backup client
//! (`X-Beebeeb-Origin: backup`), queues uploads (`origin = 'backup'`, the source read in place), keeps the records
//! (`backup_records`, per account) and turns a backup off when its folder was deleted elsewhere (rule 11).
//!
//! The pass is the one writer the protection layer protects the tree for: nothing here goes through
//! `check_app_mutation` or `check_drive_mutation`. The engine's own placeholder changes that follow a server move
//! pass the drive guard as `by_this_process` (protection lane 2's answer, 2026-11-06).

use std::collections::HashMap;
use std::sync::Arc;

use beebeeb_backup::{BackupHost, DeviceTree, HostError, UploadRequest};
use beebeeb_core::backup::{BackupRecord, FolderPresence, RebuildClock, RecordRef, RemoteEntry};
use beebeeb_types::backup::{BackupStopReason, Category};
use uuid::Uuid;

use super::digest_cache::{CategoryCache, Fallback, FolderEntry, SharedDigestCache};
use super::tree_cache::{self, KeptListing, OpsVerdict};
use crate::engine_bridge::{BackupChild, BackupFolderState, BackupSnapshot, EngineBridge};
use crate::folder_digest::{self, RequestCount, WalkError};
use std::collections::HashSet;

/// RE3-a: a listing page by page (a server with no snapshot) runs this many folder requests at full speed, then one
/// every [`PAGED_LISTING_PACE`].
const PAGED_LISTING_BURST: usize = 20;
const PAGED_LISTING_PACE: std::time::Duration = if cfg!(test) {
    std::time::Duration::ZERO
} else {
    std::time::Duration::from_millis(250)
};
/// `/sync/ops` answers at most this many ops a page (the server's default): a full page may not be the head.
const OPS_PAGE: usize = 1_000;

/// Where a full listing reads the folders from.
enum Listing<'a> {
    /// The one snapshot of the account's tree (RE3-a).
    Snapshot(&'a BackupSnapshot),
    /// `GET /files` per folder, paced after a burst (a server with no snapshot, or a test's direct listing).
    Pages { paced: bool },
}

/// One read of the ops after a kept listing's cursor.
#[derive(Clone)]
struct OpsRead {
    ops: Vec<crate::api_client::SyncOp>,
    /// The cursor the ops were read up to.
    to: i64,
    /// The read reached the head: nothing after `to`.
    caught_up: bool,
}
use crate::state_db::{BackupEnqueue, BackupLayout, BackupPassRecord, BackupStopWrite};

/// fd-18: what the device folder's one digest read of a tick says.
#[derive(Clone)]
enum DeviceDigests {
    /// Live: each child folder's digest (the category folders').
    Live(HashMap<String, String>),
    /// Gone (its state agrees).
    Gone,
    /// No usable answer (no digests, not ready, a failure): the passes list as before.
    Unusable,
}

/// fd-18: a tick's device read and the walks done with it, by category key: (category folder id, the walk).
struct DigestRound {
    device: DeviceDigests,
    walks: HashMap<String, (String, Result<Walked, WalkError>)>,
}

/// One backup's walk.
struct Walked {
    /// Every folder whose digest moved (or that the cache lacks), with the digest read for it.
    changed: HashMap<String, String>,
    /// Changed folders the cache holds no folder in, not read: their digest is the one their parent's page gave, and
    /// what is under them now is listed whole.
    unread: HashSet<String>,
    requests: u32,
    /// When the walk began: its listing is read after it.
    since: std::time::Instant,
}

/// fd-18: every folder under `root` (itself included) whose digest differs from `known` or is not in it, parents
/// before children, each with its digest. `root_now` (the root's digest as read already) equal to the known one: no
/// request. A child folder whose digest equals `known` is not entered. A changed folder in `leaves` (the cache holds
/// no folder in it) is not read: its parent's page gave its digest, and it goes to `unread` (what it holds now is
/// listed whole). A child folder gone by the time it is read is skipped (its parent's listing shows it gone).
async fn walk_changed(
    api: &crate::api_client::ApiClient,
    root: &str,
    root_now: Option<String>,
    known: &folder_digest::Known,
    leaves: &HashSet<String>,
    count: &RequestCount,
) -> Result<(HashMap<String, String>, HashSet<String>), WalkError> {
    let mut changed = HashMap::new();
    let mut unread = HashSet::new();
    if let Some(now) = root_now {
        if known.get(root) == Some(&now) {
            return Ok((changed, unread));
        }
        if leaves.contains(root) {
            changed.insert(root.to_string(), now);
            unread.insert(root.to_string());
            return Ok((changed, unread));
        }
    }
    let first = folder_digest::read_folder(api, root, count).await?;
    changed.insert(root.to_string(), first.digest.clone());
    let mut queue = std::collections::VecDeque::from([first]);
    while let Some(folder) = queue.pop_front() {
        for (child, digest) in folder.child_folders {
            if known.get(&child) == Some(&digest) {
                continue;
            }
            if leaves.contains(&child) {
                unread.insert(child.clone());
                changed.insert(child, digest);
                continue;
            }
            match folder_digest::read_folder(api, &child, count).await {
                Ok(read) => {
                    changed.insert(child, read.digest.clone());
                    queue.push_back(read);
                }
                Err(WalkError::FolderGone(_)) => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok((changed, unread))
}

/// Two listings of a folder hold the same children (what the tree reads of them, in any order).
fn same_children(a: &[BackupChild], b: &[BackupChild]) -> bool {
    let fields = |children: &[BackupChild]| {
        let mut out: Vec<(String, Option<String>, bool, u64, i64)> = children
            .iter()
            .map(|c| (c.id.clone(), c.name.clone(), c.is_folder, c.size, c.uploaded_at_ms))
            .collect();
        out.sort();
        out
    };
    a.len() == b.len() && fields(a) == fields(b)
}

/// A failure in the host's words: a kind and never a name or a path (core's rule for `HostError`).
fn host_error(kind: &str) -> impl Fn(crate::engine_bridge::EngineError) -> HostError + '_ {
    move |error| {
        tracing::warn!(
            kind,
            reason = crate::engine_bridge::error_category(&error),
            "backup: host step failed"
        );
        HostError(kind.to_string())
    }
}

fn db_error(kind: &str) -> impl Fn(rusqlite::Error) -> HostError + '_ {
    move |_| HostError(format!("{kind}: the state database refused"))
}

fn parse_id(id: &str) -> Option<Uuid> {
    Uuid::parse_str(id).ok()
}

/// Turn the backup of these categories off in the settings (the same switch the person uses). `Ok(false)`: the
/// settings could not be written; the stop notice still stands and no pass runs for a folder whose category folder
/// is forgotten.
pub fn switch_off_in_settings(identity: &'static crate::identity::EngineIdentity, keys: &[String]) -> bool {
    crate::config::DesktopConfig::update_for(identity, |cfg| {
        for key in keys {
            cfg.set_known_folder(key, false);
        }
        (true, ())
    })
    .is_ok()
}

/// Whether the switch `key` in the PC's settings file is the local data's owner's to turn off (the per-account rule
/// of P3-8): a known folder's switch is the PC's; a chosen folder's (`custom:<name>`) only when the folder is one of
/// `owners_chosen` (`backup_folders::runner_chosen`: tagged with that owner; an untagged one is nobody's). Every
/// account's chosen folders share the one backup set, so a stop or a reconcile here never turns off another
/// account's. A key that names no category: never.
pub fn owners_switch(key: &str, owners_chosen: &[crate::backup::sources::ChosenFolder]) -> bool {
    match Category::from_key(key) {
        Some(Category::Custom(_)) => owners_chosen.iter().any(|folder| folder.key() == key),
        Some(_) => true,
        None => false,
    }
}

/// The tree of a backup whose device or category folder is gone: nothing listed, and complete (there is nothing more).
fn gone_tree(device_folder_live: bool, category_folder_live: Option<bool>) -> DeviceTree {
    DeviceTree {
        device_folder_live,
        category_folder_live,
        folders: HashMap::new(),
        files: Vec::new(),
        complete: true,
    }
}

/// One folder's children into a listing's tree, keys joined with `/` under `prefix`: each file, and each folder with
/// the (key, id) to list next. A child whose id is not a uuid, or a folder whose name cannot be opened, is not taken
/// and makes the listing partial; a file's unopened name lists it by id with an empty key.
fn add_children(
    prefix: &str,
    children: impl IntoIterator<Item = BackupChild>,
    folders: &mut HashMap<String, Uuid>,
    files: &mut Vec<RemoteEntry>,
    complete: &mut bool,
) -> Vec<(String, String)> {
    let mut next = Vec::new();
    for BackupChild {
        id,
        name,
        is_folder,
        size,
        uploaded_at_ms,
        ..
    } in children
    {
        let Some(remote_id) = parse_id(&id) else {
            *complete = false;
            continue;
        };
        let key = name.as_deref().map(|name| {
            if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}/{name}")
            }
        });
        if is_folder {
            match key {
                Some(key) => {
                    folders.insert(key.clone(), remote_id);
                    next.push((key, id));
                }
                None => *complete = false,
            }
        } else {
            files.push(RemoteEntry {
                key: key.unwrap_or_default(),
                remote_id,
                size,
                uploaded_at_ms,
            });
        }
    }
    next
}

pub struct DesktopBackupHost {
    bridge: Arc<EngineBridge>,
    layout: BackupLayout,
    /// Every category this device backs up now: a deleted device folder turns all of them off at once.
    enabled: Vec<Category>,
    /// A tree the records' rebuild just listed, handed to the pass that follows it instead of listing again
    /// (review 2026-11-07): taken once.
    listed: std::sync::Mutex<HashMap<String, DeviceTree>>,
    /// RE3-a: this tick's one snapshot and when it was read (`None` inside: it could not be read; the listing goes
    /// page by page).
    snapshot: tokio::sync::Mutex<Option<(std::time::Instant, Option<Arc<BackupSnapshot>>)>>,
    /// RE3-a: this tick's ops reads by cursor (`None`: the read failed), so backups kept at one cursor read once.
    ops: tokio::sync::Mutex<HashMap<i64, Option<OpsRead>>>,
    /// fd-12: the backups' digest caches (`None`: no digests, the RE3-a listing).
    digests: Option<SharedDigestCache>,
    /// fd-18: this tick's one digest read of the device folder and the walks it decided.
    round: tokio::sync::Mutex<Option<DigestRound>>,
}

impl DesktopBackupHost {
    pub fn new(bridge: Arc<EngineBridge>, layout: BackupLayout, enabled: Vec<Category>) -> Self {
        Self {
            bridge,
            layout,
            enabled,
            listed: std::sync::Mutex::default(),
            snapshot: tokio::sync::Mutex::default(),
            ops: tokio::sync::Mutex::default(),
            digests: None,
            round: tokio::sync::Mutex::default(),
        }
    }

    /// fd-12: the regular pass lists only the folders whose digest changed, the rest from `cache` (the scheduler's,
    /// for this engine run).
    pub fn with_digest_cache(mut self, cache: SharedDigestCache) -> Self {
        self.digests = Some(cache);
        self
    }

    /// RE3-a: this device's backup of `c` for a pass, and the clock of its listing. The kept listing of an earlier pass
    /// when it can be used ([`Self::kept_tree`]: no listing, one `/sync/ops` read); else a full listing from one
    /// snapshot, kept for the next passes. `rebuild`: the records are about to be rebuilt from it, so it is listed in
    /// full and its clock measured. The pass that follows takes the same tree.
    pub async fn device_tree_for_pass(
        &self,
        c: &Category,
        rebuild: bool,
    ) -> Result<(DeviceTree, RebuildClock), HostError> {
        // fd-12: the folders whose digest changed, the rest from the digest cache; a server without digests (or not
        // ready) lists as RE3-a did. The records' rebuild keeps its clocked full listing.
        let kept = match rebuild {
            true => None,
            false => match self.digest_tree(c).await? {
                Some(tree) => Some(tree),
                None => self.kept_tree(c).await?,
            },
        };
        let (tree, clock) = match kept {
            Some(tree) => (tree, RebuildClock::UNKNOWN),
            None => self.full_listing(c, rebuild).await?,
        };
        if let Ok(mut listed) = self.listed.lock() {
            listed.insert(c.key(), tree.clone());
        }
        Ok((tree, clock))
    }

    /// The kept listing of `c`, handed to core as PARTIAL (core marks nothing removed elsewhere from it), when it is
    /// the remembered category folder's, younger than a day, read from a snapshot, and the ops since it touch none of
    /// its folders (nor its files, past [`tree_cache::TOUCHED_LISTING_GAP`]). When the ops could not be read to the
    /// head, the device and category folders are checked by id (two requests) instead. `None`: list in full.
    async fn kept_tree(&self, c: &Category) -> Result<Option<DeviceTree>, HostError> {
        let (device, key) = (self.layout.device_folder_id.as_str(), c.key());
        let Some(kept) = tree_cache::get(device, &key) else {
            return Ok(None);
        };
        let remembered = self
            .bridge
            .db()
            .backup_category_folder(&key)
            .map_err(db_error("category_folder"))?;
        if remembered.as_deref() != Some(kept.category_folder_id.as_str())
            || kept.listed_at.elapsed() >= tree_cache::FULL_LISTING_EVERY
        {
            return Ok(None);
        }
        let Some(cursor) = kept.cursor else {
            return Ok(None);
        };
        let read = self.ops_since(cursor).await;
        let checked = match &read {
            Some(read) => match tree_cache::verdict(&kept, device, &read.ops) {
                OpsVerdict::FolderTouched => return Ok(None),
                OpsVerdict::FileTouched if kept.listed_at.elapsed() >= tree_cache::TOUCHED_LISTING_GAP => {
                    return Ok(None);
                }
                // Not moved on: the touch is seen again, and lists once the gap is over.
                OpsVerdict::FileTouched => read.caught_up,
                OpsVerdict::Quiet => {
                    tree_cache::advance(device, &key, read.to);
                    read.caught_up
                }
            },
            None => false,
        };
        if !checked {
            if self.folder_state(device).await? == BackupFolderState::Gone
                || self.folder_state(&kept.category_folder_id).await? == BackupFolderState::Gone
            {
                return Ok(None);
            }
        }
        let mut tree = kept.tree;
        tree.device_folder_live = true;
        tree.category_folder_live = Some(true);
        tree.complete = false;
        Ok(Some(tree))
    }

    /// fd-12, fd-18: this device's backup of `c` from the digest walk: every folder whose digest changed (or that the
    /// cache lacks) listed from one snapshot read after the tick's walks (page by page when there is none), every other
    /// folder's children from the cache, so the tree is the one a full listing gives (keys, folders, files,
    /// `complete`). The device folder's one digest read of the tick ([`Self::digest_round`]) says whether it is live
    /// and every category folder's digest: an unchanged backup costs no request of its own. `None`: no cache, or the
    /// digests cannot be used now (logged once per run), and the pass lists as RE3-a did. One log line per pass with
    /// its request counts.
    async fn digest_tree(&self, c: &Category) -> Result<Option<DeviceTree>, HostError> {
        let Some(shared) = &self.digests else {
            return Ok(None);
        };
        if !shared.lock().is_ok_and(|cache| cache.usable(std::time::Instant::now())) {
            return Ok(None);
        }
        let key = c.key();
        let (device, prewalked) = {
            let mut round = self.round.lock().await;
            if round.is_none() {
                *round = Some(self.digest_round(shared).await?);
            }
            let round = round.as_mut().expect("read above");
            (round.device.clone(), round.walks.remove(&key))
        };
        let device_children = match device {
            DeviceDigests::Unusable => return Ok(None),
            DeviceDigests::Gone => return Ok(Some(gone_tree(false, None))),
            DeviceDigests::Live(children) => children,
        };
        let (folder_id, live, walked) = match prewalked {
            Some((folder_id, walked)) => (folder_id, Some(true), walked),
            None => {
                // Not found under the device folder by its remembered id: resolved as before (its state asked, or the
                // folder found or made), then walked on its own.
                let (folder_id, live) = self.category_folder(c).await?;
                if live == Some(false) {
                    return Ok(Some(gone_tree(true, Some(false))));
                }
                let root_now = device_children.get(&folder_id).cloned();
                let walked = self.walk(shared, &key, &folder_id, root_now).await;
                (folder_id, live, walked)
            }
        };
        let walked = match walked {
            Ok(walked) => walked,
            Err(error) => {
                self.walk_failed(shared, &error);
                return Ok(None);
            }
        };
        let mut cache = match shared.lock() {
            Ok(mut all) => all.take(&key, &folder_id),
            Err(_) => return Ok(None),
        };
        let built = self.digest_build(&key, &folder_id, &walked, &mut cache).await;
        if let Err(error) = cache.write_to(self.bridge.db(), &self.layout.device_folder_id, &key) {
            tracing::warn!(category = %key, %error, "backup: the folder digests could not be saved; the next pass tries again");
        }
        if let Ok(mut all) = shared.lock() {
            all.put(&key, cache);
        }
        let mut tree = built?;
        tree.category_folder_live = live;
        Ok(Some(tree))
    }

    /// fd-18: the tick's one digest read of the device folder (one request: its digest page lists every category
    /// folder's digest), and the walk of every backup whose category folder is the remembered one under it, all
    /// before any listing, so one snapshot read after them serves every backup. A device folder the digests call gone
    /// is confirmed by its state before it is taken as gone (it turns every backup off).
    async fn digest_round(&self, shared: &SharedDigestCache) -> Result<DigestRound, HostError> {
        let count = RequestCount::default();
        let device_id = &self.layout.device_folder_id;
        let device = match folder_digest::read_folder(self.bridge.api(), device_id, &count).await {
            Ok(read) => DeviceDigests::Live(read.child_folders.into_iter().collect()),
            Err(WalkError::FolderGone(_)) => match self.folder_state(device_id).await? {
                BackupFolderState::Gone => DeviceDigests::Gone,
                BackupFolderState::Live => DeviceDigests::Unusable,
            },
            Err(error) => {
                self.walk_failed(shared, &error);
                DeviceDigests::Unusable
            }
        };
        let mut walks = HashMap::new();
        if let DeviceDigests::Live(children) = &device {
            for c in &self.enabled {
                let Some(folder) = self.remembered_category_folder(c)? else {
                    continue;
                };
                let Some(now) = children.get(&folder) else {
                    continue;
                };
                let walked = self.walk(shared, &c.key(), &folder, Some(now.clone())).await;
                walks.insert(c.key(), (folder, walked));
            }
        }
        Ok(DigestRound { device, walks })
    }

    /// The category folder id of `c` this device remembers, when a pass may take it as live from the device folder's
    /// digest page (it is a child folder there): a chosen folder's only while its settings entry still names it
    /// ([`Self::chosen_category_folder`]'s first case). No request.
    fn remembered_category_folder(&self, c: &Category) -> Result<Option<String>, HostError> {
        let db = self.bridge.db();
        let key = c.key();
        let remembered = db.backup_category_folder(&key).map_err(db_error("category_folder"))?;
        if remembered.is_none() || !matches!(c, Category::Custom(_)) {
            return Ok(remembered);
        }
        let Some(owner) = db.owner().map_err(db_error("category_folder"))? else {
            return Ok(None);
        };
        let Ok(cfg) = crate::config::DesktopConfig::load_for(self.bridge.identity) else {
            return Ok(None);
        };
        let entry = crate::app::bindings::backup_folders::chosen_for(&cfg.backup_chosen_folders, Some(&owner))
            .into_iter()
            .find(|folder| folder.key() == key)
            .and_then(|folder| folder.folder_id);
        Ok(remembered.filter(|id| entry.as_deref() == Some(id.as_str())))
    }

    /// One backup's walk over the cache's digests. `root_now`: the category folder's digest as the device folder's
    /// page gave it (`None`: asked here when the cache knows one).
    async fn walk(
        &self,
        shared: &SharedDigestCache,
        key: &str,
        root: &str,
        root_now: Option<String>,
    ) -> Result<Walked, WalkError> {
        let (known, leaves) = shared.lock().map(|all| all.walk_view(key, root)).unwrap_or_default();
        let count = RequestCount::default();
        let since = std::time::Instant::now();
        let api = self.bridge.api();
        let root_now = match root_now {
            Some(digest) => Some(digest),
            None if known.contains_key(root) => Some(folder_digest::digest_now(api, root, &count).await?),
            None => None,
        };
        let (changed, unread) = walk_changed(api, root, root_now, &known, &leaves, &count).await?;
        Ok(Walked {
            changed,
            unread,
            requests: count.get(),
            since,
        })
    }

    /// The digests answered no usable walk: an old server or one not ready yet turns them off for a while (logged once
    /// per run); any other failure lists this pass as before.
    fn walk_failed(&self, shared: &SharedDigestCache, error: &WalkError) {
        let now = std::time::Instant::now();
        let fallback = match error {
            WalkError::Unsupported => Some((Fallback::Unsupported, std::time::Duration::ZERO)),
            WalkError::NotReady { retry_after_secs } => {
                Some((Fallback::NotReady, std::time::Duration::from_secs(*retry_after_secs)))
            }
            _ => None,
        };
        match fallback {
            Some((fallback, wait)) => {
                if shared.lock().is_ok_and(|mut all| all.fall_back(fallback, wait, now)) {
                    tracing::info!(
                        why = fallback.as_str(),
                        "backup: no folder digests; the passes list as before"
                    );
                }
            }
            None => tracing::warn!(
                %error,
                "backup: the folder digest walk failed; this pass lists as before"
            ),
        }
    }

    /// The tree of [`Self::digest_tree`] from `walked` over `cache`, which keeps what the next pass compares. Once a
    /// day (`CategoryCache::full_due`) every folder is listed from one snapshot instead and compared with the cache.
    async fn digest_build(
        &self,
        key: &str,
        root: &str,
        walked: &Walked,
        cache: &mut CategoryCache,
    ) -> Result<DeviceTree, HostError> {
        let root_id = parse_id(root).ok_or_else(|| HostError("category folder id is not a uuid".into()))?;
        let mut listings = 0usize;
        let now_ms = chrono::Utc::now().timestamp_millis();
        // The daily full comparison: only from a snapshot (never thousands of pages).
        let full = cache.full_due(now_ms) && {
            let (snapshot, asked) = self.snapshot_read_since(Some(walked.since)).await;
            listings += usize::from(asked);
            snapshot.is_some()
        };

        let mut folders = HashMap::from([(String::new(), root_id)]);
        let mut files = Vec::new();
        let mut complete = true;
        let mut listed = 0usize;
        let mut seen = std::collections::HashSet::new();
        let mut parents: HashMap<String, String> = HashMap::new();
        let mut differing: Vec<String> = Vec::new();
        let mut from_cache = 0usize;
        // (key prefix, folder id, listed whole: under a folder the walk did not read, so its own digest is unknown)
        let mut queue = std::collections::VecDeque::from([(String::new(), root.to_string(), false)]);
        while let Some((prefix, folder, whole)) = queue.pop_front() {
            if !seen.insert(folder.clone()) {
                complete = false;
                continue;
            }
            let walked_digest = walked.changed.get(&folder);
            let cached = (!whole && !full && walked_digest.is_none())
                .then(|| cache.reusable(&folder))
                .flatten()
                .map(|entry| entry.children.clone());
            let children = match cached {
                Some(children) => {
                    from_cache += 1;
                    children
                }
                None => {
                    let (children, all) = self.digest_listing(&folder, walked.since, &mut listings).await?;
                    let old = cache.folders.get(&folder);
                    // A folder the walk found unchanged whose listing differs from the cache: its digest was wrong.
                    let differs = !whole
                        && walked_digest.is_none()
                        && old.is_some_and(|old| {
                            old.complete && old.digest.is_some() && !same_children(&old.children, &children)
                        });
                    let digest = match walked_digest {
                        _ if whole || !all || differs => None,
                        Some(digest) => Some(digest.clone()),
                        None => old.and_then(|old| old.digest.clone()),
                    };
                    if differs {
                        differing.push(folder.clone());
                    }
                    cache.insert(
                        &folder,
                        FolderEntry {
                            digest,
                            children: children.clone(),
                            complete: all,
                        },
                    );
                    listed += 1;
                    complete &= all;
                    children
                }
            };
            let whole = whole || walked.unread.contains(&folder);
            for (child_key, child) in add_children(&prefix, children, &mut folders, &mut files, &mut complete) {
                parents.insert(child.clone(), folder.clone());
                queue.push_back((child_key, child, whole));
            }
        }
        // A wrong digest: the folder and every folder above it are entered again by the next walk.
        for folder in &differing {
            let mut at = Some(folder.clone());
            while let Some(folder) = at {
                cache.untrust(&folder);
                at = parents.get(&folder).cloned();
            }
        }
        if !differing.is_empty() {
            tracing::warn!(
                category = %key,
                folders = differing.len(),
                "backup: the daily check found folders whose digest did not change but whose listing did; they are listed again"
            );
        }
        cache.retain(&seen);
        if full || from_cache == 0 {
            cache.compared_in_full(now_ms);
        }
        tracing::info!(
            category = %key,
            digest_requests = walked.requests,
            listing_requests = listings,
            folders_changed = walked.changed.len(),
            folders_listed = listed,
            folders_from_cache = from_cache,
            full,
            "backup: digest pass"
        );
        Ok(DeviceTree {
            device_folder_live: true,
            category_folder_live: None,
            folders,
            files,
            complete,
        })
    }

    /// One folder's children for the digest pass, and whether they are all: from this tick's snapshot when it was
    /// read after `since` (the walk's start), else read now; with no snapshot, the folder's pages, paced.
    async fn digest_listing(
        &self,
        folder: &str,
        since: std::time::Instant,
        listings: &mut usize,
    ) -> Result<(Vec<BackupChild>, bool), HostError> {
        let (snapshot, asked) = self.snapshot_read_since(Some(since)).await;
        if asked {
            *listings += 1;
        }
        if let Some(snapshot) = snapshot {
            return Ok((self.bridge.backup_children_in(&snapshot, folder), true));
        }
        *listings += 1;
        if *listings > PAGED_LISTING_BURST {
            tokio::time::sleep(PAGED_LISTING_PACE).await;
        }
        let (children, all, _) = self
            .bridge
            .backup_list_children_clocked(folder)
            .await
            .map_err(host_error("listing"))?;
        Ok((children, all))
    }

    /// One page of the ops after `cursor`, read once per tick. `None`: the read failed, or the server's head is
    /// behind the cursor (its log was reset).
    async fn ops_since(&self, cursor: i64) -> Option<OpsRead> {
        let mut memo = self.ops.lock().await;
        if let Some(read) = memo.get(&cursor) {
            return read.clone();
        }
        let read = match self.bridge.backup_ops_since(cursor).await {
            Ok(page) => {
                let last = page.ops.iter().map(|op| op.seq_id).max();
                match (last, page.head_seq) {
                    (_, Some(head)) if head < cursor => None,
                    (Some(last), head) => Some(OpsRead {
                        caught_up: head.map_or(page.ops.len() < OPS_PAGE, |head| last >= head),
                        to: last,
                        ops: page.ops,
                    }),
                    (None, head) => Some(OpsRead {
                        ops: Vec::new(),
                        to: head.unwrap_or(cursor),
                        caught_up: true,
                    }),
                }
            }
            Err(error) => {
                tracing::warn!(
                    reason = crate::engine_bridge::error_category(&error),
                    "backup: the ops since the last listing could not be read; the folders are checked by id"
                );
                None
            }
        };
        memo.insert(cursor, read.clone());
        read
    }

    /// This tick's snapshot of the account's tree, read once for every backup the tick lists.
    async fn snapshot(&self) -> Option<Arc<BackupSnapshot>> {
        self.snapshot_read_since(None).await.0
    }

    /// This tick's snapshot, read again unless it was read at or after `since`; and whether this asked the server.
    async fn snapshot_read_since(&self, since: Option<std::time::Instant>) -> (Option<Arc<BackupSnapshot>>, bool) {
        let mut memo = self.snapshot.lock().await;
        if let Some((at, snapshot)) = memo.as_ref()
            && since.is_none_or(|since| *at >= since)
        {
            return (snapshot.clone(), false);
        }
        let read_at = std::time::Instant::now();
        let snapshot = match self.bridge.backup_snapshot().await {
            Ok(snapshot) => Some(Arc::new(snapshot)),
            Err(error) => {
                tracing::warn!(
                    reason = crate::engine_bridge::error_category(&error),
                    "backup: no snapshot; the backup is listed folder by folder, paced"
                );
                None
            }
        };
        *memo = Some((read_at, snapshot.clone()));
        (snapshot, true)
    }

    /// A full listing of `c`: from this tick's snapshot (one request for every backup), else page by page, paced.
    /// A live listing is kept for the next passes. `clocked`: one listing page of the category folder measures the
    /// server's clock for the records' rebuild (a snapshot carries none).
    async fn full_listing(&self, c: &Category, clocked: bool) -> Result<(DeviceTree, RebuildClock), HostError> {
        let device = self
            .bridge
            .backup_folder_state(&self.layout.device_folder_id)
            .await
            .map_err(host_error("device_folder_state"))?;
        if device == BackupFolderState::Gone {
            return Ok((gone_tree(false, None), RebuildClock::UNKNOWN));
        }
        let (folder_id, live) = self.category_folder(c).await?;
        if live == Some(false) {
            return Ok((gone_tree(true, Some(false)), RebuildClock::UNKNOWN));
        }
        // The records' rebuild needs each file's `uploaded_at` and the server's clock, which the snapshot lacks
        // (it has created_at/updated_at only), so a rebuild lists page by page, paced, ONCE per backup (never on
        // every pass); every other full listing is the snapshot. A rebuild's listing is not kept (no cursor): the
        // next pass lists from the snapshot, once, and keeps that.
        let snapshot = if clocked { None } else { self.snapshot().await };
        let (mut tree, clock) = match &snapshot {
            Some(snapshot) => self.list_category(&folder_id, Listing::Snapshot(snapshot)).await?,
            None => self.list_category(&folder_id, Listing::Pages { paced: true }).await?,
        };
        tree.category_folder_live = live;
        let listing = KeptListing {
            category_folder_id: folder_id,
            tree: tree.clone(),
            listed_at: std::time::Instant::now(),
            cursor: snapshot.map(|snapshot| snapshot.seq_id),
        };
        tree_cache::put(&self.layout.device_folder_id, &c.key(), listing);
        tracing::info!(
            category = %c.key(),
            files = tree.files.len(),
            folders = tree.folders.len(),
            complete = tree.complete,
            "backup: listed in full"
        );
        Ok((tree, clock))
    }

    /// [`BackupHost::device_tree`] for the records' rebuild, with the clock its listing measured (LD-0123: core's
    /// `records_from_remote_with_clock` takes it); the pass that follows takes the same listing.
    pub async fn device_tree_for_rebuild(&self, c: &Category) -> Result<(DeviceTree, RebuildClock), HostError> {
        self.device_tree_for_pass(c, true).await
    }

    /// This device's backup of `c` as the server has it now, and the clock of the listing that read it. A device or
    /// category folder that is gone lists nothing, so its clock is `RebuildClock::UNKNOWN` (nothing is rebuilt then).
    async fn device_tree_clocked(&self, c: &Category) -> Result<(DeviceTree, RebuildClock), HostError> {
        let device = self
            .bridge
            .backup_folder_state(&self.layout.device_folder_id)
            .await
            .map_err(host_error("device_folder_state"))?;
        if device == BackupFolderState::Gone {
            return Ok((gone_tree(false, None), RebuildClock::UNKNOWN));
        }
        let (folder_id, live) = self.category_folder(c).await?;
        if live == Some(false) {
            return Ok((gone_tree(true, Some(false)), RebuildClock::UNKNOWN));
        }
        let (mut tree, clock) = self.list_category(&folder_id, Listing::Pages { paced: false }).await?;
        tree.category_folder_live = live;
        Ok((tree, clock))
    }

    /// The category folder's id: the remembered one, else (a first backup, or "Back up again") the live folder of that
    /// name in the device folder (the old mirror's `TestPC/Pictures` is adopted), else a new one. A folder the person
    /// chose: [`Self::chosen_category_folder`].
    async fn category_folder(&self, c: &Category) -> Result<(String, Option<bool>), HostError> {
        if matches!(c, Category::Custom(_)) {
            return self.chosen_category_folder(c).await;
        }
        let db = self.bridge.db();
        if let Some(id) = db
            .backup_category_folder(&c.key())
            .map_err(db_error("category_folder"))?
        {
            let state = self.folder_state(&id).await?;
            return Ok((id, Some(state == BackupFolderState::Live)));
        }
        let id = self
            .bridge
            .backup_ensure_folder(&self.layout.device_folder_id, c.display_name())
            .await
            .map_err(host_error("category_folder_create"))?;
        db.set_backup_category_folder(&c.key(), Some(&id))
            .map_err(db_error("category_folder"))?;
        Ok((id, None))
    }

    /// [`BackupHost::put_record`] for a batch, in one state-DB transaction (the records' rebuild). How many stored.
    pub async fn put_records(&self, c: &Category, records: &[(RecordRef, BackupRecord)]) -> Result<usize, HostError> {
        self.bridge
            .db()
            .put_backup_records_cas(&c.key(), records)
            .map_err(db_error("put_record"))
    }

    async fn folder_state(&self, id: &str) -> Result<BackupFolderState, HostError> {
        self.bridge
            .backup_folder_state(id)
            .await
            .map_err(host_error("category_folder_state"))
    }

    /// A chosen folder's category folder (P3-10, by construction): the folder whose id its settings entry holds
    /// (`ChosenFolder::folder_id`, the local data's owner's entry), never a folder found by its name, which may be a
    /// backup the person kept. The entry outlives a sign-out, which empties the local state.
    /// - Its id, remembered here too (no stop and no sign-out since): live or gone, as every backup's folder (a gone
    ///   one stops the backup, rule 11).
    /// - Its id, not remembered (signed in again, or "Back up again" after a stop): continued while it is a live
    ///   folder in this device's folder (never one elsewhere), and remembered. A listing that is not complete answers
    ///   an error: the folder may be in the part not read, and nothing is made.
    /// - No id (a new add, or a settings file from before the id), or its folder is gone: a new folder. Its id is
    ///   minted and recorded in the entry BEFORE the create, so a create whose answer is lost is found by its id on
    ///   the next pass, never made twice; an id that cannot be recorded makes nothing.
    async fn chosen_category_folder(&self, c: &Category) -> Result<(String, Option<bool>), HostError> {
        let db = self.bridge.db();
        let key = c.key();
        let owner = db
            .owner()
            .map_err(db_error("category_folder"))?
            .ok_or_else(|| HostError("category_folder: no account is bound".into()))?;
        let cfg = crate::config::DesktopConfig::load_for(self.bridge.identity)
            .map_err(|_| HostError("category_folder: the settings could not be read".into()))?;
        let entry = crate::app::bindings::backup_folders::chosen_for(&cfg.backup_chosen_folders, Some(&owner))
            .into_iter()
            .find(|folder| folder.key() == key)
            .ok_or_else(|| HostError("category_folder: not a chosen folder of this account".into()))?;
        let remembered = db.backup_category_folder(&key).map_err(db_error("category_folder"))?;
        let parent = &self.layout.device_folder_id;
        if let Some(id) = entry.folder_id {
            if remembered.as_deref() == Some(id.as_str()) {
                let state = self.folder_state(&id).await?;
                return Ok((id, Some(state == BackupFolderState::Live)));
            }
            let (children, complete) = self
                .bridge
                .backup_list_children(parent)
                .await
                .map_err(host_error("category_folder_state"))?;
            if children.iter().any(|child| child.is_folder && child.id == id) {
                db.set_backup_category_folder(&key, Some(&id))
                    .map_err(db_error("category_folder"))?;
                return Ok((id, None));
            }
            if !complete {
                return Err(HostError(
                    "category_folder: the device folder was not listed completely".into(),
                ));
            }
        }
        let id = self
            .bridge
            .backup_new_folder_id()
            .map_err(host_error("category_folder_create"))?;
        let recorded = crate::config::DesktopConfig::update_for(self.bridge.identity, |cfg| {
            let recorded = crate::app::bindings::backup_folders::record_folder_id(cfg, &key, &owner, &id);
            (recorded, recorded)
        });
        if recorded != Ok(true) {
            return Err(HostError(
                "category_folder: the new folder's id was not recorded".into(),
            ));
        }
        self.bridge
            .backup_create_folder_as(parent, c.display_name(), &id)
            .await
            .map_err(host_error("category_folder_create"))?;
        db.set_backup_category_folder(&key, Some(&id))
            .map_err(db_error("category_folder"))?;
        Ok((id, None))
    }

    /// Every live folder and file under `category_folder`, keys joined with `/`, and the tightest clock its pages
    /// measured. A folder whose name cannot be opened is not entered and makes the listing partial; a file's is
    /// listed by id with an empty key.
    async fn list_category(
        &self,
        category_folder: &str,
        source: Listing<'_>,
    ) -> Result<(DeviceTree, RebuildClock), HostError> {
        let root = parse_id(category_folder).ok_or_else(|| HostError("category folder id is not a uuid".into()))?;
        let mut folders = HashMap::from([(String::new(), root)]);
        let mut files = Vec::new();
        let mut complete = true;
        let mut clock = RebuildClock::UNKNOWN;
        let mut queue = std::collections::VecDeque::from([(String::new(), category_folder.to_string())]);
        let mut requests = 0usize;
        while let Some((prefix, folder_id)) = queue.pop_front() {
            let children = match &source {
                Listing::Snapshot(snapshot) => self.bridge.backup_children_in(snapshot, &folder_id),
                Listing::Pages { paced } => {
                    requests += 1;
                    if *paced && requests > PAGED_LISTING_BURST {
                        tokio::time::sleep(PAGED_LISTING_PACE).await;
                    }
                    let (children, all, folder_clock) = self
                        .bridge
                        .backup_list_children_clocked(&folder_id)
                        .await
                        .map_err(host_error("listing"))?;
                    complete &= all;
                    clock = crate::engine_bridge::tighter_clock(clock, folder_clock);
                    children
                }
            };
            queue.extend(add_children(&prefix, children, &mut folders, &mut files, &mut complete));
        }
        Ok((
            DeviceTree {
                device_folder_live: true,
                category_folder_live: None,
                folders,
                files,
                complete,
            },
            clock,
        ))
    }
}

impl BackupHost for DesktopBackupHost {
    async fn device_tree(&self, c: &Category) -> Result<DeviceTree, HostError> {
        if let Some(tree) = self.listed.lock().ok().and_then(|mut listed| listed.remove(&c.key())) {
            return Ok(tree);
        }
        Ok(self.device_tree_clocked(c).await?.0)
    }

    async fn ensure_folder(&self, parent: Uuid, name: &str) -> Result<Uuid, HostError> {
        let id = self
            .bridge
            .backup_ensure_folder(&parent.to_string(), name)
            .await
            .map_err(host_error("ensure_folder"))?;
        let id = parse_id(&id).ok_or_else(|| HostError("a folder id is not a uuid".into()))?;
        tree_cache::add_folder(parent, name, id);
        Ok(id)
    }

    async fn move_file(&self, id: Uuid, new_parent: Uuid, new_name: &str) -> Result<(), HostError> {
        self.bridge
            .backup_move_file(&id.to_string(), &new_parent.to_string(), new_name)
            .await
            .map_err(host_error("move_file"))
    }

    async fn enqueue_upload(&self, req: UploadRequest) -> Result<(), HostError> {
        match self
            .bridge
            .enqueue_backup_upload(&req)
            .map_err(host_error("enqueue_upload"))?
        {
            BackupEnqueue::Queued(_) | BackupEnqueue::AlreadyQueued(_) | BackupEnqueue::AlreadyBackedUp => Ok(()),
            BackupEnqueue::NoOwner => Err(HostError("enqueue_upload: no account is bound".into())),
        }
    }

    async fn records(&self, c: &Category) -> Result<Vec<BackupRecord>, HostError> {
        self.bridge.db().backup_records(&c.key()).map_err(db_error("records"))
    }

    async fn held_keys(&self, c: &Category) -> Result<Vec<String>, HostError> {
        self.bridge
            .db()
            .backup_held_keys(&c.key())
            .map_err(db_error("held_keys"))
    }

    async fn put_record(&self, c: &Category, expected: &RecordRef, record: BackupRecord) -> Result<bool, HostError> {
        self.bridge
            .db()
            .put_backup_record_cas(&c.key(), expected, &record)
            .map_err(db_error("put_record"))
    }

    async fn mark_removed_elsewhere(&self, c: &Category, records: &[RecordRef]) -> Result<(), HostError> {
        let marked = self
            .bridge
            .db()
            .mark_backup_records_removed_elsewhere(&c.key(), records)
            .map_err(db_error("mark_removed_elsewhere"))?;
        tracing::info!(marked, asked = records.len(), "backup: records removed elsewhere");
        Ok(())
    }

    /// Rule 11 in core's pass: the folder was deleted elsewhere after this pass's notice check found nothing (a race;
    /// the check before the pass catches the usual case), so it stops as `unknown`: a folder gone with no notice.
    async fn turn_off(&self, c: &Category, why: FolderPresence) -> Result<(), HostError> {
        let device_gone = why == FolderPresence::DeviceDeleted;
        // No gate decided it: the backups' last pass and acknowledged notices stay as their gate wrote them.
        self.stop_backups(c, BackupStopReason::Unknown, device_gone, device_gone, None)
            .await
    }
}

/// LD-0139 (spec Amendment 12d): a stop a pass's gate decided: the notices check it read, and the acknowledged set core
/// answered for the backup it decided.
pub struct GateStop<'a> {
    pub check: &'a crate::backup::stop::NoticeCheck,
    pub acknowledged: Vec<String>,
}

impl DesktopBackupHost {
    /// A backup turns off on this device with core's reason (spec Amendment 12): it says why (the stop row the card
    /// shows with "Back up again") and never uploads again by itself. With `every_backup` (a removal of the account's
    /// files, a device folder that is gone, or a notice naming it) every backup of this device turns off at once; a
    /// device folder that is gone also forgets the layout.
    ///
    /// LD-0139 (Amendment 12d): each backup's queued uploads are dropped first (idempotent); then its stop row, its
    /// forgotten category folder, the forgotten layout and, for a stop `gate` decided, each stopped backup's new
    /// `last_pass_at` and acknowledged set go in ONE state-DB transaction. A crash before it commits leaves no stop and
    /// no acknowledgement (the next pass decides the same stop again); after it, both (the stop is never applied
    /// twice).
    pub async fn stop_backups(
        &self,
        c: &Category,
        reason: BackupStopReason,
        device_gone: bool,
        every_backup: bool,
        gate: Option<GateStop<'_>>,
    ) -> Result<(), HostError> {
        let db = self.bridge.db();
        let categories: Vec<Category> = if every_backup || device_gone || reason.files_removed() {
            // Every backup still on; one already off keeps the reason it stopped for.
            let stopped: Vec<String> = db
                .backup_stops()
                .map_err(db_error("turn_off"))?
                .into_iter()
                .map(|s| s.category)
                .collect();
            // Review (2026-11-07): every folder switched on, the ones this tick did not pass included (a folder
            // waiting for the OneDrive answer), so none keeps going after its backup was removed.
            let mut on: Vec<Category> = self.enabled.clone();
            if let Ok(cfg) = crate::config::DesktopConfig::load_for(self.bridge.identity) {
                // The local data's owner's switches alone (P3-8): another account's chosen folder, or an untagged one,
                // is not this device's backup of this account, and its switch is not this stop's to turn off.
                let owners = crate::app::bindings::backup_folders::runner_chosen(db, &cfg);
                for category in cfg
                    .known_folder_backup
                    .iter()
                    .filter(|key| owners_switch(key, &owners))
                    .filter_map(|key| Category::from_key(key))
                {
                    if !on.contains(&category) {
                        on.push(category);
                    }
                }
            }
            let mut all: Vec<Category> = on.into_iter().filter(|e| !stopped.contains(&e.key())).collect();
            if !all.contains(c) {
                all.push(c.clone());
            }
            all
        } else {
            vec![c.clone()]
        };
        let at_ms = chrono::Utc::now().timestamp_millis();
        let keys: Vec<String> = categories.iter().map(Category::key).collect();
        // The queued uploads go first, before the marks are read (review 2026-11-08): a failed read below must not
        // leave a backup that turns off with uploads still queued.
        for key in &keys {
            self.bridge
                .turn_off_backup_uploads(key)
                .await
                .map_err(host_error("turn_off"))?;
        }
        // Amendment 12d: every stopped backup stores core's set for it in the stop's write: the decided one its gate's
        // answer, each other one (a stop that covers every backup) its own window's ids.
        let mut marks = Vec::new();
        if let Some(gate) = gate {
            for category in &categories {
                let acknowledged = if category == c {
                    gate.acknowledged.clone()
                } else {
                    let mark = db.backup_pass_mark(&category.key()).map_err(db_error("turn_off"))?;
                    crate::backup::stop::acknowledged_after(gate.check, &mark, category)
                };
                marks.push(BackupPassRecord {
                    category: category.key(),
                    last_pass_at_ms: gate.check.server_now_ms,
                    acknowledged,
                });
            }
        }
        db.record_backup_stops(&BackupStopWrite {
            reason: reason.as_str(),
            at_ms,
            categories: &keys,
            clear_layout: device_gone,
            marks: &marks,
        })
        .map_err(db_error("turn_off"))?;
        let switched = switch_off_in_settings(self.bridge.identity, &keys);
        tracing::info!(
            turned_off = keys.len(),
            switched,
            reason = reason.as_str(),
            "backup: turned off"
        );
        Ok(())
    }
}
