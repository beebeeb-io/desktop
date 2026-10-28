// Sparse event bodies follow server folder_public.rs at server main 7dd40237.
const A: &str = "40000000-0000-4000-8000-000000000004";
const B: &str = "50000000-0000-4000-8000-000000000005";
const MEMBER: &str = "60000000-0000-4000-8000-000000000006";
struct Convergence {
    dir: tempfile::TempDir,
    root: PathBuf,
    server: ContentServer,
    bridge: EngineBridge,
    prior_versions: Vec<Vec<Vec<u8>>>,
}
impl Convergence {
    async fn new() -> Self {
        Self::with_child(true).await
    }
    async fn with_child(include_child: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sync");
        std::fs::create_dir(&root).unwrap();
        let server = ContentServer::start(&random_key(), ExpectedChunkFormat::V2);
        let bridge = test_bridge_with_api(&dir.path().join("db"), server.base.clone(), [7; 32]);
        {
            let mut w = server.state.lock().unwrap();
            w.nodes = vec![
                snap_node(&[7; 32], ROOT, "Shared", None, true, 1),
                snap_node(&[7; 32], A, "A", None, true, 1),
                snap_node(&[7; 32], B, "B", None, true, 1),
            ];
            let mut file = w.meta.clone();
            file["parent_id"] = json!(ROOT);
            file["updated_at"] = json!(1);
            file["acted_via_share_id"] = json!("guest-link");
            file["owner_wrapped_file_key"] = json!(
                base64::engine::general_purpose::STANDARD.encode(
                    beebeeb_sync::key_resolution::wrap_key_to_owner(&MasterKey::from_bytes([7; 32]), ID, &random_key())
                        .unwrap()
                )
            );
            w.meta = file.clone();
            if include_child {
                w.nodes.push(file);
            }
        }
        let prior_versions = vec![server.state.lock().unwrap().frames.clone()];
        let f = Self {
            dir,
            root,
            server,
            bridge,
            prior_versions,
        };
        let tick = f.tick().await;
        assert!(tick.conflicts.is_empty());
        if include_child {
            f.place(ID, Some(ROOT), "Shared/guest.txt");
        }
        f
    }
    fn node(&self, id: &str) -> serde_json::Value {
        self.server
            .state
            .lock()
            .unwrap()
            .nodes
            .iter()
            .find(|n| n["id"] == id)
            .unwrap()
            .clone()
    }
    fn change(&self, id: &str, patch: serde_json::Value) {
        let mut w = self.server.state.lock().unwrap();
        let n = w.nodes.iter_mut().find(|n| n["id"] == id).unwrap();
        for (k, v) in patch.as_object().unwrap() {
            n[k] = v.clone();
        }
        n["updated_at"] = json!(2);
        if id == ID {
            w.meta = n.clone();
        }
    }
    fn event(&self, kind: &str, payload: serde_json::Value) {
        let mut w = self.server.state.lock().unwrap();
        w.seq += 1;
        let seq = w.seq;
        w.ops.push(json!({"seq_id":seq,"op_type":kind,"payload":payload}));
    }
    async fn tick(&self) -> SyncTickOutcome {
        sync_tick_outcome(&self.bridge, &self.root).await.unwrap()
    }
    fn place(&self, id: &str, parent: Option<&str>, path: &str) {
        let row = self.bridge.db.get_file(id).unwrap().unwrap();
        assert_eq!(row.file_id, id);
        let final_parent = row.parent_id.as_deref();
        let winning_parent = parent;
        assert_eq!(final_parent, winning_parent);
        assert_eq!(row.path, path);
    }
    async fn quiet(&self) {
        let before = self.server.state.lock().unwrap().requests.len();
        let mut quiet_errors = 0;
        for _ in 0..3 {
            let tick = sync_tick_outcome(&self.bridge, &self.root).await;
            match tick {
                Ok(t) => {
                    assert!(t.applied_item_ids.is_empty());
                    quiet_errors += t.conflicts.len();
                }
                Err(_) => quiet_errors += 1,
            }
            crate::watcher::run_one_scan(&self.bridge, &self.root);
            self.bridge
                .process_due_operations(&self.root, now_secs())
                .await
                .unwrap();
        }
        let w = self.server.state.lock().unwrap();
        let quiet_uploads = w.requests[before..]
            .iter()
            .filter(|r| r.path.contains("/uploads/") && matches!(r.method.as_str(), "POST" | "PUT"))
            .count();
        assert_eq!((quiet_uploads, quiet_errors), (0, 0));
        drop(w);
        let rows = self.bridge.db.list_files().unwrap();
        assert_eq!(
            rows.iter()
                .filter(|r| r.status == FileStatus::Error || r.status == FileStatus::Conflict)
                .count(),
            0
        );
        for row in &rows {
            assert!(
                row.parent_id
                    .as_ref()
                    .is_none_or(|id| rows.iter().any(|p| p.file_id == *id)),
                "no orphan DB children"
            );
            assert_eq!(
                self.bridge
                    .db
                    .list_operations_for_file(&row.file_id)
                    .unwrap()
                    .iter()
                    .filter(|o| matches!(o.kind, OperationKind::UploadFile | OperationKind::UploadVersion))
                    .count(),
                0
            );
        }
        // Existing native orphan verifier with a real portable filesystem view. No CF
        // placeholders are minted by this fixture; native identities remain Task 13.
        crate::unknown_check::retry_marks(
            &self.bridge.db,
            &self.root,
            &|path| {
                if path.exists() {
                    crate::unknown_check::PlatformView::PlainFile
                } else {
                    crate::unknown_check::PlatformView::Missing
                }
            },
            &mut |_| Ok(()),
            &|| false,
        )
        .unwrap();
        let orphan_count = self.bridge.db.list_unknown_orphans().unwrap().len();
        assert_eq!(orphan_count, 0);
    }
    async fn renamed(&self, text: &str) -> SyncTickOutcome {
        let old = self.node(ID)["name_encrypted"].clone();
        let new = named(&random_key(), text, ExpectedChunkFormat::V2);
        self.change(ID, json!({"name_encrypted":new}));
        self.event(
            "file_rename",
            json!({"id":ID,"old_name_encrypted":old,"new_name_encrypted":new}),
        );
        self.tick().await
    }
    async fn moved(&self, to: &str) -> SyncTickOutcome {
        let old = self.node(ID)["parent_id"].clone();
        self.change(ID, json!({"parent_id":to}));
        self.event("file_move", json!({"id":ID,"old_parent_id":old,"new_parent_id":to}));
        self.tick().await
    }
}
#[tokio::test]
async fn sf2237d_t6_guest_and_member_add() {
    let f = Convergence::with_child(false).await;
    let guest = f.server.state.lock().unwrap().meta.clone();
    let mut member = guest.clone();
    member["id"] = json!(MEMBER);
    member["name_encrypted"] = json!(named(&random_key(), "member.txt", ExpectedChunkFormat::V2));
    member.as_object_mut().unwrap().remove("acted_via_share_id");
    member["uploaded_by"] = json!("member-account");
    member["owner_wrapped_file_key"] = json!(
        base64::engine::general_purpose::STANDARD.encode(
            beebeeb_sync::key_resolution::wrap_key_to_owner(&MasterKey::from_bytes([7; 32]), MEMBER, &random_key())
                .unwrap()
        )
    );
    f.server
        .state
        .lock()
        .unwrap()
        .nodes
        .extend([guest.clone(), member.clone()]);
    // Both guest and member creates are sparse: no owner envelope or provenance.
    for node in [guest, member] {
        f.event("file_create",json!({"id":node["id"],"name_encrypted":node["name_encrypted"],"parent_id":ROOT,"size_bytes":12,"storage_pool_id":"pool"}));
    }
    let tick = f.tick().await;
    assert!(tick.applied_item_ids.contains(&MEMBER.into()));
    f.place(ID, Some(ROOT), "Shared/guest.txt");
    f.place(MEMBER, Some(ROOT), "Shared/member.txt");
    assert!(f.bridge.db.get_owned_key_record(MEMBER).unwrap().unwrap().member_added);
    f.quiet().await;
}
#[tokio::test]
async fn sf2237d_t6_guest_rename() {
    let f = Convergence::new().await;
    let tick = f.renamed("renamed.txt").await;
    assert_eq!(tick.applied_item_ids, vec![ID]);
    f.place(ID, Some(ROOT), "Shared/renamed.txt");
    f.quiet().await;
}
#[tokio::test]
async fn sf2237d_t6_guest_move_and_parent_rename() {
    let f = Convergence::new().await;
    f.moved(A).await;
    let old = f.node(A)["name_encrypted"].clone();
    let new = enc_name(&[7; 32], A, "Renamed");
    f.change(A, json!({"name_encrypted":new}));
    f.event(
        "folder_rename",
        json!({"id":A,"old_name_encrypted":old,"new_name_encrypted":new}),
    );
    let tick = f.tick().await;
    f.place(ID, Some(A), "Renamed/guest.txt");
    assert!(
        tick.applied_item_ids.contains(&ID.into()),
        "working-set must signal descendants whose paths changed"
    );
    f.change(A, json!({"parent_id":B}));
    f.event("folder_move", json!({"id":A,"old_parent_id":null,"new_parent_id":B}));
    let moved = f.tick().await;
    f.place(A, Some(B), "B/Renamed");
    f.place(ID, Some(A), "B/Renamed/guest.txt");
    assert!(moved.applied_item_ids.contains(&ID.into()));
    assert_eq!(
        f.bridge
            .db
            .get_file_contract_state(ID)
            .unwrap()
            .unwrap()
            .local_base_version,
        0
    );
    f.quiet().await;
}
#[tokio::test]
async fn sf2237d_t6_guest_trash_restore() {
    let f = Convergence::new().await;
    f.event("file_trash", json!({"id":ID}));
    f.tick().await;
    assert_eq!(
        f.bridge.db.get_file(ID).unwrap().unwrap().status,
        FileStatus::Trashing,
        "owner trash, no permanent delete"
    );
    assert!(
        f.bridge
            .db
            .get_owned_key_record(ID)
            .unwrap()
            .unwrap()
            .owner_wrap
            .is_some()
    );
    f.event("file_restore", json!({"id":ID}));
    f.tick().await;
    f.tick().await; // existing restore contract requests authoritative snapshot
    assert_eq!(f.bridge.db.get_file(ID).unwrap().unwrap().status, FileStatus::CloudOnly);
    f.place(ID, Some(ROOT), "Shared/guest.txt");
    f.quiet().await;
}
#[tokio::test]
async fn sf2237d_t6_guest_overwrite_new_version() {
    let mut f = Convergence::new().await;
    let replacement = frames(&random_key(), ID, b"replacement!", ExpectedChunkFormat::V2);
    f.prior_versions.push(replacement.clone());
    f.server.state.lock().unwrap().frames = replacement;
    f.change(ID, json!({"version_number":2,"size_bytes":12}));
    f.event(
        "file_update",
        json!({"id":ID,"size_bytes":12,"version_number":2,"storage_pool_id":"pool"}),
    );
    f.tick().await;
    assert_eq!(
        f.bridge
            .db
            .get_file_contract_state(ID)
            .unwrap()
            .unwrap()
            .current_version,
        2
    );
    assert_eq!(&*f.bridge.do_hydrate(ID, None).await.unwrap(), b"replacement!");
    assert_eq!(f.prior_versions.len(), 2);
    assert_eq!(
        opened(&random_key(), ID, &f.prior_versions[0], ExpectedChunkFormat::V2),
        b"guest bytes!"
    );
    f.place(ID, Some(ROOT), "Shared/guest.txt");
    f.quiet().await;
}
#[tokio::test]
async fn sf2237d_t6_concurrent_rename_move_lww() {
    let f = Convergence::new().await;
    let first = named(&random_key(), "first.txt", ExpectedChunkFormat::V2);
    let winner = named(&random_key(), "winner.txt", ExpectedChunkFormat::V2);
    let old = f.node(ID)["name_encrypted"].clone();
    f.change(ID, json!({"parent_id":B,"name_encrypted":winner}));
    // Two actors' interleaved sparse events in one server-ordered catch-up batch.
    f.event(
        "file_rename",
        json!({"id":ID,"old_name_encrypted":old,"new_name_encrypted":first}),
    );
    f.event("file_move", json!({"id":ID,"old_parent_id":ROOT,"new_parent_id":A}));
    f.event(
        "file_rename",
        json!({"id":ID,"old_name_encrypted":first,"new_name_encrypted":winner}),
    );
    f.event("file_move", json!({"id":ID,"old_parent_id":A,"new_parent_id":B}));
    let tick = f.tick().await;
    assert_eq!(tick.applied_item_ids, vec![ID]);
    f.place(ID, Some(B), "B/winner.txt");
    assert_eq!(
        f.bridge
            .db
            .list_files()
            .unwrap()
            .iter()
            .filter(|r| r.file_id == ID)
            .count(),
        1
    );
    f.quiet().await;
}
#[tokio::test]
async fn sf2237d_t6_replay_restart_no_echo() {
    let mut f = Convergence::new().await;
    f.renamed("once.txt").await;
    let count = f.bridge.db.list_files().unwrap().len();
    // Crash boundary: row committed, cursor not yet committed. Re-open persisted DB.
    f.bridge.db.set_sync_cursor(0).unwrap();
    drop(f.bridge);
    f.bridge = test_bridge_with_api(&f.dir.path().join("db"), f.server.base.clone(), [7; 32]);
    f.tick().await;
    f.place(ID, Some(ROOT), "Shared/once.txt");
    assert_eq!(f.bridge.db.list_files().unwrap().len(), count);
    f.quiet().await;
}
#[tokio::test]
async fn sf2237d_t6_local_dirty_is_held() {
    let f = Convergence::new().await;
    let path = f.root.join("Shared/guest.txt");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    f.bridge.hydrate_file(ID, &path, &[f.root.as_path()]).await.unwrap();
    f.bridge
        .db
        .test_execute(&format!("UPDATE files SET local_base_version=0 WHERE file_id='{ID}'"))
        .unwrap();
    let local_edits = b"saved on this PC";
    std::fs::write(&path, local_edits).unwrap();
    f.server.state.lock().unwrap().frames = frames(&random_key(), ID, b"guest saved!", ExpectedChunkFormat::V2);
    f.change(ID, json!({"version_number":2,"size_bytes":12}));
    f.event(
        "file_update",
        json!({"id":ID,"size_bytes":12,"version_number":2,"storage_pool_id":"pool"}),
    );
    crate::watcher::dispatch_local_create(&f.bridge, &f.root, &path, "notify");
    crate::watcher::run_one_scan(&f.bridge, &f.root);
    f.tick().await;
    assert!(
        f.bridge
            .version_conflict_feed()
            .unwrap()
            .iter()
            .any(|r| r.file_id == ID && r.kind == "held_edit")
    );
    assert!(matches!(f.bridge.hydrate_file(ID,&path,&[f.root.as_path()]).await,Err(e) if e.is::<UnsentLocalChange>()));
    let held_bytes = std::fs::read(&path).unwrap();
    assert_eq!(held_bytes, local_edits);
    assert_eq!(
        sync_folder_content(&f.bridge.db, ID, &path, None).unwrap(),
        SyncFolderContent::Changed { base: None }
    );
    assert_eq!(
        &*f.bridge.do_hydrate(ID, None).await.unwrap(),
        b"guest saved!",
        "the remote version also survives"
    );
    f.quiet().await;
    assert_eq!(std::fs::read(&path).unwrap(), local_edits);
}
