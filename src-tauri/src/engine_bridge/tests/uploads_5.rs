//! `uploads_5` engine tests, split out of `engine_bridge.rs`.

use super::*;
    #[tokio::test]
    async fn a_modify_after_a_desktop_upload_sends_the_server_version_as_its_base() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [21u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);

        let created = dir.path().join("created.txt");
        std::fs::write(&created, b"twenty-eight bytes of text.\n").unwrap();
        bridge
            .queue_file_provider_create(finder_file_target(None, "t.txt", &created, None))
            .unwrap();
        drain_upload_queue(&bridge, &sync_root).await;
        let rows = bridge.db.list_files().unwrap();
        assert_eq!(rows.len(), 1, "one row after the create: {rows:?}");
        let server_id = rows[0].file_id.clone();
        assert_eq!(rows[0].status, FileStatus::Local);

        // The `/sync/ops` echo of that upload re-stamps the row with a later
        // wall-clock second, as it does on a device.
        let mut echoed = bridge.db.get_file(&server_id).unwrap().unwrap();
        echoed.remote_updated_at += 30;
        echoed.modified_at = echoed.remote_updated_at;
        bridge.db.upsert_file(&echoed).unwrap();

        let base = held_content_version(&bridge, &server_id);
        let edited = dir.path().join("edited.txt");
        std::fs::write(&edited, b"twenty-eight bytes of text.\nmore-bytes12").unwrap();
        bridge
            .queue_file_provider_modify(finder_file_target(
                Some(&server_id),
                "t.txt",
                &edited,
                Some(base.clone()),
            ))
            .unwrap();
        drain_upload_queue(&bridge, &sync_root).await;

        let state = server.finish();
        assert!(
            state.inits.iter().all(|(_, status)| *status == 201),
            "no upload may be refused (identifier {base:?}): {:?}",
            state.init_summary()
        );
        // SF-0001 (L13): the create's init also names this id, so the replace is the one
        // that carries a base.
        let replace = state
            .inits
            .iter()
            .find(|(body, _)| {
                body["file_id"] == serde_json::json!(server_id) && body["base_version_number"].is_number()
            })
            .unwrap_or_else(|| panic!("no replace upload was sent: {:?}", state.init_summary()));
        assert_eq!(
            replace.0["base_version_number"],
            serde_json::json!(1),
            "the base must be the server version, not the identifier {base:?}"
        );
        assert_eq!(
            state.latest_plaintext(&server_id, master_key),
            b"twenty-eight bytes of text.\nmore-bytes12"
        );
        assert_eq!(state.files[&server_id].versions.len(), 2);
        let row = bridge.db.get_file(&server_id).unwrap().unwrap();
        assert_eq!(
            row.status,
            FileStatus::Local,
            "the edit must not leave the file read-only"
        );
    }

    // ── Identifiers the system still holds from before the version fix ────

    /// A row as an earlier build left it after its own upload and the op
    /// echo: server version 2, wall-clock stamps at 1_791_550_370. That
    /// build told the system `1791550370` (and `1791550370:1791550370:18`).
    fn seed_legacy_row(bridge: &EngineBridge, file_id: &str, content_hash: Option<&str>) {
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: file_id.into(),
                path: "d2-fixture.txt".into(),
                status: FileStatus::Local,
                size_bytes: 18,
                modified_at: 1_791_550_370,
                content_hash: content_hash.map(str::to_string),
                remote_updated_at: 1_791_550_370,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        let mut contract = bridge.db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.current_version = 2;
        bridge.db.set_file_contract_state(&contract).unwrap();
    }

    /// Seed `file_id` as [`seed_legacy_row`] does, queue a content modify of
    /// it with `identifier` as its base, and return the queued op's
    /// `base_version`. Re-seeded per call: a queued write updates the row.
    fn queued_modify_base(
        bridge: &EngineBridge,
        dir: &Path,
        file_id: &str,
        content_hash: Option<&str>,
        identifier: &str,
    ) -> Option<i64> {
        seed_legacy_row(bridge, file_id, content_hash);
        let contents = dir.join(format!("edit-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&contents, b"edited bytes").unwrap();
        bridge
            .queue_finder_modify(finder_file_target(
                Some(file_id),
                "d2-fixture.txt",
                &contents,
                Some(identifier.to_string()),
            ))
            .unwrap();
        let queued = bridge.db.list_due_operations(i64::MAX).unwrap();
        let op = queued
            .iter()
            .rev()
            .find(|op| op.file_id.as_deref() == Some(file_id))
            .expect("the modify was queued");
        let base = op.base_version;
        bridge.db.remove_operation(&op.op_id).unwrap();
        bridge.db.set_status(file_id, FileStatus::Local).unwrap();
        base
    }

    #[test]
    fn a_legacy_identifier_of_the_current_content_is_based_on_the_server_version() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        assert_eq!(
            queued_modify_base(&bridge, dir.path(), "legacy-1", None, "1791550370"),
            Some(2),
            "the old content version of the row as it is now"
        );
        assert_eq!(
            queued_modify_base(&bridge, dir.path(), "legacy-1", None, "1791550370:1791550370:18"),
            Some(2),
            "the old full version identifier of the row as it is now"
        );

        assert_eq!(
            queued_modify_base(&bridge, dir.path(), "legacy-hash", Some("abc123"), "1791550370:abc123"),
            Some(2),
            "the old content version with the row's content hash"
        );
    }

    #[test]
    fn a_legacy_identifier_of_older_content_stays_a_stale_base() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        for older in [
            // Stamped before the latest re-stamp of the row.
            "1791550250",
            "1791550250:abc123",
            "1791550250:1791550250:18",
            // The right stamp, other content.
            "1791550370:def456",
            "1791550370:1791550370:17",
        ] {
            assert_eq!(
                queued_modify_base(&bridge, dir.path(), "legacy-2", Some("abc123"), older),
                parse_base_version_number(Some(older)),
                "{older} does not describe the row's current content and is parsed as before"
            );
        }
    }

    #[tokio::test]
    async fn a_legacy_identifier_of_older_content_is_refused_by_the_server() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let server = VersionedServerMock::start();
        server.seed_file("7bdc152a-c29e-5024-aec5-2448f7248f3c", 2);
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [22u8; 32]);
        seed_legacy_row(&bridge, "7bdc152a-c29e-5024-aec5-2448f7248f3c", None);
        let contents = dir.path().join("edit.txt");
        std::fs::write(&contents, b"edited bytes").unwrap();
        bridge
            .queue_file_provider_modify(finder_file_target(
                Some("7bdc152a-c29e-5024-aec5-2448f7248f3c"),
                "d2-fixture.txt",
                &contents,
                Some("1791550250".into()),
            ))
            .unwrap();
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert!(!state.inits.is_empty(), "the upload was attempted");
        assert!(
            state.inits.iter().all(|(_, status)| *status == 409),
            "an identifier of older content is a stale base: {:?}",
            state.init_summary()
        );
        assert_eq!(state.files["7bdc152a-c29e-5024-aec5-2448f7248f3c"].versions.len(), 2, "no version was added");
    }

    #[test]
    fn a_current_format_identifier_is_parsed_as_before() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = test_bridge(&dir.path().join("state.db"));
        for (identifier, base) in [
            ("2", Some(2)),
            ("2:abc123", Some(2)),
            ("2:1791550370:18", Some(2)),
            ("1", Some(1)),
            ("1:abc123", Some(1)),
            ("0", None),
        ] {
            assert_eq!(
                queued_modify_base(&bridge, dir.path(), "current-1", Some("abc123"), identifier),
                base,
                "{identifier}"
            );
        }
    }

    // ── This device's own queued uploads of one file form a chain ─────────

    impl VersionedServerState {
        /// Files that have at least one completed version.
        pub(super) fn files_with_content(&self) -> Vec<String> {
            self.files
                .iter()
                .filter(|(_, file)| !file.versions.is_empty())
                .map(|(id, _)| id.clone())
                .collect()
        }
    }

    /// A row the server holds at version 1 (seeded on the mock too).
    pub(super) fn seed_uploaded_row(bridge: &EngineBridge, server: &VersionedServerMock, file_id: &str) {
        server.seed_file(file_id, 1);
        server.state.lock().unwrap().authenticated_names.insert(file_id.into(), enc_name(bridge.api.master_key(), file_id, "notes.txt"));
        seed_uploaded_db_row(bridge, file_id);
    }

    /// The local half of [`seed_uploaded_row`]: the row at version 1, for a test that starts its
    /// server later.
    pub(super) fn seed_uploaded_db_row(bridge: &EngineBridge, file_id: &str) {
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: file_id.into(),
                path: "notes.txt".into(),
                status: FileStatus::Local,
                size_bytes: 10,
                modified_at: 1_700_000_000,
                content_hash: None,
                remote_updated_at: 1_700_000_000,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        let mut contract = bridge.db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.current_version = 1;
        bridge.db.set_file_contract_state(&contract).unwrap();
    }

    pub(super) fn queue_save(bridge: &EngineBridge, dir: &Path, file_id: &str, filename: &str, bytes: &[u8], base: &str) {
        fp_save(bridge, dir, file_id, filename, bytes, base);
    }

    /// A File Provider save of `bytes` to `file_id` on `base`, as the extension sends it.
    pub(super) fn fp_save(bridge: &EngineBridge, dir: &Path, file_id: &str, filename: &str, bytes: &[u8], base: &str) -> FpWrite {
        let contents = dir.join(format!("save-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&contents, bytes).unwrap();
        bridge
            .queue_file_provider_modify(finder_file_target(
                Some(file_id),
                filename,
                &contents,
                Some(base.to_string()),
            ))
            .unwrap()
    }

