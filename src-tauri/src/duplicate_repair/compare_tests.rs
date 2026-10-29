//! Task 2274, plan Task 4: the compare (spec phase 2, rows L10-L12, A4, I6) against the loopback server: B's bytes
//! are downloaded only for the number of pairs the person agreed to, decrypted chunk by chunk into zeroized buffers,
//! hashed and dropped.
use super::compare::{Counts, compare, decrypt_chunk};
use super::model::{Class, Pair, ServerFacts, ServerFile, ServerRead};
use super::server::RepairApi;
use super::server_tests::{RepairMockServer, file, get, pair};
use crate::api_client::ApiClient;
use zeroize::Zeroizing;

const MASTER: [u8; 32] = [7; 32];

fn key(file_id: &str) -> beebeeb_core::kdf::FileKey {
    beebeeb_core::kdf::derive_file_key(&beebeeb_core::kdf::MasterKey::from_bytes(MASTER), file_id.as_bytes())
}

/// B's file and its chunks, as the server holds them: each chunk encrypted under B's key.
fn stored(b: &str, size: usize, chunks: &[&[u8]]) -> Vec<(String, u16, Vec<u8>)> {
    let mut json: serde_json::Value = serde_json::from_str(&file("p", size as i64, false, 1)).unwrap();
    json["chunk_count"] = serde_json::json!(chunks.len());
    let mut routes = vec![get(&format!("/api/v1/files/{b}"), 200, json.to_string())];
    for (index, plain) in chunks.iter().enumerate() {
        let sealed = beebeeb_core::encrypt::encrypt_chunk_raw(&key(b), plain).unwrap();
        routes.push((format!("GET /api/v1/files/{b}/chunks/{index}"), 200, sealed));
    }
    routes
}

/// A pair the server read left at L10: A and B found, same parent and size, B neither shared nor versioned.
fn waiting(n: u32, b: &str, a_content: &[u8], size: usize) -> Pair {
    let mut pair = pair(n, &format!("a{n}"), b);
    pair.a.local_hash = Some(crate::engine_bridge::sha256_hex(a_content));
    let facts = ServerFacts {
        trashed: false,
        parent_id: Some("p".into()),
        size_bytes: size as i64,
        updated_at: "2026-10-29T09:00:00+00:00".into(),
        version_number: 1,
        via_share: false,
        chunk_count: 1,
    };
    pair.server = Some(ServerRead {
        a: ServerFile::Found(facts.clone()),
        b: Some(ServerFile::Found(facts)),
        live_links: Some(0),
        live_invites: Some(0),
    });
    pair
}

fn api(mock: &RepairMockServer) -> RepairApi {
    RepairApi::new(ApiClient::new(mock.base_url.clone(), "test-token".into(), MASTER))
}

/// A4: the agreed number must be exactly the number of pairs waiting, or nothing is requested at all.
#[tokio::test]
async fn the_agreed_download_count_must_match_the_pairs_waiting() {
    let mock = RepairMockServer::start(stored("b1", 4, &[b"same"]));
    let mut pairs = vec![waiting(1, "b1", b"same", 4), waiting(2, "b2", b"same", 4)];
    for agreed in [0, 1, 3] {
        let error = compare(&api(&mock), &mut pairs, agreed).await.unwrap_err().to_string();
        assert!(error.contains("2 pairs"), "{error}");
    }
    // A pair the server read has not reached yet: the compare waits for it.
    let mut unread = vec![waiting(1, "b1", b"same", 4), pair(2, "a2", "b2")];
    assert!(compare(&api(&mock), &mut unread, 1).await.is_err());
    assert_eq!(mock.requests(), Vec::<String>::new(), "nothing requested");
    assert!(pairs.iter().all(|pair| pair.class == Class::Unknown));
}

/// L12 and L11: the same bytes across two chunks are identical; one byte off differs. Only GETs.
#[tokio::test]
async fn the_same_bytes_are_identical_and_one_byte_off_differs() {
    let mut routes = stored("b1", 11, &[b"hello ", b"world"]);
    routes.extend(stored("b2", 11, &[b"hello worle"]));
    let mock = RepairMockServer::start(routes);
    let mut pairs = vec![waiting(1, "b1", b"hello world", 11), waiting(2, "b2", b"hello world", 11)];
    let counts = compare(&api(&mock), &mut pairs, 2).await.unwrap();
    assert_eq!((pairs[0].class, pairs[1].class), (Class::Identical, Class::Differs));
    assert_eq!(counts, Counts { identical: 1, differs: 1, changed: 0 });
    let requests = mock.requests();
    assert!(requests.iter().all(|line| line.starts_with("GET ")), "{requests:?}");
    assert_eq!(requests.iter().filter(|line| line.contains("/chunks/")).count(), 3);
}

/// B changed on the server since the server read: not downloaded, left waiting, counted.
#[tokio::test]
async fn a_b_that_changed_since_the_server_read_is_not_downloaded() {
    let mut routes = stored("b1", 4, &[b"same"]);
    let mut changed: serde_json::Value = serde_json::from_str(&file("p", 4, false, 1)).unwrap();
    changed["updated_at"] = serde_json::json!("2026-10-29T11:00:00+00:00");
    routes[0] = get("/api/v1/files/b1", 200, changed.to_string());
    let mock = RepairMockServer::start(routes);
    let mut pairs = vec![waiting(1, "b1", b"same", 4)];
    let counts = compare(&api(&mock), &mut pairs, 1).await.unwrap();
    assert_eq!(pairs[0].class, Class::Unknown);
    assert_eq!(counts, Counts { identical: 0, differs: 0, changed: 1 });
    assert!(!mock.requests().iter().any(|line| line.contains("/chunks/")), "{:?}", mock.requests());
}

/// A failed chunk stops the run; the error names the pair and the status, never an id (I5).
#[tokio::test]
async fn a_failed_chunk_stops_the_run_without_naming_an_id() {
    let mut routes = stored("b-secret-id", 4, &[b"same"]);
    routes[1].1 = 500;
    let mock = RepairMockServer::start(routes);
    let mut pairs = vec![waiting(1, "b-secret-id", b"same", 4)];
    let error = format!("{:#}", compare(&api(&mock), &mut pairs, 1).await.unwrap_err());
    assert!(error.contains("pair 1") && error.contains("500"), "{error}");
    assert!(!error.contains("secret-id") && !error.contains("127.0.0.1"), "{error}");
    assert_eq!(pairs[0].class, Class::Unknown);
}

/// I6: a decrypted chunk only ever exists as a zeroized buffer (the binding below does not compile otherwise).
#[test]
fn a_decrypted_chunk_is_a_zeroized_buffer() {
    let sealed = beebeeb_core::encrypt::encrypt_chunk_raw(&key("b1"), b"plain").unwrap();
    let plain: Zeroizing<Vec<u8>> = decrypt_chunk(&key("b1"), &sealed).unwrap();
    assert_eq!(plain.as_slice(), b"plain");
    assert!(decrypt_chunk(&key("other"), &sealed).is_err(), "the wrong key does not decrypt");
}
