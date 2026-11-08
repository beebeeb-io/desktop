//! LD-0113 PR 2: the Backups layout (spec §4.2 rules 3–6, §5.2) against an in-memory server: `GET`/`PUT
//! /backups/root` as server `42a05fac` answers them, paged listings with `created_at` and `system_role`, folder
//! creates, moves, trashes, and the device manifest's upload and download (chunks kept as sent).

use super::*;
use crate::backup::layout::{device_manifest, ensure_layout, resolve_root_id};
use beebeeb_core::backup::{DeviceSelf, MANIFEST_NAME, parse_manifest};
use beebeeb_types::backup::{Category, Platform};

    #[derive(Clone, Debug)]
    struct Node {
        parent: Option<String>,
        name_encrypted: String,
        is_folder: bool,
        trashed: bool,
        size: u64,
        created_at: String,
        uploaded_at: String,
        flagged: bool,
        chunks: Vec<Vec<u8>>,
        /// An upload in progress (init seen, complete not yet): not listed.
        pending: bool,
    }

    type Tree = Arc<Mutex<std::collections::BTreeMap<String, Node>>>;

    struct LayoutServer {
        base_url: String,
        nodes: Tree,
        requests: Arc<Mutex<Vec<RecordedRequest>>>,
        stop: Arc<AtomicBool>,
        handle: Option<thread::JoinHandle<()>>,
    }

    const PAGE: usize = 2;

    fn node_json(id: &str, node: &Node) -> serde_json::Value {
        serde_json::json!({
            "id": id, "name_encrypted": node.name_encrypted, "is_folder": node.is_folder,
            "is_trashed": node.trashed, "size_bytes": node.size, "parent_id": node.parent,
            "created_at": node.created_at, "uploaded_at": node.uploaded_at, "updated_at": node.uploaded_at,
            "chunk_count": node.chunks.len(),
            "system_role": if node.flagged { serde_json::json!("backups_root") } else { serde_json::Value::Null },
        })
    }

    fn query_param<'a>(path: &'a str, name: &str) -> Option<&'a str> {
        path.split_once('?')?.1.split('&').find_map(|pair| pair.strip_prefix(&format!("{name}=")))
    }

    fn respond(tree: &mut std::collections::BTreeMap<String, Node>, request: &RecordedRequest) -> Vec<u8> {
        let (method, path) = (request.method.as_str(), request.path.as_str());
        let body = || serde_json::from_slice::<serde_json::Value>(&request.body).unwrap_or_default();
        let json = |status: &str, value: serde_json::Value| http_json(status, value).into_bytes();
        let flagged = || tree.iter().find(|(_, n)| n.flagged).map(|(id, _)| id.clone());
        // LD-0139: every pass starts with the notice check (server LD-0141's filtered listing); this account has no
        // notices: one page, and `next_cursor` null.
        if method == "GET" && path.starts_with("/api/v1/notifications") {
            let empty = serde_json::json!({ "notifications": [], "unread_count": 0, "next_cursor": null });
            return http_json_dated("200 OK", empty, 0).into_bytes();
        }
        if path == "/api/v1/backups/root" && method == "GET" {
            return match flagged() {
                Some(id) => json("200 OK", serde_json::json!({ "id": id })),
                None => json("404 Not Found", serde_json::json!({ "error": "not found" })),
            };
        }
        if path == "/api/v1/backups/root" && method == "PUT" {
            let body = body();
            let folder_id = body["folder_id"].as_str().unwrap().to_string();
            if let Some(id) = flagged() {
                return json("200 OK", serde_json::json!({ "id": id, "outcome": "existing" }));
            }
            if let Some(node) = tree.get_mut(&folder_id) {
                if node.is_folder && node.parent.is_none() && !node.trashed {
                    node.flagged = true;
                    return json("200 OK", serde_json::json!({ "id": folder_id, "outcome": "claimed" }));
                }
                return json("409 Conflict", serde_json::json!({ "error": "backups_root_unclaimable" }));
            }
            let Some(name) = body["name_encrypted"].as_str() else {
                return json("409 Conflict", serde_json::json!({ "error": "backups_root_unclaimable" }));
            };
            tree.insert(folder_id.clone(), Node::folder(None, name.to_string(), "2026-11-07T08:00:00Z", true));
            return json("200 OK", serde_json::json!({ "id": folder_id, "outcome": "created" }));
        }
        if method == "GET" && (path == "/api/v1/files" || path.starts_with("/api/v1/files?")) {
            let parent = query_param(path, "parent_id").map(str::to_string);
            let offset: usize =
                query_param(path, "cursor").and_then(|c| c.strip_prefix('c')).map_or(0, |n| n.parse().unwrap());
            let children: Vec<serde_json::Value> = tree
                .iter()
                .filter(|(_, node)| node.parent == parent && !node.pending)
                .map(|(id, node)| node_json(id, node))
                .collect();
            let page: Vec<_> = children.iter().skip(offset).take(PAGE).cloned().collect();
            let next = (offset + PAGE < children.len()).then(|| format!("c{}", offset + PAGE));
            return json("200 OK", serde_json::json!({ "files": page, "next_cursor": next }));
        }
        if let Some(rest) = path.strip_prefix("/api/v1/files/") {
            if let Some((id, index)) = rest.split_once("/chunks/") {
                let index: usize = index.parse().unwrap();
                return match tree.get(id).and_then(|n| n.chunks.get(index)) {
                    Some(bytes) => http_bytes("200 OK", bytes),
                    None => json("404 Not Found", serde_json::json!({ "error": "no chunk" })),
                };
            }
            let id = rest.to_string();
            return match (method, tree.get_mut(&id)) {
                ("GET", Some(node)) => json("200 OK", node_json(&id, node)),
                ("DELETE", Some(node)) => {
                    if node.flagged {
                        return json("409 Conflict", serde_json::json!({ "error": "protected_folder", "message": "x" }));
                    }
                    node.trashed = true;
                    json("200 OK", serde_json::json!({ "id": id, "is_trashed": true }))
                }
                ("PATCH", Some(node)) => {
                    let body = body();
                    if let Some(name) = body["name_encrypted"].as_str() {
                        node.name_encrypted = name.to_string();
                    }
                    if let Some(parent) = body["parent_id"].as_str() {
                        node.parent = Some(parent.to_string());
                    }
                    json("200 OK", node_json(&id, node))
                }
                _ => json("404 Not Found", serde_json::json!({ "error": "not found" })),
            };
        }
        if method == "POST" && path == "/api/v1/files/folder" {
            let body = body();
            let id = body["folder_id"].as_str().unwrap().to_string();
            let node = Node::folder(
                body["parent_id"].as_str().map(str::to_string),
                body["name_encrypted"].as_str().unwrap().to_string(),
                "2026-11-07T08:00:00Z",
                false,
            );
            let answer = node_json(&id, &node);
            tree.insert(id, node);
            return json("201 Created", answer);
        }
        if method == "POST" && path == "/api/v1/uploads/init" {
            let body = body();
            let id = body["file_id"].as_str().unwrap().to_string();
            let chunk_count = body["chunk_count"].as_i64().unwrap();
            let mut node = Node::folder(
                body["parent_id"].as_str().map(str::to_string),
                body["file_name"].as_str().unwrap().to_string(),
                "2026-11-07T08:00:00Z",
                false,
            );
            node.is_folder = false;
            node.pending = true;
            node.size = body["file_size_bytes"].as_u64().unwrap();
            tree.insert(id.clone(), node);
            return json(
                "201 Created",
                serde_json::json!({
                    "file_id": id, "tenant_id": "tenant-1", "object_version_id": format!("v-{id}"),
                    "upload_session_id": format!("s-{id}"), "chunk_size_bytes": body["chunk_size_bytes"],
                    "chunk_count": chunk_count, "storage_format_version": 1, "storage_pool_id": "pool-1",
                    "region": "local",
                }),
            );
        }
        if let Some(rest) = path.strip_prefix("/api/v1/uploads/s-") {
            if let Some((id, index)) = rest.split_once("/chunks/") {
                let node = tree.get_mut(id).unwrap();
                let index: usize = index.parse().unwrap();
                if node.chunks.len() <= index {
                    node.chunks.resize(index + 1, Vec::new());
                }
                node.chunks[index] = request.body.clone();
                return json("200 OK", serde_json::json!({ "index": index, "size": request.body.len(), "skipped": false }));
            }
            if let Some(id) = rest.strip_suffix("/complete") {
                let node = tree.get_mut(id).unwrap();
                node.pending = false;
                return json("200 OK", serde_json::json!({ "version_number": 1, "size_bytes": node.size }));
            }
        }
        json("404 Not Found", serde_json::json!({ "error": format!("unexpected {method} {path}") }))
    }

    impl Node {
        fn folder(parent: Option<String>, name_encrypted: String, created_at: &str, flagged: bool) -> Self {
            Node {
                parent,
                name_encrypted,
                is_folder: true,
                trashed: false,
                size: 0,
                created_at: created_at.to_string(),
                uploaded_at: "2026-11-06T10:00:00Z".to_string(),
                flagged,
                chunks: Vec::new(),
                pending: false,
            }
        }
    }

    impl LayoutServer {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let nodes: Tree = Arc::default();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let (tree, log, halt) = (Arc::clone(&nodes), Arc::clone(&requests), Arc::clone(&stop));
            let handle = thread::spawn(move || {
                let started = std::time::Instant::now();
                while !halt.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(60) {
                    let mut stream = match listener.accept() {
                        Ok((stream, _)) => stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                            continue;
                        }
                        Err(e) => panic!("layout mock accept failed: {e}"),
                    };
                    stream.set_nonblocking(false).unwrap();
                    let request = read_http_request(&mut stream);
                    log.lock().unwrap().push(request.clone());
                    let response = respond(&mut tree.lock().unwrap(), &request);
                    let _ = stream.write_all(&response);
                }
            });
            Self { base_url, nodes, requests, stop, handle: Some(handle) }
        }

        fn requests(&self) -> Vec<RecordedRequest> {
            self.requests.lock().unwrap().clone()
        }

        fn count(&self, method: &str, path: &str) -> usize {
            self.requests().iter().filter(|r| r.method == method && r.path == path).count()
        }

        fn live_children(&self, parent: Option<&str>) -> Vec<(String, Node)> {
            self.nodes
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, n)| n.parent.as_deref() == parent && !n.trashed && !n.pending)
                .map(|(id, n)| (id.clone(), n.clone()))
                .collect()
        }
    }

    impl Drop for LayoutServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    struct Fixture {
        bridge: Arc<EngineBridge>,
        server: LayoutServer,
        dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let server = LayoutServer::start();
            let bridge = Arc::new(test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [42u8; 32]));
            bridge.db.set_owner(&crate::account_binding::Identity::new(Some("u-a"), Some("a@beebeeb.io"))).unwrap();
            Fixture { bridge, server, dir }
        }

        /// A server item with a self-minted id and its name sealed for it.
        fn add(&self, parent: Option<&str>, name: &str, is_folder: bool, size: u64, created_at: &str) -> String {
            let master = beebeeb_core::kdf::MasterKey::from_bytes(*self.bridge.api.master_key());
            let id = beebeeb_core::item_id::NewItemId::owner(&master).unwrap().as_str().to_string();
            let name_encrypted = owned_metadata_for_name(&self.bridge.db, self.bridge.api.master_key(), &id, name, None).unwrap();
            let mut node = Node::folder(parent.map(str::to_string), name_encrypted, created_at, false);
            node.is_folder = is_folder;
            node.size = size;
            self.server.nodes.lock().unwrap().insert(id.clone(), node);
            id
        }

        fn flag(&self, id: &str) {
            self.server.nodes.lock().unwrap().get_mut(id).unwrap().flagged = true;
        }

        fn name_of(&self, node: &Node, id: &str) -> Option<String> {
            let (key, _) = owned_key_and_pin(&self.bridge.db, self.bridge.api.master_key(), id).ok()?;
            beebeeb_core::encrypt::decrypt_name_with_key(&key, &node.name_encrypted)
        }

        /// The live child of `parent` named `name`.
        fn child(&self, parent: Option<&str>, name: &str) -> Option<(String, Node)> {
            self.server.live_children(parent).into_iter().find(|(id, node)| self.name_of(node, id).as_deref() == Some(name))
        }

        fn names(&self, parent: &str) -> Vec<String> {
            let mut names: Vec<String> = self
                .server
                .live_children(Some(parent))
                .iter()
                .filter_map(|(id, node)| self.name_of(node, id))
                .collect();
            names.sort();
            names
        }
    }

    fn glegion() -> DeviceSelf {
        DeviceSelf {
            device_id: beebeeb_core::backup::device_id_from_machine_id("4c4c4544-0042-3510-8052-b4c04f4e3732"),
            name: "GLegion".into(),
            platform: Platform::Windows,
        }
    }

    /// Spec §5.2 and §4.2 rule 5: a flagged root comes from `GET` (no `PUT`); an account from before the flag claims
    /// its legacy `Backups` folder with `PUT` (no name sent, the folder kept); an account with none creates one with a
    /// minted id and the sealed name. Each answer is cached: the next resolution sends nothing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn root_resolved_via_get_then_claimed_then_cached() {
        // 1. Flagged: GET answers, nothing is claimed.
        let fx = Fixture::new();
        let root = fx.add(None, "Backups", true, 0, "2026-05-01T10:00:00Z");
        fx.flag(&root);
        assert_eq!(resolve_root_id(&fx.bridge).await.unwrap(), root);
        assert_eq!(fx.server.count("GET", "/api/v1/backups/root"), 1);
        assert_eq!(fx.server.count("PUT", "/api/v1/backups/root"), 0, "a flagged root is never claimed again");
        assert_eq!(fx.bridge.db.backup_root().unwrap().as_deref(), Some(root.as_str()), "cached");

        // 2. Not flagged yet: the legacy `Backups` is claimed (the oldest live one named exactly so).
        let fx = Fixture::new();
        fx.add(None, "Photos", true, 0, "2026-01-01T10:00:00Z");
        fx.add(None, "Backups", false, 3, "2026-01-02T10:00:00Z"); // a FILE named Backups never counts
        let legacy = fx.add(None, "Backups", true, 0, "2026-05-01T10:00:00Z");
        fx.add(None, "notes.txt", false, 9, "2026-05-02T10:00:00Z");
        assert_eq!(resolve_root_id(&fx.bridge).await.unwrap(), legacy);
        let puts: Vec<serde_json::Value> = fx
            .server
            .requests()
            .iter()
            .filter(|r| r.method == "PUT" && r.path == "/api/v1/backups/root")
            .map(|r| serde_json::from_slice(&r.body).unwrap())
            .collect();
        assert_eq!(puts.len(), 1);
        assert_eq!(puts[0]["folder_id"], legacy.as_str());
        assert!(puts[0].get("name_encrypted").is_none(), "a claim sends no name");
        assert!(fx.server.nodes.lock().unwrap()[&legacy].flagged, "the server flagged the legacy folder");
        assert!(
            fx.server.requests().iter().all(|r| r.origin.as_deref() == Some("backup")),
            "every request is the backup's"
        );

        // 3. Cached: the next resolution sends no request at all.
        let before = fx.server.requests().len();
        assert_eq!(resolve_root_id(&fx.bridge).await.unwrap(), legacy);
        assert_eq!(fx.server.requests().len(), before, "a cached root is never looked up again");

        // 4. None at all: created with a minted id and `Backups` sealed for it.
        let fx = Fixture::new();
        let created = resolve_root_id(&fx.bridge).await.unwrap();
        let nodes = fx.server.nodes.lock().unwrap().clone();
        let node = &nodes[&created];
        assert!(node.flagged && node.is_folder && node.parent.is_none());
        assert_eq!(fx.name_of(node, &created).as_deref(), Some("Backups"));
        assert_eq!(fx.bridge.db.backup_root().unwrap().as_deref(), Some(created.as_str()));
    }

    /// Spec §4.2 rules 5–6: two legacy roots named `Backups` become one. The oldest survives; the other's children
    /// move into it, a same-named folder merges, a same-named file keeps both (`a (merged, HH.MM).jpg`), each
    /// emptied duplicate goes to Trash, and only then is the survivor claimed.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn legacy_duplicate_roots_merged_then_claimed() {
        let fx = Fixture::new();
        let old = fx.add(None, "Backups", true, 0, "2026-01-01T10:00:00Z");
        let new = fx.add(None, "Backups", true, 0, "2026-06-01T10:00:00Z");
        let old_device = fx.add(Some(&old), "GLegion", true, 0, "2026-01-01T10:00:00Z");
        let old_pictures = fx.add(Some(&old_device), "Pictures", true, 0, "2026-01-01T10:00:00Z");
        fx.add(Some(&old_pictures), "a.jpg", false, 5, "2026-01-01T10:00:00Z");
        let new_device = fx.add(Some(&new), "GLegion", true, 0, "2026-06-01T10:00:00Z");
        let new_pictures = fx.add(Some(&new_device), "Pictures", true, 0, "2026-06-01T10:00:00Z");
        let clash = fx.add(Some(&new_pictures), "a.jpg", false, 6, "2026-06-01T10:00:00Z");
        let new_documents = fx.add(Some(&new_device), "Documents", true, 0, "2026-06-01T10:00:00Z");
        fx.add(Some(&new_documents), "d.txt", false, 3, "2026-06-01T10:00:00Z");
        let phone = fx.add(Some(&new), "Guus's iPhone", true, 0, "2026-06-01T10:00:00Z");

        assert_eq!(resolve_root_id(&fx.bridge).await.unwrap(), old, "the oldest survives");

        let nodes = fx.server.nodes.lock().unwrap().clone();
        assert!(nodes[&new].trashed, "the emptied duplicate root is in Trash");
        assert!(fx.server.live_children(Some(&new)).is_empty(), "nothing is left in the duplicate");
        assert!(nodes[&new_device].trashed && nodes[&new_pictures].trashed, "merged folders emptied and trashed");
        assert!(!nodes[&new_documents].trashed, "a folder with no twin is moved, not trashed");
        assert_eq!(nodes[&phone].parent.as_deref(), Some(old.as_str()), "another device's folder moved into the survivor");
        assert_eq!(fx.names(&old), ["GLegion", "Guus's iPhone"]);
        assert_eq!(fx.names(&old_device), ["Documents", "Pictures"]);
        let pictures = fx.names(&old_pictures);
        assert_eq!(pictures.len(), 2, "both files kept: {pictures:?}");
        assert!(pictures.contains(&"a.jpg".to_string()), "the survivor's file keeps its name: {pictures:?}");
        assert_eq!(
            pictures.iter().filter(|n| n.starts_with("a (merged, ") && n.ends_with(").jpg") && !n.contains(':')).count(),
            1,
            "the clash keeps both with core's name: {pictures:?}"
        );
        assert_eq!(nodes[&clash].parent.as_deref(), Some(old_pictures.as_str()));
        assert_eq!(fx.names(&new_documents), ["d.txt"]);
        assert!(nodes[&old].flagged && !nodes[&new].flagged, "the survivor is claimed");
        let order: Vec<&str> = fx
            .server
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.method == "PUT" || r.method == "DELETE")
            .map(|r| if r.method == "PUT" { "claim" } else { "trash" })
            .collect::<Vec<_>>()
            .into_iter()
            .collect();
        assert_eq!(order.last(), Some(&"claim"), "claimed after the merge: {order:?}");
        assert_eq!(order.iter().filter(|s| **s == "trash").count(), 3, "two merged folders and the duplicate root");
    }

    /// Spec §4.2 rule 3 (b), §6.1 point 6, rule 10: the first pass adopts the old mirror's `Backups/GLegion` (my
    /// exact name, no manifest), writes this device's `.device.json` into it at once, and rebuilds the records from
    /// what the mirror already uploaded, so an unchanged file is not uploaded again and a new one is. Another
    /// device's folder (its own manifest) is left alone. With the layout forgotten, the manifest finds the folder
    /// again (rule 3 (a)) and no second manifest is written.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn first_pass_writes_manifest_and_adopts_legacy_glegion_folder() {
        let fx = Fixture::new();
        let root = fx.add(None, "Backups", true, 0, "2026-01-01T10:00:00Z");
        fx.flag(&root);
        let phone = fx.add(Some(&root), "Guus's iPhone", true, 0, "2026-02-01T10:00:00Z");
        let phone_manifest = serde_json::json!({
            "version": 1, "device_id": "ios-0f3c", "device_name": "Guus's iPhone", "created_at": "2026-02-01T10:00:00Z",
        });
        fx.bridge
            .backup_write_small_file(&phone, MANIFEST_NAME, phone_manifest.to_string().as_bytes(), "application/json")
            .await
            .unwrap();
        let legacy = fx.add(Some(&root), "GLegion", true, 0, "2026-03-01T10:00:00Z");
        let pictures = fx.add(Some(&legacy), "Pictures", true, 0, "2026-03-01T10:00:00Z");
        let source = fx.dir.path().join("Pictures");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("old.jpg"), b"mirrored").unwrap();
        // The mirror copied it after it last changed (its server copy is from 2026-11-06).
        let changed = std::time::UNIX_EPOCH + Duration::from_secs(1_767_225_600); // 2026-01-01
        std::fs::File::options().write(true).open(source.join("old.jpg")).unwrap().set_modified(changed).unwrap();
        fx.add(Some(&pictures), "old.jpg", false, 8, "2026-03-01T10:00:00Z");
        std::fs::write(source.join("new.jpg"), b"taken today").unwrap();
        let writes_before = fx.server.count("POST", "/api/v1/uploads/init");

        let me = glegion();
        let layout = ensure_layout(&fx.bridge, &me, &[Category::Pictures]).await.unwrap();
        assert_eq!(layout.root_id, root);
        assert_eq!(layout.device_folder_id, legacy, "the old mirror's folder is adopted, not doubled");
        assert_eq!(layout.device_folder_name, "GLegion");
        assert_eq!(fx.server.count("POST", "/api/v1/uploads/init") - writes_before, 1, "one manifest written");
        let (manifest_id, manifest) = fx.child(Some(&legacy), MANIFEST_NAME).expect("the manifest is in GLegion");
        let bytes = fx
            .bridge
            .backup_read_small_file(&manifest_id, manifest.chunks.len() as u32, manifest.size)
            .await
            .unwrap()
            .expect("the manifest opens on this device");
        let parsed = parse_manifest(std::str::from_utf8(&bytes).unwrap()).unwrap();
        assert_eq!(parsed, device_manifest(&me, &[Category::Pictures]));
        assert_eq!(parsed.device_id.as_deref(), Some(me.device_id.as_str()));
        assert_eq!(fx.names(&phone), [MANIFEST_NAME], "another device's folder is untouched");

        // The first pass: the mirror's copy becomes a record; only the new file is queued.
        let sources = [crate::backup::scheduler::BackupSource { category: Category::Pictures, path: source.clone() }];
        let report = crate::backup::scheduler::run_passes(&fx.bridge, layout, &fx.dir.path().join("Beebeeb"), &sources).await;
        let pass = report.passes[0].1.as_ref().expect("the pass ran");
        assert_eq!(pass.queued, 1, "only the new file is uploaded: {pass:?}");
        let records = fx.bridge.db.backup_records("pictures").unwrap();
        assert_eq!(records.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(), ["old.jpg"], "rebuilt from the server");
        let queued: Vec<String> = {
            let conn = rusqlite::Connection::open(fx.dir.path().join("state.db")).unwrap();
            let mut stmt = conn
                .prepare("SELECT json_extract(metadata_json, '$.backup.key') FROM operation_queue WHERE origin = 'backup'")
                .unwrap();
            stmt.query_map([], |row| row.get(0)).unwrap().collect::<Result<_, _>>().unwrap()
        };
        assert_eq!(queued, ["new.jpg"]);

        // Found again through the manifest, with nothing written.
        fx.bridge.db.clear_backup_layout().unwrap();
        let writes = fx.server.count("POST", "/api/v1/uploads/init");
        let again = ensure_layout(&fx.bridge, &me, &[Category::Pictures]).await.unwrap();
        assert_eq!(again.device_folder_id, legacy);
        assert_eq!(fx.server.count("POST", "/api/v1/uploads/init"), writes, "no second manifest");
        assert_eq!(fx.names(&legacy).iter().filter(|n| *n == MANIFEST_NAME).count(), 1);
    }
