//! The walker against the fake server (`test_api::fake_digests`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use beebeeb_core::folder_digest::{self as fd, ChildRecord};
use serde_json::Value;
use uuid::Uuid;

use super::*;
use crate::api_client::ApiClient;
use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler, start_fake};
use crate::test_api::large_account::{LargeAccount, seed};
use crate::test_api::{FakeApi, Reply, Request};

fn account() -> Arc<FakeAccount> {
    let (account, _) = FakeAccount::register("digests@example.invalid", "an invented password", TEST_PHRASE);
    Arc::new(account)
}

fn client(fake: &FakeApi, account: &FakeAccount) -> ApiClient {
    ApiClient::new(fake.base.clone(), account.session_token.clone(), account.master_key)
}

fn uid(kind: u32, n: u32) -> String {
    format!("d{kind:07x}-0011-4000-8000-{n:012x}")
}

/// A folder with `files` files and `folders` folders in it, under the root.
fn wide_folder(account: &FakeAccount, files: u32, folders: u32) -> String {
    let folder = uid(1, 0);
    account.add_item(&folder, None, "Wide", true, 0);
    for n in 0..files {
        account.add_item(&uid(2, n), Some(&folder), &format!("file {n}"), false, u64::from(n));
    }
    for n in 0..folders {
        account.add_item(&uid(3, n), Some(&folder), &format!("sub {n}"), true, 0);
    }
    folder
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_digest_unchanged_reference_root_costs_one_request() {
    let account = account();
    seed(&account, &LargeAccount::REFERENCE);
    let fake = start_fake(account.clone());
    let api = client(&fake, &account);
    let known = account.folder_digests_now();
    assert_eq!(known.len(), LargeAccount::REFERENCE.folders + 1);
    let count = RequestCount::default();
    let changed = changed_folders(&api, "", &known, &count).await.unwrap();
    assert!(changed.is_empty());
    assert_eq!(count.get(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_digest_one_file_six_deep_returns_exactly_its_chain() {
    let account = account();
    seed(&account, &LargeAccount::REFERENCE);
    let mut chain = vec![String::new()];
    for level in 1..=6u32 {
        let id = uid(4, level);
        let parent = (level > 1).then(|| uid(4, level - 1));
        account.add_item(&id, parent.as_deref(), &format!("level {level}"), true, 0);
        chain.push(id);
    }
    let deep = uid(5, 1);
    account.add_item(&deep, Some(&chain[6]), "deep.txt", false, 10);
    let fake = start_fake(account.clone());
    let api = client(&fake, &account);
    let known = account.folder_digests_now();

    // The file changes: new size, a new name ciphertext.
    account.add_item(&deep, Some(&chain[6]), "deep.txt", false, 11);

    let count = RequestCount::default();
    let changed = changed_folders(&api, "", &known, &count).await.unwrap();
    let ids: Vec<&str> = changed.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(ids, chain.iter().map(String::as_str).collect::<Vec<_>>());
    // Each changed folder's digest is the one the server has now.
    let now = account.folder_digests_now();
    for folder in &changed {
        assert_eq!(folder.digest, now[&folder.id]);
        assert_ne!(folder.digest, known[&folder.id]);
    }
    // The root's digest, its pages, and one page for each of the six.
    let root_pages = ((changed[0].child_count + i64::from(PAGE_LIMIT) - 1) / i64::from(PAGE_LIMIT)) as u32;
    assert_eq!(count.get(), 1 + root_pages + 6);
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_digest_folder_of_1200_reads_in_two_pages() {
    let account = account();
    let folder = wide_folder(&account, 1195, 5);
    let fake = start_fake(account.clone());
    let api = client(&fake, &account);
    let count = RequestCount::default();
    let read = read_folder(&api, &folder, &count).await.unwrap();
    assert_eq!(count.get(), 2);
    assert_eq!(read.child_count, 1200);
    assert_eq!(read.child_folders.len(), 5);
    let now = account.folder_digests_now();
    assert_eq!(read.digest, now[&folder]);
    for (id, digest) in &read.child_folders {
        assert_eq!(*digest, now[id]);
    }
    let pages: Vec<String> = fake
        .requests()
        .iter()
        .filter(|r| r.route() == "GET /api/v1/folder-digests")
        .map(|r| r.query("limit").unwrap_or_default().to_string())
        .collect();
    assert_eq!(pages, vec!["1000", "1000"]);
}

/// A fake whose second page of a read sees the folder change first (`always`: every second page does).
fn moving_fake(account: Arc<FakeAccount>, folder: String, always: bool) -> (FakeApi, Arc<AtomicU32>) {
    let inner = account_handler(account.clone());
    let moved = Arc::new(AtomicU32::new(0));
    let once = Arc::new(AtomicBool::new(false));
    let moved_in = moved.clone();
    let fake = FakeApi::start(Arc::new(move |req: &Request| {
        if req.route() == "GET /api/v1/folder-digests"
            && req.query("cursor").is_some()
            && (always || !once.swap(true, Ordering::SeqCst))
        {
            let n = moved_in.fetch_add(1, Ordering::SeqCst);
            account.add_item(&uid(6, n), Some(&folder), "late arrival", false, 1);
        }
        inner(req)
    }));
    (fake, moved)
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_digest_moving_between_pages_restarts_the_folder() {
    let account = account();
    let folder = wide_folder(&account, 1200, 0);
    let (fake, moved) = moving_fake(account.clone(), folder.clone(), false);
    let api = client(&fake, &account);
    let count = RequestCount::default();
    let read = read_folder(&api, &folder, &count).await.unwrap();
    assert_eq!(moved.load(Ordering::SeqCst), 1);
    assert_eq!(count.get(), 4, "two pages, then the folder again");
    assert_eq!(read.child_count, 1201);
    assert_eq!(read.digest, account.folder_digests_now()[&folder]);
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_digest_that_keeps_moving_fails_after_three_restarts() {
    let account = account();
    let folder = wide_folder(&account, 1200, 0);
    let (fake, _) = moving_fake(account.clone(), folder.clone(), true);
    let api = client(&fake, &account);
    let count = RequestCount::default();
    let error = read_folder(&api, &folder, &count).await.unwrap_err();
    assert!(matches!(error, WalkError::Failed(_)), "{error}");
    assert_eq!(count.get(), 2 * (MAX_RESTARTS + 1));
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_digest_old_server_is_unsupported() {
    let account = account();
    let folder = wide_folder(&account, 3, 1);
    let fake = start_fake(account.clone());
    let api = client(&fake, &account);
    account.digests_as_old_server(true);
    let count = RequestCount::default();
    assert!(matches!(
        changed_folders(&api, "", &Known::new(), &count).await,
        Err(WalkError::Unsupported)
    ));
    assert!(matches!(
        read_folder(&api, &folder, &count).await,
        Err(WalkError::Unsupported)
    ));
    // A 405 reads the same.
    let fake405 = FakeApi::start(Arc::new(|_: &Request| Reply::error(405, "method_not_allowed")));
    let api405 = client(&fake405, &account);
    assert!(matches!(
        digest_now(&api405, "", &count).await,
        Err(WalkError::Unsupported)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_digest_not_ready_carries_retry_after() {
    let account = account();
    let fake = start_fake(account.clone());
    let api = client(&fake, &account);
    account.digests_not_ready(Some(7));
    let count = RequestCount::default();
    assert!(matches!(
        digest_now(&api, "", &count).await,
        Err(WalkError::NotReady { retry_after_secs: 7 })
    ));
    assert_eq!(count.get(), 1);
    // The endpoint answered once: a not-ready answer is not retried.
    assert_eq!(fake.requests().len(), 1);
    account.digests_not_ready(None);
    assert!(digest_now(&api, "", &count).await.is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_digest_deleted_folder_is_gone() {
    let account = account();
    let folder = wide_folder(&account, 2, 0);
    let fake = start_fake(account.clone());
    let api = client(&fake, &account);
    assert!(account.trash_item(&folder));
    let count = RequestCount::default();
    match read_folder(&api, &folder, &count).await {
        Err(WalkError::FolderGone(id)) => assert_eq!(id, folder),
        other => panic!("expected FolderGone, got {other:?}"),
    }
    // The root no longer lists it.
    let walked = changed_folders(&api, "", &Known::new(), &count).await.unwrap();
    assert_eq!(walked.len(), 1);
}

async fn listing(http: &reqwest::Client, base: &str, token: &str, parent: Option<&str>) -> Vec<Value> {
    let url = match parent {
        Some(parent) => format!("{base}/api/v1/files?parent_id={parent}"),
        None => format!("{base}/api/v1/files"),
    };
    let body: Value = http
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    body["items"].as_array().unwrap().clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_digest_of_the_fake_equals_the_digest_of_its_listing() {
    let account = account();
    crate::test_api::fake_account::add_sample_tree(&account);
    let top = uid(7, 1);
    account.add_item(&top, None, "Top", true, 0);
    account.add_item(&uid(7, 2), Some(&top), "Mid", true, 0);
    account.add_item(&uid(7, 3), Some(&uid(7, 2)), "Empty", true, 0);
    account.add_item(&uid(7, 4), Some(&uid(7, 2)), "a.txt", false, 5);
    account.add_file_with_content(&uid(7, 5), Some(&top), "b.bin", b"some invented content");
    assert!(account.trash_item(&uid(7, 4)));
    let fake = start_fake(account.clone());
    let api = client(&fake, &account);
    let http = reqwest::Client::new();

    // Every folder, top-down from the listings, then each digest bottom-up from the listing JSON alone.
    let mut order: Vec<Option<String>> = vec![None];
    let mut rows: std::collections::HashMap<Option<String>, Vec<Value>> = Default::default();
    let mut at = 0;
    while at < order.len() {
        let parent = order[at].clone();
        let items = listing(&http, &fake.base, &account.session_token, parent.as_deref()).await;
        for item in &items {
            if item["is_folder"] == true {
                order.push(Some(item["id"].as_str().unwrap().to_string()));
            }
        }
        rows.insert(parent, items);
        at += 1;
    }
    let mut digests: std::collections::HashMap<Option<String>, fd::Digest> = Default::default();
    for folder in order.iter().rev() {
        let records = rows[folder]
            .iter()
            .map(|item| {
                let id = Uuid::parse_str(item["id"].as_str().unwrap()).unwrap();
                let name = fd::name_hash(item["name_encrypted"].as_str().unwrap());
                if item["is_folder"] == true {
                    ChildRecord::folder(id, name, digests[&Some(id.to_string())])
                } else {
                    ChildRecord::file(
                        id,
                        item["is_uploading"].as_bool().unwrap_or(false),
                        !item["owner_wrapped_file_key"].is_null(),
                        item["size_bytes"].as_i64().unwrap(),
                        item["version_number"].as_i64().unwrap(),
                        item["current_object_version_id"]
                            .as_str()
                            .map(|v| Uuid::parse_str(v).unwrap()),
                        name,
                    )
                }
            })
            .collect();
        digests.insert(folder.clone(), fd::folder_digest(records));
    }
    assert!(order.len() >= 5);
    let count = RequestCount::default();
    for folder in &order {
        let id = folder.clone().unwrap_or_default();
        let served = digest_now(&api, &id, &count).await.unwrap();
        assert_eq!(served, fd::to_hex(&digests[folder]), "folder {id:?}");
        let read = read_folder(&api, &id, &count).await.unwrap();
        for (child, digest) in &read.child_folders {
            assert_eq!(*digest, fd::to_hex(&digests[&Some(child.clone())]));
        }
    }
    assert_ne!(digests[&Some(uid(7, 3))], [0u8; 32]);
    assert_eq!(digests[&Some(uid(7, 3))], fd::EMPTY);
}
