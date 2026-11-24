//! `Engine::trash_items_for` against the fake API: once the server accepted the trash, the local row is parked
//! `Trashing` (Files drops it at once and a re-snapshot cannot bring it back); a refused trash leaves it alone.
//! Every name and key is invented.

use std::sync::Arc;

use super::super::{Engine, EngineConfig};
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE};
use crate::test_api::{FakeApi, Reply, Request};

type TestEngine = Engine<MemoryCredentials>;

const KEEP: &str = "b0000000-0000-4000-8000-000000000801";
const GONE: &str = "b0000000-0000-4000-8000-000000000802";
const REFUSED: &str = "b0000000-0000-4000-8000-000000000803";

fn setup() -> (
    FakeApi,
    Arc<TestEngine>,
    StateDb,
    tempfile::TempDir,
    crate::account::SessionGeneration,
) {
    let (account, _) = FakeAccount::register("trash-one@example.eu", "an invented password", TEST_PHRASE);
    let api = FakeApi::start(Arc::new(move |req: &Request| {
        let route = req.route();
        if route.starts_with("DELETE /api/v1/files/") {
            return if route.ends_with(REFUSED) {
                Reply::error(500, "boom")
            } else {
                Reply::json(200, serde_json::json!({}))
            };
        }
        Reply::error(404, "not_found")
    }));
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-trash-slot".into(),
        state_dir,
        api_base: api.base.clone(),
    };
    let engine = Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).unwrap());
    {
        let _writes = engine.writes.write().unwrap();
        engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        *engine.acct.session.lock().unwrap() = Some(Session {
            token: account.session_token.clone(),
            master_key: account.master_key,
            email: Some(account.email.clone()),
        });
        engine.remember_session_user_id(Some(account.user_id.clone()));
        engine.set_auth_present(true);
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    for (id, name) in [(KEEP, "keep.txt"), (GONE, "gone.txt"), (REFUSED, "refused.txt")] {
        let entry = FileEntry {
            file_id: id.into(),
            path: name.into(),
            status: FileStatus::CloudOnly,
            size_bytes: 10,
            modified_at: 1_794_000_000,
            content_hash: None,
            remote_updated_at: 1_794_000_000,
            parent_id: None,
            item_kind: ItemKind::File,
        };
        db.upsert_file_with_contract(&entry, |c| c.item_kind = entry.item_kind.clone())
            .unwrap();
    }
    db.set_owner(&Identity::new(Some(&account.user_id), Some(&account.email)))
        .unwrap();
    engine.use_local_state_for_test(path);
    let turn = engine.session_turn();
    (api, engine, db, dir, turn)
}

#[test]
fn a_trash_the_server_accepted_parks_the_local_row_and_only_that_one() {
    let (_api, engine, db, _dir, turn) = setup();
    engine.trash_items_for(turn, &[GONE]).unwrap();
    assert_eq!(db.get_file(GONE).unwrap().unwrap().status, FileStatus::Trashing);
    assert_eq!(db.get_file(KEEP).unwrap().unwrap().status, FileStatus::CloudOnly);
}

#[test]
fn a_trash_the_server_refused_leaves_the_local_row_in_files() {
    let (_api, engine, db, _dir, turn) = setup();
    assert!(engine.trash_items_for(turn, &[REFUSED]).is_err());
    assert_eq!(db.get_file(REFUSED).unwrap().unwrap().status, FileStatus::CloudOnly);
}
