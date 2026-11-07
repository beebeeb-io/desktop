//! LD-0113 Task D4: `DesktopBackupHost` against an in-memory server tree (paged listings, folder creates, moves),
//! and core's pass driving it.

use super::*;
use crate::backup::host::DesktopBackupHost;
use crate::state_db::BackupLayout;
use beebeeb_backup::BackupHost;
use beebeeb_core::backup::{BackupRecord, FolderPresence, RecordRef, RecordState};
use beebeeb_types::backup::Category;

    #[derive(Clone, Debug)]
    struct Node {
        parent: Option<String>,
        name_encrypted: String,
        is_folder: bool,
        trashed: bool,
        size: u64,
    }

    /// The server's tree as the backup sees it. Listings are paged two entries at a time.
    struct DriveMock {
        base_url: String,
        nodes: Arc<Mutex<std::collections::BTreeMap<String, Node>>>,
        requests: Arc<Mutex<Vec<RecordedRequest>>>,
        stop: Arc<AtomicBool>,
        handle: thread::JoinHandle<()>,
    }

    const PAGE: usize = 2;

    fn node_json(id: &str, node: &Node) -> serde_json::Value {
        serde_json::json!({
            "id": id, "name_encrypted": node.name_encrypted, "is_folder": node.is_folder,
            "is_trashed": node.trashed, "size_bytes": node.size, "parent_id": node.parent,
            "uploaded_at": "2026-11-06T10:00:00Z", "updated_at": "2026-11-06T10:00:00Z",
        })
    }

    fn query_param<'a>(path: &'a str, name: &str) -> Option<&'a str> {
        path.split_once('?')?.1.split('&').find_map(|pair| pair.strip_prefix(&format!("{name}=")))
    }

    impl DriveMock {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let nodes: Arc<Mutex<std::collections::BTreeMap<String, Node>>> = Arc::default();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let (tree, log, halt) = (Arc::clone(&nodes), Arc::clone(&requests), Arc::clone(&stop));
            let handle = thread::spawn(move || {
                let started = std::time::Instant::now();
                while !halt.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(30) {
                    let mut stream = match listener.accept() {
                        Ok((stream, _)) => stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                            continue;
                        }
                        Err(e) => panic!("drive mock accept failed: {e}"),
                    };
                    stream.set_nonblocking(false).unwrap();
                    let request = read_http_request(&mut stream);
                    log.lock().unwrap().push(request.clone());
                    let mut tree = tree.lock().unwrap();
                    let (method, path) = (request.method.as_str(), request.path.as_str());
                    let response = if method == "GET" && path.starts_with("/api/v1/files?") {
                        let parent = query_param(path, "parent_id").unwrap_or_default().to_string();
                        let offset: usize =
                            query_param(path, "cursor").and_then(|c| c.strip_prefix('c')).map_or(0, |n| n.parse().unwrap());
                        let children: Vec<serde_json::Value> = tree
                            .iter()
                            // Trashed children are listed too (flagged): the host must not take them as live.
                            .filter(|(_, node)| node.parent.as_deref() == Some(parent.as_str()))
                            .map(|(id, node)| node_json(id, node))
                            .collect();
                        let page: Vec<_> = children.iter().skip(offset).take(PAGE).cloned().collect();
                        let next = (offset + PAGE < children.len()).then(|| format!("c{}", offset + PAGE));
                        http_json("200 OK", serde_json::json!({ "files": page, "next_cursor": next }))
                    } else if method == "GET" && path.starts_with("/api/v1/files/") {
                        let id = path.trim_start_matches("/api/v1/files/");
                        match tree.get(id) {
                            Some(node) => http_json("200 OK", node_json(id, node)),
                            None => http_json("404 Not Found", serde_json::json!({ "error": "not found" })),
                        }
                    } else if method == "POST" && path == "/api/v1/files/folder" {
                        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
                        let id = body["folder_id"].as_str().unwrap().to_string();
                        let node = Node {
                            parent: body["parent_id"].as_str().map(str::to_string),
                            name_encrypted: body["name_encrypted"].as_str().unwrap().to_string(),
                            is_folder: true,
                            trashed: false,
                            size: 0,
                        };
                        let answer = node_json(&id, &node);
                        tree.insert(id, node);
                        http_json("201 Created", answer)
                    } else if method == "PATCH" && path.starts_with("/api/v1/files/") {
                        let id = path.trim_start_matches("/api/v1/files/").to_string();
                        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
                        match tree.get_mut(&id) {
                            Some(node) => {
                                if let Some(name) = body["name_encrypted"].as_str() {
                                    node.name_encrypted = name.to_string();
                                }
                                if let Some(parent) = body["parent_id"].as_str() {
                                    node.parent = Some(parent.to_string());
                                }
                                http_json("200 OK", node_json(&id, node))
                            }
                            None => http_json("404 Not Found", serde_json::json!({ "error": "not found" })),
                        }
                    } else {
                        http_json("404 Not Found", serde_json::json!({ "error": format!("unexpected {method} {path}") }))
                    };
                    drop(tree);
                    let _ = stream.write_all(response.as_bytes());
                }
            });
            Self { base_url, nodes, requests, stop, handle }
        }

        fn finish(self) -> Vec<RecordedRequest> {
            self.stop.store(true, Ordering::SeqCst);
            self.handle.join().unwrap();
            self.requests.lock().unwrap().clone()
        }
    }

    struct Fixture {
        bridge: Arc<EngineBridge>,
        server: DriveMock,
        root: String,
        device: String,
        _dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let server = DriveMock::start();
            let bridge = Arc::new(test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [42u8; 32]));
            bridge.db.set_owner(&crate::account_binding::Identity::new(Some("u-a"), Some("a@beebeeb.io"))).unwrap();
            let mut fixture = Fixture { bridge, server, root: String::new(), device: String::new(), _dir: dir };
            fixture.root = fixture.add(None, "Backups", true, 0);
            fixture.device = fixture.add(Some(&fixture.root.clone()), "GLegion", true, 0);
            fixture
        }

        /// A server item with a self-minted id and its name sealed for it, as this device would have made it.
        fn add(&self, parent: Option<&str>, name: &str, is_folder: bool, size: u64) -> String {
            let master = beebeeb_core::kdf::MasterKey::from_bytes(*self.bridge.api.master_key());
            let id = beebeeb_core::item_id::NewItemId::owner(&master).unwrap().as_str().to_string();
            let name_encrypted = owned_metadata_for_name(&self.bridge.db, self.bridge.api.master_key(), &id, name, None).unwrap();
            let node = Node { parent: parent.map(str::to_string), name_encrypted, is_folder, trashed: false, size };
            self.server.nodes.lock().unwrap().insert(id.clone(), node);
            id
        }

        fn trash(&self, id: &str) {
            self.server.nodes.lock().unwrap().get_mut(id).unwrap().trashed = true;
        }

        fn host(&self, enabled: Vec<Category>) -> DesktopBackupHost {
            let layout = BackupLayout {
                root_id: self.root.clone(),
                device_folder_id: self.device.clone(),
                device_folder_name: "GLegion".into(),
            };
            self.bridge.db.set_backup_layout(&layout).unwrap();
            DesktopBackupHost::new(Arc::clone(&self.bridge), layout, enabled)
        }

        fn creates(requests: &[RecordedRequest]) -> usize {
            requests.iter().filter(|r| r.method == "POST" && r.path == "/api/v1/files/folder").count()
        }
    }

    fn budget() -> beebeeb_core::backup::PassBudget {
        beebeeb_core::backup::PassBudget { max_uploads: 100, max_bytes: u64::MAX }
    }

    fn uuid(id: &str) -> uuid::Uuid {
        uuid::Uuid::parse_str(id).unwrap()
    }

    /// `device_tree` lists every page of every folder under the category folder: keys joined with `/`, `folders[""]`
    /// is the category folder, a trashed file is not live, and `complete` holds. Every request is the backup's.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn host_lists_the_category_tree_across_pages() {
        let fx = Fixture::new();
        let pictures = fx.add(Some(&fx.device.clone()), "Pictures", true, 0);
        fx.bridge.db.set_backup_category_folder("pictures", Some(&pictures)).unwrap();
        let trip = fx.add(Some(&pictures), "Trip", true, 0);
        let day = fx.add(Some(&trip), "Day 1", true, 0);
        for (parent, name) in [(&pictures, "a.jpg"), (&pictures, "b.jpg"), (&pictures, "c.jpg"), (&trip, "d.jpg"), (&day, "e.jpg")] {
            fx.add(Some(parent), name, false, 7);
        }
        let gone = fx.add(Some(&pictures), "trashed.jpg", false, 7);
        fx.trash(&gone);

        let tree = fx.host(vec![Category::Pictures]).device_tree(&Category::Pictures).await.unwrap();
        let requests = fx.server.finish();
        assert!(tree.device_folder_live && tree.complete);
        assert_eq!(tree.category_folder_live, Some(true));
        let mut folders: Vec<(&str, uuid::Uuid)> = tree.folders.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        folders.sort();
        assert_eq!(folders, vec![("", uuid(&pictures)), ("Trip", uuid(&trip)), ("Trip/Day 1", uuid(&day))]);
        let mut keys: Vec<&str> = tree.files.iter().map(|f| f.key.as_str()).collect();
        keys.sort();
        assert_eq!(keys, ["Trip/Day 1/e.jpg", "Trip/d.jpg", "a.jpg", "b.jpg", "c.jpg"]);
        assert!(tree.files.iter().all(|f| f.size == 7 && f.uploaded_at_ms == 1_793_959_200_000));
        assert_eq!(requests.iter().filter(|r| r.path.contains("cursor=")).count(), 2, "Pictures has 3 pages of 2");
        assert!(requests.iter().all(|r| r.origin.as_deref() == Some(beebeeb_types::backup::ORIGIN_BACKUP)));
        assert_eq!(Fixture::creates(&requests), 0);
    }

    /// A first backup adopts the device folder's existing `Pictures` (the old mirror's) instead of creating a second
    /// one, remembers it, and creates a category folder only where there is none.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn host_adopts_the_existing_category_folder_and_remembers_it() {
        let fx = Fixture::new();
        let legacy = fx.add(Some(&fx.device.clone()), "Pictures", true, 0);
        fx.add(Some(&legacy), "old.jpg", false, 3);
        let host = fx.host(vec![Category::Pictures, Category::Documents]);

        let tree = host.device_tree(&Category::Pictures).await.unwrap();
        assert_eq!(tree.category_folder_live, None, "a first backup on this device");
        assert_eq!(tree.folders[""], uuid(&legacy));
        assert_eq!(tree.files.len(), 1);
        assert_eq!(fx.bridge.db.backup_category_folder("pictures").unwrap().as_deref(), Some(legacy.as_str()));

        let documents = host.device_tree(&Category::Documents).await.unwrap();
        let requests = fx.server.finish();
        assert_eq!(Fixture::creates(&requests), 1, "only Documents had no folder");
        assert!(documents.files.is_empty() && documents.complete);
        let made = fx.bridge.db.backup_category_folder("documents").unwrap().unwrap();
        assert_eq!(documents.folders[""], uuid(&made));
    }

    /// Rule 11: a category folder deleted elsewhere turns that backup off (settings, stop notice, folder forgotten,
    /// queued uploads dropped) and nothing is queued; a deleted device folder turns every backup off and forgets the
    /// layout. Nothing is uploaded again by itself (Amendment 12).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn host_turns_off_when_the_folder_was_deleted_elsewhere() {
        let config_dir = tempfile::tempdir().unwrap();
        crate::config::CONFIG_DIR_OVERRIDE
            .scope(config_dir.path().to_path_buf(), async {
                crate::config::DesktopConfig::update(|cfg| {
                    cfg.set_known_folder("pictures", true);
                    cfg.set_known_folder("documents", true);
                    cfg.set_known_folder("music", true);
                    (true, ())
                })
                .unwrap();
                let fx = Fixture::new();
                let source = tempfile::tempdir().unwrap();
                std::fs::write(source.path().join("new.jpg"), b"never queued").unwrap();
                let pictures = fx.add(Some(&fx.device.clone()), "Pictures", true, 0);
                let documents = fx.add(Some(&fx.device.clone()), "Documents", true, 0);
                fx.bridge.db.set_backup_category_folder("pictures", Some(&pictures)).unwrap();
                let music = fx.add(Some(&fx.device.clone()), "Music", true, 0);
                fx.bridge.db.set_backup_category_folder("documents", Some(&documents)).unwrap();
                fx.bridge.db.set_backup_category_folder("music", Some(&music)).unwrap();
                fx.trash(&pictures);
                let host = fx.host(vec![Category::Pictures, Category::Documents, Category::Music]);

                let report = beebeeb_backup::run_pass(&host, source.path(), &[], &Category::Pictures, budget())
                    .await
                    .unwrap();
                assert_eq!(report.turned_off, Some(FolderPresence::CategoryDeleted));
                assert_eq!(count_backup_ops(&fx.bridge), 0, "nothing is queued");
                let stops = fx.bridge.db.backup_stops().unwrap();
                assert_eq!(stops.iter().map(|s| (s.category.as_str(), s.reason.as_str())).collect::<Vec<_>>(), [("pictures", "category_deleted")]);
                assert_eq!(fx.bridge.db.backup_category_folder("pictures").unwrap(), None);
                let cfg = crate::config::DesktopConfig::load().unwrap();
                assert_eq!(cfg.known_folder_backup, ["documents", "music"], "only Pictures is switched off");

                fx.trash(&fx.device);
                let report = beebeeb_backup::run_pass(&host, source.path(), &[], &Category::Documents, budget())
                    .await
                    .unwrap();
                assert_eq!(report.turned_off, Some(FolderPresence::DeviceDeleted));
                assert!(
                    crate::config::DesktopConfig::load().unwrap().known_folder_backup.is_empty(),
                    "a deleted device folder turns Music off too, though its pass never ran"
                );
                assert_eq!(fx.bridge.db.backup_layout().unwrap(), None, "the layout is resolved anew");
                let stops: Vec<(String, String)> =
                    fx.bridge.db.backup_stops().unwrap().into_iter().map(|s| (s.category, s.reason)).collect();
                assert_eq!(
                    stops,
                    [("documents", "device_deleted"), ("music", "device_deleted"), ("pictures", "category_deleted")]
                        .map(|(c, r)| (c.to_string(), r.to_string()))
                );
                assert_eq!(fx.bridge.db.backup_category_folder("music").unwrap(), None);
                fx.server.finish();
            })
            .await;
    }

    fn count_backup_ops(bridge: &EngineBridge) -> i64 {
        bridge
            .db
            .hold_lock_for_test()
            .query_row("SELECT COUNT(*) FROM operation_queue WHERE origin = 'backup'", [], |row| row.get(0))
            .unwrap()
    }

    /// Core's pass over a real folder through the host: every file is queued as a backup op (source in place), the
    /// planned subfolders are created under the category folder, and the pass's moves re-key their records.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_pass_through_the_host_queues_every_file_and_creates_its_folders() {
        let fx = Fixture::new();
        let source = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(source.path().join("Trip")).unwrap();
        for name in ["a.jpg", "Trip/b.jpg", "Trip/c.jpg"] {
            std::fs::write(source.path().join(name), name.as_bytes()).unwrap();
        }
        let host = fx.host(vec![Category::Pictures]);
        let report = beebeeb_backup::run_pass(&host, source.path(), &[], &Category::Pictures, budget())
            .await
            .unwrap();
        assert_eq!(report.turned_off, None);
        assert_eq!(count_backup_ops(&fx.bridge), 3, "{report:?}");
        assert_eq!(fx.bridge.db.backup_held_keys("pictures").unwrap(), ["Trip/b.jpg", "Trip/c.jpg", "a.jpg"]);
        let requests = fx.server.finish();
        assert_eq!(Fixture::creates(&requests), 2, "the category folder and Trip");
    }

    /// `put_record`'s compare-and-set (core review 4 #3): a record still at its key keeps its stored fields and only
    /// moves; a remote file no record names is stored; a remote file another key names now is left alone.
    #[test]
    fn put_record_is_compare_and_set() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(&dir.path().join("state.db")).unwrap();
        db.set_owner(&crate::account_binding::Identity::new(Some("u-a"), None)).unwrap();
        let remote = uuid::Uuid::from_u128(7);
        let record = |key: &str, size: u64, remote_id: uuid::Uuid| BackupRecord {
            key: key.into(), size, modified_ms: size as i64, remote_id, uploaded_at_ms: size as i64,
            state: RecordState::BackedUp,
        };
        let put = |key: &str, size: u64| {
            db.hold_lock_for_test()
                .execute(
                    "INSERT OR REPLACE INTO backup_records (account_owner, category, key, size, modified_ms, remote_id, uploaded_at_ms)
                     VALUES ('user:u-a', 'pictures', ?1, ?2, ?2, ?3, ?2)",
                    rusqlite::params![key, size as i64, remote.to_string()],
                )
                .unwrap()
        };
        put("old.jpg", 50);
        let expected = RecordRef { key: "old.jpg".into(), remote_id: remote };
        assert!(db.put_backup_record_cas("pictures", &expected, &record("new.jpg", 10, remote)).unwrap());
        assert_eq!(db.backup_records("pictures").unwrap(), vec![record("new.jpg", 50, remote)], "moved, stored fields kept");

        // Another key names the remote file now: nothing changes.
        assert!(!db.put_backup_record_cas("pictures", &expected, &record("third.jpg", 10, remote)).unwrap());
        assert_eq!(db.backup_records("pictures").unwrap(), vec![record("new.jpg", 50, remote)]);

        // No record names it: stored as given.
        let other = uuid::Uuid::from_u128(8);
        let missing = RecordRef { key: "gone.jpg".into(), remote_id: other };
        assert!(db.put_backup_record_cas("pictures", &missing, &record("found.jpg", 5, other)).unwrap());
        assert_eq!(db.backup_records("pictures").unwrap().len(), 2);
    }

    fn tree_bytes(root: &Path) -> (u64, usize) {
        let (mut bytes, mut entries) = (0, 0);
        for entry in walkdir_all(root) {
            entries += 1;
            bytes += std::fs::symlink_metadata(&entry).map_or(0, |m| if m.is_file() { m.len() } else { 0 });
        }
        (bytes, entries)
    }

    fn walkdir_all(root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    stack.push(path.clone());
                }
                out.push(path);
            }
        }
        out
    }

    /// Plan Task D4: a scheduler tick over a 50-file folder queues 50 uploads and adds 0 bytes (and 0 entries) under
    /// the sync root, which the old mirror filled with copies. The sync root sits inside the backed-up folder here,
    /// and nothing of it is queued.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn zero_bytes_added_under_sync_root() {
        let fx = Fixture::new();
        fx.host(vec![Category::Documents]);
        let source = tempfile::tempdir().unwrap();
        let sync_root = source.path().join("Beebeeb");
        std::fs::create_dir_all(sync_root.join("Backups").join("GLegion")).unwrap();
        std::fs::write(sync_root.join("in-the-drive.txt"), b"already synced").unwrap();
        for i in 0..50 {
            std::fs::write(source.path().join(format!("doc-{i:02}.txt")), vec![b'x'; 1000 + i]).unwrap();
        }
        let before = tree_bytes(&sync_root);

        let mut scheduler = crate::backup::scheduler::BackupScheduler::new();
        let sources = vec![crate::backup::scheduler::BackupSource {
            category: Category::Documents,
            path: source.path().to_path_buf(),
        }];
        let report = scheduler.tick(&fx.bridge, &sync_root, sources, std::time::Instant::now()).await.expect("due");
        fx.server.finish();
        assert_eq!(report.passes.len(), 1);
        let pass = report.passes[0].1.as_ref().expect("the pass ran");
        assert_eq!(pass.queued, 50, "{pass:?}");
        assert_eq!(count_backup_ops(&fx.bridge), 50, "nothing of the sync root is queued");
        assert_eq!(tree_bytes(&sync_root), before, "bytes and entries under the sync root");
    }

    /// Plan Task D4: a file added to a backed-up folder is queued on the first tick once the folder has been quiet
    /// for one debounce, not at the next 10-minute pass; a tick inside the debounce does not pass.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn new_source_file_queued_within_one_debounce() {
        let fx = Fixture::new();
        fx.host(vec![Category::Pictures]);
        let source = tempfile::tempdir().unwrap();
        let sync_root = tempfile::tempdir().unwrap();
        let debounce = Duration::from_millis(400);
        let mut scheduler = crate::backup::scheduler::BackupScheduler::with_debounce(debounce);
        let sources = || {
            vec![crate::backup::scheduler::BackupSource { category: Category::Pictures, path: source.path().to_path_buf() }]
        };
        assert!(scheduler.tick(&fx.bridge, sync_root.path(), sources(), std::time::Instant::now()).await.is_some());
        assert_eq!(count_backup_ops(&fx.bridge), 0, "the folder was empty");

        std::fs::write(source.path().join("new.jpg"), b"just taken").unwrap();
        let written = std::time::Instant::now();
        let mut early = false;
        // The watcher reports the write; a tick before the folder is quiet for a debounce does not pass.
        tokio::time::sleep(Duration::from_millis(100)).await;
        if scheduler.tick(&fx.bridge, sync_root.path(), sources(), std::time::Instant::now()).await.is_some() {
            early = true;
        }
        let mut queued_at = None;
        while written.elapsed() < Duration::from_secs(5) {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if scheduler.tick(&fx.bridge, sync_root.path(), sources(), std::time::Instant::now()).await.is_some() {
                queued_at = Some(written.elapsed());
                break;
            }
        }
        fx.server.finish();
        assert!(!early, "a tick inside the debounce ran a pass");
        let queued_at = queued_at.expect("no pass within 5 s of the change (the 10-minute pass is far off)");
        assert!(queued_at >= debounce, "{queued_at:?}");
        assert!(queued_at < debounce * 4, "queued {queued_at:?} after the change");
        assert_eq!(count_backup_ops(&fx.bridge), 1);
    }
