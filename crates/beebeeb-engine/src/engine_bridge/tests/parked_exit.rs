//! the ways out of a parked upload beyond the stale base, through the real accept and
//! the real decisions (`resolve_parked`, `StateDb::try_parked_again`, `StateDb::remove_parked_without_bytes`). Every
//! park is made as the queue's claim makes it: attempts used up, the class recorded. Every name is invented.

use super::*;
use crate::app::owned_state::parked_exit_api::keep_in_changes_not_sent;
use crate::state_db::{ParkExit, RemoveParked};

const FILE: &str = "25300000-0000-4000-8000-000000000001";
const THIS_PC: &[u8] = b"edited on this PC";

fn set_version(bridge: &EngineBridge, version: i64) {
    let mut contract = bridge.db.get_file_contract_state(FILE).unwrap().unwrap();
    contract.current_version = version;
    bridge.db.set_file_contract_state(&contract).unwrap();
}

/// A save of `notes.txt` on version 1 through the File Provider accept (the write the in-app viewer queues: a write id,
/// a staged copy), parked as `class`. The file is at `version` now. Returns the op and its staged copy.
fn parked(class: &str, version: i64) -> (tempfile::TempDir, EngineBridge, String, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    seed_uploaded_db_row(&bridge, FILE);
    fp_save(&bridge, dir.path(), FILE, "notes.txt", THIS_PC, "1");
    let op = bridge.db.list_operations_for_file(FILE).unwrap().remove(0);
    bridge
        .db
        .test_execute(&format!(
            "UPDATE operation_queue SET attempts = max_attempts, last_error = '{class}', last_error_class = '{class}'
             WHERE op_id = '{}'",
            op.op_id
        ))
        .unwrap();
    set_version(&bridge, version);
    let staged = PathBuf::from(op.payload_path.expect("a staged copy"));
    assert_eq!(
        std::fs::read(&staged).unwrap(),
        THIS_PC,
        "precondition: this PC's bytes are staged"
    );
    (dir, bridge, op.op_id, staged)
}

/// A later change of the same file, queued after the parked write.
fn queue_a_later_change(bridge: &EngineBridge) {
    bridge
        .db
        .enqueue_operation(&PendingOperation {
            op_id: "later".into(),
            kind: OperationKind::RenameFile,
            file_id: Some(FILE.into()),
            parent_id: None,
            target_path: Some("renamed.txt".into()),
            metadata_json: Some(serde_json::json!({ "name_encrypted": "sealed" }).to_string()),
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 25,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: now_secs() + 10,
            updated_at: now_secs() + 10,
        })
        .unwrap();
}

/// How many changes the queue holds (every kind but a download).
fn ops_in_queue(bridge: &EngineBridge) -> i64 {
    bridge.db.unsent_changes(0, 500, now_secs()).unwrap().total
}

/// R1: Keep the other on a write the server refused (403) keeps this PC's bytes BEFORE the write goes; a keep that
/// fails changes nothing.
#[test]
fn keep_the_other_on_a_permission_park_keeps_the_bytes_first() {
    let (dir, bridge, op_id, staged) = parked("permission_denied", 1);
    assert_eq!(bridge.db.park_exits(&op_id).unwrap(), vec![ParkExit::KeepOther]);

    let mut failing = |_: &Path, _: &str| -> anyhow::Result<PathBuf> { anyhow::bail!("no room to keep it") };
    assert!(
        bridge
            .resolve_parked(&op_id, ParkedChoice::Remote, Some(1), &mut failing, "PC", "10.00")
            .is_err()
    );
    assert!(
        bridge.db.get_operation(&op_id).unwrap().is_some(),
        "a failed keep leaves the write"
    );
    assert_eq!(std::fs::read(&staged).unwrap(), THIS_PC, "and its bytes");

    let root = dir.path().join("Changes not sent");
    let db = bridge.db.clone();
    let id = op_id.clone();
    let mut queued_while_kept = None;
    let mut keep = |source: &Path, rel: &str| -> anyhow::Result<PathBuf> {
        let kept = keep_in_changes_not_sent(&root, "2026-11-15 at 14.10.00", &id, rel, source)?;
        queued_while_kept = Some(db.get_operation(&id).unwrap().is_some());
        Ok(kept)
    };
    let done = bridge
        .resolve_parked(&op_id, ParkedChoice::Remote, Some(1), &mut keep, "PC", "10.00")
        .unwrap();
    assert_eq!(
        queued_while_kept,
        Some(true),
        "the copy is made while the write is still queued"
    );
    let ParkedResolution::KeptOther { kept_copy } = done else {
        panic!("kept the other: {done:?}")
    };
    assert_eq!(std::fs::read(&kept_copy).unwrap(), THIS_PC, "this PC's bytes are kept");
    assert!(Path::new(&kept_copy).starts_with(&root));
    assert!(
        bridge.db.get_operation(&op_id).unwrap().is_none(),
        "then the write goes"
    );
}

/// R1: Remove from the list runs only while no bytes of the change are on this PC: with its staged copy there it is
/// refused and nothing changes; with none it retires the write (and nothing else is offered for a missing copy).
#[test]
fn remove_from_the_list_only_without_bytes() {
    let (_dir, bridge, op_id, staged) = parked("payload_missing", 1);
    assert!(
        bridge.db.park_exits(&op_id).unwrap().is_empty(),
        "bytes here: no Remove offered"
    );
    assert_eq!(
        bridge.db.remove_parked_without_bytes(&op_id).unwrap(),
        RemoveParked::BytesHere
    );
    assert!(
        bridge.db.get_operation(&op_id).unwrap().is_some(),
        "refused: the write stays"
    );
    assert_eq!(std::fs::read(&staged).unwrap(), THIS_PC, "and its bytes");

    std::fs::remove_file(&staged).unwrap();
    #[cfg(not(target_os = "macos"))]
    {
        assert_eq!(bridge.db.park_exits(&op_id).unwrap(), vec![ParkExit::RemoveFromList]);
        assert_eq!(
            bridge.db.remove_parked_without_bytes(&op_id).unwrap(),
            RemoveParked::Removed
        );
        assert!(
            bridge.db.get_operation(&op_id).unwrap().is_none(),
            "no bytes: the write goes"
        );
        let row = bridge.db.get_file(FILE).unwrap().unwrap();
        assert_eq!(row.status, FileStatus::Local, "the file is Beebeeb's version as it was");
        assert_eq!(
            bridge.db.remove_parked_without_bytes(&op_id).unwrap(),
            RemoveParked::Moved,
            "a second press changes nothing"
        );
    }
    // macOS: a File Provider write's place is the system's own copy of the change, so it is never removed.
    #[cfg(target_os = "macos")]
    assert_eq!(
        bridge.db.remove_parked_without_bytes(&op_id).unwrap(),
        RemoveParked::BytesHere
    );
}

/// R2: every widened choice keeps the stale base's guards; a change queued after the write makes each one `Moved`,
/// with nothing changed.
#[test]
fn a_widened_choice_keeps_the_old_guards() {
    for (class, choice, exit) in [
        ("predecessor_lost", ParkedChoice::Remote, ParkExit::KeepOther),
        ("predecessor_lost", ParkedChoice::Both, ParkExit::KeepBoth),
        ("permission_denied", ParkedChoice::Remote, ParkExit::KeepOther),
    ] {
        let (dir, bridge, op_id, staged) = parked(class, 1);
        assert!(
            bridge.db.park_exits(&op_id).unwrap().contains(&exit),
            "{class}: offered before"
        );
        queue_a_later_change(&bridge);
        assert!(
            bridge.db.park_exits(&op_id).unwrap().is_empty(),
            "{class}: nothing offered behind a later change"
        );
        let before = ops_in_queue(&bridge);
        let root = dir.path().join("Changes not sent");
        let id = op_id.clone();
        let mut keep = |source: &Path, rel: &str| -> anyhow::Result<PathBuf> {
            Ok(keep_in_changes_not_sent(&root, "e", &id, rel, source)?)
        };
        let done = bridge
            .resolve_parked(&op_id, choice, Some(1), &mut keep, "PC", "10.00")
            .unwrap();
        assert_eq!(done, ParkedResolution::Moved, "{class} {choice:?}");
        assert!(
            bridge.db.get_operation(&op_id).unwrap().is_some(),
            "{class}: the write stays"
        );
        assert_eq!(ops_in_queue(&bridge), before, "{class}: nothing was queued");
        assert_eq!(std::fs::read(&staged).unwrap(), THIS_PC);
    }
}

/// R3: the stale base's own predicate is as it was: a widened class is not decidable by it (so the conflict window and
/// "Keep this PC's" never take one), and a stale-base write is decidable only past its base.
#[test]
fn stale_base_decidability_is_unchanged() {
    let (_dir, bridge, op_id, _staged) = parked("predecessor_lost", 2);
    assert!(
        !bridge.db.is_decidable_parked(&op_id).unwrap(),
        "a lost predecessor is not a stale base"
    );
    let mut keep = |_: &Path, _: &str| -> anyhow::Result<PathBuf> { anyhow::bail!("not asked") };
    assert_eq!(
        bridge
            .resolve_parked(&op_id, ParkedChoice::Local, Some(2), &mut keep, "PC", "10.00")
            .unwrap(),
        ParkedResolution::Moved,
        "Keep this PC's stays the stale base's"
    );
    assert_eq!(
        bridge.db.park_exits(&op_id).unwrap(),
        vec![ParkExit::KeepBoth, ParkExit::KeepOther]
    );

    let (_dir, bridge, op_id, _staged) = parked("stale_base", 1);
    assert!(!bridge.db.is_decidable_parked(&op_id).unwrap(), "not past its base");
    assert!(bridge.db.park_exits(&op_id).unwrap().is_empty());
    set_version(&bridge, 2);
    assert!(bridge.db.is_decidable_parked(&op_id).unwrap());
    assert_eq!(
        bridge.db.park_exits(&op_id).unwrap(),
        vec![ParkExit::KeepBoth, ParkExit::KeepOther, ParkExit::KeepThisPcs]
    );
}

/// R5: Try again on a `rekey_failed` park makes it due now with its attempts back to 0 and its error cleared; its base,
/// its staged bytes and its metadata stay as they were.
#[test]
fn rekey_try_again_keeps_base_and_bytes() {
    let (_dir, bridge, op_id, staged) = parked("rekey_failed", 2);
    let before = bridge.db.get_operation(&op_id).unwrap().unwrap();
    assert_eq!(bridge.db.park_exits(&op_id).unwrap(), vec![ParkExit::TryAgain]);
    let now = now_secs();
    assert!(bridge.db.try_parked_again(&op_id, now).unwrap());
    let after = bridge.db.get_operation(&op_id).unwrap().unwrap();
    assert_eq!(after.attempts, 0);
    assert_eq!(after.last_error, None);
    assert!(after.next_retry_at <= now, "due now");
    assert_eq!(after.base_version, before.base_version, "the base is not moved");
    assert_eq!(after.base_object_version_id, before.base_object_version_id);
    assert_eq!(after.payload_path, before.payload_path);
    assert_eq!(after.metadata_json, before.metadata_json);
    assert_eq!(std::fs::read(&staged).unwrap(), THIS_PC, "the bytes are as they were");
    assert!(bridge.db.park_exits(&op_id).unwrap().is_empty(), "no longer parked");
    assert!(
        !bridge.db.try_parked_again(&op_id, now).unwrap(),
        "a second press changes nothing"
    );
}

/// Keep both on a lost predecessor queues this PC's bytes as a new file named for this PC and the time, and retires
/// the write in the same transaction; a refused write (403) is not offered Keep both and is `Moved`.
#[test]
fn keep_both_on_a_lost_predecessor_queues_a_new_file() {
    let (_dir, bridge, op_id, _staged) = parked("predecessor_lost", 1);
    let mut keep = |_: &Path, _: &str| -> anyhow::Result<PathBuf> { anyhow::bail!("not asked") };
    let done = bridge
        .resolve_parked(&op_id, ParkedChoice::Both, Some(1), &mut keep, "PC", "10.00")
        .unwrap();
    let ParkedResolution::KeptBoth { file_id, name } = done else {
        panic!("kept both: {done:?}")
    };
    assert_eq!(name, "notes (PC, 10.00).txt");
    assert!(
        bridge.db.get_operation(&op_id).unwrap().is_none(),
        "the write is retired"
    );
    let copy = bridge.db.list_operations_for_file(&file_id).unwrap().remove(0);
    assert_eq!(
        std::fs::read(copy.payload_path.unwrap()).unwrap(),
        THIS_PC,
        "the new file is this PC's bytes"
    );

    let (_dir, bridge, op_id, _staged) = parked("permission_denied", 1);
    assert_eq!(
        bridge
            .resolve_parked(&op_id, ParkedChoice::Both, Some(1), &mut keep, "PC", "10.00")
            .unwrap(),
        ParkedResolution::Moved
    );
    assert!(bridge.db.get_operation(&op_id).unwrap().is_some());
}
