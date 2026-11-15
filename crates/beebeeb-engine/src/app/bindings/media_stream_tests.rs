//! The media player's range reader at the engine (`Engine::open_media_for`, `MediaReader::read_at`), against the
//! engine's own doubles: an invented account on the fake API on loopback, each file's content encrypted chunk by chunk
//! by core under that account's key (`FakeAccount::add_file_with_content`, 1 KiB chunks), served as the server serves
//! it: the file's row names its current version with the manifest (`download_version`), and a chunk request pinned to
//! a version that is no longer current answers 409. A state database of each test's own (`use_local_state_for_test`),
//! recorded as the account's. Sessions are put in memory as a sign-in and an unlock leave them. Every name, key and
//! byte is invented.
//!
//! The limits are the reader's own fields: a test sets smaller ones where the real ones are out of its reach (with
//! 1 KiB chunks, 32 MiB of cache is 32 768 chunk requests), and sets the read-ahead to 0 where it counts requests.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use serde_json::json;
use zeroize::Zeroizing;

use super::{Limits, MEDIA_MAX_READ, MEDIA_REFETCH_SLACK, MediaOrigin, MediaReadError, MediaReader, WIRE_OVERHEAD};
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::app::{Engine, EngineConfig};
use crate::host::ChannelHost;
use crate::identity::REAL;
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
    let pieces: Vec<&[u8]> = if plain.is_empty() {
        vec![plain]
    } else {
        plain.chunks(STRIDE as usize).collect()
    };
    let chunks: Vec<serde_json::Value> = pieces
        .iter()
        .enumerate()
        .map(|(index, piece)| json!({ "index": index, "size_bytes": piece.len() as u64 + WIRE_OVERHEAD }))
        .collect();
    json!({
        "object_version_id": version,
        "chunk_count": pieces.len(),
        "chunk_size_bytes": STRIDE,
        "plaintext_size_bytes": plain.len(),
        "ciphertext_size_bytes": plain.len() as u64 + WIRE_OVERHEAD * pieces.len() as u64,
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
    Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"))
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

/// Reads over three times the cache's cap of distinct chunks (forty chunks against a cap of three) never hold more
/// than the cap, and chunk 0 stays.
#[test]
fn media_cache_is_bounded() {
    let fake = fake("bounded");
    fake.put(LONG, "Invented forty.mp3", &long_clip(), 1);
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.account);
    let _state = local_state(&engine, &fake.account);
    let mut reader = open(&engine, LONG);
    let cap = 3 * STRIDE;
    reader.limits = Limits {
        cache_max: cap,
        read_ahead: 0,
        refetch_slack: MEDIA_REFETCH_SLACK,
    };
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
    reader.limits = Limits {
        cache_max: STRIDE,
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
