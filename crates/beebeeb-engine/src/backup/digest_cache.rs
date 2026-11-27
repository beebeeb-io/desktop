//! Folder digests on the backup pass (tasks fd-12, fd-18): what the regular pass trusts of a backup's tree between
//! passes, so it lists only the folders whose digest changed and takes every other folder's children from here.
//!
//! One [`CategoryCache`] per backup (category key): folder id -> (the digest of its subtree read before the listing
//! that gave its children, those children, whether that listing was complete). A folder the tree needs and this cache
//! lacks (or holds from a listing that was not complete) is listed again.
//!
//! ## The after-read rule, a pass later
//!
//! A digest goes in with the listing read AFTER it (the listing comes from a snapshot read once the walk is done). The
//! rule of `crate::folder_digest` (trust only what a digest asked after the read confirms) is kept by the next pass:
//! it compares every stored digest it relies on with one it reads then, after the listing. Equal: nothing moved since
//! the digest was read, so the listing read after it is the folder as it is now; moved: the walk enters the folder and
//! it is listed again. So no pass asks a digest only to confirm its own read.
//!
//! ## Kept across restarts
//!
//! The scheduler holds one [`DigestCache`] for the engine run and hands it to the host it builds each round; every
//! pass writes what it changed to the state DB (`backup_digest_cache`), and the first round of a run reads it back
//! ([`DigestCache::for_run`]). A stop or a lock keeps it; a sign-out empties the table (account data) and moves the
//! DB's account revision, which empties this one; another device folder or category folder starts afresh.
//!
//! Once a day ([`FULL_COMPARISON_EVERY`]) a backup is compared in full with one snapshot: every folder listed from it,
//! and a folder whose children differ from the cache while its digest said unchanged (a wrong server digest) loses its
//! digest and its parents' with it, so the walks enter it again.
//!
//! No folder id, file id or name in a log line: counts and outcomes only.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::engine_bridge::BackupChild;
use crate::folder_digest::Known;
use crate::state_db::StateDb;
use crate::state_db::backup_digest_cache::{BackupDigestRow, BackupDigestWrite};

/// How often a backup is compared in full with one snapshot (the client's check on a wrong server digest).
pub const FULL_COMPARISON_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// How long a server without digests is not asked again.
pub const UNSUPPORTED_RETRY: Duration = Duration::from_secs(60 * 60);

/// One folder of a backup as the last listing gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderEntry {
    /// The digest of the folder's subtree read before its listing (`None`: not known; the next walk enters it).
    pub digest: Option<String>,
    pub children: Vec<BackupChild>,
    /// The listing read every child of the folder.
    pub complete: bool,
}

impl FolderEntry {
    /// Listed completely and holds no folder: a walk need not read its page to know its child folders.
    pub fn leaf(&self) -> bool {
        self.complete && !self.children.iter().any(|child| child.is_folder)
    }
}

/// One backup's folders, under the category folder it was read from.
#[derive(Debug, Default)]
pub struct CategoryCache {
    pub category_folder_id: String,
    pub folders: HashMap<String, FolderEntry>,
    /// When the backup was last compared in full (Unix ms; `None`: never).
    pub full_at_ms: Option<i64>,
    /// Folders changed or forgotten since the last write to the state DB.
    dirty: HashSet<String>,
    forgotten: HashSet<String>,
    /// The full comparison of this pass, to stamp on the next write.
    full_now: Option<i64>,
}

impl CategoryCache {
    fn new(category_folder_id: &str) -> Self {
        Self {
            category_folder_id: category_folder_id.to_string(),
            ..Self::default()
        }
    }

    /// The digests the walk may skip on: those of complete listings.
    pub fn known(&self) -> Known {
        self.folders
            .iter()
            .filter(|(_, entry)| entry.complete)
            .filter_map(|(id, entry)| Some((id.clone(), entry.digest.clone()?)))
            .collect()
    }

    /// The folders a walk need not read: [`FolderEntry::leaf`].
    pub fn leaves(&self) -> HashSet<String> {
        self.folders
            .iter()
            .filter(|(_, entry)| entry.leaf())
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// The folder's cached children, when the tree may take them from here: listed completely.
    pub fn reusable(&self, folder: &str) -> Option<&FolderEntry> {
        self.folders.get(folder).filter(|entry| entry.complete)
    }

    /// Store a folder's entry (written to the state DB with the pass).
    pub fn insert(&mut self, folder: &str, entry: FolderEntry) {
        self.forgotten.remove(folder);
        self.dirty.insert(folder.to_string());
        self.folders.insert(folder.to_string(), entry);
    }

    /// Forget the digest of a folder (it is entered again by the next walk).
    pub fn untrust(&mut self, folder: &str) {
        if let Some(entry) = self.folders.get_mut(folder)
            && entry.digest.take().is_some()
        {
            self.dirty.insert(folder.to_string());
        }
    }

    /// Keep only the folders the tree reached this pass.
    pub fn retain(&mut self, seen: &HashSet<String>) {
        let gone: Vec<String> = self.folders.keys().filter(|id| !seen.contains(*id)).cloned().collect();
        for id in gone {
            self.folders.remove(&id);
            self.dirty.remove(&id);
            self.forgotten.insert(id);
        }
    }

    /// The daily full comparison is due at `now_ms` (a cache that holds nothing is listed in full anyway).
    pub fn full_due(&self, now_ms: i64) -> bool {
        !self.folders.is_empty()
            && self
                .full_at_ms
                .is_none_or(|at| now_ms.saturating_sub(at) >= FULL_COMPARISON_EVERY.as_millis() as i64)
    }

    /// The backup was compared in full (or listed whole) at `now_ms`.
    pub fn compared_in_full(&mut self, now_ms: i64) {
        self.full_at_ms = Some(now_ms);
        self.full_now = Some(now_ms);
    }

    /// Whether anything waits to be written.
    pub fn has_writes(&self) -> bool {
        !self.dirty.is_empty() || !self.forgotten.is_empty() || self.full_now.is_some()
    }

    /// Write what changed since the last write to `db` (backup `key`, under `device`). Kept for the next write
    /// when it fails.
    pub fn write_to(&mut self, db: &StateDb, device: &str, key: &str) -> Result<(), rusqlite::Error> {
        if !self.has_writes() {
            return Ok(());
        }
        let put: Vec<BackupDigestRow> = self
            .dirty
            .iter()
            .filter_map(|id| {
                let entry = self.folders.get(id)?;
                Some(BackupDigestRow {
                    folder_id: id.clone(),
                    digest: entry.digest.clone(),
                    complete: entry.complete,
                    children_json: encode_children(&entry.children),
                })
            })
            .collect();
        let forget: Vec<String> = self.forgotten.iter().cloned().collect();
        db.put_backup_digest_cache(&BackupDigestWrite {
            device_folder_id: device,
            category: key,
            category_folder_id: &self.category_folder_id,
            full_at_ms: self.full_now,
            put: &put,
            forget: &forget,
        })?;
        self.dirty.clear();
        self.forgotten.clear();
        self.full_now = None;
        Ok(())
    }
}

/// One child as stored: what the backup's tree reads of it (the content record and chunk count are not kept; the
/// tree never reads them).
#[derive(serde::Serialize, serde::Deserialize)]
struct StoredChild {
    i: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    n: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    f: bool,
    #[serde(default)]
    s: u64,
    #[serde(default)]
    u: i64,
}

fn encode_children(children: &[BackupChild]) -> String {
    let stored: Vec<StoredChild> = children
        .iter()
        .map(|child| StoredChild {
            i: child.id.clone(),
            n: child.name.clone(),
            f: child.is_folder,
            s: child.size,
            u: child.uploaded_at_ms,
        })
        .collect();
    serde_json::to_string(&stored).unwrap_or_else(|_| "[]".into())
}

/// `None`: the stored text does not read (the folder is listed again).
fn decode_children(json: &str) -> Option<Vec<BackupChild>> {
    let stored: Vec<StoredChild> = serde_json::from_str(json).ok()?;
    Some(
        stored
            .into_iter()
            .map(|child| BackupChild {
                id: child.i,
                name: child.n,
                is_folder: child.f,
                size: child.s,
                uploaded_at_ms: child.u,
                chunk_count: 0,
                record: None,
            })
            .collect(),
    )
}

/// Why the digests are not used for a while: the pass lists as it did before them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fallback {
    /// The server has no digests: asked again after [`UNSUPPORTED_RETRY`].
    Unsupported,
    /// The account's digests are not computed yet: asked again after the server's `Retry-After`.
    NotReady,
}

impl Fallback {
    pub fn as_str(self) -> &'static str {
        match self {
            Fallback::Unsupported => "unsupported",
            Fallback::NotReady => "not_ready",
        }
    }
}

/// Which account, sign-in and device folder the caches belong to.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Run {
    owner: Option<String>,
    revision: u64,
    device: String,
}

/// Every backup's cache, and whether the digests can be used now.
#[derive(Debug, Default)]
pub struct DigestCache {
    run: Option<Run>,
    categories: HashMap<String, CategoryCache>,
    unsupported_until: Option<Instant>,
    not_ready_until: Option<Instant>,
    /// Each fallback is logged once per engine run.
    logged: Vec<Fallback>,
}

/// The cache the scheduler holds and each round's host shares.
pub type SharedDigestCache = Arc<Mutex<DigestCache>>;

impl DigestCache {
    /// The caches belong to the account `db` is bound to now and device folder `device`: another account, a sign-out
    /// since (the DB's account revision moved) or another device folder empties them, and the stored caches of the
    /// new run are read back. A stop or a lock keeps them.
    pub fn for_run(&mut self, db: &StateDb, device: &str) {
        let owner = db.backup_owner_key().ok().flatten();
        let run = Run {
            owner,
            revision: db.account_revision(),
            device: device.to_string(),
        };
        if self.run.as_ref() == Some(&run) {
            return;
        }
        self.categories.clear();
        self.run = Some(run);
        let stored = match db.backup_digest_cache(device) {
            Ok((_, stored)) => stored,
            Err(error) => {
                tracing::warn!(%error, "backup: the stored folder digests could not be read; the passes list them again");
                return;
            }
        };
        let mut folders = 0usize;
        for category in stored {
            let mut cache = CategoryCache::new(&category.category_folder_id);
            cache.full_at_ms = category.full_at_ms;
            for row in category.folders {
                // A row that does not read is left out: the folder is listed again.
                if let Some(children) = decode_children(&row.children_json) {
                    cache.folders.insert(
                        row.folder_id,
                        FolderEntry {
                            digest: row.digest,
                            children,
                            complete: row.complete,
                        },
                    );
                }
            }
            folders += cache.folders.len();
            self.categories.insert(category.category, cache);
        }
        tracing::info!(
            backups = self.categories.len(),
            folders,
            "backup: folder digests read back"
        );
    }

    /// Whether a pass may walk the digests at `now`.
    pub fn usable(&self, now: Instant) -> bool {
        self.unsupported_until.is_none_or(|until| now >= until) && self.not_ready_until.is_none_or(|until| now >= until)
    }

    /// The walk's view of backup `key` under `category_folder_id`: its known digests and its leaves (none when it was
    /// read under another category folder).
    pub fn walk_view(&self, key: &str, category_folder_id: &str) -> (Known, HashSet<String>) {
        match self.categories.get(key) {
            Some(cache) if cache.category_folder_id == category_folder_id => (cache.known(), cache.leaves()),
            _ => Default::default(),
        }
    }

    /// The backup `key`'s cache, taken out for a pass (empty when it was read under another category folder).
    pub fn take(&mut self, key: &str, category_folder_id: &str) -> CategoryCache {
        match self.categories.remove(key) {
            Some(cache) if cache.category_folder_id == category_folder_id => cache,
            _ => CategoryCache::new(category_folder_id),
        }
    }

    /// Put a pass's cache back.
    pub fn put(&mut self, key: &str, cache: CategoryCache) {
        self.categories.insert(key.to_string(), cache);
    }

    /// The walk answered `fallback`: the passes list without digests until `now + retry_after` (an old server:
    /// [`UNSUPPORTED_RETRY`]). The caches are kept: every digest is compared again when the walks come back. `true`
    /// the first time this run (the caller logs it).
    pub fn fall_back(&mut self, fallback: Fallback, retry_after: Duration, now: Instant) -> bool {
        match fallback {
            Fallback::Unsupported => self.unsupported_until = Some(now + UNSUPPORTED_RETRY),
            Fallback::NotReady => self.not_ready_until = Some(now + retry_after),
        }
        if self.logged.contains(&fallback) {
            return false;
        }
        self.logged.push(fallback);
        true
    }
}

#[cfg(test)]
mod tests {
    //! The digest pass against the fake server's `LargeAccount::REFERENCE`: the backup is `Backups/Laptop` (about
    //! 9,500 files in about 675 folders); the device folder is a folder of its own per host, so the RE3-a kept
    //! listings (process-wide, by device folder) of two hosts never meet.

    use super::*;
    use crate::api_client::ApiClient;
    use crate::backup::host::DesktopBackupHost;
    use crate::engine_bridge::EngineBridge;
    use crate::state_db::{BackupLayout, StateDb};
    use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
    use crate::test_api::large_account::{LargeAccount, seed};
    use crate::test_api::{FakeApi, Request};
    use beebeeb_backup::DeviceTree;
    use beebeeb_types::backup::Category;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// `Backups/Laptop` in the seeded account (`large_account::id(1, 4)`).
    const LAPTOP: &str = "b0000001-0025-4000-8000-000000000004";
    const KEY: &str = "pictures";

    struct Fx {
        account: Arc<FakeAccount>,
        fake: FakeApi,
        bridge: Arc<EngineBridge>,
        cache: SharedDigestCache,
        device: String,
        _dir: tempfile::TempDir,
    }

    fn uid(kind: u32, n: u32) -> String {
        format!("e{kind:07x}-0012-4000-8000-{n:012x}")
    }

    /// The seeded account, served by `serve` (the plain fake unless a test wraps it).
    fn fixture_with(serve: impl FnOnce(Arc<FakeAccount>) -> FakeApi) -> Fx {
        let (account, _) = FakeAccount::register("backup-digests@example.invalid", "an invented password", TEST_PHRASE);
        let account = Arc::new(account);
        seed(&account, &LargeAccount::REFERENCE);
        let fake = serve(account.clone());
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
        db.set_owner(&crate::account_binding::Identity::new(Some("u-digests"), None))
            .unwrap();
        db.set_backup_category_folder(KEY, Some(LAPTOP)).unwrap();
        let api = ApiClient::new(fake.base.clone(), account.session_token.clone(), account.master_key);
        let bridge = Arc::new(EngineBridge::new(&crate::identity::TEST, db, Arc::new(api)));
        let fx = Fx {
            account,
            fake,
            bridge,
            cache: SharedDigestCache::default(),
            device: String::new(),
            _dir: dir,
        };
        Fx {
            device: fx.new_device(),
            ..fx
        }
    }

    fn fixture() -> Fx {
        fixture_with(crate::test_api::fake_account::start_fake)
    }

    impl Fx {
        /// A new live folder at the root to serve as a host's device folder (outside the backup: no digest of it moves).
        /// Unique in the process: the RE3-a kept listings are process-wide, by device folder.
        fn new_device(&self) -> String {
            let id = uuid::Uuid::new_v4().to_string();
            self.account.add_item(&id, None, &format!("device {id}"), true, 0);
            id
        }

        fn layout(&self, device: &str) -> BackupLayout {
            BackupLayout {
                root_id: device.to_string(),
                device_folder_id: device.to_string(),
                device_folder_name: "TestPC".into(),
            }
        }

        /// A round's host (the scheduler builds one per round) over the run's digest cache.
        fn host(&self) -> DesktopBackupHost {
            DesktopBackupHost::new(self.bridge.clone(), self.layout(&self.device), vec![Category::Pictures])
                .with_digest_cache(self.cache.clone())
        }

        /// One regular pass's tree and how many requests it made.
        async fn pass(&self, host: &DesktopBackupHost, rebuild: bool) -> (DeviceTree, usize) {
            let before = self.fake.requests().len();
            let (tree, _) = host.device_tree_for_pass(&Category::Pictures, rebuild).await.unwrap();
            (tree, self.fake.requests().len() - before)
        }

        async fn digest_pass(&self) -> (DeviceTree, usize) {
            self.pass(&self.host(), false).await
        }

        /// Today's code (no digest cache) on a device folder no host listed yet: a full listing, from one snapshot.
        async fn full_listing(&self) -> DeviceTree {
            let device = self.new_device();
            let host = DesktopBackupHost::new(self.bridge.clone(), self.layout(&device), vec![Category::Pictures]);
            self.pass(&host, false).await.0
        }

        fn digest_requests(&self) -> usize {
            self.fake
                .requests()
                .iter()
                .filter(|r: &&Request| r.route() == "GET /api/v1/folder-digests")
                .count()
        }

        fn cached_root_digest(&self) -> Option<String> {
            let cache = self.cache.lock().unwrap();
            cache.categories[KEY].folders[LAPTOP].digest.clone()
        }
    }

    /// The tree with its files in key order (the listing order is the walk's).
    fn sorted(mut tree: DeviceTree) -> DeviceTree {
        tree.files
            .sort_by(|a, b| a.key.cmp(&b.key).then(a.remote_id.cmp(&b.remote_id)));
        tree
    }

    fn assert_big(tree: &DeviceTree) {
        assert!(tree.complete, "every name opened");
        assert!(
            tree.files.len() > 5_000,
            "the Laptop backup: {} files",
            tree.files.len()
        );
        assert!(
            tree.folders.len() > 500,
            "the Laptop backup: {} folders",
            tree.folders.len()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn digest_pass_lists_once_then_only_what_changed() {
        let fx = fixture();
        let mut chain = vec![LAPTOP.to_string()];
        for level in 1..=6u32 {
            let id = uid(1, level);
            fx.account
                .add_item(&id, Some(&chain[chain.len() - 1]), &format!("level {level}"), true, 0);
            chain.push(id);
        }

        // Pass 1: nothing cached, everything listed; the tree is a full listing's.
        let (first, _) = fx.digest_pass().await;
        assert_big(&first);
        assert_eq!(sorted(first.clone()), sorted(fx.full_listing().await));
        assert!(fx.cached_root_digest().is_some(), "trusted after the read");

        // Pass 2, nothing changed: the two folder states and one digest; the same tree.
        let (second, requests) = fx.digest_pass().await;
        assert!(requests <= 3, "an idle pass: {requests} requests");
        assert_eq!(sorted(second), sorted(first));

        // A file added 6 levels deep: the chain is listed again, nothing else.
        fx.account.add_item(&uid(2, 1), Some(&chain[6]), "deep.jpg", false, 42);
        let (third, requests) = fx.digest_pass().await;
        assert!(requests <= 3 * 7 + 1, "a file 6 deep: {requests} requests");
        let deep = "level 1/level 2/level 3/level 4/level 5/level 6/deep.jpg";
        assert!(third.files.iter().any(|f| f.key == deep && f.size == 42));
        assert_eq!(sorted(third), sorted(fx.full_listing().await));

        // And the pass after that is idle again.
        let (_, requests) = fx.digest_pass().await;
        assert!(requests <= 3, "idle again: {requests} requests");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn digest_pass_sees_a_folder_renamed_elsewhere() {
        let fx = fixture();
        let (first, _) = fx.digest_pass().await;
        assert_big(&first);
        // A folder of the backup that holds files, renamed on the web.
        let old = first
            .files
            .iter()
            .find_map(|f| f.key.split_once('/').map(|(folder, _)| folder.to_string()))
            .expect("a folder with files");
        let id = first.folders[&old].to_string();
        fx.account.add_item(&id, Some(LAPTOP), "Renamed on the web", true, 0);

        let (next, requests) = fx.digest_pass().await;
        assert!(requests <= 3 * 2 + 1 + 3, "one folder renamed: {requests} requests");
        let prefix = format!("{old}/");
        assert!(!next.folders.contains_key(&old));
        assert!(
            !next
                .folders
                .keys()
                .chain(next.files.iter().map(|f| &f.key))
                .any(|k| k.starts_with(&prefix))
        );
        assert!(next.files.iter().any(|f| f.key.starts_with("Renamed on the web/")));
        assert_eq!(sorted(next), sorted(fx.full_listing().await));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn digest_pass_lists_again_what_changed_while_it_read() {
        let armed = Arc::new(AtomicBool::new(false));
        let late = uid(3, 1);
        let target = Arc::new(std::sync::Mutex::new(String::new()));
        let (armed_in, late_in, target_in) = (armed.clone(), late.clone(), target.clone());
        let fx = fixture_with(move |account| {
            let inner = account_handler(account.clone());
            FakeApi::start(Arc::new(move |req: &Request| {
                let reply = inner(req);
                // The change lands after the listing was read and before the after-read digest.
                if req.route() == "GET /api/v1/sync/snapshot" && armed_in.swap(false, Ordering::SeqCst) {
                    let folder = target_in.lock().unwrap().clone();
                    account.add_item(&late_in, Some(&folder), "late.jpg", false, 7);
                }
                reply
            }))
        });
        let (first, _) = fx.digest_pass().await;
        assert_big(&first);
        let (folder_key, folder) = first
            .folders
            .iter()
            .find(|(key, _)| !key.is_empty())
            .map(|(k, v)| (k.clone(), v.to_string()))
            .unwrap();
        *target.lock().unwrap() = folder;

        // Something changes, so the pass reads a snapshot; the late file lands after it.
        fx.account.add_item(&uid(3, 2), Some(LAPTOP), "early.jpg", false, 3);
        armed.store(true, Ordering::SeqCst);
        let (during, _) = fx.digest_pass().await;
        assert!(!armed.load(Ordering::SeqCst), "the snapshot was read");
        assert!(during.files.iter().any(|f| f.key == "early.jpg"));
        assert!(!during.files.iter().any(|f| f.remote_id.to_string() == late));
        // The stored digest is the walk's, read before the late file: the next pass's digest differs and enters it.

        let (next, _) = fx.digest_pass().await;
        let key = format!("{folder_key}/late.jpg");
        assert!(next.files.iter().any(|f| f.key == key), "listed again on the next pass");
        assert_eq!(sorted(next), sorted(fx.full_listing().await));
        assert!(fx.cached_root_digest().is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn digest_pass_on_an_old_server_lists_as_before() {
        let fx = fixture();
        fx.account.digests_as_old_server(true);
        // Today's code on its own device folder, two passes.
        let device = fx.new_device();
        let today = DesktopBackupHost::new(fx.bridge.clone(), fx.layout(&device), vec![Category::Pictures]);
        let (today_1, today_1_requests) = fx.pass(&today, false).await;
        let today = DesktopBackupHost::new(fx.bridge.clone(), fx.layout(&device), vec![Category::Pictures]);
        let (today_2, today_2_requests) = fx.pass(&today, false).await;

        let digests_before = fx.digest_requests();
        let (first, first_requests) = fx.digest_pass().await;
        let (second, second_requests) = fx.digest_pass().await;
        assert_eq!(first, today_1);
        assert_eq!(second, today_2);
        // The first pass asks once (the device folder's digests); not again within the hour.
        assert_eq!(first_requests, today_1_requests + 1);
        assert_eq!(second_requests, today_2_requests);
        assert_eq!(fx.digest_requests() - digests_before, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn digest_cache_leaves_the_rebuild_listing_alone() {
        let fx = fixture();
        let (_, _) = fx.digest_pass().await;
        let digests_before = fx.digest_requests();
        let (with_cache, with_cache_requests) = fx.pass(&fx.host(), true).await;
        let device = fx.new_device();
        let today = DesktopBackupHost::new(fx.bridge.clone(), fx.layout(&device), vec![Category::Pictures]);
        let (without, without_requests) = fx.pass(&today, true).await;
        assert_eq!(with_cache, without);
        assert_eq!(with_cache_requests, without_requests);
        assert_eq!(fx.digest_requests(), digests_before, "the rebuild asks no digest");
    }

    /// `Backups/Laptop` adopted as the category folder of this host's device folder (as the layout has it).
    fn adopt(fx: &Fx) {
        fx.account.add_item(LAPTOP, Some(&fx.device), "Pictures", true, 0);
    }

    /// [`adopt`], and the backup grown to at least 2,700 folders.
    fn adopt_and_grow(fx: &Fx) {
        adopt(fx);
        for batch in 0..30u32 {
            let parent = uid(5, batch);
            fx.account
                .add_item(&parent, Some(LAPTOP), &format!("batch {batch}"), true, 0);
            for n in 0..70u32 {
                fx.account
                    .add_item(&uid(6, batch * 100 + n), Some(&parent), &format!("folder {n}"), true, 0);
            }
        }
    }

    /// A chain of `depth` folders under the category folder; the deepest one's id.
    fn chain(fx: &Fx, kind: u32, depth: u32) -> String {
        let mut parent = LAPTOP.to_string();
        for level in 1..=depth {
            let id = uid(kind, level);
            fx.account
                .add_item(&id, Some(&parent), &format!("level {level}"), true, 0);
            parent = id;
        }
        parent
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_warm_restart_and_an_idle_pass_cost_at_most_two_requests() {
        let fx = fixture();
        adopt_and_grow(&fx);
        let (first, _) = fx.digest_pass().await;
        assert!(first.folders.len() >= 2_700, "{} folders", first.folders.len());
        assert_eq!(sorted(first.clone()), sorted(fx.full_listing().await));

        let (idle, requests) = fx.digest_pass().await;
        assert!(requests <= 2, "an idle pass: {requests} requests");
        assert_eq!(sorted(idle), sorted(first.clone()));

        // A restart: a new engine run reads the cache back from the state DB.
        let restarted = Fx {
            cache: SharedDigestCache::default(),
            ..fx
        };
        restarted
            .cache
            .lock()
            .unwrap()
            .for_run(restarted.bridge.db(), &restarted.device);
        let (warm, requests) = restarted.digest_pass().await;
        assert!(requests <= 2, "a warm start: {requests} requests");
        assert_eq!(sorted(warm), sorted(first));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn one_change_five_levels_deep_costs_at_most_depth_plus_two() {
        let fx = fixture();
        adopt(&fx);
        let deepest = chain(&fx, 1, 5);
        let (_, _) = fx.digest_pass().await;
        let (_, requests) = fx.digest_pass().await;
        assert!(requests <= 2, "idle: {requests} requests");

        fx.account.add_item(&uid(2, 1), Some(&deepest), "deep.jpg", false, 42);
        let (changed, requests) = fx.digest_pass().await;
        assert!(requests <= 5 + 2, "a file 5 levels deep: {requests} requests");
        let deep = "level 1/level 2/level 3/level 4/level 5/deep.jpg";
        assert!(changed.files.iter().any(|f| f.key == deep && f.size == 42));
        assert_eq!(sorted(changed), sorted(fx.full_listing().await));

        let (_, requests) = fx.digest_pass().await;
        assert!(requests <= 2, "idle again: {requests} requests");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_folder_moved_into_a_folder_not_read_is_listed_whole() {
        let fx = fixture();
        adopt(&fx);
        let deepest = chain(&fx, 1, 3);
        let (first, _) = fx.digest_pass().await;
        // A folder of the backup with files, moved into the empty deepest folder, and changed on the way.
        let (old, moved) = first
            .files
            .iter()
            .find_map(|f| f.key.split_once('/').map(|(folder, _)| folder.to_string()))
            .map(|old| (old.clone(), first.folders[&old].to_string()))
            .expect("a folder with files");
        fx.account.add_item(&moved, Some(&deepest), "moved", true, 0);
        fx.account.add_item(&uid(2, 2), Some(&moved), "new.jpg", false, 9);
        let (next, _) = fx.digest_pass().await;
        assert!(!next.folders.contains_key(&old));
        assert!(
            next.files
                .iter()
                .any(|f| f.key == "level 1/level 2/level 3/moved/new.jpg")
        );
        assert_eq!(sorted(next), sorted(fx.full_listing().await));
        let (_, requests) = fx.digest_pass().await;
        assert!(requests <= 2, "idle again: {requests} requests");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_daily_full_comparison_catches_a_wrong_digest() {
        let fx = fixture();
        adopt(&fx);
        let (first, _) = fx.digest_pass().await;
        // A cache that went wrong while its digests stayed (what a wrong server digest would leave): a folder's files
        // dropped from it.
        let folder = first
            .files
            .iter()
            .find_map(|f| {
                f.key
                    .rsplit_once('/')
                    .map(|(folder, _)| first.folders[folder].to_string())
            })
            .expect("a folder with files");
        {
            let mut all = fx.cache.lock().unwrap();
            let pictures = all.categories.get_mut(KEY).unwrap();
            pictures
                .folders
                .get_mut(&folder)
                .unwrap()
                .children
                .retain(|c| c.is_folder);
            pictures.full_at_ms = Some(chrono::Utc::now().timestamp_millis());
        }
        let (wrong, requests) = fx.digest_pass().await;
        assert!(requests <= 2);
        assert_ne!(sorted(wrong), sorted(first.clone()), "the digests cannot see it");

        // A day later: one snapshot, every folder compared; the tree is right again, and the folder and the folders
        // above it are walked again.
        fx.cache.lock().unwrap().categories.get_mut(KEY).unwrap().full_at_ms = Some(0);
        let (full, requests) = fx.digest_pass().await;
        assert!(requests <= 2, "the full comparison: {requests} requests");
        assert_eq!(sorted(full), sorted(first.clone()));
        assert_eq!(fx.cached_root_digest(), None);
        let (_, _) = fx.digest_pass().await;
        let (idle, requests) = fx.digest_pass().await;
        assert!(requests <= 2, "idle again: {requests} requests");
        assert_eq!(sorted(idle), sorted(first));
    }

    #[test]
    fn another_device_folder_empties_the_caches_and_an_old_server_is_asked_again_after_an_hour() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.set_owner(&crate::account_binding::Identity::new(Some("u-1"), None))
            .unwrap();
        let entry = FolderEntry {
            digest: Some("d".into()),
            children: Vec::new(),
            complete: true,
        };
        let mut cache = DigestCache::default();
        cache.for_run(&db, "dev");
        let mut pictures = cache.take(KEY, "cat");
        pictures.insert("cat", entry.clone());
        pictures.write_to(&db, "dev", KEY).unwrap();
        cache.put(KEY, pictures);

        // Read back by the next run.
        let mut next = DigestCache::default();
        next.for_run(&db, "dev");
        assert_eq!(next.take(KEY, "cat").folders["cat"], entry);
        assert!(next.take(KEY, "cat").folders.is_empty(), "taken");
        let mut other = DigestCache::default();
        other.for_run(&db, "dev");
        assert!(
            other.take(KEY, "other-cat").folders.is_empty(),
            "another category folder"
        );
        let mut moved = DigestCache::default();
        moved.for_run(&db, "dev2");
        assert!(moved.take(KEY, "cat").folders.is_empty(), "another device folder");

        let now = Instant::now();
        assert!(cache.fall_back(Fallback::NotReady, Duration::from_secs(30), now));
        assert!(!cache.usable(now));
        assert!(cache.usable(now + Duration::from_secs(30)));
        assert!(
            !cache.fall_back(Fallback::NotReady, Duration::from_secs(30), now),
            "logged once"
        );
        assert!(cache.fall_back(Fallback::Unsupported, Duration::ZERO, now));
        assert!(!cache.usable(now + Duration::from_secs(3599)));
        assert!(cache.usable(now + UNSUPPORTED_RETRY));
    }
}
