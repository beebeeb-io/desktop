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
) -> (Result<RangeOutcome, RangeSessionError>, Vec<u8>, Option<bool>, u64) {
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
        assert_eq!((met, transferred), (Some(true), len as u64));
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
    assert_eq!(met, Some(false), "the request fails, no filler (I7)");
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
    let (a, _ra) = book.add(0, 65_536, 10, k(100));
    let (b, _rb) = book.add(65_536, 65_536, 10, k(101));
    let (c, _rc) = book.add(4096, 8192, 11, k(102));
    let (d, _rd) = book.add(61_440, 8192, 12, k(103));
    // By request key: exactly that one.
    assert_eq!(book.cancel(0, 1 << 30, 101), vec![b]);
    // Without one: the demands inside the range, not one that only overlaps it.
    assert_eq!(book.cancel(0, 65_536, REQUEST_KEY_DEFAULT), vec![a, c]);
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
    assert_eq!(ra.try_recv().ok(), Some(false), "its request fails");
    assert!(rb.try_recv().is_err(), "the other one still waits");
    assert_eq!(book.requester_exited(11).1, false, "no live requester left: the session stops");
}

/// A piece goes out with a pending request it overlaps, else any pending one, else the latest transfer
/// key with no request. Red on the mutant "always the latest demand's keys".
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
    book.answer(a, true);
    book.fail_all();
    assert_eq!(book.keys_for(8192, 4096), Some(k(6, REQUEST_KEY_DEFAULT)), "read-ahead: no request");
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
