//! The media player's range reader at the engine (`Engine::open_media_for`, `MediaReader::read_at`), against the
//! engine's own doubles: an invented account on the fake API on loopback, each file's content encrypted chunk by chunk
//! by core under that account's key (`FakeAccount::add_file_with_content`, 1 KiB chunks), served as the server serves
//! it: the file's row names its current version with the manifest (`download_version`), and a chunk request pinned to
//! a version that is no longer current answers 409. A state database of each test's own (`use_local_state_for_test`),
//! recorded as the account's. Sessions are put in memory as a sign-in and an unlock leave them. Every name, key and
//! byte is invented.
//!
//! The limits are the reader's own fields: a test sets smaller ones where the shipped ones are out of its reach (with
//! 1 KiB chunks, 32 MiB of cache is 32 768 chunk requests), and sets the read-ahead to 0 where it counts requests.
//! The cache against real chunk strides is read over a counting fetch alone (`MediaReader::over_fetch`: no server,
//! no crypto), and once, ignored, against the fake server with a real 1 GiB file (MCP2's real-stride run).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;
use zeroize::Zeroizing;

use super::{
    Limits, MEDIA_CACHE_CEILING, MEDIA_CACHE_MAX, MEDIA_MAX_READ, MEDIA_REFETCH_SLACK, MEDIA_TWO_STRIDES_FROM,
    MediaOrigin, MediaReadError, MediaReader, WIRE_OVERHEAD,
};
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::app::{Engine, EngineConfig};
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use crate::test_api::fake_account::{CONTENT_CHUNK_BYTES, FakeAccount, TEST_PHRASE, account_handler};
use crate::test_api::{FakeApi, Handler, Reply, Request};

type TestEngine = Engine<MemoryCredentials>;

/// A short film: three whole chunks and a part of a fourth.
const CLIP: &str = "a0000000-0000-4000-8000-000000000601";
/// Longer than one read may be.
const BIG: &str = "a0000000-0000-4000-8000-000000000602";
/// Forty chunks.
const LONG: &str = "a0000000-0000-4000-8000-000000000603";
/// A real 1 GiB file at the desktop ladder's stride for its size (32 MiB): the ignored real-stride run only.
const FILM: &str = "a0000000-0000-4000-8000-000000000604";

const STRIDE: u64 = CONTENT_CHUNK_BYTES as u64;
/// The server's own sentence for a pinned version that is gone.
const GONE: &str = "download version unavailable; restart download from fresh metadata";

fn clip() -> Vec<u8> {
    (0..3_500u32).map(|i| (i % 251) as u8).collect()
}

/// An equal-size other version of the clip, no byte of it at the same place the same.
fn replacement() -> Vec<u8> {
    clip().iter().map(|b| b ^ 0x5a).collect()
}

fn big() -> Vec<u8> {
    (0..(4u32 << 20) + 1_024).map(|i| (i % 239) as u8).collect()
}

fn long_clip() -> Vec<u8> {
    (0..40 * 1_024u32).map(|i| (i % 241) as u8).collect()
}

/// The object id of version `n` of a file.
fn version_id(n: u32) -> String {
    format!("b0000000-0000-4000-8000-{n:012}")
}

/// The manifest the server names for a version whose plaintext is `plain`: one entry per 1 KiB chunk, each the
/// chunk's plaintext plus core's per-chunk overhead on the wire.
fn manifest(version: &str, plain: &[u8]) -> serde_json::Value {
    manifest_of(version, plain.len() as u64, STRIDE)
}

/// The manifest of a version of `len` plaintext bytes in chunks of `stride` (an empty file is one empty chunk).
fn manifest_of(version: &str, len: u64, stride: u64) -> serde_json::Value {
    let count = len.div_ceil(stride).max(1);
    let chunks: Vec<serde_json::Value> = (0..count)
        .map(|index| {
            let piece = len.saturating_sub(index * stride).min(stride);
            json!({ "index": index, "size_bytes": piece + WIRE_OVERHEAD })
        })
        .collect();
    json!({
        "object_version_id": version,
        "chunk_count": count,
        "chunk_size_bytes": stride,
        "plaintext_size_bytes": len,
        "ciphertext_size_bytes": len + WIRE_OVERHEAD * count,
        "chunks": chunks,
    })
}

struct Fake {
    api: FakeApi,
    account: Arc<FakeAccount>,
    /// Each file's current version: its object id and its plaintext (the manifest is built from it).
    current: Arc<Mutex<HashMap<String, (String, Vec<u8>)>>>,
}

impl Fake {
    /// `plain` becomes version `version` of the file `id`, and the version before it (if any) is not kept: a chunk
    /// request pinned to it answers 409 from now on.
    fn put(&self, id: &str, name: &str, plain: &[u8], version: u32) {
        self.account.add_file_with_content(id, None, name, plain);
        self.current
            .lock()
            .unwrap()
            .insert(id.to_owned(), (version_id(version), plain.to_vec()));
    }

    /// The chunk indexes asked of the file `id`, in order.
    fn chunks_asked(&self, id: &str) -> Vec<u32> {
        let prefix = format!("/api/v1/files/{id}/chunks/");
        self.api
            .requests()
            .iter()
            .filter(|req| req.method == "GET")
            .filter_map(|req| req.path.split('?').next()?.strip_prefix(prefix.as_str())?.parse().ok())
            .collect()
    }
}

fn fake(tag: &str) -> Fake {
    let (account, _) = FakeAccount::register(&format!("{tag}@example.eu"), "an invented password", TEST_PHRASE);
    let account = Arc::new(account);
    let current: Arc<Mutex<HashMap<String, (String, Vec<u8>)>>> = Arc::new(Mutex::new(HashMap::new()));
    let rest: Handler = account_handler(account.clone());
    let current_in = current.clone();
    let api = FakeApi::start(Arc::new(move |req: &Request| {
        let route = req.route();
        let Some(path) = route.strip_prefix("GET /api/v1/files/") else {
            return rest(req);
        };
        let (id, chunk) = match path.split_once("/chunks/") {
            Some((id, _)) => (id, true),
            None => (path, false),
        };
        let Some((version, plain)) = current_in.lock().unwrap().get(id).cloned() else {
            return rest(req);
        };
        if chunk {
            // A pinned request is served only for the current version.
            return match req.query("object_version_id") {
                Some(asked) if !asked.eq_ignore_ascii_case(&version) => Reply::error(409, GONE),
                _ => rest(req),
            };
        }
        let reply = rest(req);
        if reply.status != 200 {
            return reply;
        }
        let mut row: serde_json::Value = serde_json::from_slice(&reply.body).expect("the fake's row");
        row["download_version"] = manifest(&version, &plain);
        Reply::ok(row)
    }));
    Fake { api, account, current }
}

fn engine_at(base: &str) -> Arc<TestEngine> {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-player-slot".into(),
        state_dir,
        api_base: base.to_string(),
    };
    Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the handle opens"))
}

/// A session in memory, as a sign-in and an unlock of `account` leave it.
fn signed_in(engine: &TestEngine, account: &FakeAccount) {
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

fn row(db: &StateDb, id: &str, path: &str, size: i64) {
    let entry = FileEntry {
        file_id: id.into(),
        path: path.into(),
        status: FileStatus::CloudOnly,
        size_bytes: size,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: None,
        item_kind: ItemKind::File,
    };
    db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
        .unwrap();
}

/// A state database recorded as `owner`'s, with the three files' rows (online only), and the engine reading it.
fn local_state(engine: &TestEngine, owner: &FakeAccount) -> (tempfile::TempDir, StateDb) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    row(&db, CLIP, "Invented folder/Invented clip.mp4", 3_500);
    row(&db, BIG, "Invented long film.mkv", big().len() as i64);
    row(
        &db,
        LONG,
        "Invented folder/Invented forty.mp3",
        long_clip().len() as i64,
    );
    db.set_owner(&Identity::new(Some(&owner.user_id), Some(&owner.email)))
        .unwrap();
    engine.use_local_state_for_test(path);
    (dir, db)
}

/// The usual start: the clip at version 1, signed in, the local state in place.
fn clip_setup(tag: &str) -> (Fake, Arc<TestEngine>, (tempfile::TempDir, StateDb)) {
    let fake = fake(tag);
    fake.put(CLIP, "Invented clip.mp4", &clip(), 1);
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.account);
    let state = local_state(&engine, &fake.account);
    (fake, engine, state)
}

fn open(engine: &Arc<TestEngine>, id: &str) -> MediaReader {
    engine
        .open_media_for(engine.session_turn(), id)
        .expect("the file opens for playback")
}

/// The same reader with no read-ahead (a test that counts requests).
fn quiet(mut reader: MediaReader) -> MediaReader {
    reader.limits.read_ahead = 0;
    reader
}

fn read(reader: &mut MediaReader, at: u64, len: u32) -> Result<Zeroizing<Vec<u8>>, MediaReadError> {
    reader.read_at(at, len, &AtomicBool::new(false))
}

fn span(at: u64, len: u32) -> std::ops::Range<usize> {
    at as usize..at as usize + len as usize
}

/// Every file and folder below `dir`, by its path below it.
fn files_below(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(at) = stack.pop() {
        for entry in std::fs::read_dir(&at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path.clone());
            }
            found.push(path.strip_prefix(dir).unwrap().to_path_buf());
        }
    }
    found.sort();
    found
}

// ── 1. The bytes ─────────────────────────────────────────────────────────────────────────────────────────────────

/// Ranges inside one chunk, across two, the last byte and the whole file are the plaintext at those places. The
/// read-ahead is on (the reader as the player gets it).
#[test]
fn media_read_returns_the_plaintext_range() {
    let (_fake, engine, _state) = clip_setup("bytes");
    let mut reader = open(&engine, CLIP);
    assert_eq!(reader.len(), 3_500);
    assert_eq!(reader.origin(), MediaOrigin::Beebeeb);
    let content = clip();
    for (at, len) in [
        (10u64, 100u32),
        (1_000, 100),
        (3_499, 1),
        (0, 3_500),
        (2_048, 1_452),
        (1_023, 2),
    ] {
        let got = read(&mut reader, at, len).expect("a range inside the file");
        assert_eq!(got.len(), len as usize);
        assert!(got[..] == content[span(at, len)], "the plaintext of {at}..+{len}");
    }
    // A `{:?}` of the reader prints no id and no byte.
    let printed = format!("{reader:?}");
    assert!(!printed.contains(CLIP) && printed.contains("3500"), "{printed}");
}

// ── 2. Only the covering chunks ──────────────────────────────────────────────────────────────────────────────────

#[test]
fn media_read_mid_file_fetches_only_covering_chunks() {
    let (fake, engine, _state) = clip_setup("covering");
    let mut reader = quiet(open(&engine, CLIP));
    assert_eq!(
        fake.chunks_asked(CLIP),
        Vec::<u32>::new(),
        "the open reads the metadata only"
    );
    let got = read(&mut reader, 1_500, 1_000).expect("chunks 1 and 2");
    assert!(got[..] == clip()[1_500..2_500]);
    assert_eq!(
        fake.chunks_asked(CLIP),
        vec![1, 2],
        "the covering chunks only: no chunk 0 first, no chunk after"
    );
    let got = read(&mut reader, 2_100, 50).expect("inside a held chunk");
    assert!(got[..] == clip()[2_100..2_150]);
    assert_eq!(
        fake.chunks_asked(CLIP),
        vec![1, 2],
        "a range in a held chunk asks for nothing"
    );
    // Every chunk request names the version the open pinned.
    let pinned = version_id(1);
    let requests = fake.api.requests();
    let chunk_requests: Vec<&Request> = requests.iter().filter(|req| req.path.contains("/chunks/")).collect();
    assert!(
        chunk_requests.iter().all(|req| req
            .query("object_version_id")
            .is_some_and(|v| v.eq_ignore_ascii_case(&pinned))),
        "every chunk request is pinned"
    );
}

// ── 3. A lock ────────────────────────────────────────────────────────────────────────────────────────────────────

/// Mutants: (a) the session question in `read_at` deleted → the first read after the lock answers `OutOfRange`;
/// (b) the wipe in `Shared::end` skipped → the cache still holds chunks. Both red here.
#[test]
fn media_read_after_lock_is_refused() {
    let (fake, engine, _state) = clip_setup("lock");
    let mut reader = quiet(open(&engine, CLIP));
    let content = clip();
    assert!(read(&mut reader, 0, 100).expect("chunk 0")[..] == content[..100]);
    assert!(read(&mut reader, 1_500, 100).expect("chunk 1")[..] == content[1_500..1_600]);
    assert!(
        !reader.shared.lock().chunks.is_empty(),
        "precondition: the reader holds decrypted chunks"
    );
    let asked = fake.chunks_asked(CLIP).len();

    engine.forget_session(SessionTransition::Lock);

    // The session is asked first, before the range and the cache: even a range outside the file answers that.
    let len = reader.len();
    assert_eq!(read(&mut reader, len, 1).err(), Some(MediaReadError::NotSignedIn));
    {
        let cache = reader.shared.lock();
        assert!(
            cache.chunks.is_empty() && cache.held == 0,
            "what the reader held is wiped at the change"
        );
    }
    // Every later read too, of a range it held before.
    assert_eq!(read(&mut reader, 0, 100).err(), Some(MediaReadError::NotSignedIn));
    // The same account signed in again: still nothing (the reader is for the session it was opened for).
    signed_in(&engine, &fake.account);
    assert_eq!(read(&mut reader, 0, 100).err(), Some(MediaReadError::NotSignedIn));
    assert_eq!(
        fake.chunks_asked(CLIP).len(),
        asked,
        "nothing was asked for after the lock"
    );
}

// ── 4. The cache's bound ─────────────────────────────────────────────────────────────────────────────────────────

/// Reads over three times the cache's cap of distinct chunks (forty chunks against a cap of four) never hold more
/// than the cap, and chunk 0 stays (it is a quarter of the cap: pinned).
#[test]
fn media_cache_is_bounded() {
    let fake = fake("bounded");
    fake.put(LONG, "Invented forty.mp3", &long_clip(), 1);
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.account);
    let _state = local_state(&engine, &fake.account);
    let mut reader = open(&engine, LONG);
    let cap = 4 * STRIDE;
    reader.limits = Limits {
        cache_floor: cap,
        cache_ceiling: MEDIA_CACHE_CEILING,
        two_strides_from: MEDIA_TWO_STRIDES_FROM,
        read_ahead: 0,
        refetch_slack: MEDIA_REFETCH_SLACK,
    };
    assert_eq!(reader.limits.cache_cap(STRIDE), cap);
    let content = long_clip();
    let mut distinct = 0u64;
    for index in (0..40u64).chain((0..40u64).rev()) {
        let at = index * STRIDE + 7;
        let got = read(&mut reader, at, 100).expect("a range inside the file");
        assert!(got[..] == content[span(at, 100)], "chunk {index}");
        let cache = reader.shared.lock();
        let real: u64 = cache.chunks.values().map(|chunk| chunk.len() as u64).sum();
        assert!(cache.held <= cap && real == cache.held, "held {} of {cap}", cache.held);
        assert!(cache.chunks.contains_key(&0), "chunk 0 is never evicted");
        distinct += STRIDE;
    }
    assert!(
        distinct > 3 * cap,
        "precondition: more than three caps of chunks were read"
    );
}

// ── 5. Bounds ────────────────────────────────────────────────────────────────────────────────────────────────────

/// Mutant (d): `offset + len <= len()` loosened by one → `3_490 + 11` and `3_501 + 0` are not refused. Red here.
#[test]
fn media_read_past_end_refused() {
    let (fake, engine, _state) = clip_setup("past-end");
    let mut reader = quiet(open(&engine, CLIP));
    let content = clip();
    assert!(read(&mut reader, 3_490, 10).expect("the last ten bytes")[..] == content[3_490..]);
    assert_eq!(read(&mut reader, 3_500, 0).expect("nothing, at the end").len(), 0);
    assert_eq!(read(&mut reader, 3_490, 11).err(), Some(MediaReadError::OutOfRange));
    assert_eq!(read(&mut reader, 3_501, 0).err(), Some(MediaReadError::OutOfRange));
    assert_eq!(read(&mut reader, u64::MAX, 1).err(), Some(MediaReadError::OutOfRange));
    assert_eq!(
        read(&mut reader, u64::MAX - 5, MEDIA_MAX_READ).err(),
        Some(MediaReadError::OutOfRange)
    );
    // A refusal is not the end of the reader.
    assert!(read(&mut reader, 0, 10).expect("the start")[..] == content[..10]);
    assert_eq!(fake.chunks_asked(CLIP), vec![3, 0], "a refused range asks for nothing");
}

#[test]
fn media_read_over_max_refused() {
    let fake = fake("over-max");
    fake.put(BIG, "Invented long film.mkv", &big(), 1);
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.account);
    let _state = local_state(&engine, &fake.account);
    let mut reader = quiet(open(&engine, BIG));
    assert!(
        u64::from(MEDIA_MAX_READ) + 1 < reader.len(),
        "precondition: the range is inside the file"
    );
    assert_eq!(
        read(&mut reader, 0, MEDIA_MAX_READ + 1).err(),
        Some(MediaReadError::OutOfRange)
    );
    assert_eq!(fake.chunks_asked(BIG).len(), 0, "refused before anything was asked for");
    let got = read(&mut reader, 5_000, 10).expect("a short range of the same file");
    assert!(got[..] == big()[5_000..5_010]);
}

// ── 6. The budget ────────────────────────────────────────────────────────────────────────────────────────────────

/// A seek loop over two chunks with a cache that holds one: every read is a fetch. The budget is twice the file
/// (no slack here): 7 000 wire bytes, each chunk 1 024 + 28, so six fetches pass and the seventh is refused before
/// it is asked for. Mutant (c): the budget check removed → twenty reads pass. Red here.
#[test]
fn media_refetch_budget_stops() {
    let (fake, engine, _state) = clip_setup("budget");
    let mut reader = open(&engine, CLIP);
    // A cap of one chunk: the ceiling (three strides would pass it).
    reader.limits = Limits {
        cache_floor: STRIDE,
        cache_ceiling: STRIDE,
        two_strides_from: MEDIA_TWO_STRIDES_FROM,
        read_ahead: 0,
        refetch_slack: 0,
    };
    let answers: Vec<Option<MediaReadError>> = (0..20u64)
        .map(|round| {
            let at = if round % 2 == 0 { STRIDE } else { 2 * STRIDE };
            read(&mut reader, at, 10).err()
        })
        .collect();
    let first_refusal = answers
        .iter()
        .position(Option::is_some)
        .expect("the budget ends the seek loop");
    assert_eq!(first_refusal, 6, "{answers:?}");
    assert!(
        answers[first_refusal..]
            .iter()
            .all(|answer| *answer == Some(MediaReadError::OverBudget)),
        "{answers:?}"
    );
    assert_eq!(
        fake.chunks_asked(CLIP).len(),
        6,
        "the refused fetch was never asked for"
    );
    // Every later call too, whatever it asks.
    assert_eq!(read(&mut reader, 0, 1).err(), Some(MediaReadError::OverBudget));
}

// ── 7. One version ───────────────────────────────────────────────────────────────────────────────────────────────

/// Another device replaces the file with an equal-size version mid-play, and the version opened is not kept: every
/// answer is the opened version's bytes or `Gone`, never the new version's, never a mix.
#[test]
fn media_reads_one_pinned_version() {
    let (fake, engine, _state) = clip_setup("replace");
    let mut reader = quiet(open(&engine, CLIP));
    let (old, new) = (clip(), replacement());
    assert!(read(&mut reader, 0, 1_024).expect("chunk 0 of the version opened")[..] == old[..1_024]);

    fake.put(CLIP, "Invented clip.mp4", &new, 2);

    let mut gone = 0;
    for (at, len) in [(2_048u64, 1_024u32), (0, 1_024), (1_000, 2_000), (3_000, 500)] {
        match read(&mut reader, at, len) {
            Ok(bytes) => {
                assert!(bytes[..] == old[span(at, len)], "MIXED or new bytes at {at}..+{len}");
                assert!(bytes[..] != new[span(at, len)]);
            }
            Err(why) => {
                assert_eq!(why, MediaReadError::Gone, "at {at}..+{len}");
                gone += 1;
            }
        }
    }
    assert!(gone > 0, "precondition: the replaced version was asked for");
}

// ── 8. Nothing written ───────────────────────────────────────────────────────────────────────────────────────────

/// Ten reads leave the local state's folder and the sync folder as they were (no file, no temp file, no cache file)
/// and the file's row as it was. (The engine's own state dir is one for every test of the process, written by the
/// others while this one runs: it is not listed here.)
#[test]
fn media_read_writes_no_file() {
    let (fake, engine, (state_dir, db)) = clip_setup("no-file");
    let root = tempfile::tempdir().unwrap();
    engine.use_sync_root_for_test(root.path().to_path_buf());
    let mut reader = quiet(open(&engine, CLIP));
    let row_before = db.get_file(CLIP).unwrap().unwrap();
    let contract_before = db.get_file_contract_state(CLIP).unwrap();
    let state_before = files_below(state_dir.path());
    let content = clip();
    for round in 0..10u64 {
        let at = (round * 337) % 3_000;
        assert!(read(&mut reader, at, 400).expect("a range")[..] == content[span(at, 400)]);
    }
    assert!(
        !fake.chunks_asked(CLIP).is_empty(),
        "precondition: chunks were downloaded"
    );
    assert_eq!(
        files_below(state_dir.path()),
        state_before,
        "no file in the local state's folder"
    );
    assert_eq!(
        files_below(root.path()),
        Vec::<PathBuf>::new(),
        "nothing in the sync folder"
    );
    let row_after = db.get_file(CLIP).unwrap().unwrap();
    assert_eq!(row_after.status, FileStatus::CloudOnly);
    assert_eq!(
        (row_after.status, row_after.size_bytes),
        (row_before.status, row_before.size_bytes)
    );
    assert_eq!(db.get_file_contract_state(CLIP).unwrap(), contract_before);
    assert_eq!(db.kept_rows().unwrap().len(), 0, "nothing is recorded as kept");
}

// ── 9. The copy on this PC ───────────────────────────────────────────────────────────────────────────────────────

/// The account's folder on this PC for a test, with the clip kept there (its row `status`, its bytes on disk).
fn kept(engine: &TestEngine, db: &StateDb, status: FileStatus) -> tempfile::TempDir {
    db.set_status(CLIP, status).unwrap();
    let root = tempfile::tempdir().unwrap();
    engine.use_sync_root_for_test(root.path().to_path_buf());
    let folder = root.path().join("Invented folder");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("Invented clip.mp4"), clip()).unwrap();
    root
}

#[test]
fn media_local_copy_is_read_without_download() {
    let (fake, engine, (_dir, db)) = clip_setup("kept");
    let _root = kept(&engine, &db, FileStatus::Local);
    let mut reader = open(&engine, CLIP);
    assert_eq!(reader.origin(), MediaOrigin::ThisPc);
    assert_eq!(reader.len(), 3_500);
    let content = clip();
    for (at, len) in [(10u64, 100u32), (1_000, 100), (3_499, 1), (0, 3_500)] {
        assert!(read(&mut reader, at, len).expect("from the copy")[..] == content[span(at, len)]);
    }
    assert_eq!(fake.chunks_asked(CLIP).len(), 0, "nothing was downloaded");
    assert_eq!(reader.origin(), MediaOrigin::ThisPc, "every read came from the copy");
}

/// The copy goes away mid-play: the reader goes on with Beebeeb's chunks of the version the copy was. A copy with a
/// change that is not in Beebeeb yet never does (Beebeeb's version is other content): `Failed`.
#[test]
fn media_local_copy_falls_back_only_to_its_own_version() {
    let (fake, engine, (_dir, db)) = clip_setup("fallback");
    let root = kept(&engine, &db, FileStatus::Local);
    let mut contract = db.get_file_contract_state(CLIP).unwrap().unwrap();
    contract.current_object_version_id = Some(version_id(1));
    db.set_file_contract_state(&contract).unwrap();
    let mut reader = quiet(open(&engine, CLIP));
    let content = clip();
    assert!(read(&mut reader, 0, 100).expect("from the copy")[..] == content[..100]);
    assert_eq!(reader.origin(), MediaOrigin::ThisPc);

    std::fs::remove_file(root.path().join("Invented folder").join("Invented clip.mp4")).unwrap();
    assert!(read(&mut reader, 1_500, 100).expect("from Beebeeb")[..] == content[1_500..1_600]);
    assert_eq!(reader.origin(), MediaOrigin::Beebeeb);
    assert_eq!(fake.chunks_asked(CLIP), vec![1]);

    // A copy with a change not sent: never Beebeeb's version in its place.
    let root = kept(&engine, &db, FileStatus::Uploading);
    let mut reader = quiet(open(&engine, CLIP));
    assert_eq!(reader.origin(), MediaOrigin::ThisPc);
    std::fs::remove_file(root.path().join("Invented folder").join("Invented clip.mp4")).unwrap();
    assert_eq!(read(&mut reader, 1_500, 100).err(), Some(MediaReadError::Failed));
    assert_eq!(fake.chunks_asked(CLIP), vec![1], "nothing more was downloaded");
}

// ── 10. An idle reader at a lock ─────────────────────────────────────────────────────────────────────────────────

/// A paused player: one read, then none. A lock wipes what the reader holds within a second, with no read to do it
/// (a read wipes by itself, so this test reads again only once the cache is seen empty). The read-ahead is off: the
/// worker watches the session for every reader. Mutant (f): the worker's session question deleted → red here.
#[test]
fn media_idle_reader_is_wiped_at_a_lock() {
    let (_fake, engine, _state) = clip_setup("idle");
    let mut reader = quiet(open(&engine, CLIP));
    assert!(read(&mut reader, 0, 100).expect("chunk 0")[..] == clip()[..100]);
    assert!(
        reader.cached_bytes() > 0,
        "precondition: the reader holds a decrypted chunk"
    );

    engine.forget_session(SessionTransition::Lock);

    let deadline = Instant::now() + Duration::from_secs(1);
    while reader.cached_bytes() > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        reader.cached_bytes(),
        0,
        "the idle reader's plaintext is wiped within a second of the lock"
    );
    assert!(reader.shared.lock().chunks.is_empty());
    assert_eq!(read(&mut reader, 0, 100).err(), Some(MediaReadError::NotSignedIn));
}

// ── 11. The cache against real chunk strides ─────────────────────────────────────────────────────────────────────

/// Fetches per chunk index, counted as each is asked for.
type Fetches = Arc<Mutex<HashMap<u32, u32>>>;

/// A reader of `len` bytes in chunks of `stride` over a counting fetch: chunk `i` is the byte `i as u8` throughout.
fn counting(len: u64, stride: u64, limits: Limits) -> (MediaReader, Fetches) {
    let fetches: Fetches = Arc::new(Mutex::new(HashMap::new()));
    let counted = fetches.clone();
    let fetch = move |index: u32, _stop: &dyn Fn() -> bool| -> Result<Zeroizing<Vec<u8>>, MediaReadError> {
        *counted.lock().unwrap().entry(index).or_insert(0) += 1;
        let start = u64::from(index) * stride;
        let size = usize::try_from(len.saturating_sub(start).min(stride)).expect("a test chunk's size");
        Ok(Zeroizing::new(vec![index as u8; size]))
    };
    (MediaReader::over_fetch(len, stride, limits, Box::new(fetch)), fetches)
}

/// Waits (five seconds at most) until the read-ahead has fetched what the last read asked of it: a play that lets
/// the worker keep up, so a count of fetches does not hang on the threads' timing.
fn settle(reader: &MediaReader) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !reader.read_ahead_idle() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// The chunks fetched more than once, as `(index, fetches)`, in index order.
fn fetched_twice(fetches: &Fetches) -> Vec<(u32, u32)> {
    let mut twice: Vec<(u32, u32)> = fetches
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, n)| **n > 1)
        .map(|(index, n)| (*index, *n))
        .collect();
    twice.sort_unstable();
    twice
}

/// The lead's test at its real numbers: a 1 GiB file in chunks of 32 MiB (the desktop ladder's stride for it), 200
/// reads of the longest read in a row, the shipped limits, the read-ahead on. Every chunk is fetched once, no read
/// passes the budget, and the cache holds the next chunk beside the one played. Red at the base: a 32 MiB cache
/// holds one such chunk, so every read fetches its chunk again. Mutant (e): the cap → `MEDIA_CACHE_MAX` → red here.
#[test]
fn media_sequential_play_at_real_stride_fetches_each_chunk_once() {
    const LEN: u64 = 1 << 30;
    const CHUNK: u64 = 32 << 20;
    let (mut reader, fetches) = counting(LEN, CHUNK, Limits::SHIPPED);
    let step = u64::from(MEDIA_MAX_READ);
    for round in 0..200u64 {
        let at = round * step;
        let answer = read(&mut reader, at, MEDIA_MAX_READ);
        assert_ne!(
            answer.as_ref().err(),
            Some(&MediaReadError::OverBudget),
            "read {round} passed the budget"
        );
        let got = answer.unwrap_or_else(|why| panic!("read {round}: {why:?}"));
        let index = (at / CHUNK) as u8;
        assert!(
            got.len() == step as usize && got[0] == index && got[got.len() - 1] == index,
            "read {round}: the bytes of chunk {index}"
        );
        settle(&reader);
    }
    let held = reader.cached_bytes();
    assert!(
        (2 * CHUNK..=3 * CHUNK).contains(&held),
        "the chunk played and the next one are held: {held} bytes"
    );
    drop(reader);
    assert_eq!(fetched_twice(&fetches), Vec::new(), "chunks fetched more than once");
    let crossed = fetches.lock().unwrap().len();
    assert!(crossed >= 25, "precondition: the play crossed {crossed} chunks");
}

/// A stride whose three chunks pass the ceiling (the cap is the ceiling, 2 500) and a chunk 0 of more than a quarter
/// of it (1 000): chunk 0 is not pinned, sequential play evicts it, and every later chunk is still fetched once.
/// Mutant (g): chunk 0 pinned always → chunk 0 stays (and the chunk played is evicted) → red here.
#[test]
fn media_large_first_chunk_is_not_pinned() {
    const CHUNK: u64 = 1_000;
    const LEN: u64 = 10 * CHUNK;
    let limits = Limits {
        cache_floor: 100,
        cache_ceiling: 2_500,
        two_strides_from: MEDIA_TWO_STRIDES_FROM,
        read_ahead: 300,
        refetch_slack: 0,
    };
    assert_eq!(limits.cache_cap(CHUNK), 2_500, "precondition: the cap is the ceiling");
    let (mut reader, fetches) = counting(LEN, CHUNK, limits);
    for round in 0..40u64 {
        let at = round * 250;
        let got = read(&mut reader, at, 250).expect("a range inside the file");
        let index = (at / CHUNK) as u8;
        assert!(
            got.iter().all(|byte| *byte == index),
            "read {round}: the bytes of chunk {index}"
        );
        settle(&reader);
        let held = reader.cached_bytes();
        assert!(held <= 2_500, "read {round}: held {held} of 2 500");
    }
    assert!(
        !reader.shared.lock().chunks.contains_key(&0),
        "chunk 0 was evicted once the play went past it"
    );
    drop(reader);
    let expected: HashMap<u32, u32> = (0..10).map(|index| (index, 1)).collect();
    assert_eq!(*fetches.lock().unwrap(), expected, "every chunk fetched once");
}

/// MCP2's real-stride run (`--ignored`, on the testrig): a real 1 GiB file in chunks of 32 MiB, encrypted by core under
/// the account's key and served by the fake as the server serves it (its version named, every chunk request pinned),
/// played from its first byte to its last with the shipped limits and the read-ahead on, at the threads' own pace.
/// Prints the fetches of every chunk index; each is asked for once.
#[test]
#[ignore = "MCP2's real-stride run: a real 1 GiB file (about 1.5 GiB of memory); run with --ignored on the testrig"]
fn media_real_stride_run_counts_fetches_per_chunk() {
    use beebeeb_core::chunk_stream::{ChunkEncryptor, ExpectedChunkFormat};
    const LEN: u64 = 1 << 30;
    const CHUNK: u64 = 32 << 20;
    let byte_at = |position: u64| (position % 253) as u8;

    let (account, _) = FakeAccount::register("film@example.eu", "an invented password", TEST_PHRASE);
    let account = Arc::new(account);
    account.add_item(FILM, None, "Invented film.mkv", false, LEN);
    // As an upload of this account stores it (`FakeAccount::add_file_with_content`, at the real stride).
    let master = beebeeb_core::kdf::MasterKey::from_bytes(account.master_key);
    let file_key = beebeeb_core::kdf::derive_file_key(&master, FILM.as_bytes());
    let mut encryptor = ChunkEncryptor::for_push_with_file_key(file_key, FILM, LEN, CHUNK)
        .expect("a chunk encryptor")
        .with_recorded_pin(ExpectedChunkFormat::Unrecorded)
        .expect("the format of the name");
    let frames: Vec<Vec<u8>> = (0..LEN.div_ceil(CHUNK))
        .map(|index| {
            let start = index * CHUNK;
            let plain: Vec<u8> = (start..(start + CHUNK).min(LEN)).map(byte_at).collect();
            encryptor.push_chunk(&plain).expect("an encrypted chunk").data
        })
        .collect();
    let version = encryptor
        .version_commitment_after(None)
        .expect("the upload's commitment");
    encryptor.finish().expect("the last chunk");
    let name_key = beebeeb_core::kdf::derive_file_key(&master, FILM.as_bytes());
    let name_encrypted = beebeeb_core::encrypt::encrypt_name_for_version(
        &name_key,
        "Invented film.mkv",
        None,
        ExpectedChunkFormat::Unrecorded,
        version,
    )
    .expect("the completion name");

    let rest: Handler = account_handler(account.clone());
    let token = account.session_token.clone();
    let frames = Arc::new(frames);
    let api = FakeApi::start(Arc::new(move |req: &Request| {
        let route = req.route();
        let Some(path) = route.strip_prefix("GET /api/v1/files/") else {
            return rest(req);
        };
        let (id, chunk) = match path.split_once("/chunks/") {
            Some((id, index)) => (id, Some(index)),
            None => (path, None),
        };
        if id != FILM {
            return rest(req);
        }
        if req.bearer() != Some(token.as_str()) {
            return Reply::error(401, "unauthorized");
        }
        let Some(index) = chunk else {
            let reply = rest(req);
            if reply.status != 200 {
                return reply;
            }
            let mut row: serde_json::Value = serde_json::from_slice(&reply.body).expect("the fake's row");
            row["name_encrypted"] = json!(name_encrypted);
            row["chunk_count"] = json!(frames.len());
            row["chunk_size_bytes"] = json!(CHUNK);
            row["download_version"] = manifest_of(&version_id(1), LEN, CHUNK);
            return Reply::ok(row);
        };
        if req
            .query("object_version_id")
            .is_some_and(|asked| !asked.eq_ignore_ascii_case(&version_id(1)))
        {
            return Reply::error(409, GONE);
        }
        match index.parse::<usize>().ok().and_then(|index| frames.get(index)) {
            Some(frame) => Reply::bytes(200, frame.clone()),
            None => Reply::error(404, "not_found"),
        }
    }));
    let fake = Fake {
        api,
        account,
        current: Arc::new(Mutex::new(HashMap::new())),
    };
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.account);
    let _state = local_state(&engine, &fake.account);

    let mut reader = open(&engine, FILM);
    assert_eq!((reader.len(), reader.origin()), (LEN, MediaOrigin::Beebeeb));
    let step = u64::from(MEDIA_MAX_READ);
    for round in 0..LEN / step {
        let at = round * step;
        let got = read(&mut reader, at, MEDIA_MAX_READ).unwrap_or_else(|why| panic!("read {round}: {why:?}"));
        assert!(
            got.len() == step as usize && got[0] == byte_at(at) && got[got.len() - 1] == byte_at(at + step - 1),
            "read {round}: the plaintext at its place"
        );
    }
    drop(reader);

    let mut per_index: HashMap<u32, u32> = HashMap::new();
    for index in fake.chunks_asked(FILM) {
        *per_index.entry(index).or_insert(0) += 1;
    }
    let count = u32::try_from(LEN.div_ceil(CHUNK)).unwrap();
    for index in 0..count {
        println!("chunk {index}: {} fetches", per_index.get(&index).copied().unwrap_or(0));
    }
    assert!(
        (0..count).all(|index| per_index.contains_key(&index)),
        "precondition: every chunk was asked for"
    );
    assert!(
        per_index.values().all(|fetches| *fetches <= 1),
        "a chunk fetched more than once: {per_index:?}"
    );
}

// ── 12. Core's largest stride ────────────────────────────────────────────────────────────────────────────────────

/// Row 9 (lead ruling 22:12): core's largest stride (256 MiB, for files over 4 GiB) holds the chunk played AND the
/// next one, whatever the ceiling (384 MiB), so a play finds the next chunk read ahead at every boundary instead of
/// fetching it there. The play runs at the shipped limits scaled down by 1 024 (a 256 KiB stride, every limit and the
/// read size divided by 1 024, a file of 16½ chunks: "over 4 GiB"); the shipped numbers are asserted on the cap itself
/// (two real 256 MiB chunks are not allocated in a unit test). Red at the base: the cap was the ceiling, less than two
/// chunks, so nothing was read ahead and every boundary was a fetch by the read itself. Mutant (row 9): the
/// two-strides clause in `Limits::cache_cap` removed → red here (boundary 1).
#[test]
fn media_largest_stride_has_the_next_chunk_ready_at_each_boundary() {
    const SCALE: u32 = 10;
    const CHUNK: u64 = (256 << 20) >> SCALE;
    const LEN: u64 = 16 * CHUNK + CHUNK / 2;
    let shipped = Limits::SHIPPED;
    let scaled = Limits {
        cache_floor: shipped.cache_floor >> SCALE,
        cache_ceiling: shipped.cache_ceiling >> SCALE,
        two_strides_from: shipped.two_strides_from >> SCALE,
        read_ahead: shipped.read_ahead >> SCALE,
        refetch_slack: shipped.refetch_slack >> SCALE,
    };
    let cap = scaled.cache_cap(CHUNK);
    let (mut reader, fetches) = counting(LEN, CHUNK, scaled);
    let step = u64::from(MEDIA_MAX_READ >> SCALE);
    let mut boundaries = 0u32;
    let mut at = 0u64;
    while at < LEN {
        let len = step.min(LEN - at);
        let index = u32::try_from(at / CHUNK).expect("a test chunk's index");
        if index > 0 && at % CHUNK == 0 {
            let next = CHUNK.min(LEN - at);
            let cache = reader.shared.lock();
            assert!(
                cache.chunks.contains_key(&index),
                "boundary {index}: the next chunk was read ahead before the play reached it"
            );
            assert_eq!(
                cache.held,
                CHUNK + next,
                "boundary {index}: the chunk played and the next one are held"
            );
            boundaries += 1;
        }
        let got = read(&mut reader, at, u32::try_from(len).expect("a read's length"))
            .unwrap_or_else(|why| panic!("read at {at}: {why:?}"));
        assert!(
            got.len() as u64 == len && got.iter().all(|byte| *byte == index as u8),
            "read at {at}: the bytes of chunk {index}"
        );
        settle(&reader);
        let held = reader.cached_bytes();
        assert!(held <= cap, "read at {at}: held {held} of {cap}");
        at += len;
    }
    assert_eq!(boundaries, 16, "precondition: the play crossed every boundary");
    drop(reader);
    let expected: HashMap<u32, u32> = (0..17).map(|index| (index, 1)).collect();
    assert_eq!(*fetches.lock().unwrap(), expected, "every chunk fetched once");

    // The shipped numbers: two chunks of 256 MiB, and every smaller stride as before.
    assert_eq!(cap, 2 * CHUNK, "the scaled cap is two chunks, as the shipped one");
    assert_eq!(
        shipped.cache_cap(256 << 20),
        512 << 20,
        "two chunks of core's largest stride"
    );
    assert_eq!(
        shipped.cache_cap(128 << 20),
        MEDIA_CACHE_CEILING,
        "three chunks of 128 MiB: the ceiling"
    );
    assert_eq!(shipped.cache_cap(32 << 20), 96 << 20, "three chunks of 32 MiB");
    assert_eq!(shipped.cache_cap(1 << 20), MEDIA_CACHE_MAX, "the floor");
}

// ── 13. A seek with the read-ahead on ────────────────────────────────────────────────────────────────────────────

/// What a held fetch is told, and tells.
#[derive(Default)]
struct Hold {
    /// The held fetch may answer.
    released: AtomicBool,
    /// The held fetch answered (its chunk, or `Cancelled` when its stop said so first).
    answered: AtomicBool,
}

/// Waits (five seconds at most) until `done` says so.
fn wait_until(done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// [`counting`], but the fetch of chunk `held` (the read-ahead's, in these tests) waits until the test releases it or
/// its stop says so (as Beebeeb's fetch stops: `Cancelled`), five seconds at most. The fetch of chunk `releaser`, when
/// named (the seek's own read), releases it first, waits for its answer, and then 100 ms more, so the worker has done
/// with that answer what the code does with it.
fn holding(
    len: u64,
    stride: u64,
    limits: Limits,
    held: u32,
    releaser: Option<u32>,
) -> (MediaReader, Fetches, Arc<Hold>) {
    let fetches: Fetches = Arc::new(Mutex::new(HashMap::new()));
    let hold = Arc::new(Hold::default());
    let (counted, told) = (fetches.clone(), hold.clone());
    let fetch = move |index: u32, stop: &dyn Fn() -> bool| -> Result<Zeroizing<Vec<u8>>, MediaReadError> {
        *counted.lock().unwrap().entry(index).or_insert(0) += 1;
        if index == held {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stopped = false;
            while !told.released.load(Ordering::SeqCst) && Instant::now() < deadline {
                if stop() {
                    stopped = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            told.answered.store(true, Ordering::SeqCst);
            if stopped {
                return Err(MediaReadError::Cancelled);
            }
        }
        if Some(index) == releaser {
            told.released.store(true, Ordering::SeqCst);
            wait_until(|| told.answered.load(Ordering::SeqCst));
            std::thread::sleep(Duration::from_millis(100));
        }
        let start = u64::from(index) * stride;
        let size = usize::try_from(len.saturating_sub(start).min(stride)).expect("a test chunk's size");
        Ok(Zeroizing::new(vec![index as u8; size]))
    };
    (
        MediaReader::over_fetch(len, stride, limits, Box::new(fetch)),
        fetches,
        hold,
    )
}

/// Row 10: a forward seek while the read-ahead fetches inside its window (chunk 1 under way, chunk 2 still to come)
/// gives that window up at once: the fetch under way is stopped and not kept, chunk 2 is never asked for, the seek's
/// bytes are chunk 12's, and the new window (chunks 13 and 14) is read ahead. Mutants (row 10): (a) the generation
/// step in `seek_to` removed → chunk 1 is kept → red here; (a') its `job.next = job.end` removed → chunk 2 is fetched
/// while the seek reads → red here.
#[test]
fn media_forward_seek_with_read_ahead_gives_up_the_window() {
    const CHUNK: u64 = 1_000;
    // Room for eight chunks: nothing is evicted in this test, so a chunk kept is still there to be seen.
    let limits = Limits {
        cache_floor: 8 * CHUNK,
        cache_ceiling: MEDIA_CACHE_CEILING,
        two_strides_from: MEDIA_TWO_STRIDES_FROM,
        read_ahead: 2_500,
        refetch_slack: 0,
    };
    let (mut reader, fetches, _hold) = holding(20 * CHUNK, CHUNK, limits, 1, Some(12));
    assert!(
        read(&mut reader, 0, 250)
            .expect("chunk 0")
            .iter()
            .all(|byte| *byte == 0)
    );
    wait_until(|| reader.shared.lock().in_flight == Some(1));
    {
        let cache = reader.shared.lock();
        assert!(
            cache.in_flight == Some(1) && cache.job.next < cache.job.end,
            "precondition: chunk 1 under way, more of the window to come"
        );
    }

    let got = read(&mut reader, 12 * CHUNK, 250).expect("the seek's range");
    assert!(
        got.len() == 250 && got.iter().all(|byte| *byte == 12),
        "the seek's bytes are chunk 12's"
    );
    settle(&reader);
    {
        let cache = reader.shared.lock();
        assert!(!cache.chunks.contains_key(&1), "the old window's fetch was not kept");
        assert!(
            cache.chunks.contains_key(&13) && cache.chunks.contains_key(&14),
            "the new window was read ahead"
        );
    }
    let counts = fetches.lock().unwrap().clone();
    assert_eq!(
        counts.get(&2),
        None,
        "the rest of the old window was never asked for: {counts:?}"
    );
    assert!(
        [1, 12, 13, 14].iter().all(|index| counts.get(index) == Some(&1)),
        "{counts:?}"
    );
}

/// Row 10: a seek right after the read-ahead took the LAST chunk of its window (chunk 1 under way, nothing more to
/// come, so `seek_to` has nothing to give up) still begins a new window once the seek's read is done: the fetch under
/// way is not kept. Then a backward seek begins one too: the chunk after it is read ahead before any read asks for it.
/// The reads in a row before them move one window on (no new one). Mutants (row 10): (b) the generation step of a new
/// window in `ahead` removed → chunk 1 is kept → red here; (c) `ahead`'s window test made always true (every read
/// moves the window on) → chunk 1 is kept, and after the backward seek chunk 6 is never read ahead → red here.
#[test]
fn media_seek_after_the_windows_last_chunk_begins_a_new_window() {
    const CHUNK: u64 = 1_000;
    let limits = Limits {
        cache_floor: 4 * CHUNK,
        cache_ceiling: MEDIA_CACHE_CEILING,
        two_strides_from: MEDIA_TWO_STRIDES_FROM,
        read_ahead: 300,
        refetch_slack: 0,
    };
    let (mut reader, fetches, hold) = holding(20 * CHUNK, CHUNK, limits, 1, None);
    for at in [0, 250, 500, 750] {
        assert!(
            read(&mut reader, at, 250)
                .expect("chunk 0")
                .iter()
                .all(|byte| *byte == 0)
        );
    }
    wait_until(|| reader.shared.lock().in_flight == Some(1));
    let generation = {
        let cache = reader.shared.lock();
        assert!(
            cache.in_flight == Some(1) && cache.job.next == cache.job.end,
            "precondition: chunk 1 under way, the window's last"
        );
        assert_eq!(cache.job.generation, 0, "the reads in a row moved one window on");
        cache.job.generation
    };

    // Forward, to a chunk past the window.
    let got = read(&mut reader, 12 * CHUNK, 250).expect("the forward seek's range");
    assert!(got.iter().all(|byte| *byte == 12), "the forward seek's bytes");
    hold.released.store(true, Ordering::SeqCst);
    wait_until(|| hold.answered.load(Ordering::SeqCst));
    settle(&reader);
    assert!(
        !reader.shared.lock().chunks.contains_key(&1),
        "the old window's fetch was not kept"
    );

    // Backward, to a chunk never fetched.
    let got = read(&mut reader, 5 * CHUNK + 700, 250).expect("the backward seek's range");
    assert!(got.iter().all(|byte| *byte == 5), "the backward seek's bytes");
    settle(&reader);
    assert!(
        reader.shared.lock().chunks.contains_key(&6),
        "the chunk after the backward seek was read ahead"
    );
    assert!(
        read(&mut reader, 6 * CHUNK, 250)
            .expect("chunk 6")
            .iter()
            .all(|byte| *byte == 6)
    );
    assert_eq!(
        fetches.lock().unwrap().get(&6),
        Some(&1),
        "chunk 6 fetched once, by the read-ahead"
    );
    assert_eq!(
        reader.shared.lock().job.generation,
        generation + 2,
        "each seek began one window, the reads in a row none"
    );
}
