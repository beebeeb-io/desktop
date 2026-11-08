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
        /// The server's `uploaded_at` (RFC 3339); `None` lists the fixture's fixed time.
        uploaded_at: Option<String>,
    }

    /// The server's tree as the backup sees it. Listings are paged two entries at a time.
    struct DriveMock {
        base_url: String,
        nodes: Arc<Mutex<std::collections::BTreeMap<String, Node>>>,
        requests: Arc<Mutex<Vec<RecordedRequest>>>,
        stop: Arc<AtomicBool>,
        handle: thread::JoinHandle<()>,
        /// LD-0123: when set, every listing answers with a `Date` header this many seconds ahead of the device's
        /// clock (a device clock that runs behind the server's).
        date_ahead_s: Arc<Mutex<Option<i64>>>,
        /// LD-0139: the rows `GET /api/v1/notifications` answers with (always with a `Date`); `None` answers 500.
        notices: Arc<Mutex<Option<Vec<serde_json::Value>>>>,
    }

    const PAGE: usize = 2;

    fn node_json(id: &str, node: &Node) -> serde_json::Value {
        let uploaded_at = node.uploaded_at.as_deref().unwrap_or("2026-11-06T10:00:00Z");
        serde_json::json!({
            "id": id, "name_encrypted": node.name_encrypted, "is_folder": node.is_folder,
            "is_trashed": node.trashed, "size_bytes": node.size, "parent_id": node.parent,
            "uploaded_at": uploaded_at, "updated_at": uploaded_at,
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
            let date_ahead_s: Arc<Mutex<Option<i64>>> = Arc::default();
            let notices: Arc<Mutex<Option<Vec<serde_json::Value>>>> = Arc::new(Mutex::new(Some(Vec::new())));
            let (tree, log, halt) = (Arc::clone(&nodes), Arc::clone(&requests), Arc::clone(&stop));
            let (ahead, rows) = (Arc::clone(&date_ahead_s), Arc::clone(&notices));
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
                    let response = if method == "GET" && path.starts_with("/api/v1/notifications") {
                        let ahead_s = ahead.lock().unwrap().unwrap_or(0);
                        match rows.lock().unwrap().clone() {
                            Some(rows) => {
                                http_json_dated("200 OK", serde_json::json!({ "notifications": rows }), ahead_s)
                            }
                            None => http_json("500 Internal Server Error", serde_json::json!({ "error": "down" })),
                        }
                    } else if method == "GET" && path.starts_with("/api/v1/files?") {
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
                        let listing = serde_json::json!({ "files": page, "next_cursor": next });
                        match *ahead.lock().unwrap() {
                            Some(seconds) => http_json_dated("200 OK", listing, seconds),
                            None => http_json("200 OK", listing),
                        }
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
                            uploaded_at: None,
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
            Self { base_url, nodes, requests, stop, handle, date_ahead_s, notices }
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
            self.add_stamped(parent, name, is_folder, size, None)
        }

        /// [`Self::add`] with the server's `uploaded_at` for the item.
        fn add_stamped(
            &self,
            parent: Option<&str>,
            name: &str,
            is_folder: bool,
            size: u64,
            uploaded_at: Option<String>,
        ) -> String {
            let master = beebeeb_core::kdf::MasterKey::from_bytes(*self.bridge.api.master_key());
            let id = beebeeb_core::item_id::NewItemId::owner(&master).unwrap().as_str().to_string();
            let name_encrypted = owned_metadata_for_name(&self.bridge.db, self.bridge.api.master_key(), &id, name, None).unwrap();
            let parent = parent.map(str::to_string);
            let node = Node { parent, name_encrypted, is_folder, trashed: false, size, uploaded_at };
            self.server.nodes.lock().unwrap().insert(id.clone(), node);
            id
        }

        fn trash(&self, id: &str) {
            self.server.nodes.lock().unwrap().get_mut(id).unwrap().trashed = true;
        }

        fn layout(&self) -> BackupLayout {
            let layout = BackupLayout {
                root_id: self.root.clone(),
                device_folder_id: self.device.clone(),
                device_folder_name: "GLegion".into(),
            };
            self.bridge.db.set_backup_layout(&layout).unwrap();
            layout
        }

        fn host(&self, enabled: Vec<Category>) -> DesktopBackupHost {
            DesktopBackupHost::new(Arc::clone(&self.bridge), self.layout(), enabled)
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

    /// Rule 11 inside core's pass (a folder deleted after the pass's notice check): a category folder deleted elsewhere
    /// turns that backup off as core's `unknown` (settings, stop notice, folder forgotten, queued uploads dropped) and
    /// nothing is queued; a deleted device folder turns every backup off and forgets the layout. Nothing is uploaded
    /// again by itself (Amendment 12).
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
                let reasons: Vec<(&str, &str)> = stops.iter().map(|s| (s.category.as_str(), s.reason.as_str())).collect();
                assert_eq!(reasons, [("pictures", "unknown")]);
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
                    [("documents", "unknown"), ("music", "unknown"), ("pictures", "unknown")]
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

    /// LD-0123 (LD-0139): the records' rebuild uses the clock its listing measured from the server's `Date` header.
    /// Here the device clock runs 3 minutes behind the server's. A copy the server stamped 30 s after the file last
    /// changed counts as backed up (core's 10-minute floor alone would upload it again); one stamped 0.5 s after it,
    /// inside the measurement's margin, is uploaded again; so is one stamped 2 minutes later by the device's clock,
    /// which is a minute BEFORE the change on the server's.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_rebuild_with_a_measured_clock_keeps_a_timely_copy_and_uploads_one_inside_the_margin_again() {
        let fx = Fixture::new();
        *fx.server.date_ahead_s.lock().unwrap() = Some(180);
        // Whole seconds an hour ago, so every file system keeps the mtime exactly.
        let now_s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let changed = std::time::UNIX_EPOCH + Duration::from_secs(now_s - 3600);
        let changed_ms = i64::try_from((now_s - 3600) * 1000).unwrap();
        let source = tempfile::tempdir().unwrap();
        for name in ["early.jpg", "margin.jpg", "timely.jpg"] {
            let path = source.path().join(name);
            std::fs::write(&path, b"7 bytes").unwrap();
            std::fs::File::options().write(true).open(&path).unwrap().set_modified(changed).unwrap();
        }
        let stamp = |after_ms: i64| {
            let at = chrono::DateTime::from_timestamp_millis(changed_ms + after_ms).unwrap();
            Some(at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        };
        let pictures = fx.add(Some(&fx.device.clone()), "Pictures", true, 0);
        fx.bridge.db.set_backup_category_folder("pictures", Some(&pictures)).unwrap();
        let timely = fx.add_stamped(Some(&pictures), "timely.jpg", false, 7, stamp(180_000 + 30_000));
        fx.add_stamped(Some(&pictures), "margin.jpg", false, 7, stamp(180_000 + 500));
        fx.add_stamped(Some(&pictures), "early.jpg", false, 7, stamp(120_000));
        let host = fx.host(vec![Category::Pictures]);

        let (tree, clock) = host.device_tree_for_rebuild(&Category::Pictures).await.unwrap();
        let (pictures, src) = (Category::Pictures, source.path());
        let stored = crate::backup::scheduler::rebuild_records_if_empty(&host, src, &[], &pictures, &tree, clock)
            .await
            .unwrap();
        let records: Vec<(String, uuid::Uuid)> =
            fx.bridge.db.backup_records("pictures").unwrap().into_iter().map(|r| (r.key, r.remote_id)).collect();
        assert_eq!(records, [("timely.jpg".to_string(), uuid(&timely))], "only the copy made after the change counts");
        assert_eq!(stored, 1);

        let report = beebeeb_backup::run_pass(&host, source.path(), &[], &Category::Pictures, budget()).await.unwrap();
        assert_eq!(fx.bridge.db.backup_held_keys("pictures").unwrap(), ["early.jpg", "margin.jpg"], "{report:?}");
        assert_eq!(count_backup_ops(&fx.bridge), 2, "{report:?}");
        let requests = fx.server.finish();
        // One listing of Pictures (its first page; 3 files at 2 a page is one more with a cursor).
        let first_pages = requests
            .iter()
            .filter(|r| r.method == "GET" && r.path.starts_with("/api/v1/files?") && !r.path.contains("cursor="))
            .count();
        assert_eq!(first_pages, 1, "the pass takes the rebuild's listing instead of listing again");
    }

    /// Of two measured clocks the smaller margin wins, and a measured clock beats an unknown one either way round.
    #[test]
    fn the_tighter_clock_is_kept() {
        use beebeeb_core::backup::RebuildClock;
        let wide = RebuildClock { offset_ms: Some(10), margin_ms: 2_900 };
        let tight = RebuildClock { offset_ms: Some(12), margin_ms: 2_100 };
        assert_eq!(crate::engine_bridge::tighter_clock(wide, tight), tight);
        assert_eq!(crate::engine_bridge::tighter_clock(tight, wide), tight);
        assert_eq!(crate::engine_bridge::tighter_clock(RebuildClock::UNKNOWN, wide), wide);
        assert_eq!(crate::engine_bridge::tighter_clock(wide, RebuildClock::UNKNOWN), wide);
        let unknown = RebuildClock::UNKNOWN;
        assert_eq!(crate::engine_bridge::tighter_clock(unknown, unknown), unknown);
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

    // ── LD-0139: the notice check and core's `backup_stop` (spec Amendments 12 and 12c) ─────────────────────────────

    use crate::backup::scheduler;

    fn now_ms() -> i64 {
        chrono::Utc::now().timestamp_millis()
    }

    /// A `notifications` row as the server lists it.
    fn notice_row(kind: &str, data: serde_json::Value, created_at_ms: i64) -> serde_json::Value {
        let created_at = chrono::DateTime::from_timestamp_millis(created_at_ms).unwrap();
        serde_json::json!({
            "id": uuid::Uuid::new_v4(), "type": kind, "title": "Backups", "body": null, "data": data, "read": false,
            "created_at": created_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        })
    }

    /// A source folder with `files` files of 5 bytes.
    fn source_with(files: usize) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..files {
            std::fs::write(dir.path().join(format!("f{i}.txt")), b"bytes").unwrap();
        }
        dir
    }

    fn switch_on(keys: &[&str]) {
        crate::config::DesktopConfig::update(|cfg| {
            for key in keys {
                cfg.set_known_folder(key, true);
            }
            (true, ())
        })
        .unwrap();
    }

    fn stops(bridge: &EngineBridge) -> Vec<(String, String)> {
        bridge.db.backup_stops().unwrap().into_iter().map(|s| (s.category, s.reason)).collect()
    }

    async fn run(fx: &Fixture, sources: &[(Category, &Path)], sync_root: &Path) -> scheduler::TickReport {
        let sources: Vec<scheduler::BackupSource> = sources
            .iter()
            .map(|(category, path)| scheduler::BackupSource { category: category.clone(), path: path.to_path_buf() })
            .collect();
        scheduler::run_passes(&fx.bridge, fx.layout(), sync_root, &sources).await
    }

    /// Amendment 12 step 1: a `files_removed` notice newer than the last pass turns EVERY backup off with its reason,
    /// the folders still there; nothing is queued, and the card says core's words with "Back up again".
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_files_removed_notice_turns_every_backup_off_with_cores_words() {
        let config_dir = tempfile::tempdir().unwrap();
        crate::config::CONFIG_DIR_OVERRIDE
            .scope(config_dir.path().to_path_buf(), async {
                switch_on(&["pictures", "documents"]);
                let fx = Fixture::new();
                let (pictures, documents, sync_root) = (source_with(2), source_with(2), tempfile::tempdir().unwrap());
                let last = now_ms() - 120_000;
                fx.bridge.db.set_backup_last_pass("pictures", last).unwrap();
                fx.bridge.db.set_backup_last_pass("documents", last).unwrap();
                let erase_data = serde_json::json!({ "reason": "erase_request" });
                let erase = notice_row("files_removed", erase_data, last + 60_000);
                *fx.server.notices.lock().unwrap() = Some(vec![erase]);

                let report = run(
                    &fx,
                    &[(Category::Pictures, pictures.path()), (Category::Documents, documents.path())],
                    sync_root.path(),
                )
                .await;
                assert_eq!(count_backup_ops(&fx.bridge), 0, "nothing is queued: {:?}", report.passes);
                assert_eq!(
                    stops(&fx.bridge),
                    [("documents", "erase_request"), ("pictures", "erase_request")]
                        .map(|(c, r)| (c.to_string(), r.to_string()))
                );
                assert!(crate::config::DesktopConfig::load().unwrap().known_folder_backup.is_empty());
                let all_stops = fx.bridge.db.backup_stops().unwrap();
                let view = crate::backup::stop::stop_view(&all_stops, "documents", "Documents", false);
                assert_eq!(view.stop_words.as_deref(), Some(beebeeb_core::backup::MSG_STOP_ERASE_REQUEST));
                assert_eq!(view.stop_action, Some("Back up again"));
                let requests = fx.server.finish();
                assert_eq!(Fixture::creates(&requests), 0, "decided before the listing could create a category folder");
            })
            .await;
    }

    /// A notice older than the backup's last pass counts for nothing: the pass runs, and its `last_pass_at` moves to
    /// the check's `Date` (server clock, here 3 minutes ahead of this machine).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_notice_older_than_the_last_pass_changes_nothing_and_the_pass_moves_its_mark() {
        let config_dir = tempfile::tempdir().unwrap();
        crate::config::CONFIG_DIR_OVERRIDE
            .scope(config_dir.path().to_path_buf(), async {
                switch_on(&["pictures"]);
                let fx = Fixture::new();
                let (pictures, sync_root) = (source_with(3), tempfile::tempdir().unwrap());
                let last = now_ms() - 60_000;
                fx.bridge.db.set_backup_last_pass("pictures", last).unwrap();
                let old = notice_row("files_removed", serde_json::json!({ "reason": "account_lapsed" }), last - 1);
                *fx.server.notices.lock().unwrap() = Some(vec![old]);
                *fx.server.date_ahead_s.lock().unwrap() = Some(180);

                let report = run(&fx, &[(Category::Pictures, pictures.path())], sync_root.path()).await;
                assert_eq!(count_backup_ops(&fx.bridge), 3, "{:?}", report.passes);
                assert!(stops(&fx.bridge).is_empty());
                let mark = fx.bridge.db.backup_pass_mark("pictures").unwrap().last_pass_at_ms.unwrap();
                let ahead = mark - now_ms();
                assert!((170_000..=181_000).contains(&ahead), "the mark is the server's Date: {ahead} ms ahead");
                fx.server.finish();
            })
            .await;
    }

    /// Amendment 12c: a notice check that fails runs no pass (nothing queued, nothing turned off) and the report says
    /// so; the next pass with a working check runs.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_notice_check_runs_no_pass_and_the_next_one_retries() {
        let config_dir = tempfile::tempdir().unwrap();
        crate::config::CONFIG_DIR_OVERRIDE
            .scope(config_dir.path().to_path_buf(), async {
                switch_on(&["pictures"]);
                let fx = Fixture::new();
                let (pictures, sync_root) = (source_with(2), tempfile::tempdir().unwrap());
                *fx.server.notices.lock().unwrap() = None;

                let report = run(&fx, &[(Category::Pictures, pictures.path())], sync_root.path()).await;
                assert!(report.check_failed);
                assert_eq!(count_backup_ops(&fx.bridge), 0, "nothing is uploaded unchecked");
                assert!(stops(&fx.bridge).is_empty());
                assert_eq!(fx.bridge.db.backup_pass_mark("pictures").unwrap().last_pass_at_ms, None);

                *fx.server.notices.lock().unwrap() = Some(Vec::new());
                let report = run(&fx, &[(Category::Pictures, pictures.path())], sync_root.path()).await;
                assert!(!report.check_failed);
                assert_eq!(count_backup_ops(&fx.bridge), 2, "{:?}", report.passes);
                fx.server.finish();
            })
            .await;
    }

    /// Amendment 12 steps 2 and 3, per backup: a `backup_deleted` notice naming this device's Pictures folder stops it
    /// as `backup_deleted`; Documents, gone with no notice, stops as `unknown`; each in core's words.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_backup_deleted_notice_and_a_folder_gone_without_one_say_different_words() {
        let config_dir = tempfile::tempdir().unwrap();
        crate::config::CONFIG_DIR_OVERRIDE
            .scope(config_dir.path().to_path_buf(), async {
                switch_on(&["pictures", "documents"]);
                let fx = Fixture::new();
                let (pictures, documents, sync_root) = (source_with(1), source_with(1), tempfile::tempdir().unwrap());
                let pictures_folder = fx.add(Some(&fx.device.clone()), "Pictures", true, 0);
                let documents_folder = fx.add(Some(&fx.device.clone()), "Documents", true, 0);
                fx.bridge.db.set_backup_category_folder("pictures", Some(&pictures_folder)).unwrap();
                fx.bridge.db.set_backup_category_folder("documents", Some(&documents_folder)).unwrap();
                let last = now_ms() - 120_000;
                fx.bridge.db.set_backup_last_pass("pictures", last).unwrap();
                fx.bridge.db.set_backup_last_pass("documents", last).unwrap();
                fx.trash(&pictures_folder);
                fx.trash(&documents_folder);
                let data = serde_json::json!({ "device_folder_id": fx.device, "category_folder_id": pictures_folder });
                *fx.server.notices.lock().unwrap() = Some(vec![notice_row("backup_deleted", data, last + 60_000)]);

                run(
                    &fx,
                    &[(Category::Pictures, pictures.path()), (Category::Documents, documents.path())],
                    sync_root.path(),
                )
                .await;
                assert_eq!(
                    stops(&fx.bridge),
                    [("documents", "unknown"), ("pictures", "backup_deleted")]
                        .map(|(c, r)| (c.to_string(), r.to_string()))
                );
                let words = |key, folder| {
                    let all_stops = fx.bridge.db.backup_stops().unwrap();
                    crate::backup::stop::stop_view(&all_stops, key, folder, false).stop_words.unwrap()
                };
                let pictures_words = words("pictures", "Pictures");
                let from_another = "'s backup of Pictures was deleted from another device.";
                assert!(pictures_words.ends_with(from_another), "{pictures_words}");
                let documents_words = words("documents", "Documents");
                let gone = "'s backup of Documents was deleted, so it is off.";
                assert!(documents_words.ends_with(gone), "{documents_words}");
                assert_eq!(count_backup_ops(&fx.bridge), 0);
                fx.server.finish();
            })
            .await;
    }

    /// "Back up again" (the folder switch's `restart`) starts the backup over from that moment: the notice that
    /// stopped it is older than its new baseline, and the records of the files the reset removed are forgotten, so the
    /// next pass rebuilds them from the server (which holds none) and uploads both files again.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn back_up_again_starts_over_past_the_notice_that_stopped_it() {
        let config_dir = tempfile::tempdir().unwrap();
        crate::config::CONFIG_DIR_OVERRIDE
            .scope(config_dir.path().to_path_buf(), async {
                switch_on(&["pictures"]);
                let fx = Fixture::new();
                let (pictures, sync_root) = (source_with(2), tempfile::tempdir().unwrap());
                // Both files were backed up before the reset removed them: records with their exact fingerprints.
                let now_s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
                let mtime_s = now_s - 600;
                for name in ["f0.txt", "f1.txt"] {
                    let path = pictures.path().join(name);
                    let file = std::fs::File::options().write(true).open(&path).unwrap();
                    file.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(mtime_s)).unwrap();
                    let record = BackupRecord {
                        key: name.into(),
                        size: 5,
                        modified_ms: i64::try_from(mtime_s * 1000).unwrap(),
                        remote_id: uuid::Uuid::new_v4(),
                        uploaded_at_ms: 1,
                        state: RecordState::BackedUp,
                    };
                    let expected = RecordRef { key: record.key.clone(), remote_id: record.remote_id };
                    assert!(fx.bridge.db.put_backup_record_cas("pictures", &expected, &record).unwrap());
                }
                let last = now_ms() - 120_000;
                fx.bridge.db.set_backup_last_pass("pictures", last).unwrap();
                let reset_data = serde_json::json!({ "reason": "account_reset" });
                let reset = notice_row("files_removed", reset_data, last + 60_000);
                *fx.server.notices.lock().unwrap() = Some(vec![reset]);
                run(&fx, &[(Category::Pictures, pictures.path())], sync_root.path()).await;
                assert_eq!(stops(&fx.bridge), [("pictures".to_string(), "account_reset".to_string())]);

                // The card's "Back up again" is `set_known_folder_backup(key, true)`: the switch on, then `restart`.
                switch_on(&["pictures"]);
                crate::backup::stop::restart(&fx.bridge.db, "pictures").unwrap();
                let report = run(&fx, &[(Category::Pictures, pictures.path())], sync_root.path()).await;
                assert!(stops(&fx.bridge).is_empty(), "{:?}", report.passes);
                assert_eq!(count_backup_ops(&fx.bridge), 2, "{:?}", report.passes);
                fx.server.finish();
            })
            .await;
    }

    /// Review (2026-11-07): switching a backup back on is not a fresh setup when it has passed before: a removal of
    /// the account's files while it was off still stops it at its next pass.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_removal_while_switched_off_still_stops_the_backup_when_it_is_switched_on() {
        let config_dir = tempfile::tempdir().unwrap();
        crate::config::CONFIG_DIR_OVERRIDE
            .scope(config_dir.path().to_path_buf(), async {
                let fx = Fixture::new();
                let (pictures, sync_root) = (source_with(2), tempfile::tempdir().unwrap());
                let last = now_ms() - 120_000;
                fx.bridge.db.set_backup_last_pass("pictures", last).unwrap();
                let erase_data = serde_json::json!({ "reason": "erase_request" });
                *fx.server.notices.lock().unwrap() = Some(vec![notice_row("files_removed", erase_data, last + 60_000)]);

                // Switched on again after the erase (`set_known_folder_backup(key, true)`).
                switch_on(&["pictures"]);
                crate::backup::stop::restart(&fx.bridge.db, "pictures").unwrap();
                run(&fx, &[(Category::Pictures, pictures.path())], sync_root.path()).await;
                assert_eq!(stops(&fx.bridge), [("pictures".to_string(), "erase_request".to_string())]);
                assert_eq!(count_backup_ops(&fx.bridge), 0);
                fx.server.finish();
            })
            .await;
    }

    /// Review (2026-11-07): a backup with a stop row is never passed, even when its switch is still on (the settings
    /// write failed): nothing is listed, created or queued for it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_stopped_backup_whose_switch_stayed_on_is_not_passed() {
        let config_dir = tempfile::tempdir().unwrap();
        crate::config::CONFIG_DIR_OVERRIDE
            .scope(config_dir.path().to_path_buf(), async {
                switch_on(&["pictures"]);
                let fx = Fixture::new();
                let (pictures, sync_root) = (source_with(2), tempfile::tempdir().unwrap());
                let stop = crate::state_db::BackupStop { category: "pictures".into(), reason: "unknown".into(), at_ms: 1 };
                fx.bridge.db.record_backup_stop(&stop).unwrap();

                let report = run(&fx, &[(Category::Pictures, pictures.path())], sync_root.path()).await;
                assert_eq!(count_backup_ops(&fx.bridge), 0, "{:?}", report.passes);
                let requests = fx.server.finish();
                assert_eq!(Fixture::creates(&requests), 0);
                assert!(!requests.iter().any(|r| r.path.starts_with("/api/v1/files?")), "nothing is listed for it");
            })
            .await;
    }

    /// Amendment 12 step 2: a `backup_deleted` notice naming this device's folder (no category) turns every backup of
    /// the device off, even when the folder was restored from Trash before the pass; the layout stays (the folder is
    /// there), and "Back up again" finds it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_notice_naming_this_devices_folder_turns_every_backup_off() {
        let config_dir = tempfile::tempdir().unwrap();
        crate::config::CONFIG_DIR_OVERRIDE
            .scope(config_dir.path().to_path_buf(), async {
                switch_on(&["pictures", "documents"]);
                let fx = Fixture::new();
                let (pictures, documents, sync_root) = (source_with(1), source_with(1), tempfile::tempdir().unwrap());
                let last = now_ms() - 120_000;
                fx.bridge.db.set_backup_last_pass("pictures", last).unwrap();
                fx.bridge.db.set_backup_last_pass("documents", last).unwrap();
                let data = serde_json::json!({ "device_folder_id": fx.device });
                *fx.server.notices.lock().unwrap() = Some(vec![notice_row("backup_deleted", data, last + 60_000)]);

                run(
                    &fx,
                    &[(Category::Pictures, pictures.path()), (Category::Documents, documents.path())],
                    sync_root.path(),
                )
                .await;
                assert_eq!(
                    stops(&fx.bridge),
                    [("documents", "backup_deleted"), ("pictures", "backup_deleted")]
                        .map(|(c, r)| (c.to_string(), r.to_string()))
                );
                assert!(fx.bridge.db.backup_layout().unwrap().is_some(), "the device folder is there: the layout stays");
                assert_eq!(count_backup_ops(&fx.bridge), 0);
                fx.server.finish();
            })
            .await;
    }
