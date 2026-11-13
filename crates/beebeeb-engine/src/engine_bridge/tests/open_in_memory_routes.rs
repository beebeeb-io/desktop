//! `EngineBridge::open_in_memory` on every download route (a viewer's open): the pinned per-chunk API route, the
//! direct route from storage, and the route of a file whose metadata names no version. Each test names the route
//! it ran by what the loopback pair saw. The failure kinds are the ones a viewer shows
//! (`app::viewer_api::failure`), read from the error each route really makes.
//!
//! Against `direct_download::test_support`'s loopback pair, or a loopback mock of this file's own for metadata
//! that does not tell the truth about the file's size. Never the network; every key and byte is invented.

use std::collections::VecDeque;

use serde_json::{Value, json};

use super::*;
use crate::app::viewer_api::{OpenFileError, failure};
use crate::direct_download::Transport;
use crate::direct_download::test_support::{
    ApiScript, GrantScript, Mock, Pair, Reply, Seen, StorageScript, Version, content, file_key,
};
use crate::engine_bridge::open_in_memory::{CAPPED_FRAME_ROOM, capped_body_len};

const KEY: [u8; 32] = [43; 32];
const STRIDE: u64 = 1024;
/// 11 chunks: ten full ones and 77 bytes.
const SIZE: usize = 10 * 1024 + 77;
const NO_LIMIT: u64 = 1 << 20;

fn bridge_on(dir: &Path, base: &str, transport: Transport) -> EngineBridge {
    let bridge = test_bridge_with_api(&dir.join("state.db"), base.to_string(), KEY);
    seed_bridge_row(
        &bridge,
        TEST_FILE_ID,
        "Invented/open.bin",
        None,
        FileStatus::CloudOnly,
        SIZE as i64,
    );
    bridge.seams.use_transport(transport);
    bridge
}

/// What the open came to, in a shape a test compares.
#[derive(Debug, PartialEq, Eq)]
enum Came {
    Opened(Vec<u8>),
    TooLarge(u64),
    /// Given up while it downloaded, with this much plaintext held.
    OverLimit(u64),
    Refused(OpenFileError),
}

async fn open(bridge: &EngineBridge, max: u64) -> Came {
    match bridge.open_in_memory(TEST_FILE_ID, max, None).await {
        Ok(InMemoryOpen::Opened(bytes)) => Came::Opened(bytes.to_vec()),
        Ok(InMemoryOpen::TooLarge { size }) => Came::TooLarge(size),
        Ok(InMemoryOpen::OverLimit { held }) => Came::OverLimit(held),
        Err(error) => Came::Refused(failure(error)),
    }
}

fn pinned_chunk_requests(pair: &Pair) -> usize {
    pair.api_chunk_requests()
        .iter()
        .filter(|seen| seen.query().contains("object_version_id="))
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pinned_file_opens_by_the_pinned_api_route() {
    let dir = tempfile::tempdir().unwrap();
    let pair = Pair::start(KEY, TEST_FILE_ID, STRIDE, SIZE);
    let bridge = bridge_on(dir.path(), &pair.api.base, Transport::Api);
    assert_eq!(open(&bridge, NO_LIMIT).await, Came::Opened(pair.plain()));
    assert_eq!(
        pinned_chunk_requests(&pair),
        11,
        "every chunk, each bound to the version"
    );
    assert_eq!(pair.api_chunk_requests().len(), 11, "and no unpinned request");
    assert_eq!(pair.storage_requests(), 0);
    // Nothing about the file is recorded.
    assert_eq!(
        bridge.db.get_file(TEST_FILE_ID).unwrap().unwrap().status,
        FileStatus::CloudOnly
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pinned_file_opens_by_the_direct_route() {
    let dir = tempfile::tempdir().unwrap();
    let pair = Pair::start(KEY, TEST_FILE_ID, STRIDE, SIZE);
    let bridge = bridge_on(dir.path(), &pair.api.base, Transport::Auto);
    assert_eq!(open(&bridge, NO_LIMIT).await, Came::Opened(pair.plain()));
    assert_eq!(pair.storage_requests(), 11, "every chunk came from storage");
    assert_eq!(pair.api_chunk_requests().len(), 0);
    assert_eq!(
        bridge.db.get_file(TEST_FILE_ID).unwrap().unwrap().status,
        FileStatus::CloudOnly
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pinned_file_over_the_limit_is_refused_before_anything_is_asked_for_on_both_routes() {
    for transport in [Transport::Api, Transport::Auto] {
        let dir = tempfile::tempdir().unwrap();
        let pair = Pair::start(KEY, TEST_FILE_ID, STRIDE, SIZE);
        let bridge = bridge_on(dir.path(), &pair.api.base, transport);
        assert_eq!(open(&bridge, SIZE as u64 - 1).await, Came::TooLarge(SIZE as u64));
        assert_eq!(pair.api_chunk_requests().len(), 0);
        assert_eq!(pair.grant_requests().len(), 0);
        assert_eq!(pair.storage_requests(), 0);
        // At the limit exactly it opens.
        assert_eq!(open(&bridge, SIZE as u64).await, Came::Opened(pair.plain()));
    }
}

fn script_chunk(pair: &Pair, index: u32, script: Vec<ApiScript>) {
    pair.knobs().api_chunks.insert(index, VecDeque::from(script));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_pinned_api_route_says_session_ended_gone_and_failed_by_kind() {
    for (status, kind) in [
        (401, OpenFileError::SessionEnded),
        (404, OpenFileError::NotFound),
        (403, OpenFileError::Failed),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let pair = Pair::start(KEY, TEST_FILE_ID, STRIDE, SIZE);
        script_chunk(&pair, 3, vec![ApiScript::Status(status)]);
        let bridge = bridge_on(dir.path(), &pair.api.base, Transport::Api);
        assert_eq!(
            open(&bridge, NO_LIMIT).await,
            Came::Refused(kind),
            "a {status} on a chunk"
        );
    }
    // A chunk that does not authenticate is a failed download, not a state of the session or the connection.
    let dir = tempfile::tempdir().unwrap();
    let pair = Pair::start(KEY, TEST_FILE_ID, STRIDE, SIZE);
    script_chunk(&pair, 2, vec![ApiScript::Tamper]);
    let bridge = bridge_on(dir.path(), &pair.api.base, Transport::Api);
    assert_eq!(open(&bridge, NO_LIMIT).await, Came::Refused(OpenFileError::Failed));
}

/// The connection drops on a chunk, on every one of the route's own tries (they wait 1 s, 2 s and 4 s).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_pinned_api_route_says_offline_when_a_chunk_gets_no_answer() {
    let dir = tempfile::tempdir().unwrap();
    let pair = Pair::start(KEY, TEST_FILE_ID, STRIDE, SIZE);
    script_chunk(&pair, 1, vec![ApiScript::HangUp; 4]);
    let bridge = bridge_on(dir.path(), &pair.api.base, Transport::Api);
    assert_eq!(open(&bridge, NO_LIMIT).await, Came::Refused(OpenFileError::Offline));
    // The route did try again before it gave up: the kind is not read from the first blip.
    let tries = pair
        .api_chunk_requests()
        .iter()
        .filter(|seen| seen.path().ends_with("/chunks/1"))
        .count();
    assert_eq!(tries, 4);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_direct_route_says_session_ended_and_gone_by_kind() {
    for (status, kind) in [(401, OpenFileError::SessionEnded), (404, OpenFileError::NotFound)] {
        let dir = tempfile::tempdir().unwrap();
        let pair = Pair::start(KEY, TEST_FILE_ID, STRIDE, SIZE);
        pair.knobs()
            .grants
            .push_back(GrantScript::Json(status, json!({"error": "scripted"})));
        let bridge = bridge_on(dir.path(), &pair.api.base, Transport::Auto);
        assert_eq!(
            open(&bridge, NO_LIMIT).await,
            Came::Refused(kind),
            "a {status} on a grant"
        );
        assert_eq!(pair.grant_requests().len(), 1, "the direct route was the one asked");
    }
}

/// Storage fails for a chunk: the open still ends with the whole file (the route's own fallback), in memory.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_direct_route_falls_back_for_a_chunk_storage_does_not_give() {
    let dir = tempfile::tempdir().unwrap();
    let pair = Pair::start(KEY, TEST_FILE_ID, STRIDE, SIZE);
    pair.knobs()
        .storage
        .insert(4, VecDeque::from(vec![StorageScript::Status(500); 8]));
    let bridge = bridge_on(dir.path(), &pair.api.base, Transport::Auto);
    assert_eq!(open(&bridge, NO_LIMIT).await, Came::Opened(pair.plain()));
    assert!(pair.storage_requests() > 0);
    assert!(
        pinned_chunk_requests(&pair) > 0,
        "the chunk came by the pinned API route"
    );
}

// ── A file whose metadata names no version ──────────────────────────────────────────────────────────

/// A loopback API for ONE file with no `download_version`, whose metadata is the test's own (it may not tell the
/// truth), and whose chunks are `version`'s frames.
fn unpinned_api(version: Version, metadata: Value) -> Mock {
    let file = format!("/api/v1/files/{TEST_FILE_ID}");
    Mock::start(Arc::new(move |req: &Seen| {
        if req.method == "GET" && req.path() == file {
            return Reply::json(200, metadata.clone());
        }
        match req
            .path()
            .strip_prefix(&format!("{file}/chunks/"))
            .and_then(|index| index.parse::<usize>().ok())
            .and_then(|index| version.frames.get(index))
        {
            Some(frame) => Reply::bytes(frame.clone()),
            None => Reply::json(404, json!({"error": "not found"})),
        }
    }))
}

/// The metadata the pair serves for a file with no version, with `edit` applied.
fn unpinned_metadata(version: &Version, edit: impl FnOnce(&mut serde_json::Map<String, Value>)) -> Value {
    let name = beebeeb_core::encrypt::encrypt_name(
        &beebeeb_core::kdf::MasterKey::from_bytes(KEY),
        TEST_FILE_ID,
        "open.bin",
        None,
    )
    .unwrap();
    let mut metadata = json!({
        "id": TEST_FILE_ID, "size_bytes": version.plain.len(), "chunk_count": version.chunk_count(),
        "chunk_size_bytes": version.stride, "name_encrypted": name, "download_version": Value::Null,
    });
    edit(metadata.as_object_mut().unwrap());
    metadata
}

fn version_of(stride: u64, size: usize) -> Version {
    Version::new(
        &file_key(KEY, TEST_FILE_ID),
        "11111111-2222-4333-8444-555555555555",
        stride,
        content(size, 11),
    )
}

fn chunk_requests(api: &Mock) -> usize {
    api.count(|seen| seen.path().contains("/chunks/"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_with_no_version_opens_by_the_api_route_alone_also_where_direct_is_offered() {
    let dir = tempfile::tempdir().unwrap();
    let pair = Pair::start(KEY, TEST_FILE_ID, STRIDE, SIZE);
    pair.knobs().legacy = true;
    // Direct is offered and the transport would take it: with a limit and no manifest it is not used.
    let bridge = bridge_on(dir.path(), &pair.api.base, Transport::Auto);
    assert_eq!(open(&bridge, SIZE as u64).await, Came::Opened(pair.plain()));
    assert_eq!(pair.api_chunk_requests().len(), 11);
    assert_eq!(pinned_chunk_requests(&pair), 0, "no pin was invented");
    assert_eq!(pair.storage_requests(), 0);
    assert_eq!(pair.grant_requests().len(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn with_no_version_a_stated_or_implied_size_over_the_limit_is_refused_before_a_chunk() {
    let version = version_of(STRIDE, SIZE);
    // Stated.
    let api = unpinned_api(version.clone(), unpinned_metadata(&version, |_| {}));
    let dir = tempfile::tempdir().unwrap();
    let bridge = bridge_on(dir.path(), &api.base, Transport::Auto);
    assert_eq!(open(&bridge, 5_000).await, Came::TooLarge(SIZE as u64));
    assert_eq!(chunk_requests(&api), 0);

    // No size stated: eleven chunks of 1024 are at least ten full ones.
    let api = unpinned_api(
        version.clone(),
        unpinned_metadata(&version, |row| {
            row.remove("size_bytes");
        }),
    );
    let dir = tempfile::tempdir().unwrap();
    let bridge = bridge_on(dir.path(), &api.base, Transport::Auto);
    assert_eq!(open(&bridge, 5_000).await, Came::TooLarge(10 * 1024));
    assert_eq!(chunk_requests(&api), 0);

    // A small size stated, against the chunk count: the larger of the two is what is held to the limit.
    let api = unpinned_api(
        version.clone(),
        unpinned_metadata(&version, |row| {
            row.insert("size_bytes".into(), json!(10));
        }),
    );
    let dir = tempfile::tempdir().unwrap();
    let bridge = bridge_on(dir.path(), &api.base, Transport::Auto);
    assert_eq!(open(&bridge, 5_000).await, Came::TooLarge(10 * 1024));
    assert_eq!(chunk_requests(&api), 0);

    // Neither a size nor a stride, and more chunks than the limit has bytes: every chunk but the last holds one
    // byte at least.
    let api = unpinned_api(
        version.clone(),
        unpinned_metadata(&version, |row| {
            row.remove("size_bytes");
            row.remove("chunk_size_bytes");
            row.insert("chunk_count".into(), json!(u32::MAX));
        }),
    );
    let dir = tempfile::tempdir().unwrap();
    let bridge = bridge_on(dir.path(), &api.base, Transport::Auto);
    assert_eq!(open(&bridge, 5_000).await, Came::TooLarge(u64::from(u32::MAX) - 1));
    assert_eq!(chunk_requests(&api), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn with_no_version_a_chunk_count_that_is_not_a_count_downloads_nothing() {
    let version = version_of(STRIDE, SIZE);
    for count in [json!(-1), json!(u64::from(u32::MAX) + 1)] {
        let api = unpinned_api(
            version.clone(),
            unpinned_metadata(&version, |row| {
                row.insert("chunk_count".into(), count.clone());
            }),
        );
        let dir = tempfile::tempdir().unwrap();
        let bridge = bridge_on(dir.path(), &api.base, Transport::Auto);
        assert_eq!(
            open(&bridge, NO_LIMIT).await,
            Came::Refused(OpenFileError::Failed),
            "{count}"
        );
        assert_eq!(chunk_requests(&api), 0);
    }
}

/// The metadata understates the file (a size of 10 bytes, no stride): the pre-check passes, and the running total
/// stops the download at the first chunk that passes the limit.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn with_no_version_an_understated_size_is_stopped_by_the_running_total() {
    let version = version_of(STRIDE, SIZE);
    let api = unpinned_api(
        version.clone(),
        unpinned_metadata(&version, |row| {
            row.insert("size_bytes".into(), json!(10));
            row.remove("chunk_size_bytes");
        }),
    );
    let dir = tempfile::tempdir().unwrap();
    let bridge = bridge_on(dir.path(), &api.base, Transport::Auto);
    // Two chunks fit (2,048 bytes); the third passes 3,000. No size is claimed for the file: only that it
    // passed the limit, with three chunks held.
    assert_eq!(open(&bridge, 3_000).await, Came::OverLimit(3 * 1024));
    assert_eq!(chunk_requests(&api), 3, "the other eight chunks were never asked for");
}

/// One chunk far longer than the limit, behind metadata that says 10 bytes: its body is given up while it streams.
/// Nothing of it was decrypted when it was given up, which only the streaming cap gives; had the body been read
/// whole and decrypted, the total's check would have held 400,000 bytes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn with_no_version_one_oversized_chunk_body_is_given_up_while_it_streams() {
    let version = version_of(400_000, 400_000);
    assert_eq!(version.chunk_count(), 1);
    let api = unpinned_api(
        version.clone(),
        unpinned_metadata(&version, |row| {
            row.insert("size_bytes".into(), json!(10));
            row.remove("chunk_size_bytes");
        }),
    );
    let dir = tempfile::tempdir().unwrap();
    let bridge = bridge_on(dir.path(), &api.base, Transport::Auto);
    assert_eq!(open(&bridge, 1_000).await, Came::OverLimit(0));
    assert_eq!(chunk_requests(&api), 1);
    // Under the limit the same file opens: the cap is not a refusal of long chunks.
    assert_eq!(open(&bridge, 400_000).await, Came::Opened(version.plain.clone()));
}

/// A pinned chunk's body breaks off part-way on every try: the connection was lost during the chunks.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_pinned_api_route_says_offline_when_a_chunk_body_breaks_off() {
    let version = version_of(STRIDE, SIZE);
    let file = format!("/api/v1/files/{TEST_FILE_ID}");
    let mut metadata = unpinned_metadata(&version, |_| {});
    metadata["download_version"] = version.manifest();
    let api = Mock::start(Arc::new(move |req: &Seen| {
        if req.method == "GET" && req.path() == file {
            return Reply::json(200, metadata.clone());
        }
        match req
            .path()
            .strip_prefix(&format!("{file}/chunks/"))
            .and_then(|index| index.parse::<usize>().ok())
        {
            // The headers and 100 bytes arrive, then nothing, then the connection closes.
            Some(2) => Reply::bytes(version.frames[2].clone()).stalling(100, std::time::Duration::from_millis(50)),
            Some(index) => Reply::bytes(version.frames[index].clone()),
            None => Reply::json(404, json!({"error": "not found"})),
        }
    }));
    let dir = tempfile::tempdir().unwrap();
    let bridge = bridge_on(dir.path(), &api.base, Transport::Api);
    assert_eq!(open(&bridge, NO_LIMIT).await, Came::Refused(OpenFileError::Offline));
    assert_eq!(
        api.count(|seen| seen.path().ends_with("/chunks/2")),
        4,
        "after the route's own tries"
    );
}

/// A chunk of a file with no version may be the old JSON envelope, which writes every byte as a number and a comma:
/// the cap on a body leaves room for the plaintext still allowed in that form, and for nothing like the file.
#[test]
fn the_cap_on_a_chunk_body_leaves_room_for_the_old_envelope_of_what_is_still_allowed() {
    const LEFT: usize = 1_000_000;
    // The longest way a byte is written: "255,".
    let envelope =
        serde_json::to_vec(&json!({ "nonce": vec![255u8; 12], "ciphertext": vec![255u8; LEFT + 16] })).unwrap();
    let cap = capped_body_len(LEFT as u64, 0);
    assert!(envelope.len() as u64 <= cap, "{} > {cap}", envelope.len());
    // And it is a bound: not twice that envelope.
    assert!(cap < 2 * envelope.len() as u64);
    // What is held counts against it; at or past the limit only a chunk's own framing is left.
    assert_eq!(capped_body_len(5_000, 4_000), capped_body_len(1_000, 0));
    assert_eq!(capped_body_len(1_000, 1_000), CAPPED_FRAME_ROOM);
    assert_eq!(capped_body_len(1_000, 9_000), CAPPED_FRAME_ROOM);
    assert_eq!(capped_body_len(u64::MAX, 0), u64::MAX);
}
