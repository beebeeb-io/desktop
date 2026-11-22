//! RC2-4: after a corrupt `state.db` is moved aside and the files are listed again, a file on disk whose bytes are
//! the server's current version gets that version as its agreed content (checked against the server's content), so
//! a later edit is sent; a file whose bytes differ stays held as a real edit, nothing overwritten, both versions
//! kept. Through the engine's own client, bridge, scan and state db, against the fake server. Everything invented.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use beebeeb_engine::api_client::ApiClient;
use beebeeb_engine::engine_bridge::{EngineBridge, sync_tick};
use beebeeb_engine::local_state::{REBUILT_CHECK_FILENAME, RebuiltCheck, StateDbCheck, move_corrupt_state_db_aside};
use beebeeb_engine::state_db::{OperationKind, StateDb};
use beebeeb_engine::state_paths::STATE_DIR_OVERRIDE;
use beebeeb_engine::test_api::FakeApi;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, start_fake};

const FILE: &str = "c2400000-0000-4000-8000-000000000001";
const SERVER_BYTES: &[u8] = b"version one, as the account holds it";

struct Rebuilt {
    _dir: tempfile::TempDir,
    state_dir: PathBuf,
    sync_root: PathBuf,
    account: Arc<FakeAccount>,
    _fake: FakeApi,
    bridge: EngineBridge,
}

/// An account holding `notes.txt`, a sync folder holding `on_disk` at that name, and a `state.db` that was corrupt:
/// moved aside, a fresh one opened (as the start does), listed again by the first pull. The row is a synced file
/// with no agreed content, as the rebuild leaves the files that are on disk.
async fn rebuilt(on_disk: &[u8]) -> Rebuilt {
    let dir = tempfile::tempdir().unwrap();
    let state_dir = dir.path().join("state");
    let sync_root = dir.path().join("Beebeeb");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::create_dir_all(&sync_root).unwrap();
    let (account, _) = FakeAccount::register("rebuilt@example.invalid", "an invented password", TEST_PHRASE);
    let account = Arc::new(account);
    account.add_file_with_content(FILE, None, "notes.txt", SERVER_BYTES);
    let fake = start_fake(account.clone());
    std::fs::write(sync_root.join("notes.txt"), on_disk).unwrap();

    let db_path = state_dir.join("state.db");
    std::fs::write(&db_path, vec![0xABu8; 8192]).unwrap();
    assert!(matches!(
        move_corrupt_state_db_aside(&state_dir).unwrap(),
        StateDbCheck::MovedAside(_)
    ));
    assert!(
        state_dir.join(REBUILT_CHECK_FILENAME).exists(),
        "the rebuild owes the check"
    );
    let db = Arc::new(StateDb::open(&db_path).unwrap());
    let api = Arc::new(ApiClient::new(
        fake.base.clone(),
        account.session_token.clone(),
        account.master_key,
    ));
    let bridge = EngineBridge::new(&beebeeb_engine::identity::REAL, db, api);
    let ticked = STATE_DIR_OVERRIDE
        .scope(state_dir.clone(), sync_tick(&bridge, &sync_root))
        .await;
    ticked.expect("the first pull");
    assert!(bridge.db.get_file(FILE).unwrap().is_some(), "listed again");
    // What the rebuild leaves for a file that is on disk: synced, no content agreed with the server.
    bridge
        .db
        .test_execute(&format!(
            "UPDATE files SET status = 'local', local_hash = NULL, local_base_version = 0, \
             fetched_min_version = -1, fetched_hash = NULL WHERE file_id = '{FILE}'"
        ))
        .unwrap();
    Rebuilt {
        _dir: dir,
        state_dir,
        sync_root,
        account,
        _fake: fake,
        bridge,
    }
}

fn scan(r: &Rebuilt) {
    tokio::task::block_in_place(|| beebeeb_engine::watcher::run_one_scan(&r.bridge, &r.sync_root));
}

fn held(r: &Rebuilt) -> Vec<String> {
    r.bridge
        .db
        .list_held_edits()
        .unwrap()
        .into_iter()
        .filter(|(id, ..)| id == FILE)
        .map(|(_, _, reason, _)| reason)
        .collect()
}

fn uploads(r: &Rebuilt) -> usize {
    r.bridge
        .db
        .list_operations_for_file(FILE)
        .unwrap()
        .into_iter()
        .filter(|op| matches!(op.kind, OperationKind::UploadVersion | OperationKind::UploadFile))
        .count()
}

fn agreed_hash(r: &Rebuilt) -> Option<String> {
    r.bridge
        .db
        .get_file_contract_state(FILE)
        .unwrap()
        .and_then(|contract| contract.local_hash)
}

async fn in_state<F: std::future::Future>(state_dir: &Path, f: F) -> F::Output {
    STATE_DIR_OVERRIDE.scope(state_dir.to_path_buf(), f).await
}

#[tokio::test(flavor = "multi_thread")]
async fn after_rebuild_an_unchanged_file_is_agreed_and_its_later_edit_is_sent() {
    let r = rebuilt(SERVER_BYTES).await;
    let state_dir = r.state_dir.clone();
    in_state(&state_dir, async {
        scan(&r);
        assert_eq!(held(&r), vec!["content_unknown".to_string()], "not checked yet");

        sync_tick(&r.bridge, &r.sync_root).await.unwrap();
        assert!(held(&r).is_empty(), "checked: the not-checked-yet row shrinks");
        assert_eq!(
            agreed_hash(&r).as_deref(),
            Some(beebeeb_engine::engine_bridge::sha256_hex(SERVER_BYTES).as_str()),
            "the server's current version is its agreed content"
        );
        assert_eq!(uploads(&r), 0, "an unchanged file sends nothing");
        assert_eq!(std::fs::read(r.sync_root.join("notes.txt")).unwrap(), SERVER_BYTES);

        // The person edits it: the edit is sent on the agreed version.
        let edited: &[u8] = b"edited on this PC after the rebuild";
        std::fs::write(r.sync_root.join("notes.txt"), edited).unwrap();
        scan(&r);
        assert!(held(&r).is_empty(), "the edit is not held");
        assert_eq!(uploads(&r), 1, "the edit is queued");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        r.bridge.process_due_operations(&r.sync_root, now).await.unwrap();
        assert_eq!(uploads(&r), 0, "the edit is sent");
        assert_eq!(
            r.account.item_version(FILE).map(|(_, size)| size),
            Some(edited.len() as u64),
            "the server holds the edit"
        );
    })
    .await;

    // Once every file is checked and the settle time is over, the check ends.
    let mut owed = RebuiltCheck::read(&state_dir)
        .unwrap()
        .expect("still owed inside the settle time");
    owed.pulled_at = Some(0);
    owed.save(&state_dir).unwrap();
    in_state(&state_dir, async {
        sync_tick(&r.bridge, &r.sync_root).await.unwrap();
    })
    .await;
    assert!(RebuiltCheck::read(&state_dir).unwrap().is_none(), "the check ended");
}

#[tokio::test(flavor = "multi_thread")]
async fn after_rebuild_a_changed_file_stays_held_and_nothing_is_overwritten() {
    let mine: &[u8] = b"changed on this PC before the state was damaged";
    let r = rebuilt(mine).await;
    let state_dir = r.state_dir.clone();
    let before = r.account.item_version(FILE);
    in_state(&state_dir, async {
        scan(&r);
        assert_eq!(held(&r), vec!["content_unknown".to_string()]);
        sync_tick(&r.bridge, &r.sync_root).await.unwrap();
        scan(&r);
        assert_eq!(
            held(&r),
            vec!["content_unknown".to_string()],
            "still held as a real edit"
        );
        assert_eq!(agreed_hash(&r), None, "never agreed");
        assert_eq!(uploads(&r), 0, "not sent");
        assert_eq!(
            std::fs::read(r.sync_root.join("notes.txt")).unwrap(),
            mine,
            "this PC's bytes are kept"
        );
    })
    .await;
    assert_eq!(r.account.item_version(FILE), before, "the account's version is kept");
    let owed = RebuiltCheck::read(&state_dir).unwrap().expect("owed");
    assert!(owed.differing.contains(FILE), "not downloaded again");
}
