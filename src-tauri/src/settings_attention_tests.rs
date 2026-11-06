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
    for (id, path) in [("changed", "Films/holiday.mp4"), ("emptying", "Films/other.mp4"), ("live", "clip.mp4"), ("uploading", "notes.txt")] {
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
    // "orphan" has no files row: it could never be discarded (`missing`), so it is no row (review).
    for id in ["changed", "emptying", "uploading", "orphan"] {
        db.record_poison(id, "corrupt", Some((1, 10)), 1).unwrap();
    }
    for id in ["changed", "uploading", "orphan"] {
        db.mark_poison_needs_attention(id).unwrap();
    }
    let board = TransferBoard::new();
    let _live = board.begin_named("live", "clip.mp4", Direction::Stream, 10);
    // An upload of the person's change never offers "discard" (review).
    let _up = board.begin_named("uploading", "notes.txt", Direction::Up, 10);
    let json = |db: Option<&StateDb>| serde_json::to_value(board.snapshot_with_attention(db)).unwrap();

    let rows = json(Some(&db));
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 3, "the live transfer, the upload and the changed file (no orphan row): {rows:?}");
    assert!(rows.iter().all(|r| r["fileId"] != "orphan"), "a poison with no file row is no row");
    let up = rows.iter().find(|r| r["fileId"] == "uploading").unwrap();
    assert!(up.get("attention").is_none() && up.get("action").is_none(), "an upload row keeps its own shape: {up}");
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
    assert_eq!(rows.as_array().unwrap().len(), 2, "the live transfer and the upload: {rows}");

    // A file on the board that needs the person carries it on its own row (no second row).
    db.record_poison("live", "corrupt", Some((1, 10)), 1).unwrap();
    db.mark_poison_needs_attention("live").unwrap();
    let rows = json(Some(&db));
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let live = rows.iter().find(|r| r["fileId"] == "live").unwrap();
    assert_eq!((live["direction"].as_str(), live["attention"].as_str()), (Some("stream"), Some("changed_on_this_device")));

    // The tray's recent list shows no row for the orphan either: every needs-attention row has an exit.
    let tray = super::tray_recent_activity_from_db(&db);
    assert!(tray.iter().all(|i| i.id != "orphan"), "no tray row without a file row");
    assert!(tray.iter().any(|i| i.id == "live" && i.attention.is_some()));

    // No DB: the board alone.
    assert!(json(None).as_array().unwrap().iter().all(|r| r.get("attention").is_none()));
}

/// Review of #225 (lead 2026-11-06: no file name while the vault is locked): `get_active_transfers`
/// adds the needs-attention rows, each naming a file from state.db, only while the active account
/// holds a session. Red on the mutants "`vault_unlocked` is always true" and "`shown_transfers`
/// ignores the lock".
#[test]
fn ld0105_u1_a_locked_vault_shows_no_needs_attention_row() {
    use crate::account_commands::{shown_transfers, vault_unlocked};
    use crate::transfer_progress::{NeedsAttention, TransferBoard};
    let state = crate::AppState::default();
    crate::account::synthesize_single_account(&state, crate::account::AccountId("acct-locked".into()));
    let attention = [NeedsAttention { file_id: "changed".into(), path: "Films/holiday.mp4".into(), size: 10 }];
    let board = TransferBoard::new();
    let locked = vault_unlocked(&state);
    let shown = serde_json::to_string(&shown_transfers(board.snapshot_dtos(), locked, &attention)).unwrap();
    let unlocked = serde_json::to_string(&shown_transfers(board.snapshot_dtos(), true, &attention)).unwrap();
    assert!(!locked, "no session: the vault is locked");
    assert!(!shown.contains("holiday.mp4") && !shown.contains("changed_on_this_device"), "locked: no name, no row: {shown}");
    assert!(unlocked.contains("holiday.mp4"), "the control: unlocked, the row shows: {unlocked}");
}

/// Delta review P2-2 (lead 2026-11-06: blocks the merge): the needs-attention rows kept for a slow read
/// never reach another session. A's rows are kept under engine generation 1. A sign-out or an account
/// switch starts a new engine (generation 2): B's poll whose read times out shows nothing of A's. No
/// engine, or a lock, clears them. A discarded row never comes back from an older read. Red on the
/// mutants "the kept rows ignore the generation", "a lock or no engine keeps the rows", "no fence".
#[test]
fn ld0105_u1_kept_needs_attention_rows_never_reach_another_session() {
    use crate::transfer_progress::{AttentionCache, NeedsAttention, attention_for_poll};
    let a = vec![NeedsAttention { file_id: "a-file".into(), path: "A/holiday.mp4".into(), size: 1 }];
    let mut cache = AttentionCache::new();
    cache.store(1, 1, a.clone());
    assert_eq!(attention_for_poll(&mut cache, true, Some(1), Some(a.clone())), a, "a fresh read");
    assert_eq!(attention_for_poll(&mut cache, true, Some(1), None), a, "a slow read: the rows kept for this engine");
    assert!(attention_for_poll(&mut cache, true, Some(2), None).is_empty(), "B (a new engine) never sees A's rows");

    cache.store(1, 2, a.clone());
    assert!(attention_for_poll(&mut cache, true, None, None).is_empty(), "no engine: nothing");
    assert!(attention_for_poll(&mut cache, true, Some(1), None).is_empty(), "no engine cleared the kept rows");

    cache.store(1, 3, a.clone());
    assert!(attention_for_poll(&mut cache, false, Some(1), None).is_empty(), "locked: nothing");
    assert!(attention_for_poll(&mut cache, true, Some(1), None).is_empty(), "a lock cleared the kept rows");

    cache.store(1, 10, a.clone());
    cache.forget("a-file", 11);
    cache.store(1, 10, a.clone());
    assert!(attention_for_poll(&mut cache, true, Some(1), None).is_empty(), "a read started before the discard does not bring the row back");
    cache.store(1, 11, a.clone());
    assert_eq!(attention_for_poll(&mut cache, true, Some(1), None), a, "a later read is kept");
}
