use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::pin::Pin;

use beebeeb_core::kdf::MasterKey;
use beebeeb_core::search_index::{DEFAULT_NUM_SHARDS, EncryptedShard, SearchIndex, tokenize};
use beebeeb_core::search_sync::{self, ShardCoord, ShardRef};
use serde::Serialize;

use crate::api_client::{ApiClient, SearchShardRef};
use crate::state_db::StateDb;

type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = anyhow::Result<T>> + Send + 'a>>;

pub(crate) trait SearchShardStore: Sync {
    fn master_key_bytes(&self) -> [u8; 32];
    fn fetch_manifest<'a>(&'a self) -> StoreFuture<'a, Vec<SearchShardRef>>;
    fn get_shard<'a>(&'a self, bucket: u32, page: u32) -> StoreFuture<'a, Option<Vec<u8>>>;
    fn put_shard<'a>(&'a self, shard: EncryptedShard) -> StoreFuture<'a, SearchShardRef>;
    fn delete_shard<'a>(&'a self, bucket: u32, page: u32) -> StoreFuture<'a, ()>;
}

impl SearchShardStore for ApiClient {
    fn master_key_bytes(&self) -> [u8; 32] {
        *self.master_key()
    }

    fn fetch_manifest<'a>(&'a self) -> StoreFuture<'a, Vec<SearchShardRef>> {
        Box::pin(async move { self.fetch_search_shard_manifest().await })
    }

    fn get_shard<'a>(&'a self, bucket: u32, page: u32) -> StoreFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move { self.get_search_shard(bucket, page).await })
    }

    fn put_shard<'a>(&'a self, shard: EncryptedShard) -> StoreFuture<'a, SearchShardRef> {
        Box::pin(async move { self.put_search_shard(&shard).await })
    }

    fn delete_shard<'a>(&'a self, bucket: u32, page: u32) -> StoreFuture<'a, ()> {
        Box::pin(async move { self.delete_search_shard(bucket, page).await })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalSearchRecord {
    pub(crate) file_id: String,
    pub(crate) name: String,
    pub(crate) rel_path: String,
    pub(crate) status: String,
    pub(crate) size_bytes: i64,
    pub(crate) modified_at: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct LocalSearchIndex {
    pub(crate) index: SearchIndex,
    pub(crate) records: BTreeMap<String, LocalSearchRecord>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DesktopSearchIndexState {
    Ready,
    Syncing,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct DesktopSearchResult {
    pub(crate) file_id: String,
    pub(crate) name: String,
    pub(crate) rel_path: String,
    pub(crate) status: String,
    pub(crate) size_bytes: i64,
    pub(crate) modified_at: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct DesktopSearchResponse {
    pub(crate) query: String,
    pub(crate) index_state: DesktopSearchIndexState,
    pub(crate) indexed_file_count: usize,
    pub(crate) results: Vec<DesktopSearchResult>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct SearchIndexSyncSummary {
    pub(crate) indexed_file_count: usize,
    pub(crate) remote_manifest_count: usize,
    pub(crate) put_count: usize,
    pub(crate) get_count: usize,
    pub(crate) delete_count: usize,
    pub(crate) num_shards: u32,
}

pub(crate) fn syncing_response(query: String) -> DesktopSearchResponse {
    DesktopSearchResponse {
        query,
        index_state: DesktopSearchIndexState::Syncing,
        indexed_file_count: 0,
        results: Vec::new(),
    }
}

pub(crate) fn build_local_index(db: &StateDb) -> anyhow::Result<LocalSearchIndex> {
    let mut records = BTreeMap::new();
    let mut pairs = Vec::new();
    for entry in db.list_files()? {
        if entry.is_dir() {
            continue;
        }
        let name = leaf_name(&entry.path);
        if name.trim().is_empty() {
            continue;
        }
        pairs.push((entry.file_id.clone(), name.clone()));
        records.insert(
            entry.file_id.clone(),
            LocalSearchRecord {
                file_id: entry.file_id,
                name,
                rel_path: entry.path,
                status: entry.status.as_str().to_string(),
                size_bytes: entry.size_bytes,
                modified_at: entry.modified_at,
            },
        );
    }
    Ok(LocalSearchIndex {
        index: SearchIndex::build(&pairs, DEFAULT_NUM_SHARDS),
        records,
    })
}

pub(crate) fn query_local_index(local: &LocalSearchIndex, query: &str, limit: usize) -> DesktopSearchResponse {
    let trimmed = query.trim();
    let mut results: Vec<_> = if trimmed.is_empty() {
        Vec::new()
    } else {
        local
            .index
            .query(trimmed)
            .into_iter()
            .filter_map(|file_id| local.records.get(&file_id))
            .cloned()
            .collect()
    };

    results.sort_by(|a, b| compare_records_for_query(a, b, trimmed));
    results.truncate(limit);

    DesktopSearchResponse {
        query: query.to_string(),
        index_state: DesktopSearchIndexState::Ready,
        indexed_file_count: local.index.file_count(),
        results: results
            .into_iter()
            .map(|r| DesktopSearchResult {
                file_id: r.file_id,
                name: r.name,
                rel_path: r.rel_path,
                status: r.status,
                size_bytes: r.size_bytes,
                modified_at: r.modified_at,
            })
            .collect(),
    }
}

pub(crate) fn local_index_signature(db: &StateDb) -> anyhow::Result<u64> {
    let local = build_local_index(db)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    local.index.file_count().hash(&mut hasher);
    for (file_id, record) in &local.records {
        file_id.hash(&mut hasher);
        record.name.hash(&mut hasher);
        record.rel_path.hash(&mut hasher);
    }
    Ok(hasher.finish())
}

pub(crate) async fn load_remote_index(api: &ApiClient) -> anyhow::Result<Option<SearchIndex>> {
    load_remote_index_from_store(api).await
}

async fn load_remote_index_from_store(store: &impl SearchShardStore) -> anyhow::Result<Option<SearchIndex>> {
    let manifest = store.fetch_manifest().await?;
    if manifest.is_empty() {
        return Ok(None);
    }
    let mut shards = Vec::new();
    for coord in &manifest {
        if let Some(blob) = store.get_shard(coord.bucket, coord.page).await? {
            shards.push(EncryptedShard {
                bucket: coord.bucket,
                page: coord.page,
                blob,
            });
        }
    }
    let master_key = MasterKey::from_bytes(store.master_key_bytes());
    Ok(Some(SearchIndex::from_encrypted_shards(
        &master_key,
        &shards,
        DEFAULT_NUM_SHARDS,
    )?))
}

pub(crate) async fn sync_local_index_to_remote(
    db: &StateDb,
    api: &ApiClient,
) -> anyhow::Result<SearchIndexSyncSummary> {
    sync_local_index_to_store(db, api).await
}

async fn sync_local_index_to_store(
    db: &StateDb,
    store: &impl SearchShardStore,
) -> anyhow::Result<SearchIndexSyncSummary> {
    let local = build_local_index(db)?;
    let master_key = MasterKey::from_bytes(store.master_key_bytes());
    let encrypted = local.index.encrypt_shards(&master_key)?;
    let remote_manifest = store.fetch_manifest().await?;

    let remote_versions: BTreeMap<(u32, u32), i64> = remote_manifest
        .iter()
        .map(|r| ((r.bucket, r.page), r.version))
        .collect();
    let local_refs: Vec<ShardRef> = encrypted
        .iter()
        .map(|s| ShardRef {
            bucket: s.bucket,
            page: s.page,
            version: remote_versions
                .get(&(s.bucket, s.page))
                .copied()
                .unwrap_or(0)
                .saturating_add(1),
        })
        .collect();
    let remote_refs: Vec<ShardRef> = remote_manifest.iter().map(to_core_shard_ref).collect();
    let plan = search_sync::diff_manifest(&local_refs, &remote_refs, true);

    let shard_map: BTreeMap<ShardCoord, EncryptedShard> = encrypted
        .into_iter()
        .map(|s| {
            (
                ShardCoord {
                    bucket: s.bucket,
                    page: s.page,
                },
                s,
            )
        })
        .collect();

    let mut put_count = 0usize;
    for coord in &plan.to_put {
        if let Some(shard) = shard_map.get(coord) {
            store.put_shard(shard.clone()).await?;
            put_count += 1;
        }
    }
    let mut delete_count = 0usize;
    for coord in &plan.to_delete {
        store.delete_shard(coord.bucket, coord.page).await?;
        delete_count += 1;
    }

    Ok(SearchIndexSyncSummary {
        indexed_file_count: local.index.file_count(),
        remote_manifest_count: remote_manifest.len(),
        put_count,
        get_count: plan.to_get.len(),
        delete_count,
        num_shards: DEFAULT_NUM_SHARDS,
    })
}

fn to_core_shard_ref(r: &SearchShardRef) -> ShardRef {
    ShardRef {
        bucket: r.bucket,
        page: r.page,
        version: r.version,
    }
}

fn compare_records_for_query(a: &LocalSearchRecord, b: &LocalSearchRecord, query: &str) -> Ordering {
    let ar = rank_name(&a.name, query);
    let br = rank_name(&b.name, query);
    ar.cmp(&br)
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        .then_with(|| a.rel_path.to_lowercase().cmp(&b.rel_path.to_lowercase()))
        .then_with(|| a.file_id.cmp(&b.file_id))
}

fn rank_name(name: &str, query: &str) -> u8 {
    let n = tokenize(name);
    let q = tokenize(query);
    let needle = q.normalized.trim();
    if needle.is_empty() {
        return 9;
    }
    if n.normalized.trim() == needle {
        return 0;
    }
    if n.normalized.starts_with(needle) {
        return 1;
    }
    if q.tokens
        .iter()
        .any(|qt| n.tokens.iter().any(|nt| nt.as_str() == qt.as_str()))
    {
        return 2;
    }
    if q.tokens.iter().any(|qt| n.tokens.iter().any(|nt| nt.starts_with(qt))) {
        return 3;
    }
    if n.normalized.contains(needle) {
        return 4;
    }
    5
}

fn leaf_name(path: &str) -> String {
    let trimmed = path.trim_matches(|c| c == '/' || c == '\\');
    trimmed
        .rsplit(|c| c == '/' || c == '\\')
        .next()
        .unwrap_or(trimmed)
        .to_string()
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::{Arc, Mutex};

    use beebeeb_core::kdf::MasterKey;
    use beebeeb_core::search_index::DEFAULT_NUM_SHARDS;

    use crate::api_client::ApiClient;
    use crate::state_db::{FileContractState, FileEntry, FileStatus, ItemKind, StateDb};

    fn seed_file(db: &StateDb, file_id: &str, path: &str, kind: ItemKind) {
        db.upsert_file(&FileEntry {
            file_id: file_id.to_string(),
            path: path.to_string(),
            status: FileStatus::Local,
            size_bytes: 123,
            modified_at: 1_775_000_000,
            content_hash: None,
            remote_updated_at: 1_775_000_000,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .expect("seed file");

        if kind == ItemKind::Folder {
            let mut contract: FileContractState = db
                .get_file_contract_state(file_id)
                .expect("contract lookup")
                .expect("default contract row");
            contract.item_kind = ItemKind::Folder;
            db.set_file_contract_state(&contract).expect("mark folder");
        }
    }

    #[test]
    fn local_index_builds_from_state_db_leaf_file_names_and_skips_folders() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = StateDb::open(dir.path().join("state.db")).expect("state db");
        seed_file(&db, "cafe-notes", "Archive/CaféNotes.txt", ItemKind::File);
        seed_file(&db, "photo", "Design/myVacationPhoto.png", ItemKind::File);
        seed_file(&db, "folder", "Archive/CaféFolder", ItemKind::Folder);

        let local = super::build_local_index(&db).expect("local index");

        assert_eq!(local.index.num_shards(), DEFAULT_NUM_SHARDS);
        assert_eq!(local.index.file_count(), 2);
        assert_eq!(local.records.get("cafe-notes").unwrap().name, "CaféNotes.txt");
        assert_eq!(local.records.get("photo").unwrap().name, "myVacationPhoto.png");
        assert_eq!(local.index.query("cafe"), BTreeSet::from(["cafe-notes".to_string()]));
        assert_eq!(local.index.query("photo"), BTreeSet::from(["photo".to_string()]));
        assert!(
            local.index.query("archive").is_empty(),
            "parent folder segments must not be indexed as file-name tokens"
        );
        assert!(
            local.index.query("folder").is_empty(),
            "folder rows are not file results"
        );
    }

    #[test]
    fn local_query_uses_core_tokenization_and_returns_ranked_results() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = StateDb::open(dir.path().join("state.db")).expect("state db");
        seed_file(&db, "start", "CaféNotes.txt", ItemKind::File);
        seed_file(&db, "middle", "Projects/Project_Cafe_Archive.txt", ItemKind::File);
        seed_file(&db, "camel", "Design/myVacationPhoto.png", ItemKind::File);
        seed_file(&db, "miss", "Budget.xlsx", ItemKind::File);
        let local = super::build_local_index(&db).expect("local index");

        let cafe = super::query_local_index(&local, "cafe", 10);
        assert_eq!(
            cafe.results.iter().map(|r| r.file_id.as_str()).collect::<Vec<_>>(),
            vec!["start", "middle"],
            "a file name starting with the folded query ranks before a later token match"
        );

        let camel = super::query_local_index(&local, "vacation photo", 10);
        assert_eq!(
            camel.results.iter().map(|r| r.file_id.as_str()).collect::<Vec<_>>(),
            vec!["camel"],
            "camelCase names must be searchable through core tokenization"
        );

        let empty = super::query_local_index(&local, "nonexistent", 10);
        assert!(empty.results.is_empty());
        assert_eq!(empty.index_state, super::DesktopSearchIndexState::Ready);
        assert_eq!(empty.indexed_file_count, 4);
    }

    #[test]
    fn shard_sync_round_trips_with_web_compatible_wire_contract() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = StateDb::open(dir.path().join("state.db")).expect("state db");
        seed_file(&db, "cafe-notes", "Archive/CaféNotes.txt", ItemKind::File);
        seed_file(&db, "photo", "Design/myVacationPhoto.png", ItemKind::File);

        let local = super::build_local_index(&db).expect("local index");
        let master_key = [9u8; 32];
        let mk = MasterKey::from_bytes(master_key);
        let encrypted = local.index.encrypt_shards(&mk).expect("encrypted shards");
        let used: BTreeSet<(u32, u32)> = encrypted.iter().map(|s| (s.bucket, s.page)).collect();
        let stale_bucket = (0..DEFAULT_NUM_SHARDS)
            .find(|b| !used.contains(&(*b, 0)))
            .expect("unused stale bucket");

        let api = ApiClient::new("https://api.beebeeb.io/".into(), "fixture-token".into(), master_key);
        assert_eq!(
            api.search_shard_manifest_url(),
            "https://api.beebeeb.io/api/v1/search-index/shards"
        );
        assert_eq!(
            api.search_shard_url(7, 2),
            "https://api.beebeeb.io/api/v1/search-index/shards/7/2"
        );

        let store = FixtureSearchStore::new(master_key, "fixture-token");
        store.insert_shard(stale_bucket, 0, 9, vec![1, 2, 3, 4]);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let summary = rt
            .block_on(super::sync_local_index_to_store(&db, &store))
            .expect("sync local index");

        assert_eq!(summary.indexed_file_count, 2);
        assert_eq!(summary.remote_manifest_count, 1);
        assert_eq!(summary.put_count, encrypted.len());
        assert_eq!(summary.delete_count, 1);
        assert_eq!(summary.get_count, 0);

        let restored = rt
            .block_on(super::load_remote_index_from_store(&store))
            .expect("load remote index")
            .expect("remote index present");
        assert_eq!(restored.file_count(), 2);
        assert_eq!(restored.query("cafe"), BTreeSet::from(["cafe-notes".to_string()]));
        assert_eq!(restored.query("photo"), BTreeSet::from(["photo".to_string()]));

        let requests = store.requests();
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].path, "/api/v1/search-index/shards");
        assert!(
            requests.iter().any(|r| r.method == "PUT"
                && r.path.starts_with("/api/v1/search-index/shards/")
                && r.authorization.as_deref() == Some("Bearer fixture-token")
                && r.content_type.as_deref() == Some("application/octet-stream")
                && !r.body.is_empty()),
            "shard PUT must use web's raw octet-stream contract with bearer auth"
        );
        assert!(
            requests
                .iter()
                .any(|r| r.method == "DELETE" && r.path == format!("/api/v1/search-index/shards/{stale_bucket}/0")),
            "authoritative full rebuild deletes remote-only stale shards"
        );
        assert!(
            requests.iter().any(|r| r.method == "GET"
                && r.path.starts_with("/api/v1/search-index/shards/")
                && r.path != "/api/v1/search-index/shards"),
            "remote round-trip must fetch raw shard bytes by bucket/page"
        );
    }

    #[derive(Clone, Debug)]
    struct RecordedRequest {
        method: String,
        path: String,
        authorization: Option<String>,
        content_type: Option<String>,
        body: Vec<u8>,
    }

    #[derive(Default)]
    struct FixtureState {
        shards: BTreeMap<(u32, u32), (i64, Vec<u8>)>,
        next_version: i64,
        requests: Vec<RecordedRequest>,
    }

    struct FixtureSearchStore {
        master_key: [u8; 32],
        token: String,
        state: Arc<Mutex<FixtureState>>,
    }

    impl FixtureSearchStore {
        fn new(master_key: [u8; 32], token: &str) -> Self {
            Self {
                master_key,
                token: token.to_string(),
                state: Arc::new(Mutex::new(FixtureState {
                    next_version: 10,
                    ..FixtureState::default()
                })),
            }
        }

        fn insert_shard(&self, bucket: u32, page: u32, version: i64, blob: Vec<u8>) {
            self.state
                .lock()
                .expect("fixture mutex")
                .shards
                .insert((bucket, page), (version, blob));
        }

        fn requests(&self) -> Vec<RecordedRequest> {
            self.state.lock().expect("fixture mutex").requests.clone()
        }

        fn record(
            &self,
            method: &str,
            path: String,
            authorization: Option<String>,
            content_type: Option<String>,
            body: Vec<u8>,
        ) {
            self.state
                .lock()
                .expect("fixture mutex")
                .requests
                .push(RecordedRequest {
                    method: method.to_string(),
                    path,
                    authorization,
                    content_type,
                    body,
                });
        }

        fn auth(&self) -> Option<String> {
            Some(format!("Bearer {}", self.token))
        }
    }

    impl super::SearchShardStore for FixtureSearchStore {
        fn master_key_bytes(&self) -> [u8; 32] {
            self.master_key
        }

        fn fetch_manifest<'a>(&'a self) -> super::StoreFuture<'a, Vec<crate::api_client::SearchShardRef>> {
            Box::pin(async move {
                self.record(
                    "GET",
                    "/api/v1/search-index/shards".to_string(),
                    self.auth(),
                    None,
                    Vec::new(),
                );
                let shards = self
                    .state
                    .lock()
                    .expect("fixture mutex")
                    .shards
                    .iter()
                    .map(|(&(bucket, page), (version, _))| crate::api_client::SearchShardRef {
                        bucket,
                        page,
                        version: *version,
                    })
                    .collect();
                Ok(shards)
            })
        }

        fn get_shard<'a>(&'a self, bucket: u32, page: u32) -> super::StoreFuture<'a, Option<Vec<u8>>> {
            Box::pin(async move {
                self.record(
                    "GET",
                    format!("/api/v1/search-index/shards/{bucket}/{page}"),
                    self.auth(),
                    None,
                    Vec::new(),
                );
                Ok(self
                    .state
                    .lock()
                    .expect("fixture mutex")
                    .shards
                    .get(&(bucket, page))
                    .map(|(_, blob)| blob.clone()))
            })
        }

        fn put_shard<'a>(
            &'a self,
            shard: beebeeb_core::search_index::EncryptedShard,
        ) -> super::StoreFuture<'a, crate::api_client::SearchShardRef> {
            Box::pin(async move {
                self.record(
                    "PUT",
                    format!("/api/v1/search-index/shards/{}/{}", shard.bucket, shard.page),
                    self.auth(),
                    Some("application/octet-stream".to_string()),
                    shard.blob.clone(),
                );
                let version = {
                    let mut guard = self.state.lock().expect("fixture mutex");
                    let version = guard.next_version;
                    guard.next_version += 1;
                    guard.shards.insert((shard.bucket, shard.page), (version, shard.blob));
                    version
                };
                Ok(crate::api_client::SearchShardRef {
                    bucket: shard.bucket,
                    page: shard.page,
                    version,
                })
            })
        }

        fn delete_shard<'a>(&'a self, bucket: u32, page: u32) -> super::StoreFuture<'a, ()> {
            Box::pin(async move {
                self.record(
                    "DELETE",
                    format!("/api/v1/search-index/shards/{bucket}/{page}"),
                    self.auth(),
                    None,
                    Vec::new(),
                );
                self.state.lock().expect("fixture mutex").shards.remove(&(bucket, page));
                Ok(())
            })
        }
    }

    #[test]
    fn api_client_search_shard_urls_match_web_paths() {
        let api = ApiClient::new("https://api.beebeeb.io/".into(), "tok".into(), [0u8; 32]);
        assert_eq!(
            api.search_shard_manifest_url(),
            "https://api.beebeeb.io/api/v1/search-index/shards"
        );
        assert_eq!(
            api.search_shard_url(12, 3),
            "https://api.beebeeb.io/api/v1/search-index/shards/12/3"
        );
    }

    fn shard_path_coord(path: &str) -> Option<(u32, u32)> {
        let suffix = path.strip_prefix("/api/v1/search-index/shards/")?;
        let (bucket, page) = suffix.split_once('/')?;
        Some((bucket.parse().ok()?, page.parse().ok()?))
    }

    #[test]
    fn fixture_contract_paths_parse_like_server_routes() {
        assert_eq!(shard_path_coord("/api/v1/search-index/shards/63/0"), Some((63, 0)));
        assert_eq!(shard_path_coord("/api/v1/search-index/shards"), None);
    }
}
