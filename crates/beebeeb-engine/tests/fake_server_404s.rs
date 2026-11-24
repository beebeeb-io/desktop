//! The fake server's two former 404s, read through the engine's own client (`ApiClient`): `GET /api/v1/files/count`
//! (the Trash badge) and the search-index shard routes (`PUT`, then read back by the manifest and by `GET`). A request
//! with another account's token is `401` and reads or stores nothing. Every account and blob here is invented.

use std::sync::Arc;

use beebeeb_core::search_index::EncryptedShard;
use beebeeb_engine::api_client::{ApiClient, MAX_SEARCH_SHARD_BLOB_BYTES};
use beebeeb_engine::test_api::FakeApi;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, start_fake};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn start() -> (Arc<FakeAccount>, FakeApi) {
    let (account, _phrase) = FakeAccount::register("fake-404s@example.invalid", "an invented password", TEST_PHRASE);
    let account = Arc::new(account);
    let fake = start_fake(account.clone());
    (account, fake)
}

fn client(fake: &FakeApi, token: &str, account: &FakeAccount) -> ApiClient {
    ApiClient::new(fake.base.clone(), token.to_string(), account.master_key)
}

#[test]
fn the_trash_count_loads_and_follows_the_trash() {
    let (account, fake) = start();
    let api = client(&fake, &account.session_token, &account);
    let runtime = runtime();
    assert_eq!(runtime.block_on(api.trashed_file_count()).unwrap(), 0, "an empty Trash");
    account.add_file_with_content("f-1", None, "one.txt", b"one");
    account.add_file_with_content("f-2", None, "two.txt", b"two");
    assert!(account.trash_item("f-1"));
    assert_eq!(
        runtime.block_on(api.trashed_file_count()).unwrap(),
        1,
        "one trashed, one live"
    );
    assert!(account.trash_item("f-2"));
    assert_eq!(runtime.block_on(api.trashed_file_count()).unwrap(), 2);
}

#[test]
fn another_accounts_count_is_refused() {
    let (account, fake) = start();
    account.add_file_with_content("f-1", None, "one.txt", b"one");
    assert!(account.trash_item("f-1"));
    let stranger = client(&fake, "another-accounts-token", &account);
    let refused = runtime().block_on(stranger.trashed_file_count()).unwrap_err();
    assert!(
        format!("{refused:#}").contains("401"),
        "refused as unauthorized: {refused:#}"
    );
}

fn shard(bucket: u32, page: u32, blob: &[u8]) -> EncryptedShard {
    EncryptedShard {
        bucket,
        page,
        blob: blob.to_vec(),
    }
}

#[test]
fn a_search_shard_upload_is_accepted_and_read_back() {
    let (account, fake) = start();
    let api = client(&fake, &account.session_token, &account);
    let runtime = runtime();
    assert!(runtime.block_on(api.fetch_search_shard_manifest()).unwrap().is_empty());

    let first = runtime
        .block_on(api.put_search_shard(&shard(7, 2, b"sealed-one")))
        .unwrap();
    assert_eq!((first.bucket, first.page), (7, 2));
    let again = runtime
        .block_on(api.put_search_shard(&shard(7, 2, b"sealed-two")))
        .unwrap();
    assert!(again.version > first.version, "a new put bumps the version");

    let manifest = runtime.block_on(api.fetch_search_shard_manifest()).unwrap();
    assert_eq!(manifest, vec![again]);
    assert_eq!(
        runtime.block_on(api.get_search_shard(7, 2)).unwrap(),
        Some(b"sealed-two".to_vec())
    );
    assert_eq!(
        runtime.block_on(api.get_search_shard(7, 3)).unwrap(),
        None,
        "an absent shard reads as none"
    );

    runtime.block_on(api.delete_search_shard(7, 2)).unwrap();
    assert!(runtime.block_on(api.fetch_search_shard_manifest()).unwrap().is_empty());
}

#[test]
fn another_accounts_shard_requests_are_refused_and_store_nothing() {
    let (account, fake) = start();
    let owner = client(&fake, &account.session_token, &account);
    let stranger = client(&fake, "another-accounts-token", &account);
    let runtime = runtime();
    runtime.block_on(owner.put_search_shard(&shard(1, 0, b"mine"))).unwrap();

    let put = runtime
        .block_on(stranger.put_search_shard(&shard(1, 0, b"theirs")))
        .unwrap_err();
    assert!(format!("{put:#}").contains("401"), "put refused: {put:#}");
    let get = runtime.block_on(stranger.get_search_shard(1, 0)).unwrap_err();
    assert!(format!("{get:#}").contains("401"), "get refused: {get:#}");
    assert!(
        runtime.block_on(stranger.fetch_search_shard_manifest()).is_err(),
        "manifest refused"
    );
    assert!(
        runtime.block_on(stranger.delete_search_shard(1, 0)).is_err(),
        "delete refused"
    );
    assert_eq!(
        runtime.block_on(owner.get_search_shard(1, 0)).unwrap(),
        Some(b"mine".to_vec()),
        "the owner's shard stands"
    );
}

#[test]
fn an_oversized_shard_is_refused_by_the_fake_as_by_the_client() {
    let (account, fake) = start();
    let api = client(&fake, &account.session_token, &account);
    // The client refuses it before sending; a raw PUT shows the fake enforces the same cap.
    let blob = vec![1u8; MAX_SEARCH_SHARD_BLOB_BYTES + 1];
    assert!(runtime().block_on(api.put_search_shard(&shard(2, 0, &blob))).is_err());
    let status = std::net::TcpStream::connect(fake.base.trim_start_matches("http://")).and_then(|mut s| {
        use std::io::{Read, Write};
        write!(
            s,
            "PUT /api/v1/search-index/shards/2/0 HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            account.session_token,
            blob.len()
        )?;
        s.write_all(&blob)?;
        let mut out = String::new();
        s.read_to_string(&mut out)?;
        Ok(out)
    });
    assert!(status.unwrap().starts_with("HTTP/1.1 413"), "413 over the cap");
}
