//! Folder digests on the backup pass (task fd-12): what the regular pass trusts of a backup's tree between passes, so
//! it lists only the folders whose digest changed (`crate::folder_digest::changed_folders`) and takes every other
//! folder's children from here.
//!
//! One [`CategoryCache`] per backup (category key): folder id -> (the digest trusted for its subtree, its children as
//! the last listing gave them, whether that listing was complete). A digest goes in only by the after-read rule
//! (`crate::folder_digest`): the category folder's digest, asked after the pass built its tree, is still the one the
//! walk read. An entry without a digest is never in the walk's [`Known`], so the next walk enters it again; a folder
//! the tree needs and this cache lacks (or holds from a listing that was not complete) is listed again.
//!
//! In memory only, never on disk: the backup scheduler holds one [`DigestCache`] for the engine run and hands it to
//! the host it builds each round. A stop, a lock or a sign-out (the scheduler's flag epoch moves) or another device
//! folder empties it ([`DigestCache::for_run`]).
//!
//! No folder id, file id or name in a log line: counts and outcomes only.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::engine_bridge::BackupChild;
use crate::folder_digest::Known;

/// One folder of a backup as the last listing gave it.
#[derive(Debug, Clone)]
pub struct FolderEntry {
    /// The digest trusted for the folder's subtree (`None`: listed, not trusted; the next walk enters it).
    pub digest: Option<String>,
    pub children: Vec<BackupChild>,
    /// The listing read every child of the folder.
    pub complete: bool,
}

/// One backup's folders, under the category folder it was read from.
#[derive(Debug, Default)]
pub struct CategoryCache {
    pub category_folder_id: String,
    pub folders: HashMap<String, FolderEntry>,
}

impl CategoryCache {
    /// The digests the walk may skip on: the trusted entries of complete listings.
    pub fn known(&self) -> Known {
        self.folders
            .iter()
            .filter(|(_, entry)| entry.complete)
            .filter_map(|(id, entry)| Some((id.clone(), entry.digest.clone()?)))
            .collect()
    }

    /// The folder's cached children, when the tree may take them from here: listed completely.
    pub fn reusable(&self, folder: &str) -> Option<&FolderEntry> {
        self.folders.get(folder).filter(|entry| entry.complete)
    }
}

/// Why the digests are not used for a while: the pass lists as it did before them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fallback {
    /// The server has no digests: not asked again in this engine run.
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

/// Every backup's cache for one engine run, and whether the digests can be used now.
#[derive(Debug, Default)]
pub struct DigestCache {
    /// The sign-in (the scheduler's flag epoch) and the device folder the caches belong to.
    run: Option<(u64, String)>,
    categories: HashMap<String, CategoryCache>,
    unsupported: bool,
    not_ready_until: Option<Instant>,
    /// Each fallback is logged once per engine run.
    logged: Vec<Fallback>,
}

/// The cache the scheduler holds and each round's host shares.
pub type SharedDigestCache = Arc<Mutex<DigestCache>>;

impl DigestCache {
    /// The caches belong to sign-in `epoch` and device folder `device`; another one empties them first.
    pub fn for_run(&mut self, epoch: u64, device: &str) {
        let run = Some((epoch, device.to_string()));
        if self.run != run {
            self.categories.clear();
            self.run = run;
        }
    }

    /// Whether a pass may walk the digests at `now`.
    pub fn usable(&self, now: Instant) -> bool {
        !self.unsupported && self.not_ready_until.is_none_or(|until| now >= until)
    }

    /// The backup `key`'s cache, taken out for a pass (empty when it was read under another category folder).
    pub fn take(&mut self, key: &str, category_folder_id: &str) -> CategoryCache {
        match self.categories.remove(key) {
            Some(cache) if cache.category_folder_id == category_folder_id => cache,
            _ => CategoryCache {
                category_folder_id: category_folder_id.to_string(),
                folders: HashMap::new(),
            },
        }
    }

    /// Put a pass's cache back.
    pub fn put(&mut self, key: &str, cache: CategoryCache) {
        self.categories.insert(key.to_string(), cache);
    }

    /// The walk answered `fallback`: the passes list without digests (for the run, or until `retry_after`).
    /// `true` the first time this run (the caller logs it).
    pub fn fall_back(&mut self, fallback: Fallback, retry_after: Duration, now: Instant) -> bool {
        match fallback {
            Fallback::Unsupported => {
                self.unsupported = true;
                self.categories.clear();
            }
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
        assert_eq!(
            fx.cached_root_digest(),
            None,
            "not trusted: the digest moved after the read"
        );

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
        // The first pass asks once (and the two folder states it read before the answer); never again this run.
        assert_eq!(first_requests, today_1_requests + 3);
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

    #[test]
    fn a_sign_out_or_another_device_empties_the_caches() {
        let mut cache = DigestCache::default();
        cache.for_run(1, "dev");
        let mut pictures = cache.take(KEY, "cat");
        pictures.folders.insert(
            "cat".into(),
            FolderEntry {
                digest: Some("d".into()),
                children: Vec::new(),
                complete: true,
            },
        );
        cache.put(KEY, pictures);
        cache.for_run(1, "dev");
        assert_eq!(cache.take(KEY, "cat").known().len(), 1);
        cache.put(KEY, CategoryCache::default());
        assert!(cache.take(KEY, "cat").folders.is_empty(), "another category folder");
        cache.put(
            KEY,
            CategoryCache {
                category_folder_id: "cat".into(),
                folders: HashMap::from([(
                    "cat".into(),
                    FolderEntry {
                        digest: Some("d".into()),
                        children: Vec::new(),
                        complete: true,
                    },
                )]),
            },
        );
        cache.for_run(2, "dev");
        assert!(cache.take(KEY, "cat").folders.is_empty(), "signed out and in");
        let now = Instant::now();
        assert!(cache.fall_back(Fallback::NotReady, Duration::from_secs(30), now));
        assert!(!cache.usable(now));
        assert!(cache.usable(now + Duration::from_secs(30)));
        assert!(
            !cache.fall_back(Fallback::NotReady, Duration::from_secs(30), now),
            "logged once"
        );
        assert!(cache.fall_back(Fallback::Unsupported, Duration::ZERO, now));
        assert!(!cache.usable(now + Duration::from_secs(3600)));
    }
}
