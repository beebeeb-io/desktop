//! Root enumeration, shared-root and 1694 add-subitems payload tests.

use super::*;


    // ------------------------------------------------------------------
    // Task 1701: Finder single root. The root container enumerates the
    // user's REAL top-level tree; the synthetic namespace containers
    // (`My files` / `Shared with me` / `Offline` / `Conflicts`) stop being a
    // Finder surface. Ruling: .claude/tasks/decisions/
    // finder-sidebar-single-root.md. RED-first: these were seen failing
    // against the namespace-split enumeration before the root arm changed.
    //
    // parent_id finding (spec point 1): `files.parent_id` stores the
    // SERVER's parent UUID, or NULL for vault-root items — never a
    // `namespace:` id (`normalize_parent_id` strips those from Finder
    // writes). The namespace parentage existed ONLY at payload-assembly
    // time, as the `file_entry_payload*` fallback identifier. So no state.db
    // remap is needed: pointing the fallback at the root container presents
    // top-level items as the root's children.
    // ------------------------------------------------------------------

    #[test]
    fn test_1701_root_container_enumerates_real_files_not_namespaces() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.upsert_file(&FileEntry {
            file_id: "1701-file".into(),
            path: "/notes.txt".into(),
            status: FileStatus::Local,
            size_bytes: 3,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        db.upsert_file(&FileEntry {
            file_id: "1701-folder".into(),
            path: "/Projects".into(),
            status: FileStatus::Local,
            size_bytes: 0,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::Folder,
        })
        .unwrap();
        // A shared root stays in state.db (NO data loss) but is no longer a
        // Finder surface — the ruling sends shared files to the webapp.
        seed_shared_root(&db, "1701-shared", "Shared with me/Team", PERMISSION_READ);

        let items = list_file_provider_items(&db, FP_ROOT_APPLE);
        let names: Vec<&str> = items.iter().map(|i| i.filename.as_str()).collect();
        assert!(
            items.iter().all(|i| i.kind != "namespace"),
            "zero synthetic namespace items may be enumerated at the root, got {names:?}"
        );
        for synthetic in ["My files", "Shared with me", "Offline", "Conflicts"] {
            assert!(
                !items.iter().any(|i| i.filename == synthetic),
                "synthetic container {synthetic:?} must not be enumerated at the root"
            );
        }
        assert!(
            items.iter().any(|i| i.identifier == "1701-file"),
            "the user's top-level file must be a child of the root container"
        );
        assert!(
            items.iter().any(|i| i.identifier == "1701-folder"),
            "the user's top-level folder must be a child of the root container"
        );
        assert!(
            !items.iter().any(|i| i.identifier == "1701-shared"),
            "shared roots are not a Finder surface (1701 ruling: webapp only)"
        );
    }

    #[test]
    fn test_1701_top_level_items_are_presented_as_children_of_the_root() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.upsert_file(&FileEntry {
            file_id: "1701-file".into(),
            path: "/notes.txt".into(),
            status: FileStatus::Local,
            size_bytes: 3,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        // With a contract whose parent is the server root (`parent_id` NULL —
        // exactly what snapshot ingest writes for top-level rows).
        let mut contract = db.get_file_contract_state("1701-file").unwrap().unwrap();
        contract.namespace = Namespace::MyFiles;
        contract.item_kind = ItemKind::File;
        db.set_file_contract_state(&contract).unwrap();

        let items = list_file_provider_items(&db, FP_ROOT_APPLE);
        let item = items.iter().find(|i| i.identifier == "1701-file").unwrap();
        assert_eq!(
            item.parent_identifier, FP_ROOT_APPLE,
            "a top-level item must be presented as a child of the ROOT container, not of a synthetic namespace"
        );
    }

    /// Spec 2026-10-09 §7.2 (final review): the root lists the rows with no parent, as a
    /// folder lists its children and as every reply names the parent, whatever the path
    /// says. A row with no name is still left out.
    #[test]
    fn the_root_lists_the_rows_without_a_parent_whatever_their_path() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let row = |file_id: &str, path: &str, parent: Option<&str>| {
            db.upsert_file(&FileEntry {
                file_id: file_id.into(),
                path: path.into(),
                status: FileStatus::Uploading,
                size_bytes: 3,
                modified_at: 1,
                content_hash: None,
                remote_updated_at: 1,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
            let mut contract = db.get_file_contract_state(file_id).unwrap().unwrap();
            contract.parent_id = parent.map(str::to_string);
            db.set_file_contract_state(&contract).unwrap();
        };
        // A Finder create in a folder this Mac has no row for keeps its name alone as the path.
        row("named-alone-in-a-folder", "n1.txt", Some("folder-1"));
        row("top-level", "top.txt", None);
        row("no-name", "", None);

        let root: Vec<String> = list_file_provider_items(&db, FP_ROOT_APPLE)
            .into_iter()
            .map(|item| item.identifier)
            .collect();
        assert_eq!(root, vec!["top-level".to_string()]);
        let folder: Vec<String> = list_file_provider_items(&db, "folder-1")
            .into_iter()
            .map(|item| item.identifier)
            .collect();
        assert_eq!(folder, vec!["named-alone-in-a-folder".to_string()]);
    }

    #[test]
    fn test_1701_change_feed_top_level_item_parents_onto_the_root() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.upsert_file(&FileEntry {
            file_id: "1701-file".into(),
            path: "/notes.txt".into(),
            status: FileStatus::Local,
            size_bytes: 3,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        db.record_file_change("1701-file", crate::state_db::FpChangeKind::Modified, None)
            .unwrap();

        let (changes, _) = db.list_file_changes_paged(None, 100).unwrap().unwrap();
        let payloads = file_provider_change_payloads(&db, changes);
        let payload = payloads.iter().find(|p| p.file_id == "1701-file").unwrap();
        let item = payload
            .item
            .as_ref()
            .expect("a modified row carries the full item payload");
        assert_eq!(
            item.parent_identifier, FP_ROOT_APPLE,
            "the change feed must present a top-level item as a child of the ROOT container — the Swift materialized-set filter keys off this parent"
        );
    }

    #[test]
    fn test_1701_change_feed_excludes_live_shared_with_me_rows_from_payloads() {
        // Codex P1 (PR #99 review): a shared root's contract is `SharedWithMe`
        // with a nil parent (engine_bridge shared-root ingestion), so the
        // change feed's FP_ROOT_APPLE fallback would parent its didUpdate
        // payload onto the ROOT container. The Swift materialized-set filter
        // treats a root parent as always materialized — and fails OPEN while
        // the set is unknown — so a shared file would surface directly beneath
        // "Beebeeb", breaking the 1701 ruling (shared content is webapp-only:
        // .claude/tasks/decisions/finder-sidebar-single-root.md). Live shared
        // rows must therefore ride the change feed with NO item payload (the
        // Swift enumerator skips payload-less rows, exactly like a row that
        // vanished); the change row itself still reports and advances the
        // anchor.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_shared_root(&db, "shared-live", "Shared with me/Docs", PERMISSION_READ);
        db.record_file_change("shared-live", crate::state_db::FpChangeKind::Modified, None)
            .unwrap();
        db.upsert_file(&FileEntry {
            file_id: "mine-live".into(),
            path: "/mine.txt".into(),
            status: FileStatus::Local,
            size_bytes: 3,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        db.record_file_change("mine-live", crate::state_db::FpChangeKind::Modified, None)
            .unwrap();

        let (changes, _) = db.list_file_changes_paged(None, 100).unwrap().unwrap();
        let payloads = file_provider_change_payloads(&db, changes);
        let shared = payloads.iter().find(|p| p.file_id == "shared-live").unwrap();
        assert!(
            shared.item.is_none(),
            "a live SharedWithMe row must not carry an item payload — its FP_ROOT_APPLE fallback would surface it beneath the root"
        );
        let mine = payloads.iter().find(|p| p.file_id == "mine-live").unwrap();
        assert!(mine.item.is_some(), "a live MyFiles row keeps its ride-along payload");
    }

    #[test]
    fn test_1701_shared_root_folder_payload_keeps_read_only_caps_with_add_subitems() {
        // Migrated from `test_shared_namespace_lists_roots_with_permission_
        // capabilities` (task 1694/1697): the `namespace:shared_with_me`
        // container is NO LONGER an enumeration surface (1701 ruling — shared
        // files live in the webapp), so the intent moves to the payload
        // builder itself: a shared root is a LIVE folder, and its capability
        // contract is unchanged should the system ever hold it.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_shared_root(&db, "read-root", "Shared with me/Read only", PERMISSION_READ);
        seed_shared_root(
            &db,
            "write-root",
            "Shared with me/Editable",
            PERMISSION_READ | PERMISSION_WRITE,
        );

        let read_only = db.get_file("read-root").unwrap().unwrap();
        let payload = file_entry_payload_for_db(&db, &read_only, FP_ROOT_APPLE);
        assert_eq!(payload.kind, "folder");
        // Task 1694 (lead rule): every LIVE folder accepts adding sub-items,
        // gated only on kind+status — NOT on permission_bits. A read-only
        // shared root is still a live folder, so it carries
        // READ | ADD_SUBITEMS (unauthorized writes are rejected at the write
        // path, not by hiding the folder's drop affordance).
        assert_eq!(payload.capabilities, CAP_READ | CAP_ADD_SUBITEMS);

        let editable = db.get_file("write-root").unwrap().unwrap();
        let payload = file_entry_payload_for_db(&db, &editable, FP_ROOT_APPLE);
        assert_eq!(
            payload.capabilities & (CAP_READ | CAP_WRITE | CAP_RENAME | CAP_DELETE),
            CAP_READ | CAP_WRITE | CAP_RENAME | CAP_DELETE
        );
    }

    #[test]
    fn test_is_authorized_peer_matches_only_same_uid() {
        // The real cross-UID rejection cannot be exercised end-to-end in a
        // sandboxed test runner (it needs root/setuid to connect as a different
        // real UID), so this pins the exact comparison the live peer-cred check
        // uses (task 1247).
        assert!(is_authorized_peer(1000, 1000));
        assert!(!is_authorized_peer(1000, 1001));
        assert!(!is_authorized_peer(0, 1000));
        assert!(is_authorized_peer(0, 0));
    }

    // ── Task 1694: Finder blocks adding files to folders — add-subitems
    // capability bit ─────────────────────────────────────────────────────────

    // Task 1701: the namespace-payload variant of this test
    // (`test_1694_namespace_payload_grants_add_subitems`) died WITH
    // `namespace_payload` — the synthetic containers are no longer built, and
    // dead code does not ship. The 1694 intent lives on in the folder payload
    // tests below and in the 1701 shared-root test above.

    #[test]
    fn test_1694_local_folder_payload_grants_add_subitems() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_1694_item(
            &db,
            "1694-local-folder",
            FileStatus::Local,
            PERMISSION_READ | PERMISSION_WRITE,
            ItemKind::Folder,
        );
        let entry = db.get_file("1694-local-folder").unwrap().unwrap();
        let payload = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);
        assert_eq!(payload.kind, "folder");
        assert!(
            payload.capabilities & CAP_ADD_SUBITEMS != 0,
            "a live Local folder must grant ADD_SUBITEMS (task 1694), got {:#b}",
            payload.capabilities
        );
    }

    #[test]
    fn test_1694_cloud_only_folder_payload_grants_add_subitems() {
        // The majority real-world case: CloudOnly entries dominate the tree
        // (Guus's vault: 8195 cloud_only files), so the fix is worthless
        // unless a CloudOnly folder also accepts drops.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_1694_item(
            &db,
            "1694-cloud-folder",
            FileStatus::CloudOnly,
            PERMISSION_READ | PERMISSION_WRITE,
            ItemKind::Folder,
        );
        let entry = db.get_file("1694-cloud-folder").unwrap().unwrap();
        let payload = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);
        assert_eq!(payload.kind, "folder");
        assert!(
            payload.capabilities & CAP_ADD_SUBITEMS != 0,
            "a live CloudOnly folder must grant ADD_SUBITEMS (task 1694), got {:#b}",
            payload.capabilities
        );
    }

    #[test]
    fn test_1694_trashing_folder_payload_does_not_grant_add_subitems() {
        // `Trashing` is the read-only terminal state (a locally-deleted item
        // whose server-trash is pending); a folder on its way out must NOT
        // accept new sub-items — in either payload branch.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_1694_item(
            &db,
            "1694-trashing-folder",
            FileStatus::Trashing,
            PERMISSION_READ | PERMISSION_WRITE,
            ItemKind::Folder,
        );
        let entry = db.get_file("1694-trashing-folder").unwrap().unwrap();
        let contract_payload = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);
        assert_eq!(contract_payload.kind, "folder");
        assert!(
            contract_payload.capabilities & CAP_ADD_SUBITEMS == 0,
            "a Trashing folder must NOT grant ADD_SUBITEMS, got {:#b}",
            contract_payload.capabilities
        );

        let no_contract_payload = file_entry_payload_without_contract(&entry, FP_ROOT_APPLE);
        assert!(
            no_contract_payload.capabilities & CAP_ADD_SUBITEMS == 0,
            "a Trashing folder must NOT grant ADD_SUBITEMS in the no-contract fallback either, got {:#b}",
            no_contract_payload.capabilities
        );
    }

    #[test]
    fn test_1694_file_payload_does_not_grant_add_subitems() {
        // Only folders accept sub-items; a file payload must never carry the
        // bit, whatever its status.
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_1694_item(
            &db,
            "1694-local-file",
            FileStatus::Local,
            PERMISSION_READ | PERMISSION_WRITE,
            ItemKind::File,
        );
        let entry = db.get_file("1694-local-file").unwrap().unwrap();
        let payload = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);
        assert_eq!(payload.kind, "file");
        assert!(
            payload.capabilities & CAP_ADD_SUBITEMS == 0,
            "a file payload must NOT grant ADD_SUBITEMS, got {:#b}",
            payload.capabilities
        );
    }

    #[test]
    fn test_1694_no_contract_folder_payload_grants_add_subitems_and_reports_folder_kind() {
        // The no-contract fallback used to hardcode kind "file". It must
        // derive the kind from FileEntry.item_kind (the files.item_kind
        // column) — otherwise a contract-less folder is mis-reported as a
        // file AND loses the folder ADD grant.
        let entry = FileEntry {
            file_id: "1694-nc-folder".into(),
            path: "My files/1694-nc-folder".into(),
            status: FileStatus::Local,
            size_bytes: 0,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::Folder,
        };
        let payload = file_entry_payload_without_contract(&entry, FP_ROOT_APPLE);
        assert_eq!(
            payload.kind, "folder",
            "no-contract fallback must derive kind from FileEntry.item_kind, not hardcode \"file\""
        );
        assert!(
            payload.capabilities & CAP_ADD_SUBITEMS != 0,
            "a no-contract live folder must grant ADD_SUBITEMS, got {:#b}",
            payload.capabilities
        );
    }

    #[test]
    fn test_1694_no_contract_file_payload_does_not_grant_add_subitems() {
        let entry = FileEntry {
            file_id: "1694-nc-file".into(),
            path: "My files/1694-nc-file.txt".into(),
            status: FileStatus::Local,
            size_bytes: 0,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        };
        let payload = file_entry_payload_without_contract(&entry, FP_ROOT_APPLE);
        assert_eq!(payload.kind, "file");
        assert!(
            payload.capabilities & CAP_ADD_SUBITEMS == 0,
            "a no-contract file payload must NOT grant ADD_SUBITEMS, got {:#b}",
            payload.capabilities
        );
    }

    fn seed_shared_root(db: &StateDb, file_id: &str, path: &str, permissions: i64) {
        db.upsert_file(&FileEntry {
            file_id: file_id.into(),
            path: path.into(),
            status: FileStatus::Local,
            size_bytes: 0,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        let mut contract = db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.namespace = Namespace::SharedWithMe;
        contract.shared_root_id = Some(file_id.into());
        contract.share_id = Some(format!("invite-{file_id}"));
        contract.permission_bits = permissions;
        contract.item_kind = ItemKind::Folder;
        contract.content_type = Some("public.folder".into());
        db.set_file_contract_state(&contract).unwrap();
    }
