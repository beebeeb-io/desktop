//! LD-0105 Amendment 4: the needs-attention row's IPC shape (the U1 lane renders it).

use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};

/// A poisoned file that changed on this device is an Error row with its reason and its one action; a
/// poison the sweep may still empty is no row of its own. Red on the mutant "every poison is a
/// needs-attention row" (the filter on `needs_attention` removed).
#[test]
fn ld0105_a4_a_needs_attention_row_carries_its_reason_and_its_one_action() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    for (id, path) in [("changed", "Films/holiday.mp4"), ("emptying", "Films/other.mp4")] {
        db.upsert_file(&FileEntry {
            file_id: id.into(),
            path: path.into(),
            status: FileStatus::Error,
            size_bytes: 10,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        db.record_poison(id, "corrupt", Some((1, 10)), 1).unwrap();
    }
    db.mark_poison_needs_attention("changed").unwrap();
    let items = super::tray_recent_activity_from_db(&db);
    let rows: Vec<_> = items.iter().filter(|i| i.attention.is_some()).collect();
    assert_eq!(rows.len(), 1, "only the changed one needs the person");
    let json = serde_json::to_value(rows[0]).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"id": "changed", "name": "holiday.mp4", "status": "Couldn't finish downloading", "icon": "file",
            "ok": false, "active": null, "progress": null, "attention": "changed_on_this_device", "action": "discard_and_download_again"})
    );
    // Every other row keeps today's shape: no new keys.
    assert!(items.iter().filter(|i| i.attention.is_none()).all(|i| {
        let v = serde_json::to_value(i).unwrap();
        v.get("attention").is_none() && v.get("action").is_none()
    }));
}
