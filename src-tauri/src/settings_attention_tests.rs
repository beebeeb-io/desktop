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

/// Desktop #225 into #226's UI (lead 2026-11-06): `get_active_transfers` carries a needs-attention
/// row, in the shape `src/desktopApi.ts` `ActiveTransferRow` reads, until the poison row ends (the last
/// step of `discard_and_download_again`); a file on the board carries it on its own row; a poison the
/// sweep may still empty is no row. Red on the mutants "the board alone" (no attention rows) and
/// "every poison is a needs-attention row".
#[test]
fn ld0105_u1_the_board_carries_a_needs_attention_row_until_the_discard_ends_it() {
    use crate::transfer_progress::{Direction, TransferBoard};
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    for (id, path) in [("changed", "Films/holiday.mp4"), ("emptying", "Films/other.mp4"), ("live", "clip.mp4")] {
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
    }
    for id in ["changed", "emptying"] {
        db.record_poison(id, "corrupt", Some((1, 10)), 1).unwrap();
    }
    db.mark_poison_needs_attention("changed").unwrap();
    let board = TransferBoard::new();
    let _live = board.begin_named("live", "clip.mp4", Direction::Stream, 10);
    let json = |db: Option<&StateDb>| serde_json::to_value(board.snapshot_with_attention(db)).unwrap();

    let rows = json(Some(&db));
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 2, "the live transfer and the changed file: {rows:?}");
    let changed = rows.iter().find(|r| r["fileId"] == "changed").expect("the changed file has a row");
    assert_eq!(
        *changed,
        serde_json::json!({"fileId": "changed", "name": "holiday.mp4", "folder": "Films", "direction": "down", "done": 0,
            "total": 10, "buffered": 0, "speedBps": 0, "attention": "changed_on_this_device",
            "action": "discard_and_download_again", "failed": false, "failedAtSecs": null})
    );
    let live = rows.iter().find(|r| r["fileId"] == "live").unwrap();
    assert!(live.get("attention").is_none() && live.get("action").is_none(), "a healthy row keeps #226's shape: {live}");

    // The discard's last step ends the poison row: the row goes with it.
    assert!(db.clear_poison("changed").unwrap());
    let rows = json(Some(&db));
    assert_eq!(rows.as_array().unwrap().len(), 1, "only the live transfer: {rows}");

    // A file on the board that needs the person carries it on its own row (no second row).
    db.record_poison("live", "corrupt", Some((1, 10)), 1).unwrap();
    db.mark_poison_needs_attention("live").unwrap();
    let rows = json(Some(&db));
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0]["fileId"].as_str(), rows[0]["direction"].as_str(), rows[0]["attention"].as_str()), (Some("live"), Some("stream"), Some("changed_on_this_device")));

    // No DB: the board alone.
    assert!(json(None).as_array().unwrap().iter().all(|r| r.get("attention").is_none()));
}
