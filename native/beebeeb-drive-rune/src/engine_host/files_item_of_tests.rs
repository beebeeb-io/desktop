//! `FilesSource::item_of` on the real engine: an id of the signed-in account's local state gives its row; the same
//! id under another account gives `None`. Every name is invented.

use std::sync::Arc;

use beebeeb_engine::account_binding::Identity;
use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{FakeApi, Handler, Request};
use serde_json::Value;
use zeroize::Zeroizing;

use crate::engine_host::EngineFilesSource;
use crate::frames::files::source::FilesSource;

const PASSWORD: &str = "an invented password for the item lookup";

fn sign_in(engine: &Engine<MemoryCredentials>, account: &FakeAccount, phrase: &str) {
    engine
        .sign_in_password(&account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(phrase.to_string()))
        .expect("the phrase unlocks");
}

#[test]
fn an_id_of_this_accounts_state_is_found_and_another_accounts_is_not() {
    let (first, first_phrase) = FakeAccount::register("item-of-one@example.eu", PASSWORD, TEST_PHRASE);
    let (second, second_phrase) = FakeAccount::register("item-of-two@example.eu", PASSWORD, TEST_PHRASE);
    let (first, second) = (Arc::new(first), Arc::new(second));
    let (one, two): (Handler, Handler) = (account_handler(first.clone()), account_handler(second.clone()));
    let (second_email, second_token) = (second.email.clone(), second.session_token.clone());
    let fake = FakeApi::start(Arc::new(move |req: &Request| {
        let theirs = if req.route().starts_with("POST /api/v1/opaque/") {
            req.json()
                .get("email")
                .and_then(Value::as_str)
                .is_some_and(|sent| sent.eq_ignore_ascii_case(&second_email))
        } else {
            req.bearer() == Some(second_token.as_str())
        };
        if theirs { two(req) } else { one(req) }
    }));
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "item-of-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: fake.base.clone(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    let engine = Arc::new(engine);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    let entry = FileEntry {
        file_id: "f-invented".into(),
        path: "Invented note.txt".into(),
        status: FileStatus::Local,
        size_bytes: 1_000,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: ItemKind::File,
    };
    db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
        .unwrap();
    db.set_owner(&Identity::new(Some(&first.user_id), Some(&first.email))).unwrap();
    engine.use_local_state_for_test(path);

    sign_in(&engine, &first, &first_phrase);
    let source = EngineFilesSource::new(engine.clone());
    let found = source.item_of("f-invented").expect("this account's item is found");
    assert_eq!((found.id.as_str(), found.name.as_str(), found.is_folder), ("f-invented", "Invented note.txt", false));
    assert!(source.item_of("f-not-there").is_none());

    sign_in(&engine, &second, &second_phrase);
    assert!(source.item_of("f-invented").is_none(), "another account's id is not answered");
}
