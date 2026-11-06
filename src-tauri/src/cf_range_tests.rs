//! Tests of `cf_range` (LD-0105 W2, contract Amendment 4). The session tests run core's real
//! `range_session` on its scripted transport (no socket) through the desktop's `StreamSink`.

use std::sync::{Arc, Mutex};

use beebeeb_core::chunk_stream::{ChunkEncryptor, ChunkFormat, ExpectedChunkFormat};
use beebeeb_core::direct_transfer::window::Geometry;
use beebeeb_core::direct_transfer::{CAP_OBJECT_DOWNLOAD_V1, DISCOVERY_PATH, DownloadGrantRequest, download_grants_path};
use beebeeb_core::kdf::FileKey;
use beebeeb_transfer::test_support::scripted::{Reply, Request, ScriptedIo};
use beebeeb_transfer::{
    ApiReply, DirectShared, RangeControl, RangeIdentity, RangeLessor, RangeOutcome, RangeRequest, RangeSessionError,
    RangeStats, StorageReply, range_session,
};

use super::*;

const FILE_ID: &str = "11111111-2222-4333-8444-555555555555";
const VERSION: &str = "22222222-3333-4444-8555-666666666666";
const KEY: [u8; 32] = [9u8; 32];

/// A v1 file of `len` bytes in chunks of `stride`, as the server stores it.
struct Fx {
    frames: Vec<Vec<u8>>,
    plain: Vec<u8>,
    geometry: Geometry,
}

fn fx(len: usize, stride: u64) -> Arc<Fx> {
    let plain: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
    let mut enc = ChunkEncryptor::for_push_with_file_key(FileKey::from_bytes(KEY), FILE_ID, len as u64, stride).unwrap();
    let frames: Vec<Vec<u8>> = plain.chunks(stride as usize).map(|p| enc.push_chunk(p).unwrap().data).collect();
    let s = enc.finish().unwrap();
    let geometry = Geometry {
        chunk_count: frames.len() as u32,
        chunk_size: stride,
        encrypted_size: s.total_ciphertext,
        original_size: s.total_plaintext,
    };
    Arc::new(Fx { frames, plain, geometry })
}

fn json(status: u16, value: serde_json::Value) -> Reply {
    Reply::Api(ApiReply {
        status,
        content_type: Some("application/json".into()),
        retry_after: None,
        body: value.to_string().into_bytes(),
    })
}

/// The direct-transfer server for `fx`; chunk `tamper` (if any) arrives with a flipped byte, from
/// storage and from the pinned API route alike.
fn server(fx: Arc<Fx>, tamper: Option<usize>) -> ScriptedIo {
    ScriptedIo::new(move |req, _, _| {
        let frame = |i: usize| {
            let mut f = fx.frames[i].clone();
            if Some(i) == tamper {
                f[20] ^= 1;
            }
            f
        };
        match req {
            Request::Api(call) if call.path == DISCOVERY_PATH => json(
                200,
                serde_json::json!({"download": [CAP_OBJECT_DOWNLOAD_V1], "upload": [], "storage_hosts": ["storage.test"],
                    "storage_buckets": [{"host": "storage.test", "bucket": "k"}], "max_window": 32, "url_ttl_seconds": 300}),
            ),
            Request::Api(call) if call.path == download_grants_path(FILE_ID) => {
                let r: DownloadGrantRequest = serde_json::from_slice(call.body.as_deref().unwrap()).unwrap();
                let g = fx.geometry;
                let end = (r.start_index + r.count).min(g.chunk_count);
                let grants: Vec<_> = (r.start_index..end)
                    .map(|i| serde_json::json!({"index": i, "method": "GET", "length": g.chunk_length(i).unwrap(),
                        "url": format!("https://storage.test/k/obj/{i}?X-Amz-Signature=S&i={i}"), "expires_at": "2099-01-01T00:00:00Z"}))
                    .collect();
                json(
                    200,
                    serde_json::json!({"transport": CAP_OBJECT_DOWNLOAD_V1, "file_id": FILE_ID, "object_version_id": VERSION,
                        "chunk_count": g.chunk_count, "chunk_size": g.chunk_size, "encrypted_size": g.encrypted_size,
                        "original_size": g.original_size, "grants": grants, "next_index": (end < g.chunk_count).then_some(end)}),
                )
            }
            Request::Api(call) if call.path.contains("/chunks/") => {
                let i: usize = call.path.split_once("/chunks/").unwrap().1.split('?').next().unwrap().parse().unwrap();
                Reply::Api(ApiReply {
                    status: 200,
                    content_type: Some("application/octet-stream".into()),
                    retry_after: None,
                    body: frame(i),
                })
            }
            Request::StorageGet { url, .. } => {
                let i: usize = url.rsplit("i=").next().unwrap().parse().unwrap();
                let f = frame(i);
                Reply::Storage(StorageReply {
                    status: 200,
                    content_length: Some(f.len() as u64),
                    body: Some(bytes::Bytes::from(f)),
                })
            }
            other => panic!("unexpected request {other:?}"),
        }
    })
}

/// What reached "Windows", in order: `("identity" | "transfer", offset, len)`.
type Log = Arc<Mutex<Vec<(&'static str, u64, u64)>>>;

struct Recorder {
    log: Log,
    buf: Arc<Mutex<Vec<u8>>>,
}

impl PieceOut for Recorder {
    fn transfer(&mut self, _keys: RequestKeys, offset: u64, plain: &[u8]) -> std::io::Result<()> {
        self.log.lock().unwrap().push(("transfer", offset, plain.len() as u64));
        self.buf.lock().unwrap()[offset as usize..offset as usize + plain.len()].copy_from_slice(plain);
        Ok(())
    }
}

struct LogRecords(Log);

impl IdentityRecords for LogRecords {
    fn record(&mut self, _identity: &RangeIdentity) -> std::io::Result<()> {
        self.0.lock().unwrap().push(("identity", 0, 0));
        Ok(())
    }
}

fn request(fx: &Fx, on_disk: u64, recorded: Option<RangeIdentity>) -> RangeRequest {
    RangeRequest {
        api_base: "https://api.test".into(),
        file_id: FILE_ID.into(),
        object_version_id: VERSION.into(),
        file_size: fx.geometry.original_size,
        pin: ExpectedChunkFormat::Unrecorded,
        geometry: Some(fx.geometry),
        recorded,
        on_disk_bytes: on_disk,
        forced_api: false,
        jitter_seed: 1,
    }
}

struct Quiet;
impl beebeeb_transfer::Observer for Quiet {}

const KEYS: RequestKeys = RequestKeys {
    transfer_key: 7,
    request_key: 70,
};

/// One whole-file demand through `StreamSink`, as a non-player's open makes it (spec S2).
async fn whole_file<R: IdentityRecords>(
    fx: &Arc<Fx>,
    tamper: Option<usize>,
    records: R,
    log: Log,
) -> (Result<RangeOutcome, RangeSessionError>, Vec<u8>, Option<Answer>, u64) {
    let io = server(fx.clone(), tamper);
    let book = Arc::new(Mutex::new(DemandBook::default()));
    let (id, answer) = book.lock().unwrap().add(0, fx.geometry.original_size, 1, KEYS);
    let control = RangeControl::new();
    control.demand(id, 0, fx.geometry.original_size);
    let buf = Arc::new(Mutex::new(vec![0u8; fx.plain.len()]));
    let mut sink = StreamSink::new(book.clone(), Recorder { log, buf: buf.clone() }, records, fx.geometry.original_size);
    let shared = DirectShared::default();
    let out = range_session(
        &io,
        &shared,
        &RangeLessor::new(1 << 30),
        request(fx, 0, None),
        FileKey::from_bytes(KEY),
        &control,
        &mut sink,
        &Quiet,
    )
    .await;
    let met = answer.await.ok();
    let plain = buf.lock().unwrap().clone();
    (out, plain, met, sink.transferred)
}

// ── The piece check (D-3) ─────────────────────────────────────────────────────────────────────────

/// Red on the mutant "the check accepts any offset": `piece_is_transferable` returning `len > 0`.
#[test]
fn ld0105_w2_a_piece_is_4k_aligned_or_ends_on_eof() {
    let size = 1_000_000;
    assert!(piece_is_transferable(0, 4096, size));
    assert!(piece_is_transferable(995_328, 4672, size), "the last piece ends on EoF");
    assert!(!piece_is_transferable(1, 4096, size), "an unaligned start");
    assert!(!piece_is_transferable(4096, 1000, size), "a short piece that is not the last");
    assert!(!piece_is_transferable(995_328, 8192, size), "past the end of the file");
    assert!(!piece_is_transferable(0, 0, size), "an empty piece");
}

/// Spec S1 vectors: 1 MiB chunks (storage format v1, iOS uploads) and 1,000,000-byte chunks (not a
/// multiple of 4 KiB: the carry) through core's real session and the desktop sink. Every piece Windows
/// gets is 4 KiB aligned or ends on EoF, they tile the file without overlap, and the bytes are the
/// file's. A vector test of the integrated path; its red-first mutant is the check above (a sink that
/// let an unaligned piece through would fail `transfer` here instead of reaching Windows).
#[tokio::test]
async fn ld0105_w2_every_piece_of_a_real_session_is_aligned_with_carry_and_the_bytes_are_the_file() {
    for (len, stride) in [(3 * (1 << 20) + 12_345, 1u64 << 20), (3_456_789, 1_000_000)] {
        let fx = fx(len, stride);
        let log: Log = Arc::default();
        let (out, plain, met, transferred) = whole_file(&fx, None, LogRecords(log.clone()), log.clone()).await;
        assert!(matches!(out, Ok(RangeOutcome::Whole(_))), "stride {stride}: {out:?}");
        assert_eq!(plain, fx.plain, "stride {stride}: the bytes are the file's");
        assert_eq!((met, transferred), (Some(Answer::Met), len as u64));
        let mut pieces: Vec<(u64, u64)> = log.lock().unwrap().iter().filter(|e| e.0 == "transfer").map(|e| (e.1, e.2)).collect();
        pieces.sort();
        let mut end = 0;
        for (offset, n) in &pieces {
            assert!(piece_is_transferable(*offset, *n, len as u64), "stride {stride}: piece {offset}+{n}");
            assert_eq!(*offset, end, "stride {stride}: no gap, no overlap");
            end = offset + n;
        }
        assert_eq!(end, len as u64);
    }
}

// ── The 1640 order (spec S9) ──────────────────────────────────────────────────────────────────────

/// The identity and the candidate are durable before the first transfer, with the real state.db
/// recorder; the candidate carries no hash. Red on the mutant "`StreamSink::transfer` without the
/// identity check, `DbRecords::record` recording nothing": the first event is then a transfer, and the
/// candidate is missing.
#[tokio::test]
async fn ld0105_s9_identity_and_candidate_are_recorded_before_the_first_transfer() {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap());
    db.upsert_file(&crate::state_db::FileEntry {
        file_id: FILE_ID.into(),
        path: "/clip.mp4".into(),
        status: crate::state_db::FileStatus::CloudOnly,
        size_bytes: 0,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: crate::state_db::ItemKind::File,
    })
    .unwrap();
    db.mark_no_fetched_content(FILE_ID).unwrap();
    let fx = fx(2 * 4096 + 77, 4096);
    let log: Log = Arc::default();
    struct Both(DbRecords, Log);
    impl IdentityRecords for Both {
        fn record(&mut self, identity: &RangeIdentity) -> std::io::Result<()> {
            self.0.record(identity)?;
            let candidate = self.0.db.fetched_content(&self.0.file_id).unwrap();
            self.1.lock().unwrap().push(("identity", candidate.as_ref().map_or(0, |c| c.0 as u64), u64::from(candidate.as_ref().is_some_and(|c| c.1.is_none()))));
            Ok(())
        }
    }
    let records = Both(
        DbRecords {
            db: db.clone(),
            file_id: FILE_ID.into(),
            version_number: Some(5),
        },
        log.clone(),
    );
    let (out, _, _, _) = whole_file(&fx, None, records, log.clone()).await;
    assert!(matches!(out, Ok(RangeOutcome::Whole(_))), "{out:?}");
    let log = log.lock().unwrap();
    assert_eq!(log[0], ("identity", 5, 1), "first: the identity, with the candidate (version 5, no hash)");
    assert!(log[1..].iter().all(|e| e.0 == "transfer"), "{log:?}");
    let identity = db.range_identity(FILE_ID).unwrap().expect("recorded");
    assert_eq!((identity.object_version_id.as_str(), identity.format.as_str()), (VERSION, "v1"));
}

/// The sink refuses a transfer before the identity (I4), whatever core does.
#[test]
fn ld0105_i4_the_sink_refuses_a_transfer_before_the_identity() {
    let log: Log = Arc::default();
    let book = Arc::new(Mutex::new(DemandBook::default()));
    book.lock().unwrap().add(0, 4096, 1, KEYS);
    let mut sink = StreamSink::new(book, Recorder { log: log.clone(), buf: Arc::new(Mutex::new(vec![0; 4096])) }, LogRecords(log.clone()), 4096);
    assert!(beebeeb_transfer::RangeSink::transfer(&mut sink, 0, &[0; 4096]).is_err());
    assert!(log.lock().unwrap().is_empty(), "nothing reached Windows");
}

/// Promotion only after the one "whole" verdict (I6). Red on the mutant "`Closed` promotes".
#[test]
fn ld0105_s9_only_whole_promotes_and_a_failure_after_a_write_poisons() {
    let stats = RangeStats::default();
    assert_eq!(after_session(&Ok(RangeOutcome::Whole(stats))), AfterSession::Promote);
    assert_eq!(after_session(&Ok(RangeOutcome::Closed(stats))), AfterSession::Leave);
    assert_eq!(
        after_session(&Ok(RangeOutcome::UseApi(beebeeb_transfer::UseApiReason::NotSupported))),
        AfterSession::WholeFilePath
    );
    let after_write = |cause| {
        Err(RangeSessionError::DirectFailedAfterWrite {
            transferred_bytes: 4096,
            cause: Box::new(cause),
        })
    };
    assert_eq!(after_session(&after_write(RangeSessionError::Unauthentic { index: 3 })), AfterSession::Poison("corrupt"));
    assert_eq!(after_session(&after_write(RangeSessionError::FileChanged)), AfterSession::Poison("version_gone"));
    assert_eq!(after_session(&Err(RangeSessionError::IdentityMismatch)), AfterSession::Poison("identity_mismatch"));
    assert_eq!(after_session(&Err(RangeSessionError::SizeMismatch)), AfterSession::ResolveSize);
    assert_eq!(after_session(&Err(RangeSessionError::FileChanged)), AfterSession::Fail, "nothing written: no poison");
}

/// A tampered chunk after earlier ones were transferred: the session fails after a write, the demand
/// fails (I7), and the fetch path poisons (spec S6). The real session, the real sink.
#[tokio::test]
async fn ld0105_s6_corrupt_at_chunk_k_fails_the_demand_and_poisons() {
    let fx = fx(5 * 4096, 4096);
    let log: Log = Arc::default();
    let (out, plain, met, transferred) = whole_file(&fx, Some(3), LogRecords(log.clone()), log).await;
    assert_eq!(after_session(&out), AfterSession::Poison("corrupt"), "{out:?}");
    assert_eq!(met, Some(Answer::Failed), "the request fails, no filler (I7)");
    assert!(transferred < 5 * 4096 && plain[3 * 4096..4 * 4096].iter().all(|b| *b == 0), "chunk 3 never reached Windows");
}

// ── 0783 ─────────────────────────────────────────────────────────────────────────────────────────

/// Red on the mutant "no size check" (`size_check` always `Ok`).
#[test]
fn ld0105_0783_a_pinned_size_that_is_not_the_placeholders_fails_before_any_request() {
    assert_eq!(size_check(1_000, 1_000), Ok(()));
    assert_eq!(size_check(1_000, 1_028), Err(1_000), "the encrypted-size placeholder of task 0783");
}

// ── Demands: keys, cancel, requester exit ──────────────────────────────────────────────────────────

/// Red on the mutant "cancel by overlap" (a cancel of one read also cancels its neighbour).
#[test]
fn ld0105_w2_a_cancel_maps_to_the_demands_it_names() {
    let mut book = DemandBook::default();
    let k = |r| RequestKeys {
        transfer_key: 1,
        request_key: r,
    };
    let (a, mut ra) = book.add(0, 65_536, 10, k(100));
    let (b, _rb) = book.add(65_536, 65_536, 10, k(101));
    let (c, _rc) = book.add(4096, 8192, 11, k(102));
    let (d, _rd) = book.add(61_440, 8192, 12, k(103));
    // By request key: exactly that one.
    assert_eq!(book.cancel(0, 1 << 30, 101), vec![b]);
    // Without one: the demands inside the range, not one that only overlaps it.
    assert_eq!(book.cancel(0, 65_536, REQUEST_KEY_DEFAULT), vec![a, c]);
    assert_eq!(ra.try_recv().ok(), Some(Answer::Cancelled), "a cancelled request gets no transfer, not a failure");
    assert!(!book.is_idle(), "d only overlapped the cancelled range");
    assert_eq!(book.cancel(61_440, 8192, REQUEST_KEY_DEFAULT), vec![d]);
    assert!(book.is_idle());
}

/// Red on the mutant "requester_exited fails every demand" (another live requester's demand dies).
#[test]
fn ld0105_s5_a_requester_that_exits_fails_only_its_own_demands() {
    let mut book = DemandBook::default();
    let (a, mut ra) = book.add(0, 4096, 10, KEYS);
    let (_b, mut rb) = book.add(4096, 4096, 11, KEYS);
    let (gone, others) = book.requester_exited(10);
    assert_eq!((gone, others), (vec![a], true));
    assert_eq!(ra.try_recv().ok(), Some(Answer::Failed), "its request fails");
    assert!(rb.try_recv().is_err(), "the other one still waits");
    assert_eq!(book.requester_exited(11).1, false, "no live requester left: the session stops");
}

/// A piece goes out with a pending request it overlaps, else any pending one; with none pending, no
/// keys at all (desktop #225: no transfer on `CF_REQUEST_KEY_DEFAULT`). Red on the mutant "always the
/// latest demand's keys".
#[test]
fn ld0105_w2_a_piece_takes_the_keys_of_the_request_it_serves() {
    let mut book = DemandBook::default();
    assert_eq!(book.keys_for(0, 4096), None);
    let k = |t, r| RequestKeys {
        transfer_key: t,
        request_key: r,
    };
    let (a, _ra) = book.add(0, 4096, 1, k(5, 50));
    let (_b, _rb) = book.add(1 << 20, 4096, 1, k(6, 60));
    assert_eq!(book.keys_for(0, 4096), Some(k(5, 50)));
    assert_eq!(book.keys_for(1 << 20, 4096), Some(k(6, 60)));
    assert_eq!(book.keys_for(8192, 4096), Some(k(6, 60)), "over-served on a pending request");
    book.answer(a, Answer::Met);
    book.fail_all();
    assert_eq!(book.keys_for(8192, 4096), None, "no request pending: no keys");
}

#[test]
fn ld0105_i3_a_recorded_identity_round_trips_and_an_unreadable_one_is_none() {
    let v2 = RangeIdentity {
        object_version_id: VERSION.into(),
        format: ChunkFormat::V2,
        version_id: Some([0xab; beebeeb_core::chunk_stream::VERSION_ID_LEN]),
    };
    assert_eq!(recorded_identity(Some(&identity_record(&v2))), Some(v2));
    let v1 = RangeIdentity {
        object_version_id: VERSION.into(),
        format: ChunkFormat::V1Legacy,
        version_id: None,
    };
    assert_eq!(recorded_identity(Some(&identity_record(&v1))), Some(v1));
    let bad = crate::state_db::RecordedIdentity {
        object_version_id: VERSION.into(),
        format: "v2".into(),
        version_id_hex: Some("zz".into()),
    };
    assert_eq!(recorded_identity(Some(&bad)), None, "never trusted when unreadable");
    assert_eq!(recorded_identity(None), None);
}

// ── Contract Amendment 4: the poison key ───────────────────────────────────────────────────────────

/// Red on the mutants "a changed key empties" and "no key empties".
#[test]
fn ld0105_a4_only_an_unchanged_content_key_empties_the_file() {
    assert_eq!(key_verdict(Some((10, 4096)), Some((10, 4096))), KeyVerdict::Empty, "our own restore writes: unchanged");
    assert_eq!(key_verdict(Some((10, 4096)), Some((11, 4096))), KeyVerdict::NeedsAttention, "a content write");
    assert_eq!(key_verdict(Some((10, 4096)), Some((10, 4100))), KeyVerdict::NeedsAttention, "a size change");
    assert_eq!(key_verdict(None, Some((10, 4096))), KeyVerdict::NeedsAttention, "no key recorded");
    assert_eq!(key_verdict(Some((10, 4096)), None), KeyVerdict::NeedsAttention, "the key cannot be read now");
}

/// Red on the mutant "Missing is cleared at any sweep".
#[test]
fn ld0105_a4_missing_is_cleared_only_at_the_first_sweep_of_a_new_process() {
    assert!(clear_missing(SweepKind::ProcessStart));
    assert!(!clear_missing(SweepKind::Tick));
    assert!(!clear_missing(SweepKind::Close));
}

/// Red on the mutant "a needs-attention row is opened on every tick".
#[test]
fn ld0105_a4_a_needs_attention_row_is_checked_at_start_and_on_close_only() {
    assert!(!sweep_opens(true, SweepKind::Tick));
    assert!(sweep_opens(true, SweepKind::ProcessStart) && sweep_opens(true, SweepKind::Close));
    assert!(sweep_opens(false, SweepKind::Tick), "an ordinary poison is emptied at the next tick");
}

// ── Desktop #225: the independent review's P2s ──────────────────────────────────────────────────

/// Review P2 (cf_range.rs:125, streaming.rs:532), lead ruling: no transfer on
/// `CF_REQUEST_KEY_DEFAULT`, so nothing between a player's reads or after its close. Red on the mutant
/// `READ_AHEAD_ON_DEFAULT_KEY = true`.
#[test]
fn ld0105_p2_no_piece_goes_out_on_the_default_key() {
    assert!(!READ_AHEAD_ON_DEFAULT_KEY, "off until a native rung measures it");
    let mut book = DemandBook::default();
    let (a, _ra) = book.add(0, 4096, 1, KEYS);
    assert_eq!(book.keys_for(1 << 20, 4096), Some(KEYS), "over-served under the live request");
    book.answer(a, Answer::Met);
    assert_eq!(book.keys_for(1 << 20, 4096), None, "between two reads: no request, no keys");
    assert_eq!(
        book.keys_for_with(1 << 20, 4096, true),
        Some(RequestKeys { transfer_key: 7, request_key: REQUEST_KEY_DEFAULT }),
        "the off-by-default path is kept for the native rung"
    );
}

/// What reached "Windows", with whether the keys belonged to a pending request at that moment.
type KeyLog = Arc<Mutex<Vec<(RequestKeys, u64, bool)>>>;

struct LiveRecorder {
    book: Arc<Mutex<DemandBook>>,
    log: KeyLog,
    buf: Arc<Mutex<Vec<u8>>>,
}

impl PieceOut for LiveRecorder {
    fn transfer(&mut self, keys: RequestKeys, offset: u64, plain: &[u8]) -> std::io::Result<()> {
        let live = self.book.lock().unwrap().is_pending(keys);
        self.log.lock().unwrap().push((keys, offset, live));
        self.buf.lock().unwrap()[offset as usize..offset as usize + plain.len()].copy_from_slice(plain);
        Ok(())
    }
}

/// A piece that finds no pending request is withheld: not handed over, the session closed (once),
/// never promoted. Red on the mutant "transfer it on the default key".
#[test]
fn ld0105_p2_a_piece_with_no_pending_request_is_withheld_and_closes_the_session() {
    let book = Arc::new(Mutex::new(DemandBook::default()));
    let log: KeyLog = Arc::default();
    let closed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c = closed.clone();
    let out = LiveRecorder { book: book.clone(), log: log.clone(), buf: Arc::new(Mutex::new(vec![0u8; 3 * 4096])) };
    let mut sink = StreamSink::new(book.clone(), out, LogRecords(Arc::default()), 3 * 4096)
        .on_withheld(move || {
            c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
    let identity = RangeIdentity { object_version_id: VERSION.into(), format: ChunkFormat::V1Legacy, version_id: None };
    sink.record_identity(&identity).unwrap();
    let (a, _ra) = book.lock().unwrap().add(0, 4096, 1, KEYS);
    sink.transfer(0, &[1u8; 4096]).unwrap();
    sink.demand_met(a);
    sink.transfer(4096, &[2u8; 4096]).unwrap();
    sink.transfer(8192, &[3u8; 4096]).unwrap();
    assert_eq!(log.lock().unwrap().len(), 1, "only the piece with a pending request went out");
    assert!(sink.withheld);
    assert_eq!(closed.load(std::sync::atomic::Ordering::SeqCst), 1, "the session is closed once");
    assert_eq!(sink.transferred, 4096, "the withheld pieces are not counted as handed over");
}

/// Review P2 (cf_range.rs:125 + streaming.rs:532) through core's real `range_session`: read-ahead
/// fetched while no request is pending waits in `GatedIo` and goes out under the NEXT request's keys;
/// nothing goes out between the two reads, nothing on `CF_REQUEST_KEY_DEFAULT`, and the file still
/// ends whole. Red on the mutants "`READ_AHEAD_ON_DEFAULT_KEY = true`" (3 default-key transfers
/// between the reads) and "no gate" (the read-ahead is withheld, the session closes: not whole).
#[tokio::test]
async fn ld0105_p2_read_ahead_waits_for_a_pending_request_and_goes_out_under_its_keys() {
    let fx = fx(4 << 20, 1 << 20);
    let book = Arc::new(Mutex::new(DemandBook::default()));
    let io = GatedIo { inner: server(fx.clone(), None), book: book.clone() };
    let log: KeyLog = Arc::default();
    let buf = Arc::new(Mutex::new(vec![0u8; fx.plain.len()]));
    let out = LiveRecorder { book: book.clone(), log: log.clone(), buf: buf.clone() };
    let mut sink = StreamSink::new(book.clone(), out, LogRecords(Arc::default()), fx.geometry.original_size);
    let control = RangeControl::new();
    let first = RequestKeys { transfer_key: 1, request_key: 10 };
    let second = RequestKeys { transfer_key: 2, request_key: 20 };
    let (id1, met1) = book.lock().unwrap().add(0, 4096, 1, first);
    control.demand(id1, 0, 4096);
    let shared = DirectShared::default();
    let lessor = RangeLessor::new(1 << 30);
    let session = range_session(&io, &shared, &lessor, request(&fx, 0, None), FileKey::from_bytes(KEY), &control, &mut sink, &Quiet);
    let driver = async {
        assert_eq!(met1.await.ok(), Some(Answer::Met));
        // Core reads ahead to the whole file; give it time to (not) hand anything over.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let between = log.lock().unwrap().len();
        let (id2, met2) = book.lock().unwrap().add(2 << 20, 4096, 1, second);
        control.demand(id2, 2 << 20, 4096);
        assert_eq!(met2.await.ok(), Some(Answer::Met));
        between
    };
    // A mutant that ends the session early leaves the second read unanswered: red, never a hang.
    let (out, between) = tokio::time::timeout(std::time::Duration::from_secs(20), async { tokio::join!(session, driver) })
        .await
        .expect("the session and both reads end");
    let log = log.lock().unwrap().clone();
    assert!(matches!(out, Ok(RangeOutcome::Whole(_))), "{out:?}");
    assert_eq!(*buf.lock().unwrap(), fx.plain, "the bytes are the file's");
    assert!(log.iter().all(|(k, _, live)| *live && k.request_key != REQUEST_KEY_DEFAULT), "{log:?}");
    assert!(between >= 1 && log[..between].iter().all(|(k, _, _)| *k == first), "first read: {log:?}");
    assert!(
        log.len() > between && log[between..].iter().all(|(k, _, _)| *k == second),
        "the read-ahead waited and went out under the second request's keys: {log:?}"
    );
}

/// A closing session's fetched chunks are never released, even with a request pending; an open one's
/// wait ends when a request is added. Red on the mutant "no closed check".
#[tokio::test]
async fn ld0105_p2_a_closing_session_releases_no_fetched_chunk() {
    let book = Mutex::new(DemandBook::default());
    let wait = tokio::time::timeout(std::time::Duration::from_millis(50), until_requested(&book)).await;
    assert!(wait.is_err(), "no request pending: the chunk waits");
    let book = Arc::new(book);
    let waiter = {
        let book = book.clone();
        tokio::spawn(async move { until_requested(&book).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let _pending = book.lock().unwrap().add(0, 4096, 1, KEYS);
    tokio::time::timeout(std::time::Duration::from_secs(5), waiter).await.expect("a request wakes it").unwrap();
    book.lock().unwrap().close();
    let wait = tokio::time::timeout(std::time::Duration::from_millis(50), until_requested(&book)).await;
    assert!(wait.is_err(), "closing: never released, even with a request pending");
}

struct FakeProcess(Arc<std::sync::atomic::AtomicBool>);

impl Exits for FakeProcess {
    fn exited(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// Review P2 (streaming.rs:93), spec S5 / §9: a requester is watched for the session's whole life, so a
/// player killed BETWEEN two reads (no request pending) still stops the session. Red on the mutant
/// "only a requester with a pending request is watched".
#[test]
fn ld0105_p2_a_killed_requester_between_reads_stops_the_session() {
    let long = std::time::Duration::from_secs(3600);
    let (vlc, other) = (Arc::new(std::sync::atomic::AtomicBool::new(false)), Arc::new(std::sync::atomic::AtomicBool::new(false)));
    let mut watched = vec![(10u32, FakeProcess(vlc.clone()))];
    let mut book = DemandBook::default();
    let (a, _ra) = book.add(0, 4096, 10, KEYS);
    book.answer(a, Answer::Met);
    assert_eq!(session_tick(&mut watched, &mut book, long), Tick::default(), "alive, between reads");
    vlc.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(session_tick(&mut watched, &mut book, long).end, Some(SessionEnd::RequesterGone), "killed between reads");
    // Two requesters: the one that exits loses its pending request; the other's keeps the session.
    let mut watched = vec![(10u32, FakeProcess(Arc::new(true.into()))), (11u32, FakeProcess(other))];
    let mut book = DemandBook::default();
    let (mine, _rm) = book.add(0, 4096, 10, KEYS);
    let (_theirs, _rt) = book.add(4096, 4096, 11, KEYS);
    let tick = session_tick(&mut watched, &mut book, long);
    assert_eq!(tick, Tick { cancel: vec![mine], end: None });
    assert_eq!(watched.len(), 1, "the exited one is no longer watched");
}

/// Review P2 (streaming.rs:528): a session with no request pending ends after the idle limit
/// (releasing its lease, ticket and row); one with a pending request never does. Red on the mutant
/// "no idle end".
#[test]
fn ld0105_p2_an_idle_session_ends_after_the_idle_limit() {
    let mut watched: Vec<(u32, FakeProcess)> = Vec::new();
    let mut book = DemandBook::default();
    let (a, _ra) = book.add(0, 4096, 10, KEYS);
    assert_eq!(session_tick(&mut watched, &mut book, std::time::Duration::ZERO).end, None, "a request is pending");
    book.answer(a, Answer::Met);
    assert_eq!(session_tick(&mut watched, &mut book, IDLE_LIMIT).end, None, "just answered");
    assert_eq!(session_tick(&mut watched, &mut book, std::time::Duration::ZERO).end, Some(SessionEnd::Idle));
    assert_eq!(IDLE_LIMIT, std::time::Duration::from_secs(30));
}

/// Review P2 (poison.rs:538), Amendment 4: the poison keeps the key the session took before its first
/// transfer, and only while the file still has it; a write during the session is needs attention,
/// never emptied. Red on the mutant "the key read at the failure" (`poison_key` returning `now`).
#[test]
fn ld0105_p2_a_write_during_the_session_is_needs_attention_not_emptied() {
    let (before, written) = (Some((100, 4096)), Some((200, 4096)));
    let recorded = poison_key(before, written);
    assert_eq!(recorded, None, "a write during the session: no key");
    assert_eq!(key_verdict(recorded, written), KeyVerdict::NeedsAttention, "never emptied");
    assert_eq!(poison_key(before, before), before);
    assert_eq!(key_verdict(poison_key(before, before), before), KeyVerdict::Empty, "the attempt's own bytes");
    assert_eq!(poison_key(None, before), None, "no key before the first transfer");
}

/// Review P2 (cf_range.rs:125) and P3 (streaming.rs:300), lead rulings: "whole" promotes only with
/// every byte ON DISK (never "sent"), and IN_SYNC comes only after the read-back's hash, with the
/// content key unchanged. Red on the mutants "trust sent" (no ONDISK check) and "in sync before the
/// read-back" (`InSync` without a read).
#[test]
fn ld0105_p2_promotion_needs_every_byte_on_disk_and_comes_after_the_read_back() {
    let k = Some((1, 2));
    let read = |key| Some(("h".to_string(), key));
    assert_eq!(promotion(false, false, k, read(k)), Promotion::NotWhole, "sent is not on disk");
    assert_eq!(promotion(true, true, k, read(k)), Promotion::NotWhole, "a withheld piece");
    assert_eq!(promotion(false, true, k, None), Promotion::NotWhole, "no read-back, no promotion");
    assert_eq!(promotion(false, true, k, read(Some((3, 2)))), Promotion::Changed);
    assert_eq!(promotion(false, true, None, read(k)), Promotion::Changed, "no key before: not ours to vouch for");
    assert_eq!(promotion(false, true, k, read(k)), Promotion::InSync("h".into()));
    let size = 3 * 4096 + 100;
    assert!(covers_whole(&[(4096, 2 * 4096 + 100), (0, 4096)], size));
    assert!(!covers_whole(&[(0, 4096), (8192, 4096 + 100)], size), "a hole");
    assert!(!covers_whole(&[(0, size - 1)], size), "one byte short");
    assert!(!covers_whole(&[], size));
}
