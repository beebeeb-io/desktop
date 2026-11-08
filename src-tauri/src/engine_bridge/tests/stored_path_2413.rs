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
    assert!(stored.is_empty(), "{} of {} stored:\n{}", stored.len(), 17 * 4, stored.join("\n"));
    assert_eq!(refused, 17 * 4);
    // A plain name under the same rules is stored as before, and so is a long CJK name Windows holds (300 bytes,
    // 100 UTF-16 units).
    let entry = apply_own(&db, &row("a0000000-0000-4000-8000-0000000000ff", "Report 2026.pdf", false, false), "docs").unwrap();
    assert_eq!(entry.path, "docs/Report 2026.pdf");
    let long = "漢".repeat(100);
    let entry = apply_own(&db, &row("a0000000-0000-4000-8000-0000000000fe", &long, false, false), "docs").unwrap();
    assert_eq!(entry.path, format!("docs/{long}"));
    // A decrypted name is trimmed as before (a row stored under the trimmed name keeps its path); a plaintext name
    // field is taken as it is, so its trailing space is refused.
    let entry = apply_own(&db, &row("a0000000-0000-4000-8000-0000000000fd", "trailing ", false, false), "docs").unwrap();
    assert_eq!(entry.path, "docs/trailing");
    assert!(apply_own(&db, &row("a0000000-0000-4000-8000-0000000000fc", "trailing ", true, false), "docs").is_none());
}

/// Windows and macOS hold a name of up to 255 UTF-16 units, however many bytes it takes: a CJK title broken by spaces
/// or by `、`, 80 emoji, exactly 255 units. Such a row is stored, and its path maps onto the sync folder; 256 units is
/// refused. Linux holds 255 bytes, so there only the Windows rules store them.
#[test]
fn task_2413_a_long_name_the_host_holds_is_stored() {
    let held = [
        format!("{}報告", "第三四半期 計画 ".repeat(11)), // 101 UTF-16 units, 259 bytes
        "東京、大阪、".repeat(17),                       // 102 units, 306 bytes
        "😀".repeat(80),                                 // 160 units, 320 bytes
        format!("{}a", "😀".repeat(127)),                // 255 units, 509 bytes
        "漢".repeat(255),                                // 255 units, 765 bytes
    ];
    let too_long = ["😀".repeat(128), format!("{}a", "漢".repeat(255))]; // 256 units each
    let root = tempfile::tempdir().unwrap();
    let (mut checked, mut wrong) = (0, Vec::new());
    for windows in [true, false] {
        let _rules = HostRules::windows(windows);
        // Without the Windows rules the build's own length applies: macOS counts UTF-16 units too, Linux bytes.
        let holds = windows || cfg!(target_os = "macos");
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        for (n, name) in held.iter().chain(&too_long).enumerate() {
            let expected = holds && n < held.len();
            let id = format!("a2000000-0000-4000-8000-{:02}{n:010}", u8::from(windows));
            let stored = apply_own(&db, &row(&id, name, false, false), "docs").map(|entry| entry.path);
            let mapped = crate::engine_bridge::local_file_path_under_sync_root(root.path(), &format!("docs/{name}")).is_ok();
            if stored != expected.then(|| format!("docs/{name}")) || db.get_file(&id).unwrap().is_some() != expected {
                wrong.push(format!("stored {:?}", (name.encode_utf16().count(), name.len(), windows)));
            }
            if n < held.len() && mapped != holds {
                wrong.push(format!("onto the sync folder {:?}", (name.encode_utf16().count(), name.len(), windows)));
            }
            checked += 1;
        }
    }
    assert!(wrong.is_empty(), "wrong for (UTF-16 units, bytes, Windows rules):\n{}", wrong.join("\n"));
    assert_eq!(checked, 2 * (5 + 2));
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
            "COM1.txt", "LPT9.log", "a:b.txt", "report.txt:stream", "trailing."
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
        let after = bridge.db().get_file_contract_state(&id).unwrap().unwrap();
        // macOS: the snapshot's version fill and raise (spec §6.3.2) runs before the row is read, by id, and changes
        // only the version and the size, never the path.
        let (mut now_fields, mut then_fields) = (fields(&now), fields(&entry));
        if cfg!(target_os = "macos") {
            (now_fields.2, then_fields.2) = (0, 0);
        } else {
            assert_eq!(after.current_version, contract.current_version, "{id}: version");
        }
        assert_eq!(now_fields, then_fields, "{id}: the row as it was");
        assert_eq!(after.parent_id, contract.parent_id, "{id}: parent");
    }
}

/// The rows of a folder the store refused are not placed either: not under it, and never at the top level. A child
/// created by an op under it waits for the snapshot instead of landing at the top level. Renamed to a name this host
/// holds, the folder asks for the snapshot, which places it with its rows.
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
        nodes: vec![grandchild_row.clone(), child_row.clone(), row(folder, "CON", false, true)],
    };
    apply_snapshot(&bridge, dir.path(), &snapshot, 200, 200, &mut Vec::new()).unwrap();
    for id in [folder, child, grandchild] {
        assert!(bridge.db().get_file(id).unwrap().is_none(), "{id} is not placed");
    }
    assert!(bridge.db().list_files().unwrap().is_empty(), "nothing at the top level either");

    // The op stream: the folder's create is refused, so the child's create names a parent this device does not hold.
    // It waits, and asks for no re-snapshot (one would refuse the folder again); a parent never seen asks for one.
    let op = |seq_id: i64, op_type: &str, payload: serde_json::Value| crate::api_client::SyncOp {
        seq_id,
        op_type: op_type.into(),
        payload,
    };
    let apply_op = |op: crate::api_client::SyncOp| apply_sync_op(&bridge, dir.path(), &op, 200, &mut Vec::new()).unwrap();
    // Read and clear a pending re-snapshot request (the same calls on every platform).
    let take_resnapshot = || match bridge.db().peek_resnapshot_request().unwrap() {
        Some(seen) => bridge.db().clear_resnapshot_request(seen).unwrap(),
        None => false,
    };
    let applied = apply_op(op(10, "folder_create", serde_json::json!({ "id": folder, "name_encrypted": "CON" })));
    assert!(applied.is_empty(), "{applied:?}");
    let child_create = |seq_id: i64, id: &str, parent: &str| {
        op(seq_id, "file_create", serde_json::json!({
            "id": id, "parent_id": parent, "name_encrypted": "a.txt", "version_number": 1, "size_bytes": 3,
        }))
    };
    assert!(apply_op(child_create(11, grandchild, folder)).is_empty());
    assert!(bridge.db().list_files().unwrap().is_empty(), "the child is not placed at the top level");
    assert_eq!(bridge.db().peek_resnapshot_request().unwrap(), None, "no re-snapshot for a refused parent");
    let orphan = "e0000000-0000-4000-8000-000000000004";
    assert!(apply_op(child_create(12, orphan, "e0000000-0000-4000-8000-0000000000ff")).is_empty());
    assert!(bridge.db().get_file(orphan).unwrap().is_none(), "not at the top level either");
    assert!(take_resnapshot(), "a parent never seen: the snapshot will place it");
    // Renamed to a name this host holds, the folder is still not placed by the rename (it names no parent), which asks
    // for the snapshot that places it, and its rows under it.
    let applied = apply_op(op(13, "folder_rename", serde_json::json!({ "id": folder, "new_name_encrypted": "Plans" })));
    assert!(applied.is_empty(), "{applied:?}");
    assert!(bridge.db().get_file(folder).unwrap().is_none());
    assert!(take_resnapshot(), "the snapshot places the renamed folder");
    let snapshot = crate::api_client::SyncSnapshot {
        seq_id: 14,
        nodes: vec![grandchild_row, child_row, row(folder, "Plans", false, true)],
    };
    apply_snapshot(&bridge, dir.path(), &snapshot, 300, 300, &mut Vec::new()).unwrap();
    for (id, path) in [(folder, "Plans"), (child, "Plans/inner"), (grandchild, "Plans/inner/a.txt")] {
        assert_eq!(bridge.db().get_file(id).unwrap().map(|row| row.path).as_deref(), Some(path));
    }
}

fn sync_op(seq_id: i64, op_type: &str, payload: serde_json::Value) -> crate::api_client::SyncOp {
    crate::api_client::SyncOp { seq_id, op_type: op_type.into(), payload }
}

/// Read and clear a pending re-snapshot request (the same calls on every platform).
fn take_resnapshot(bridge: &EngineBridge) -> bool {
    match bridge.db().peek_resnapshot_request().unwrap() {
        Some(seen) => bridge.db().clear_resnapshot_request(seen).unwrap(),
        None => false,
    }
}

/// A row the store refused and never held has no stored path to keep a parent from: a rename, an update (with or
/// without a name) or a move never places it, at the top level or with an empty path. A rename, and a move into a
/// folder this device holds, may change the store's answer, so they ask for the snapshot that places it under its
/// parent; the others ask for nothing. A row this device misses for another reason (a refusal from before a restart is
/// not remembered) is placed by the snapshot too.
#[test]
fn task_2413_a_refused_row_stays_refused_through_a_rename_an_update_and_a_move() {
    let _rules = HostRules::windows(true);
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    let docs = "e2000000-0000-4000-8000-000000000001";
    let other = "e2000000-0000-4000-8000-000000000002";
    let refused_folder = "e2000000-0000-4000-8000-000000000003";
    let file = "e2000000-0000-4000-8000-000000000004";
    seed_bridge_entry(&bridge, docs, "docs", None, FileStatus::CloudOnly, true, 0);
    seed_bridge_entry(&bridge, other, "other", None, FileStatus::CloudOnly, true, 0);
    let apply_op = |op: crate::api_client::SyncOp| apply_sync_op(&bridge, dir.path(), &op, 200, &mut Vec::new()).unwrap();
    let placed = || {
        let mut paths: Vec<String> = bridge.db().list_files().unwrap().into_iter().map(|row| row.path).collect();
        paths.sort();
        paths
    };
    let as_seeded = ["docs".to_string(), "other".to_string()];
    assert!(apply_op(sync_op(10, "folder_create", serde_json::json!({ "id": refused_folder, "name_encrypted": "AUX" }))).is_empty());
    let create = serde_json::json!({ "id": file, "parent_id": docs, "name_encrypted": "CON.txt", "version_number": 1, "size_bytes": 3 });
    assert!(apply_op(sync_op(11, "file_create", create)).is_empty());
    assert_eq!(placed(), as_seeded, "the create is refused");
    assert!(!take_resnapshot(&bridge));

    // (op, whether it asks for a re-snapshot)
    let ops = [
        (sync_op(12, "file_rename", serde_json::json!({ "id": file, "new_name_encrypted": "con2.txt" })), true),
        (sync_op(13, "file_update", serde_json::json!({ "id": file, "size_bytes": 9, "version_number": 2 })), false),
        (sync_op(14, "file_update", serde_json::json!({ "id": file, "name_encrypted": "CON.txt", "size_bytes": 9 })), false),
        (sync_op(15, "file_move", serde_json::json!({ "id": file, "new_parent_id": other })), true),
        (sync_op(16, "file_move", serde_json::json!({ "id": file, "new_parent_id": refused_folder })), false),
        (sync_op(17, "file_move", serde_json::json!({ "id": file })), false),
    ];
    let mut checked = 0;
    for (op, asks) in ops {
        let label = format!("{} {}", op.op_type, op.payload);
        assert!(apply_op(op).is_empty(), "{label}: nothing applied");
        assert_eq!(placed(), as_seeded, "{label}: never placed");
        assert!(bridge.db().get_file(file).unwrap().is_none(), "{label}: no row, not even one with an empty path");
        assert_eq!(take_resnapshot(&bridge), asks, "{label}: re-snapshot");
        checked += 1;
    }
    assert_eq!(checked, 6);

    // A row this device never saw (or refused before a restart): never placed by these ops either; the snapshot is.
    let missed = "e2000000-0000-4000-8000-000000000005";
    for (n, op) in [
        sync_op(20, "file_rename", serde_json::json!({ "id": missed, "new_name_encrypted": "late.txt" })),
        sync_op(21, "file_update", serde_json::json!({ "id": missed, "name_encrypted": "late.txt", "size_bytes": 4 })),
        sync_op(22, "file_update", serde_json::json!({ "id": missed, "size_bytes": 4 })),
        sync_op(23, "file_move", serde_json::json!({ "id": missed })),
    ]
    .into_iter()
    .enumerate()
    {
        assert!(apply_op(op).is_empty(), "op {n}");
        assert_eq!(placed(), as_seeded, "op {n}: never placed");
        assert!(take_resnapshot(&bridge), "op {n}: the snapshot places it");
    }
}

/// The rows below a refused folder are recorded as refused too (by the snapshot, and by an op under a refused folder),
/// so an op under any of them waits without asking for a re-snapshot: the snapshot it would ask for skips them again.
#[test]
fn task_2413_ops_below_a_refused_folder_ask_for_no_resnapshot() {
    let _rules = HostRules::windows(true);
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    let folder = "e3000000-0000-4000-8000-000000000001";
    let sub = "e3000000-0000-4000-8000-000000000002";
    let mut sub_row = row(sub, "sub", false, true);
    sub_row["parent_id"] = serde_json::json!(folder);
    let snapshot = crate::api_client::SyncSnapshot { seq_id: 9, nodes: vec![sub_row, row(folder, "CON", false, true)] };
    apply_snapshot(&bridge, dir.path(), &snapshot, 200, 200, &mut Vec::new()).unwrap();
    assert!(bridge.db().list_files().unwrap().is_empty());
    take_resnapshot(&bridge);
    let apply_op = |op: crate::api_client::SyncOp| apply_sync_op(&bridge, dir.path(), &op, 200, &mut Vec::new()).unwrap();
    let file_create = |seq_id: i64, id: &str, parent: &str| {
        sync_op(seq_id, "file_create", serde_json::json!({
            "id": id, "parent_id": parent, "name_encrypted": "a.txt", "version_number": 1, "size_bytes": 3,
        }))
    };
    let deeper = "e3000000-0000-4000-8000-000000000010";
    let ops = [
        file_create(10, "e3000000-0000-4000-8000-000000000011", sub),
        file_create(11, "e3000000-0000-4000-8000-000000000012", sub),
        sync_op(12, "folder_create", serde_json::json!({ "id": deeper, "parent_id": sub, "name_encrypted": "deeper" })),
        file_create(13, "e3000000-0000-4000-8000-000000000013", deeper),
        file_create(14, "e3000000-0000-4000-8000-000000000014", deeper),
    ];
    let mut asked = Vec::new();
    for (n, op) in ops.into_iter().enumerate() {
        assert!(apply_op(op).is_empty(), "op {n}");
        assert!(bridge.db().list_files().unwrap().is_empty(), "op {n}: not placed");
        if take_resnapshot(&bridge) {
            asked.push(n);
        }
    }
    assert_eq!(asked, Vec::<usize>::new(), "the ops (of 5) that asked for a re-snapshot");
}

/// A refused rename of a stored folder changes nothing, reports nothing as applied, and leaves its rows in place.
#[test]
fn task_2413_a_refused_rename_of_a_stored_folder_changes_nothing() {
    let _rules = HostRules::windows(true);
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    let folder = "e1000000-0000-4000-8000-000000000001";
    let child = "e1000000-0000-4000-8000-000000000002";
    seed_bridge_entry(&bridge, folder, "Plans", None, FileStatus::CloudOnly, true, 10);
    seed_bridge_entry(&bridge, child, "Plans/a.txt", Some(folder), FileStatus::CloudOnly, false, 10);
    let op = crate::api_client::SyncOp {
        seq_id: 5,
        op_type: "folder_rename".into(),
        payload: serde_json::json!({ "id": folder, "new_name_encrypted": "Q1: plan" }),
    };
    let applied = apply_sync_op(&bridge, dir.path(), &op, 200, &mut Vec::new()).unwrap();
    assert!(applied.is_empty(), "{applied:?}");
    assert_eq!(bridge.db().get_file(folder).unwrap().unwrap().path, "Plans");
    assert_eq!(bridge.db().get_file(child).unwrap().unwrap().path, "Plans/a.txt");
}

/// The same name rule for a shared row: a decrypted name that is not one plain name refuses the row, as for an own row
/// (the placeholder name stays for a name that does not decrypt), so no two refused items share a path.
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
    let shared = |id: &str, name: &str, is_folder: bool| {
        apply_shared_metadata_file_row(
            &db,
            &serde_json::json!({ "id": id, "name_encrypted": sealed(name), "size_bytes": 12, "is_folder": is_folder }),
            &root,
            1_700_000_000,
            Some(root.file_id.clone()),
            "Shared with me/Shared folder",
            Some(&file_key),
        )
        .unwrap()
    };
    let names = [
        ("CON", false), ("nul.txt", false), ("report:v2.txt", false), ("notes.", false), ("a\\b.txt", false),
        ("..", false), ("a/b", false), ("Q1: plan", true), ("Q2: plan", true),
    ];
    let mut stored = Vec::new();
    for (n, (name, is_folder)) in names.iter().enumerate() {
        let id = format!("f0000000-0000-4000-8000-{n:012}");
        match shared(&id, name, *is_folder) {
            Some(entry) => stored.push(format!("{name:?} -> {:?}", entry.path)),
            None => assert!(db.get_file(&id).unwrap().is_none(), "{name:?}: no row"),
        }
    }
    assert!(stored.is_empty(), "{} of {} stored:\n{}", stored.len(), names.len(), stored.join("\n"));
    // A plain name is stored, its outer spaces trimmed as before.
    let entry = shared("f0000000-0000-4000-8000-0000000000ff", " notes.txt ", false).unwrap();
    assert_eq!(entry.path, "Shared with me/Shared folder/notes.txt");
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
