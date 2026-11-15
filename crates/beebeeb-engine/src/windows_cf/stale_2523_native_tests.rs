//! Task 2523 (F1), the Windows-native half on a QA sync root (a child of the LD-0105 rungs: their QA root and
//! census are reused).

use super::*;

/// A whole, in-sync placeholder `name` holding `bytes` (FileIdentity `id`), kept when `pinned`, and its row: `local`,
/// agreed at version 1 with exactly those bytes, current version 2 of `new_len` bytes (a pull raised it).
fn stale_2523_copy(
    qa: &QaRoot,
    db: &crate::state_db::StateDb,
    id: &str,
    name: &str,
    bytes: &[u8],
    new_len: i64,
    pinned: bool,
) -> crate::state_db::StaleCopy {
    let path = qa.path(name);
    std::fs::write(&path, bytes).unwrap();
    placeholders::convert_to_in_sync_placeholder(&path, id).unwrap();
    if pinned {
        placeholders::set_pin_state(&path, true, false).unwrap();
    }
    db.test_execute(&format!(
        "INSERT INTO files (file_id, path, status, item_kind, size_bytes, modified_at, current_version,
                            local_base_version, local_hash, fetched_min_version, pin_state)
         VALUES ('{id}', '{name}', 'local', 'file', {new_len}, 1700000500, 2, 1, '{}', 0, '{}')",
        crate::engine_bridge::sha256_hex(bytes),
        if pinned { "pinned" } else { "inherit" }
    ))
    .unwrap();
    db.list_stale_agreed_copies(64)
        .unwrap()
        .into_iter()
        .find(|copy| copy.file_id == id)
        .expect("the rule names it")
}

/// Task 2523 (F1), the Windows half on a QA sync root: an agreed copy behind its current version is emptied under
/// one exclusive handle (its bytes read through that handle first), carries the current version's size, is in sync,
/// and a kept one keeps its pin; the row records it. A copy whose bytes changed is never touched, nor one open
/// elsewhere. This is also the proof that the step's handle reads the file's bytes (`hash_through`).
#[test]
fn stale_2523_native_an_agreed_copy_behind_its_version_is_emptied_at_the_new_size() {
    use crate::windows_cf::stale_copy::{Refreshed, refresh};
    let mut qa = QaRoot::new(crate::windows_cf::HYDRATION_POLICY);
    qa.connect(refuse_every_fetch); // this process is the connected provider: only it may set IN_SYNC
    let state = tempfile::tempdir().unwrap();
    let db = crate::state_db::StateDb::open(state.path().join("state.db")).unwrap();
    let old = b"version one";
    let plain = stale_2523_copy(&qa, &db, "qa-stale", "stale.txt", old, 17, false);
    let kept = stale_2523_copy(&qa, &db, "qa-stale-kept", "stale-kept.txt", old, 17, true);
    let changed = stale_2523_copy(&qa, &db, "qa-stale-changed", "stale-changed.txt", old, 17, false);
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(qa.path("stale-changed.txt"), b"an edit made here").unwrap();
    let busy = stale_2523_copy(&qa, &db, "qa-stale-open", "stale-open.txt", old, 17, false);
    let open = std::fs::File::open(qa.path("stale-open.txt")).unwrap();
    // Review P2-2: kept in Explorer, not yet recorded in the database (its row says no pin).
    let explorer = stale_2523_copy(&qa, &db, "qa-stale-explorer", "stale-explorer.txt", old, 17, false);
    placeholders::set_pin_state(&qa.path("stale-explorer.txt"), true, false).unwrap();
    assert!(!explorer.pinned, "precondition: the database names no pin");
    let outcomes: Vec<(&str, Refreshed)> = [
        ("stale.txt", &plain),
        ("stale-kept.txt", &kept),
        ("stale-changed.txt", &changed),
        ("stale-open.txt", &busy),
        ("stale-explorer.txt", &explorer),
    ]
    .into_iter()
    .map(|(name, copy)| (name, refresh(&db, copy, &qa.path(name))))
    .collect();
    drop(open);
    let after: Vec<_> = ["stale.txt", "stale-kept.txt", "stale-changed.txt", "stale-open.txt"]
        .into_iter()
        .map(|name| {
            (
                name,
                census_of(&qa.path(name)),
                std::fs::metadata(qa.path(name)).unwrap().len(),
            )
        })
        .collect();
    println!("stale-2523-native: outcomes {outcomes:?} census {after:?}");
    assert_eq!(
        outcomes.iter().map(|(_, o)| *o).collect::<Vec<_>>(),
        vec![
            Refreshed::Emptied { kept: false },
            Refreshed::Emptied { kept: true },
            Refreshed::Changed,
            Refreshed::Later("in_use"),
            Refreshed::Emptied { kept: true },
        ]
    );
    assert_eq!(after[0].1, (OnDisk::Nothing, true, false), "emptied, in sync");
    assert_eq!(
        after[1].1,
        (OnDisk::Nothing, true, true),
        "emptied, in sync, its pin restored"
    );
    assert_eq!((after[0].2, after[1].2), (17, 17), "the current version's size");
    assert_eq!(after[2].1.0, OnDisk::Wholly, "a change is never touched");
    assert_eq!(
        std::fs::read(qa.path("stale-changed.txt")).unwrap(),
        b"an edit made here"
    );
    assert_eq!(after[3].1, (OnDisk::Wholly, true, false), "untouched while open");
    for id in ["qa-stale", "qa-stale-kept", "qa-stale-explorer"] {
        let row = db.get_file(id).unwrap().unwrap();
        let contract = db.get_file_contract_state(id).unwrap().unwrap();
        assert_eq!(
            (row.status, contract.local_hash),
            (crate::state_db::FileStatus::CloudOnly, None),
            "{id}"
        );
    }
    // Review P3-5: the changed copy, the same as when it was read, is not opened or read again (no step runs); once
    // it is written again it is read again (the identity is checked, then the bytes).
    let again = |db: &crate::state_db::StateDb| {
        let mut steps = Vec::new();
        let outcome =
            crate::windows_cf::stale_copy::refresh_with(db, &changed, &qa.path("stale-changed.txt"), &mut |step, _| {
                steps.push(step)
            });
        (outcome, steps)
    };
    let unchanged = again(&db);
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(qa.path("stale-changed.txt"), b"an edit made here, then another").unwrap();
    let rewritten = again(&db);
    println!("stale-2523-native p3-5: unchanged {unchanged:?} rewritten {rewritten:?}");
    assert_eq!(
        unchanged,
        (Refreshed::Changed, vec![]),
        "not read again while unchanged"
    );
    assert_eq!(
        rewritten,
        (
            Refreshed::Changed,
            vec![crate::engine_bridge::RefreshStep::CheckIdentity]
        ),
        "read again once written"
    );
    let left: Vec<String> = db
        .list_stale_agreed_copies(64)
        .unwrap()
        .into_iter()
        .map(|copy| copy.file_id)
        .collect();
    assert_eq!(left, vec!["qa-stale-changed".to_string(), "qa-stale-open".to_string()]);
}
