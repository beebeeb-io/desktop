//! Task 2413: a server row is stored only when every segment of its path is one name this host can hold, own and
//! shared rows alike; a refused row leaves a stored one exactly as it was, is never pruned for it, and never puts the
//! rows of its folder at the top level. And `placeholder_path_under`, the one map from a stored path onto the sync
//! root, takes plain Windows names only, split on both separators, contained by component. Fixture rows only; the
//! Windows rules run on any host through `HostRules`. Native Windows: `stored_path_2413_windows`.

use super::*;
use crate::stored_path::HostRules;

const MK: [u8; 32] = [7u8; 32];

/// A server row for `id` whose name resolves to `name`: a plaintext `name_encrypted`, which core reads back as it is
/// (its legacy plaintext format), or the plaintext `name` field a row with no `name_encrypted` falls back to.
fn row(id: &str, name: &str, via_name_field: bool, is_folder: bool) -> serde_json::Value {
    let mut row = serde_json::json!({ "id": id, "size_bytes": 10, "updated_at": 50, "is_folder": is_folder });
    row[if via_name_field { "name" } else { "name_encrypted" }] = serde_json::json!(name);
    row
}

fn apply_own(db: &StateDb, row: &serde_json::Value, parent: &str) -> Option<FileEntry> {
    apply_metadata_file_row(
        db,
        row,
        Namespace::MyFiles,
        None,
        None,
        PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
        100,
        &MK,
        parent,
    )
    .unwrap()
}

/// What a refused update must leave as it was.
fn fields(entry: &FileEntry) -> (String, String, i64, i64, i64, Option<String>, Option<String>) {
    (
        entry.path.clone(),
        entry.status.as_str().to_string(),
        entry.size_bytes,
        entry.modified_at,
        entry.remote_updated_at,
        entry.content_hash.clone(),
        entry.parent_id.clone(),
    )
}

/// Every class the rule covers, with the Windows rules on.
const REFUSED_WITH_WINDOWS_RULES: &[&str] = &[
    "..",
    ".",
    "/top/level.txt",
    "C:\\x.txt",
    "C:x.txt",
    "\\\\server\\share\\x.txt",
    "a/b.txt",
    "a\\b.txt",
    "..\\..\\x.txt",
    "../../x.txt",
    "CON",
    "nul.txt",
    "COM1.txt",
    "LPT9.log",
    "a:b.txt",
    "report.txt:stream",
    "trailing.",
    "trailing ",
];

#[test]
fn task_2413_an_own_row_whose_name_is_not_one_plain_name_is_never_stored() {
    let _rules = HostRules::windows(true);
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let (mut refused, mut stored) = (0, Vec::new());
    for (n, name) in REFUSED_WITH_WINDOWS_RULES.iter().enumerate() {
        for (m, via_name_field) in [false, true].into_iter().enumerate() {
            for parent in ["", "docs"] {
                let id = format!("a0000000-0000-4000-8000-{n:06}{m:02}{:04}", parent.len());
                let entry = apply_own(&db, &row(&id, name, via_name_field, false), parent);
                match (entry, db.get_file(&id).unwrap()) {
                    (None, None) => refused += 1,
                    (_, row) => stored.push(format!("{name:?} under {parent:?} -> {:?}", row.map(|row| row.path))),
                }
            }
        }
    }
    assert!(stored.is_empty(), "{} of {} stored:\n{}", stored.len(), 18 * 4, stored.join("\n"));
    assert_eq!(refused, 18 * 4);
    // A plain name under the same rules is stored as before.
    let entry = apply_own(&db, &row("a0000000-0000-4000-8000-0000000000ff", "Report 2026.pdf", false, false), "docs").unwrap();
    assert_eq!(entry.path, "docs/Report 2026.pdf");
}

/// Without the Windows rules (Linux, macOS) the names that are ordinary file names there are kept as they are, so
/// they keep round-tripping; the names that are not one name anywhere are refused.
#[test]
fn task_2413_without_the_windows_rules_only_what_is_not_one_name_anywhere_is_refused() {
    let _rules = HostRules::windows(false);
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let mut stored = Vec::new();
    for (n, name) in REFUSED_WITH_WINDOWS_RULES.iter().enumerate() {
        let id = format!("b0000000-0000-4000-8000-{n:012}");
        if let Some(entry) = apply_own(&db, &row(&id, name, false, false), "") {
            assert_eq!(entry.path, *name, "kept as it is");
            stored.push(*name);
        } else {
            assert!(db.get_file(&id).unwrap().is_none(), "{name:?}");
        }
    }
    assert_eq!(
        stored,
        [
            "C:\\x.txt", "C:x.txt", "\\\\server\\share\\x.txt", "a\\b.txt", "..\\..\\x.txt", "CON", "nul.txt",
            "COM1.txt", "LPT9.log", "a:b.txt", "report.txt:stream", "trailing.", "trailing "
        ]
    );
}

#[test]
fn task_2413_a_parent_path_with_a_segment_that_is_not_plain_refuses_the_row() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    {
        let _rules = HostRules::windows(false);
        for (n, parent) in ["docs/../..", "../up", "docs//x"].iter().enumerate() {
            let id = format!("c0000000-0000-4000-8000-{n:012}");
            assert!(apply_own(&db, &row(&id, "a.txt", false, false), parent).is_none(), "{parent:?}");
            assert!(db.get_file(&id).unwrap().is_none());
        }
    }
    let _rules = HostRules::windows(true);
    for (n, parent) in ["a:b", "docs/CON", "docs\\..\\..", "docs."].iter().enumerate() {
        let id = format!("c1000000-0000-4000-8000-{n:012}");
        assert!(apply_own(&db, &row(&id, "a.txt", false, false), parent).is_none(), "{parent:?}");
        assert!(db.get_file(&id).unwrap().is_none());
    }
}

/// A refused update leaves the stored row exactly as it was (path, status, size, times, version), and the snapshot
/// neither moves nor prunes it.
#[test]
fn task_2413_a_refused_update_leaves_the_stored_row_as_it_was_and_unpruned() {
    let _rules = HostRules::windows(true);
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    let local = "d0000000-0000-4000-8000-000000000001";
    let cloud = "d0000000-0000-4000-8000-000000000002";
    seed_bridge_entry(&bridge, local, "docs/report.txt", None, FileStatus::Local, false, 10);
    seed_bridge_entry(&bridge, cloud, "plan.txt", None, FileStatus::CloudOnly, false, 10);
    let before: Vec<_> = [local, cloud]
        .iter()
        .map(|id| (bridge.db().get_file(id).unwrap().unwrap(), bridge.db().get_file_contract_state(id).unwrap().unwrap()))
        .collect();
    let mut renamed_local = row(local, "CON", false, false);
    renamed_local["size_bytes"] = serde_json::json!(999);
    renamed_local["version_number"] = serde_json::json!(7);
    let snapshot = crate::api_client::SyncSnapshot {
        seq_id: 9,
        nodes: vec![renamed_local, row(cloud, "plan.txt:stream", false, false)],
    };
    apply_snapshot(&bridge, dir.path(), &snapshot, 200, 200, &mut Vec::new()).unwrap();
    for (entry, contract) in before {
        let id = entry.file_id.clone();
        let now = bridge.db().get_file(&id).unwrap().unwrap_or_else(|| panic!("{id}: not pruned"));
        assert_eq!(fields(&now), fields(&entry), "{id}: the row as it was");
        let after = bridge.db().get_file_contract_state(&id).unwrap().unwrap();
        assert_eq!(after.current_version, contract.current_version, "{id}: version");
        assert_eq!(after.parent_id, contract.parent_id, "{id}: parent");
    }
}

/// The rows of a folder the store refused are not placed either: not under it, and never at the top level. A child
/// created by an op under it waits for the snapshot instead of landing at the top level.
#[test]
fn task_2413_rows_under_a_refused_new_folder_are_never_placed_at_the_top_level() {
    let _rules = HostRules::windows(true);
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    let folder = "e0000000-0000-4000-8000-000000000001";
    let child = "e0000000-0000-4000-8000-000000000002";
    let grandchild = "e0000000-0000-4000-8000-000000000003";
    let mut child_row = row(child, "inner", false, true);
    child_row["parent_id"] = serde_json::json!(folder);
    let mut grandchild_row = row(grandchild, "a.txt", false, false);
    grandchild_row["parent_id"] = serde_json::json!(child);
    let snapshot = crate::api_client::SyncSnapshot {
        seq_id: 9,
        nodes: vec![grandchild_row, child_row, row(folder, "CON", false, true)],
    };
    apply_snapshot(&bridge, dir.path(), &snapshot, 200, 200, &mut Vec::new()).unwrap();
    for id in [folder, child, grandchild] {
        assert!(bridge.db().get_file(id).unwrap().is_none(), "{id} is not placed");
    }
    assert!(bridge.db().list_files().unwrap().is_empty(), "nothing at the top level either");

    // The op stream: the folder's create is refused, so the child's create names a parent this device does not hold.
    let create = |seq_id: i64, op_type: &str, payload: serde_json::Value| crate::api_client::SyncOp {
        seq_id,
        op_type: op_type.into(),
        payload,
    };
    let applied = apply_sync_op(
        &bridge,
        dir.path(),
        &create(10, "folder_create", serde_json::json!({ "id": folder, "name_encrypted": "CON" })),
        200,
        &mut Vec::new(),
    )
    .unwrap();
    assert!(applied.is_empty(), "{applied:?}");
    assert_eq!(bridge.db().peek_resnapshot_request().unwrap(), None);
    apply_sync_op(
        &bridge,
        dir.path(),
        &create(11, "file_create", serde_json::json!({ "id": grandchild, "parent_id": folder, "name_encrypted": "a.txt" })),
        200,
        &mut Vec::new(),
    )
    .unwrap();
    assert!(bridge.db().list_files().unwrap().is_empty(), "the child is not placed at the top level");
    assert!(bridge.db().peek_resnapshot_request().unwrap().is_some(), "the snapshot will place it");
}

/// The same name rule for a shared row: a decrypted name that is not one plain name shows as the placeholder name,
/// as an undecryptable one does, and is checked as it is (never trimmed into another name).
#[test]
fn task_2413_a_shared_name_meets_the_same_rule_as_an_own_name() {
    let _rules = HostRules::windows(true);
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let root = SharedRootMapping {
        invite_id: "invite-folder".into(),
        file_id: "929317e7-4e2f-583b-895f-8f0152b1530d".into(),
        display_name: "Shared folder".into(),
        is_folder: true,
        size_bytes: 0,
        content_type: None,
        owner_email: Some("owner@example.com".into()),
        sender_public_key: None,
        encrypted_file_key: None,
        encrypted_folder_key: None,
        file_name_encrypted: None,
        permission_bits: PERMISSION_READ,
        approved_at: None,
    };
    let file_key = beebeeb_core::kdf::FileKey::from_bytes([0x66u8; 32]);
    let sealed = |name: &str| {
        let metadata = serde_json::json!({ "name": name, "mime_type": "text/plain" }).to_string();
        serde_json::to_string(&beebeeb_core::encrypt::encrypt_metadata(&file_key, &metadata).unwrap()).unwrap()
    };
    let names = ["CON", "nul.txt", "report:v2.txt", "notes.", "notes.txt ", "a\\b.txt", "..", "a/b"];
    let mut shown = Vec::new();
    for (n, name) in names.iter().enumerate() {
        let id = format!("f0000000-0000-4000-8000-{n:012}");
        let entry = apply_shared_metadata_file_row(
            &db,
            &serde_json::json!({ "id": id, "name_encrypted": sealed(name), "size_bytes": 12, "is_folder": false }),
            &root,
            1_700_000_000,
            Some(root.file_id.clone()),
            "Shared with me/Shared folder",
            Some(&file_key),
        )
        .unwrap()
        .unwrap();
        if entry.path != "Shared with me/Shared folder/Encrypted file" {
            shown.push(format!("{name:?} -> {:?}", entry.path));
        }
    }
    assert!(shown.is_empty(), "{} of {} shown as another name:\n{}", shown.len(), names.len(), shown.join("\n"));
    // Under a parent with a segment that is not plain, nothing is stored.
    let entry = apply_shared_metadata_file_row(
        &db,
        &serde_json::json!({ "id": "f1000000-0000-4000-8000-000000000001", "name_encrypted": sealed("a.txt"), "is_folder": false }),
        &root,
        1_700_000_000,
        Some(root.file_id.clone()),
        "Shared with me/a:b",
        Some(&file_key),
    )
    .unwrap();
    assert!(entry.is_none());
    assert!(db.get_file("f1000000-0000-4000-8000-000000000001").unwrap().is_none());
}

/// `placeholder_path_under` applies the Windows rules on every host: split on both separators, every segment one plain
/// name, the result the root and exactly one plain component per segment.
#[test]
fn task_2413_placeholder_path_under_takes_plain_windows_names_only() {
    let root = Path::new("/sync/Beebeeb");
    let refused = [
        "",
        "/",
        "../up.txt",
        "docs/../../up.txt",
        "docs/./report.txt",
        "docs//report.txt",
        "docs\\..\\..\\outside.txt",
        "..\\outside.txt",
        "docs\\.\\report.txt",
        "C:\\Windows\\x.txt",
        "C:x.txt",
        "\\\\server\\share\\x.txt",
        "\\top.txt",
        "CON",
        "docs/nul.txt",
        "docs/COM1.txt",
        "a:b.txt",
        "docs/report.txt:stream",
        "docs/trailing.",
        "docs/trailing ",
        "trailing./x.txt",
        "star*.txt",
    ];
    let mapped: Vec<String> = refused
        .iter()
        .filter_map(|rel| EngineBridge::placeholder_path_under(root, rel).map(|path| format!("{rel:?} -> {}", path.display())))
        .collect();
    assert!(mapped.is_empty(), "{} of {} mapped:\n{}", mapped.len(), refused.len(), mapped.join("\n"));
    assert_eq!(refused.len(), 22);
    for (rel, segments) in [
        ("docs/report.txt", ["docs", "report.txt"].as_slice()),
        ("/docs/report.txt", ["docs", "report.txt"].as_slice()),
        ("docs\\report.txt", ["docs", "report.txt"].as_slice()),
        ("Notes 2026/élan.txt", ["Notes 2026", "élan.txt"].as_slice()),
        ("a.b.c", ["a.b.c"].as_slice()),
    ] {
        let path = EngineBridge::placeholder_path_under(root, rel).unwrap_or_else(|| panic!("{rel:?}"));
        let below: Vec<_> = path.strip_prefix(root).unwrap().components().collect();
        assert_eq!(below.len(), segments.len(), "{rel:?}");
        for (component, segment) in below.iter().zip(segments) {
            assert_eq!(component, &std::path::Component::Normal(std::ffi::OsStr::new(segment)), "{rel:?}");
        }
    }
}
