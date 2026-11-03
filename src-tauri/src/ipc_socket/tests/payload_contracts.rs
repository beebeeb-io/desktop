//! 1697/1698 item payload, reparent/trash and contract tests.

use super::*;


    // ------------------------------------------------------------------
    // Task 1697: capabilities completion + item metadata. RED-first: these
    // were seen failing before the CAP_REPARENT/CAP_TRASH bits and the
    // dates/childItemCount/version-split fields existed.
    // ------------------------------------------------------------------

    fn live_file_entry(status: FileStatus) -> FileEntry {
        FileEntry {
            file_id: "1697-item".into(),
            path: "/docs/report.txt".into(),
            status,
            size_bytes: 120,
            modified_at: 1_700_000_100,
            content_hash: None,
            remote_updated_at: 1_700_000_200,
            parent_id: None,
            item_kind: ItemKind::File,
        }
    }

    fn live_contract() -> crate::state_db::FileContractState {
        crate::state_db::FileContractState {
            file_id: "1697-item".into(),
            namespace: crate::state_db::Namespace::MyFiles,
            parent_id: None,
            shared_root_id: None,
            share_id: None,
            owner_email: None,
            permission_bits: 0,
            item_kind: crate::state_db::ItemKind::File,
            content_type: None,
            current_version: 7,
            current_object_version_id: None,
            local_base_version: 0,
            local_hash: None,
            cache_path: None,
            cache_bytes: 0,
            pin_state: crate::state_db::PinState::Inherit,
            inherited_pin_state: crate::state_db::PinState::Unpinned,
            last_sync_at: 0,
        }
    }

    #[test]
    fn test_1697_local_item_payload_grants_reparent_and_trash() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let entry = live_file_entry(FileStatus::Local);
        db.upsert_file(&entry).unwrap();
        db.set_file_contract_state(&live_contract()).unwrap();
        let payload = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);
        assert_eq!(
            payload.capabilities & CAP_REPARENT,
            CAP_REPARENT,
            "a live item must advertise reparenting (drag-move), got {:#b}",
            payload.capabilities
        );
        assert_eq!(
            payload.capabilities & CAP_TRASH,
            CAP_TRASH,
            "a live item must advertise trashing, got {:#b}",
            payload.capabilities
        );
    }

    #[test]
    fn test_1697_cloud_only_item_keeps_reparent_and_trash_unset() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let entry = live_file_entry(FileStatus::CloudOnly);
        db.upsert_file(&entry).unwrap();
        db.set_file_contract_state(&live_contract()).unwrap();
        let payload = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);
        assert_eq!(
            payload.capabilities & CAP_REPARENT,
            0,
            "read-only terminal states stay read-only"
        );
        assert_eq!(
            payload.capabilities & CAP_TRASH,
            0,
            "read-only terminal states stay read-only"
        );
    }

    #[test]
    fn test_1697_trashing_item_keeps_reparent_and_trash_unset() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let entry = live_file_entry(FileStatus::Trashing);
        db.upsert_file(&entry).unwrap();
        db.set_file_contract_state(&live_contract()).unwrap();
        let payload = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);
        assert_eq!(
            payload.capabilities & (CAP_REPARENT | CAP_TRASH),
            0,
            "an item on its way out cannot be moved or trashed again"
        );
    }

    #[test]
    fn test_1697_payload_carries_dates_and_folder_child_count() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.upsert_file(&FileEntry {
            file_id: "folder-1697".into(),
            path: "/docs".into(),
            status: FileStatus::Local,
            size_bytes: 0,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::Folder,
        })
        .unwrap();
        // item_kind is contract-owned (upsert_file never writes it) — the
        // contract write is what makes this row a folder.
        db.set_file_contract_state(&crate::state_db::FileContractState {
            file_id: "folder-1697".into(),
            item_kind: ItemKind::Folder,
            ..live_contract()
        })
        .unwrap();
        for (id, path) in [("c-1", "/docs/a.txt"), ("c-2", "/docs/b.txt")] {
            db.upsert_file(&FileEntry {
                file_id: id.into(),
                path: path.into(),
                status: FileStatus::CloudOnly,
                size_bytes: 1,
                modified_at: 1,
                content_hash: None,
                remote_updated_at: 1,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
            db.set_file_contract_state(&crate::state_db::FileContractState {
                file_id: id.into(),
                parent_id: Some("folder-1697".into()),
                ..live_contract()
            })
            .unwrap();
        }
        let folder = db.get_file("folder-1697").unwrap().unwrap();
        let payload = file_entry_payload_for_db(&db, &folder, FP_ROOT_APPLE);
        assert_eq!(
            payload.child_item_count,
            Some(2),
            "childItemCount must be the real child count, was hardcoded 0 before 1697"
        );
        assert!(payload.modified_at.is_some(), "modification date must be populated");
        assert!(payload.created_at.is_some(), "creation date must be populated");
        let file = db.get_file("c-1").unwrap().unwrap();
        let file_payload = file_entry_payload_for_db(&db, &file, "folder-1697");
        assert_eq!(file_payload.child_item_count, None, "files have no child count");
    }

    #[test]
    fn test_1697_version_split_content_vs_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let mut entry = live_file_entry(FileStatus::Local);
        entry.content_hash = Some("abc123".into());
        db.upsert_file(&entry).unwrap();
        db.set_file_contract_state(&live_contract()).unwrap();

        let before = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);
        // A rename: metadata changes, content does not.
        let mut renamed = entry.clone();
        renamed.path = "/docs/report-renamed.txt".into();
        db.upsert_file(&renamed).unwrap();
        let after = file_entry_payload_for_db(&db, &renamed, FP_ROOT_APPLE);
        assert_eq!(
            after.content_version, before.content_version,
            "a rename must not change the content version (it would force a re-download)"
        );
        assert_ne!(
            after.metadata_version, before.metadata_version,
            "a rename must change the metadata version"
        );
        assert_ne!(
            before.content_version, before.metadata_version,
            "the two versions are distinct identities"
        );
    }

    /// A row as a completed desktop upload leaves it: server version 1, and
    /// `remote_updated_at` / `modified_at` set to the wall-clock second of
    /// the upload.
    fn uploaded_row() -> (FileEntry, crate::state_db::FileContractState) {
        let entry = FileEntry {
            file_id: "uploaded-item".into(),
            path: "/t.txt".into(),
            status: FileStatus::Local,
            size_bytes: 28,
            modified_at: 1_791_550_250,
            content_hash: None,
            remote_updated_at: 1_791_550_250,
            parent_id: None,
            item_kind: ItemKind::File,
        };
        let contract = crate::state_db::FileContractState {
            file_id: "uploaded-item".into(),
            current_version: 1,
            ..live_contract()
        };
        (entry, contract)
    }

    #[test]
    fn version_identifiers_lead_with_the_server_version_after_an_upload() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let (entry, contract) = uploaded_row();
        db.upsert_file(&entry).unwrap();
        db.set_file_contract_state(&contract).unwrap();

        let payload = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);
        assert_eq!(
            payload.content_version.as_deref(),
            Some("1"),
            "the content version (the extension's write base) must be the server version"
        );
        assert_eq!(payload.version_identifier.as_deref(), Some("1:1791550250:28"));
        assert_eq!(
            crate::engine_bridge::parse_base_version_number(payload.content_version.as_deref()),
            Some(1)
        );
    }

    #[test]
    fn an_op_echo_leaves_the_content_version_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let (entry, contract) = uploaded_row();
        for hash in [None, Some("abc123".to_string())] {
            let mut entry = entry.clone();
            entry.content_hash = hash.clone();
            db.upsert_file(&entry).unwrap();
            db.set_file_contract_state(&contract).unwrap();
            let before = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);

            // The `/sync/ops` echo of the same upload: a later wall-clock
            // second, the same server version.
            let mut echoed = entry.clone();
            echoed.remote_updated_at += 30;
            echoed.modified_at = echoed.remote_updated_at;
            db.upsert_file(&echoed).unwrap();
            let after = file_entry_payload_for_db(&db, &echoed, FP_ROOT_APPLE);
            assert_eq!(
                after.content_version, before.content_version,
                "an op echo is not a content change (hash {hash:?})"
            );
        }
    }

    #[test]
    fn the_payload_without_a_contract_carries_no_base_version() {
        let (entry, _) = uploaded_row();
        let payload = file_entry_payload_without_contract(&entry, FP_ROOT_APPLE);
        assert_eq!(
            crate::engine_bridge::parse_base_version_number(payload.content_version.as_deref()),
            None,
            "no contract means no known server version: {:?}",
            payload.content_version
        );
        assert_eq!(
            crate::engine_bridge::parse_base_version_number(payload.version_identifier.as_deref()),
            None,
            "{:?}",
            payload.version_identifier
        );
    }

    // ── Task 1698 part 1: contentPolicy pin mapping ─────────────────────────

    #[test]
    fn test_1698_pinned_payload_carries_effective_pin_state() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        // Own pin: the row's own pin_state = Pinned.
        let entry = live_file_entry(FileStatus::Local);
        db.upsert_file(&entry).unwrap();
        let mut contract = live_contract();
        contract.pin_state = crate::state_db::PinState::Pinned;
        db.set_file_contract_state(&contract).unwrap();
        let payload = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);
        assert!(
            payload.pinned,
            "a row whose OWN pin_state is Pinned must report pinned=true"
        );

        // Inherited pin: own state Inherit + inherited_pin_state = Pinned.
        let child = FileEntry {
            file_id: "1698-child".into(),
            path: "/docs/report.txt".into(),
            status: FileStatus::CloudOnly,
            size_bytes: 10,
            modified_at: 1_700_000_100,
            content_hash: None,
            remote_updated_at: 1_700_000_200,
            parent_id: Some("1697-item".into()),
            item_kind: ItemKind::File,
        };
        db.upsert_file(&child).unwrap();
        let mut child_contract = live_contract();
        child_contract.file_id = "1698-child".into();
        child_contract.parent_id = Some("1697-item".into());
        child_contract.pin_state = crate::state_db::PinState::Inherit;
        child_contract.inherited_pin_state = crate::state_db::PinState::Pinned;
        db.set_file_contract_state(&child_contract).unwrap();
        let child_payload = file_entry_payload_for_db(&db, &child, FP_ROOT_APPLE);
        assert!(
            child_payload.pinned,
            "the EFFECTIVE pin state (inherit resolution) must drive pinned=true"
        );

        // Unpinned: own Inherit + inherited Unpinned (the default).
        let mut unpinned_contract = live_contract();
        unpinned_contract.pin_state = crate::state_db::PinState::Inherit;
        unpinned_contract.inherited_pin_state = crate::state_db::PinState::Unpinned;
        db.set_file_contract_state(&unpinned_contract).unwrap();
        let unpinned_payload = file_entry_payload_for_db(&db, &entry, FP_ROOT_APPLE);
        assert!(!unpinned_payload.pinned, "an unpinned row must report pinned=false");
    }

    // ── Task 1698 part 2: trash container payloads ───────────────────────────

    #[test]
    fn test_1698_trash_container_enumerates_trashing_items_with_trash_parent() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        // A locally-trashed item parked in `Trashing` (the trash-view row).
        let mut entry = live_file_entry(FileStatus::Trashing);
        entry.path = "/doomed.txt".into();
        db.upsert_file(&entry).unwrap();
        db.set_file_contract_state(&live_contract()).unwrap();

        // The trash container lists it, parented at the trash container.
        let items = list_file_provider_items(&db, FP_TRASH_APPLE);
        assert_eq!(
            items.len(),
            1,
            "the trash container must enumerate the Trashing row: {:?}",
            items.iter().map(|i| i.filename.clone()).collect::<Vec<_>>()
        );
        assert_eq!(
            items[0].parent_identifier, FP_TRASH_APPLE,
            "a trash-container child must be parented at the trash container"
        );

        // The ROOT must NOT surface it (it moved out of "Beebeeb").
        let root_items = list_file_provider_items(&db, FP_ROOT_APPLE);
        assert!(
            !root_items.iter().any(|i| i.identifier == "1697-item"),
            "a Trashing item must not be enumerated under the root"
        );
    }

    #[test]
    fn test_1698_children_of_a_trashed_folder_stay_inside_the_folder() {
        // The ruling: the trash view mirrors the server trash. A trashed
        // FOLDER's children (also trashed server-side by the cascade) must
        // still enumerate INSIDE the folder (their parent row is Trashing),
        // not as siblings of the folder at the trash root.
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let mut folder = live_file_entry(FileStatus::Trashing);
        folder.file_id = "1698-folder".into();
        folder.path = "/Trashed-Project".into();
        folder.item_kind = ItemKind::Folder;
        db.upsert_file(&folder).unwrap();
        let mut folder_contract = live_contract();
        folder_contract.file_id = "1698-folder".into();
        folder_contract.item_kind = ItemKind::Folder;
        db.set_file_contract_state(&folder_contract).unwrap();

        let mut child = live_file_entry(FileStatus::Trashing);
        child.file_id = "1698-child".into();
        child.path = "/Trashed-Project/leaf.txt".into();
        db.upsert_file(&child).unwrap();
        let mut child_contract = live_contract();
        child_contract.file_id = "1698-child".into();
        child_contract.parent_id = Some("1698-folder".into());
        db.set_file_contract_state(&child_contract).unwrap();

        // Top-of-trash: only the FOLDER (the child's parent row is Trashing).
        let items = list_file_provider_items(&db, FP_TRASH_APPLE);
        assert_eq!(
            items.iter().map(|i| i.identifier.clone()).collect::<Vec<_>>(),
            vec!["1698-folder"],
            "only the top-of-trash folder is a direct trash-container child, got {:?}",
            items.iter().map(|i| i.identifier.clone()).collect::<Vec<_>>()
        );
        // The folder's own enumeration (the system enumerates the trashed
        // folder's container) still lists the child.
        let children = list_file_provider_items(&db, "1698-folder");
        assert_eq!(
            children.len(),
            1,
            "a trashed folder's children must still enumerate inside it"
        );
        assert_eq!(
            children[0].parent_identifier, "1698-folder",
            "the child stays parented inside the trashed folder"
        );
    }

    #[test]
    fn test_1698_live_container_enumeration_excludes_trashing_children() {
        // A LIVE folder must not list its just-trashed child (the item moved
        // to the trash container); the trash container lists it instead.
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let mut folder = live_file_entry(FileStatus::Local);
        folder.file_id = "1698-folder".into();
        folder.path = "/Projects".into();
        folder.item_kind = ItemKind::Folder;
        db.upsert_file(&folder).unwrap();
        let mut folder_contract = live_contract();
        folder_contract.file_id = "1698-folder".into();
        folder_contract.item_kind = ItemKind::Folder;
        db.set_file_contract_state(&folder_contract).unwrap();

        let mut child = live_file_entry(FileStatus::Trashing);
        child.file_id = "1698-child".into();
        child.path = "/Projects/leaf.txt".into();
        db.upsert_file(&child).unwrap();
        let mut child_contract = live_contract();
        child_contract.file_id = "1698-child".into();
        child_contract.parent_id = Some("1698-folder".into());
        db.set_file_contract_state(&child_contract).unwrap();

        let children = list_file_provider_items(&db, "1698-folder");
        assert!(
            children.is_empty(),
            "a LIVE folder must not enumerate its Trashing children, got {:?}",
            children.iter().map(|i| i.identifier.clone()).collect::<Vec<_>>()
        );
        let items = list_file_provider_items(&db, FP_TRASH_APPLE);
        assert_eq!(items.len(), 1, "the trashed child is enumerated in the trash container");
        assert_eq!(items[0].parent_identifier, FP_TRASH_APPLE);
    }

    #[test]
    fn test_1698_change_payload_maps_trashing_status_to_trash_container() {
        // Ruling step 3: the change feed presents a Trashing item as a child
        // of the trash container, so the working set moves it into macOS
        // Trash without a content re-download.
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let entry = live_file_entry(FileStatus::Local);
        db.upsert_file(&entry).unwrap();
        db.set_file_contract_state(&live_contract()).unwrap();
        // The trash: a status flip records a `Modified` change (set_status).
        db.set_status("1697-item", FileStatus::Trashing).unwrap();

        let changes = db.list_file_changes_paged(None, 100).unwrap().unwrap().0;
        let payloads = file_provider_change_payloads(&db, changes);
        // The LAST change is the status flip (set_status); its payload must
        // carry the Trashing status under the trash container.
        let payload = payloads
            .last()
            .and_then(|change| change.item.as_ref())
            .expect("the status-flip change carries the item");
        assert_eq!(
            payload.status, "trashing",
            "the flip's payload carries the Trashing status"
        );
        assert_eq!(
            payload.parent_identifier, FP_TRASH_APPLE,
            "the change feed must present a Trashing item under the trash container"
        );
        assert_eq!(payload.identifier, "1697-item");
    }

    #[test]
    fn review_p2_6_a_foreign_item_is_marked_added_by_someone_else() {
        let entry = |file_id: &str| FileEntry {
            file_id: file_id.into(),
            path: "/added.txt".into(),
            status: FileStatus::Local,
            size_bytes: 0,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        };
        let foreign = entry("20000000-0000-8000-8000-0000000000e1");
        let owner = entry("40000000-0000-4000-8000-0000000000e2");
        assert!(
            file_entry_payload_without_contract(&foreign, FP_ROOT_APPLE).added_by_someone_else,
            "an F id says the item was added by someone else"
        );
        assert!(
            !file_entry_payload_without_contract(&owner, FP_ROOT_APPLE).added_by_someone_else,
            "an owner id does not"
        );
    }
