//! LD-0105 W2: the rules of the Windows fetch callback's streaming path, pure and compiled on every
//! platform so they are tested everywhere (the Windows glue is `windows_cf/streaming.rs`).
//!
//! A Cloud Files `FETCH_DATA` becomes a demand on the process's one range session for the
//! placeholder (`beebeeb_transfer::range_session`, the contract's `RangeSession`). Core decides what
//! to fetch, authenticates every frame (I1) and cuts it into 4 KiB pieces (D-3); this module is the
//! sink those pieces go through on their way to `CfExecute(TRANSFER_DATA)`:
//!
//! - [`DemandBook`]: the pending requests of one placeholder, with the keys each must be answered
//!   with, the `CANCEL_FETCH_DATA` → demand mapping, and which requester processes still wait;
//! - [`StreamSink`]: the `RangeSink`. Nothing is transferred before the identity is recorded (I4),
//!   and a piece that is not 4 KiB aligned (or ending on EoF) is refused before it reaches Windows;
//! - [`after_session`]: what the callback does with a session's end (spec S6, S9, task 0783).
//!
//! Nothing here is secret: offsets, lengths, keys that are opaque numbers, process ids.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use beebeeb_transfer::{RangeIdentity, RangeOutcome, RangeSessionError, RangeSink};

/// Cloud Files' transfer granularity (D-3).
pub(crate) const PAGE: u64 = 4096;

/// D-3: a `TRANSFER_DATA` starts on a 4 KiB boundary, and its length is a multiple of 4 KiB unless
/// the piece ends on the end of the file. Core's `PieceCutter` cuts that way; this is the desktop's
/// own check right before `CfExecute`, so a cutter bug fails closed instead of reaching Windows.
pub(crate) fn piece_is_transferable(offset: u64, len: u64, file_size: u64) -> bool {
    len > 0
        && offset % PAGE == 0
        && offset
            .checked_add(len)
            .is_some_and(|end| end <= file_size && (end % PAGE == 0 || end == file_size))
}

/// The keys one Cloud Files request is answered with (`CF_CALLBACK_INFO.TransferKey`/`RequestKey`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RequestKeys {
    pub transfer_key: i64,
    pub request_key: i64,
}

/// `CF_REQUEST_KEY_DEFAULT`: a transfer that answers no particular request (read-ahead while no
/// request is pending), on the file's latest transfer key.
pub(crate) const REQUEST_KEY_DEFAULT: i64 = 0;

/// Read-ahead on `CF_REQUEST_KEY_DEFAULT` (a transfer while no request is pending, or after the
/// opener closed): **off**. Decision (lead, 2026-11-06, desktop #225): read-ahead transfers happen
/// only under a pending `FETCH_DATA`'s own keys, while that request is live, until a native rung
/// shows Windows accepts default-key transfers reliably. Off, a piece with no pending request is never
/// handed to Windows: [`GatedIo`] holds a fetched chunk until a request is pending, and the sink
/// withholds a piece that still finds none (and closes the session, [`StreamSink::withheld`]).
pub(crate) const READ_AHEAD_ON_DEFAULT_KEY: bool = false;

/// A session with no pending request, and none added or answered for this long, ends (desktop #225
/// review P2): its lease, its 1640 ticket and its Activity row are released, as "not whole".
pub(crate) const IDLE_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

/// How a demand was answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Answer {
    /// Every byte it asked for was transferred.
    Met,
    /// Fail its remaining range (I7: never filler).
    Failed,
    /// Windows cancelled the request: return without any transfer (the request is gone).
    Cancelled,
}

struct Demand {
    offset: u64,
    len: u64,
    requester: u32,
    keys: RequestKeys,
    answer: Option<tokio::sync::oneshot::Sender<Answer>>,
}

/// The pending requests of one placeholder's session.
#[derive(Default)]
pub(crate) struct DemandBook {
    next: u64,
    live: BTreeMap<u64, Demand>,
    last_keys: Option<RequestKeys>,
    /// When a demand was last added or answered: an idle session is retired only after a while.
    last_activity: Option<std::time::Instant>,
    /// The session is closing: no fetched chunk is released to it any more ([`GatedIo`]).
    closed: bool,
    /// Woken when a demand is added or the session closes ([`GatedIo`]'s wait).
    changed: Arc<tokio::sync::Notify>,
}

impl DemandBook {
    /// A `FETCH_DATA` for `[offset, offset + len)` from process `requester`: its demand id, and where
    /// its answer arrives (`true`: every byte transferred; `false`: fail its remaining range, I7).
    pub(crate) fn add(&mut self, offset: u64, len: u64, requester: u32, keys: RequestKeys) -> (u64, tokio::sync::oneshot::Receiver<Answer>) {
        self.next += 1;
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.live.insert(
            self.next,
            Demand {
                offset,
                len,
                requester,
                keys,
                answer: Some(tx),
            },
        );
        self.last_keys = Some(keys);
        self.last_activity = Some(std::time::Instant::now());
        self.changed.notify_waiters();
        (self.next, rx)
    }

    /// The session is closing: nothing more is released to it, and [`GatedIo`]'s waits never end.
    pub(crate) fn close(&mut self) {
        self.closed = true;
        self.changed.notify_waiters();
    }

    /// Answer demand `id` (no-op when it was answered already, or cancelled).
    pub(crate) fn answer(&mut self, id: u64, answer: Answer) {
        self.last_activity = Some(std::time::Instant::now());
        if let Some(mut demand) = self.live.remove(&id)
            && let Some(tx) = demand.answer.take()
        {
            let _ = tx.send(answer);
        }
    }

    /// No request pending, and none added or answered for `quiet`.
    pub(crate) fn idle_for(&self, quiet: std::time::Duration) -> bool {
        self.live.is_empty() && self.last_activity.is_none_or(|at| at.elapsed() >= quiet)
    }

    /// The keys a piece at `[offset, offset + len)` goes out with: a pending request it overlaps,
    /// else any pending request (D-3: a provider may over-serve, under that live request's keys).
    /// `None` when no request is pending: the piece is not handed to Windows
    /// ([`READ_AHEAD_ON_DEFAULT_KEY`] off).
    pub(crate) fn keys_for(&self, offset: u64, len: u64) -> Option<RequestKeys> {
        self.keys_for_with(offset, len, READ_AHEAD_ON_DEFAULT_KEY)
    }

    /// [`DemandBook::keys_for`]; with `default_key` on, a piece with no pending request goes out on
    /// the latest request's transfer key with `CF_REQUEST_KEY_DEFAULT` (unmeasured on Windows).
    fn keys_for_with(&self, offset: u64, len: u64, default_key: bool) -> Option<RequestKeys> {
        let end = offset.saturating_add(len);
        let overlapping = self
            .live
            .values()
            .find(|d| d.offset < end && offset < d.offset.saturating_add(d.len));
        overlapping
            .or_else(|| self.live.values().next_back())
            .map(|d| d.keys)
            .or_else(|| {
                self.last_keys.filter(|_| default_key).map(|k| RequestKeys {
                    transfer_key: k.transfer_key,
                    request_key: REQUEST_KEY_DEFAULT,
                })
            })
    }

    /// Are these the keys of a pending request? (A test's check that every piece went out under one.)
    #[cfg(test)]
    pub(crate) fn is_pending(&self, keys: RequestKeys) -> bool {
        self.live.values().any(|d| d.keys == keys)
    }

    /// `CANCEL_FETCH_DATA` for `[offset, offset + len)` with `request_key` (0 when Windows gave none):
    /// the demands it cancels. A request key names its request exactly; without one, the pending
    /// demands whose range lies inside the cancelled range. The demands are removed; their callers
    /// return without a transfer (the request is gone).
    pub(crate) fn cancel(&mut self, offset: u64, len: u64, request_key: i64) -> Vec<u64> {
        let end = offset.saturating_add(len);
        let ids: Vec<u64> = self
            .live
            .iter()
            .filter(|(_, d)| {
                if request_key != REQUEST_KEY_DEFAULT {
                    d.keys.request_key == request_key
                } else {
                    d.offset >= offset && d.offset.saturating_add(d.len) <= end
                }
            })
            .map(|(id, _)| *id)
            .collect();
        for id in &ids {
            self.answer(*id, Answer::Cancelled);
        }
        ids
    }

    /// Process `pid` exited (its `OpenProcess(SYNCHRONIZE)` handle was signalled): its demands, and
    /// whether any other live requester still has one (spec S5: when none has, the session stops).
    pub(crate) fn requester_exited(&mut self, pid: u32) -> (Vec<u64>, bool) {
        let ids: Vec<u64> = self.live.iter().filter(|(_, d)| d.requester == pid).map(|(id, _)| *id).collect();
        for id in &ids {
            self.answer(*id, Answer::Failed);
        }
        (ids, !self.live.is_empty())
    }

    /// No request is pending.
    pub(crate) fn is_idle(&self) -> bool {
        self.live.is_empty()
    }

    /// Fail every pending demand (the session ended): no request is left waiting (I7).
    pub(crate) fn fail_all(&mut self) -> Vec<u64> {
        let ids: Vec<u64> = self.live.keys().copied().collect();
        for id in &ids {
            self.answer(*id, Answer::Failed);
        }
        ids
    }
}

/// Where a piece goes: `CfExecute(TRANSFER_DATA)` on Windows, a recorder in tests.
pub(crate) trait PieceOut: Send {
    fn transfer(&mut self, keys: RequestKeys, offset: u64, plain: &[u8]) -> std::io::Result<()>;
}

/// The durable side of the first transfer: the identity (I3, I4, task 2180) and the 1640 candidate
/// `(version, NULL hash)` (spec S9), both before any byte is handed over.
pub(crate) trait IdentityRecords: Send {
    fn record(&mut self, identity: &RangeIdentity) -> std::io::Result<()>;
}

/// The range session's sink for one placeholder.
pub(crate) struct StreamSink<P: PieceOut, R: IdentityRecords> {
    pub(crate) book: Arc<Mutex<DemandBook>>,
    pub(crate) out: P,
    pub(crate) records: R,
    pub(crate) file_size: u64,
    /// Plaintext bytes handed to Windows by this session.
    pub(crate) transferred: u64,
    /// A piece found no pending request and was NOT handed to Windows (desktop #225: no transfer on
    /// `CF_REQUEST_KEY_DEFAULT`). Core counts it as out, so this session can no longer answer a demand
    /// for it: it is closed ([`StreamSink::on_withheld`]), and it never promotes.
    pub(crate) withheld: bool,
    identity_recorded: bool,
    progress: Option<Box<dyn Fn(u64) + Send>>,
    on_withheld: Option<Box<dyn FnMut() + Send>>,
}

impl<P: PieceOut, R: IdentityRecords> StreamSink<P, R> {
    pub(crate) fn new(book: Arc<Mutex<DemandBook>>, out: P, records: R, file_size: u64) -> Self {
        StreamSink {
            book,
            out,
            records,
            file_size,
            transferred: 0,
            withheld: false,
            identity_recorded: false,
            progress: None,
            on_withheld: None,
        }
    }

    /// Called once, when the first piece is withheld: the Windows glue closes the session there.
    pub(crate) fn on_withheld(mut self, close: impl FnMut() + Send + 'static) -> Self {
        self.on_withheld = Some(Box::new(close));
        self
    }

    /// Called with the bytes transferred so far after every piece (the Activity row's progress).
    pub(crate) fn with_progress(mut self, progress: impl Fn(u64) + Send + 'static) -> Self {
        self.progress = Some(Box::new(progress));
        self
    }

    fn book(&self) -> std::sync::MutexGuard<'_, DemandBook> {
        self.book.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl<P: PieceOut, R: IdentityRecords> RangeSink for StreamSink<P, R> {
    fn record_identity(&mut self, identity: &RangeIdentity) -> std::io::Result<()> {
        self.records.record(identity)?;
        self.identity_recorded = true;
        Ok(())
    }

    fn transfer(&mut self, offset: u64, plain: &[u8]) -> std::io::Result<()> {
        // I4: the identity is durable before the first byte reaches the placeholder.
        if !self.identity_recorded {
            return Err(std::io::Error::other("no identity recorded"));
        }
        let len = plain.len() as u64;
        if !piece_is_transferable(offset, len, self.file_size) {
            return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
        }
        if self.withheld {
            return Ok(());
        }
        let Some(keys) = self.book().keys_for(offset, len) else {
            // No request is pending (a cancel or an exit raced the chunk's release): the piece is
            // not handed over, and the session ends, never "whole" (the promotion checks ONDISK too).
            self.withheld = true;
            if let Some(close) = self.on_withheld.as_mut() {
                close();
            }
            return Ok(());
        };
        self.out.transfer(keys, offset, plain)?;
        self.transferred += len;
        if let Some(progress) = &self.progress {
            progress(self.transferred);
        }
        Ok(())
    }

    fn demand_met(&mut self, id: u64) {
        self.book().answer(id, Answer::Met);
    }

    fn demand_failed(&mut self, id: u64) {
        self.book().answer(id, Answer::Failed);
    }
}

/// What the fetch path does once a range session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AfterSession {
    /// `RangeOutcome::Whole` (I6, the only "whole"): IN_SYNC, and the bytes become agreed content.
    Promote,
    /// Closed or stopped early: what was transferred stays, never "whole".
    Leave,
    /// Nothing was transferred and the direct and pinned routes cannot serve it: today's whole-file
    /// path serves the request.
    WholeFilePath,
    /// Spec S6: the placeholder holds bytes of a failed attempt (or another identity): poison it,
    /// with this status word.
    Poison(&'static str),
    /// Task 0783: the version's size is not the placeholder's; nothing transferred.
    ResolveSize,
    /// Nothing transferred; the requests fail (the next open tries again).
    Fail,
}

/// The status word of a failure after a write (never error text).
fn reason_word(cause: &RangeSessionError) -> &'static str {
    match cause {
        RangeSessionError::Unauthentic { .. } => "corrupt",
        RangeSessionError::FileChanged => "version_gone",
        RangeSessionError::ApiNeeded(_) => "use_api",
        RangeSessionError::IdentityMismatch => "identity_mismatch",
        RangeSessionError::SizeMismatch => "size_mismatch",
        RangeSessionError::Sink(_) => "transfer",
        RangeSessionError::Incomplete => "incomplete",
        RangeSessionError::Unauthorized | RangeSessionError::Forbidden => "auth",
        RangeSessionError::DirectFailedAfterWrite { cause, .. } => reason_word(cause),
    }
}

/// Spec S6, S9, S10 and task 0783 for a session's result.
pub(crate) fn after_session(result: &Result<RangeOutcome, RangeSessionError>) -> AfterSession {
    match result {
        Ok(RangeOutcome::Whole(_)) => AfterSession::Promote,
        Ok(RangeOutcome::Closed(_)) => AfterSession::Leave,
        Ok(RangeOutcome::UseApi(_)) => AfterSession::WholeFilePath,
        // I4: a mismatch is poisoned and emptied even though this session wrote nothing.
        Err(RangeSessionError::IdentityMismatch) => AfterSession::Poison("identity_mismatch"),
        Err(e @ RangeSessionError::DirectFailedAfterWrite { .. }) => AfterSession::Poison(reason_word(e)),
        Err(RangeSessionError::SizeMismatch) => AfterSession::ResolveSize,
        Err(_) => AfterSession::Fail,
    }
}

/// The format word state.db records for a chunk format.
pub(crate) fn format_word(format: beebeeb_core::chunk_stream::ChunkFormat) -> &'static str {
    match format {
        beebeeb_core::chunk_stream::ChunkFormat::V1Legacy => "v1",
        beebeeb_core::chunk_stream::ChunkFormat::V2 => "v2",
    }
}

/// The identity a range session compares with (`RangeRequest::recorded`), from what state.db holds.
/// A record whose format word or version id cannot be read is no identity (so bytes on disk are a
/// mismatch: never trusted).
pub(crate) fn recorded_identity(record: Option<&crate::state_db::RecordedIdentity>) -> Option<RangeIdentity> {
    use beebeeb_core::chunk_stream::{ChunkFormat, VERSION_ID_LEN};
    let record = record?;
    let format = match record.format.as_str() {
        "v1" => ChunkFormat::V1Legacy,
        "v2" => ChunkFormat::V2,
        _ => return None,
    };
    let version_id = match (&record.version_id_hex, format) {
        (None, ChunkFormat::V1Legacy) => None,
        (Some(hex), ChunkFormat::V2) => {
            let bytes = (0..hex.len())
                .step_by(2)
                .map(|i| hex.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok()))
                .collect::<Option<Vec<u8>>>()?;
            Some(<[u8; VERSION_ID_LEN]>::try_from(bytes.as_slice()).ok()?)
        }
        _ => return None,
    };
    Some(RangeIdentity {
        object_version_id: record.object_version_id.clone(),
        format,
        version_id,
    })
}

/// [`recorded_identity`]'s inverse: what state.db records for `identity`.
pub(crate) fn identity_record(identity: &RangeIdentity) -> crate::state_db::RecordedIdentity {
    crate::state_db::RecordedIdentity {
        object_version_id: identity.object_version_id.clone(),
        format: format_word(identity.format).to_string(),
        version_id_hex: identity
            .version_id
            .map(|id| id.iter().map(|b| format!("{b:02x}")).collect()),
    }
}

/// Spec S9 (task 0783), before the session starts: the pinned version's plaintext size must be the
/// size Windows holds for the placeholder (`info.FileSize`). `Err(size)`: fail the fetch and resolve
/// the placeholder to the version's size; nothing was asked of the server yet.
pub(crate) fn size_check(pinned_plaintext: u64, file_size: u64) -> Result<(), u64> {
    if pinned_plaintext == file_size { Ok(()) } else { Err(pinned_plaintext) }
}

/// The state.db side of [`IdentityRecords`]: the identity (I3, I4) and then the 1640 candidate
/// `(version, NULL hash)` (spec S9), in one call, before the first transfer.
pub(crate) struct DbRecords {
    pub(crate) db: std::sync::Arc<crate::state_db::StateDb>,
    pub(crate) file_id: String,
    /// The server's version number of the pinned version (`download_version.version_number`).
    pub(crate) version_number: Option<i64>,
}

impl IdentityRecords for DbRecords {
    fn record(&mut self, identity: &RangeIdentity) -> std::io::Result<()> {
        let io = |e: anyhow::Error| std::io::Error::other(format!("state.db: {}", e.root_cause()));
        match self.db.record_range_identity(&self.file_id, &identity_record(identity)) {
            Ok(true) => {}
            Ok(false) => return Err(std::io::Error::from(std::io::ErrorKind::NotFound)),
            Err(e) => return Err(io(e.into())),
        }
        match self.version_number {
            Some(version) => self.db.record_fetch_candidate(&self.file_id, version),
            None => self.db.mark_content_unknown(&self.file_id),
        }
        .map_err(|e| io(e.into()))
    }
}

/// Contract Amendment 4: what the sweep does with a poisoned placeholder, given the content key the
/// poison recorded and the one the file has now (`LastWriteTime` + size, read under the empty
/// step's exclusive handle).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyVerdict {
    /// The bytes are the failed attempt's own: empty the file in S6's order.
    Empty,
    /// Something wrote to the file, or no key was recorded: never emptied, never cleared silently;
    /// the row shows "needs attention" until the person discards it.
    NeedsAttention,
}

pub(crate) fn key_verdict(recorded: Option<(i64, i64)>, now: Option<(i64, i64)>) -> KeyVerdict {
    match (recorded, now) {
        (Some(recorded), Some(now)) if recorded == now => KeyVerdict::Empty,
        _ => KeyVerdict::NeedsAttention,
    }
}

/// Which sweep this is (Amendment 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SweepKind {
    /// The first sweep of this process, before any tick.
    ProcessStart,
    /// A periodic engine tick.
    Tick,
    /// A handle on the file closed.
    Close,
}

/// Amendment 4: a needs-attention row is checked at engine start and on close only, never with a
/// per-tick exclusive open.
pub(crate) fn sweep_opens(needs_attention: bool, kind: SweepKind) -> bool {
    !needs_attention || kind != SweepKind::Tick
}

/// Amendment 4: a poison whose file is missing is cleared only when it is missing at the first
/// sweep of a new process (before any tick), never at a later tick.
pub(crate) fn clear_missing(kind: SweepKind) -> bool {
    kind == SweepKind::ProcessStart
}

/// Desktop #225 (review P2, lead ruling 2026-11-06): the range session's transport, gated so a
/// fetched chunk reaches the session (and so the sink) only while a request is pending. Core reads
/// ahead with no request pending (between a player's reads, and after its close); with
/// [`READ_AHEAD_ON_DEFAULT_KEY`] off its chunks wait here, booked against the process budget (S8),
/// until the next `FETCH_DATA` of the file, and go out under that request's keys. A closing session's
/// chunks are never released. Only chunk bodies wait (storage GETs and the pinned `/chunks/` route);
/// discovery and grant requests go through at once.
pub(crate) struct GatedIo<I> {
    pub(crate) inner: I,
    pub(crate) book: Arc<Mutex<DemandBook>>,
}

/// Wait until `book` has a pending request; never returns once the session is closing.
pub(crate) async fn until_requested(book: &Mutex<DemandBook>) {
    if READ_AHEAD_ON_DEFAULT_KEY {
        return;
    }
    loop {
        let changed = book.lock().unwrap_or_else(std::sync::PoisonError::into_inner).changed.clone();
        let wake = changed.notified();
        tokio::pin!(wake);
        // Registered before the check, so a demand added in between still wakes it.
        wake.as_mut().enable();
        let (closed, requested) = {
            let b = book.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            (b.closed, !b.live.is_empty())
        };
        if closed {
            return std::future::pending().await;
        }
        if requested {
            return;
        }
        wake.await;
    }
}

impl<I: beebeeb_transfer::DirectIo> beebeeb_transfer::DirectIo for GatedIo<I> {
    fn api(&self, call: beebeeb_transfer::ApiCall) -> impl Future<Output = beebeeb_transfer::ApiReply> + Send {
        let chunk = call.path.contains("/chunks/");
        let reply = self.inner.api(call);
        let book = self.book.clone();
        async move {
            let reply = reply.await;
            if chunk {
                until_requested(&book).await;
            }
            reply
        }
    }

    fn storage_get(
        &self,
        url: &beebeeb_transfer::SignedUrl,
        expected_len: u64,
        total: std::time::Duration,
        idle: std::time::Duration,
        connect: std::time::Duration,
    ) -> impl Future<Output = beebeeb_transfer::StorageReply> + Send {
        let reply = self.inner.storage_get(url, expected_len, total, idle, connect);
        let book = self.book.clone();
        async move {
            let reply = reply.await;
            until_requested(&book).await;
            reply
        }
    }

    fn storage_put(
        &self,
        url: &beebeeb_transfer::SignedUrl,
        body: bytes::Bytes,
        headers: &[(&'static str, String)],
        total: std::time::Duration,
    ) -> impl Future<Output = u16> + Send {
        self.inner.storage_put(url, body, headers, total)
    }

    fn sleep(&self, d: std::time::Duration) -> impl Future<Output = ()> + Send {
        self.inner.sleep(d)
    }

    fn wall_ms(&self) -> i64 {
        self.inner.wall_ms()
    }

    fn mono_ms(&self) -> i64 {
        self.inner.mono_ms()
    }
}

/// A requester process the session watches for its whole life (spec S5; desktop #225 review P2).
pub(crate) trait Exits {
    /// The process exited (its `OpenProcess(SYNCHRONIZE)` handle is signalled). A requester that
    /// cannot be watched (our own process, an open that failed) never exits.
    fn exited(&self) -> bool;
}

/// Why a session's watch ends it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionEnd {
    /// A requester exited and no live requester has a pending request (spec S5, §9: its GETs stop).
    RequesterGone,
    /// No request pending, none added or answered for [`IDLE_LIMIT`]: the lease, the 1640 ticket and
    /// the Activity row are released, as "not whole".
    Idle,
}

/// One look of the session's watch (every 250 ms): the requesters that exited (their demands
/// fail; `cancel` lists them for core), and whether the session ends.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Tick {
    pub(crate) cancel: Vec<u64>,
    pub(crate) end: Option<SessionEnd>,
}

/// The watch's rule. Every requester the session served is in `watched`, with or without a pending
/// request (a player between two reads is watched too).
pub(crate) fn session_tick<P: Exits>(watched: &mut Vec<(u32, P)>, book: &mut DemandBook, idle_limit: std::time::Duration) -> Tick {
    let mut tick = Tick::default();
    let mut exited = false;
    watched.retain(|(pid, process)| {
        if !process.exited() {
            return true;
        }
        let (ids, _) = book.requester_exited(*pid);
        tick.cancel.extend(ids);
        exited = true;
        false
    });
    tick.end = if exited && book.is_idle() {
        Some(SessionEnd::RequesterGone)
    } else if book.idle_for(idle_limit) {
        Some(SessionEnd::Idle)
    } else {
        None
    };
    tick
}

/// Amendment 4 with the key the session took before its first transfer (desktop #225 review P2): the
/// poison records that key only while the file still has it. A write during the session moved it, so
/// the bytes are not only the failed attempt's: no key (needs attention, never emptied). No key
/// before the first transfer: no key either.
pub(crate) fn poison_key(before: Option<(i64, i64)>, now: Option<(i64, i64)>) -> Option<(i64, i64)> {
    before.filter(|b| now == Some(*b))
}

/// What a session that ended `Whole` may promote (I6, and the lead's rulings of 2026-11-06).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Promotion {
    /// Every byte is on disk, the read-back's sha256 is this, and the content key is still the one
    /// taken before the first transfer: IN_SYNC, `Local`, agreed content (in that order, after the read).
    InSync(String),
    /// Wholly on disk, but the file changed since the session started (or its key was never read):
    /// not in sync, the NULL-hash candidate stays.
    Changed,
    /// Not every byte is on disk (a piece withheld, or the platform kept less than was sent), or the
    /// read-back failed: what was transferred stays, never "whole".
    NotWhole,
}

/// The promotion's verdict. `on_disk_whole`: ONDISK covers `[0, size)` (the on-disk range query, never
/// the bytes sent). `read_back`: the sha256 read from disk under the no-fetch scope and the content key
/// after the read (`None`: the read failed); read only when the first two checks passed.
pub(crate) fn promotion(
    withheld: bool,
    on_disk_whole: bool,
    key_before: Option<(i64, i64)>,
    read_back: Option<(String, Option<(i64, i64)>)>,
) -> Promotion {
    if withheld || !on_disk_whole {
        return Promotion::NotWhole;
    }
    match (key_before, read_back) {
        (_, None) => Promotion::NotWhole,
        (Some(before), Some((hash, after))) if after == Some(before) => Promotion::InSync(hash),
        _ => Promotion::Changed,
    }
}

/// Do these on-disk ranges `(offset, length)` cover `[0, size)`?
pub(crate) fn covers_whole(ranges: &[(u64, u64)], size: u64) -> bool {
    let mut ranges: Vec<(u64, u64)> = ranges.iter().copied().filter(|(_, len)| *len > 0).collect();
    ranges.sort_unstable();
    let mut end = 0u64;
    for (offset, len) in ranges {
        if offset > end {
            return false;
        }
        end = end.max(offset.saturating_add(len));
    }
    end >= size
}

/// Desktop #225 review P1 (lead ruling 2026-11-06): W3 declines an open only when the file can
/// stream: state.db holds the version its placeholder was minted from (the session's pin, S3; a
/// legacy file has none), and the direct range route is there (not forced to the API, and the server
/// offers direct downloads). Any other file gets the whole file through S0/S2, never declined.
pub(crate) fn file_streams(recorded_version: bool, forced_api: bool, direct_offered: bool) -> bool {
    recorded_version && !forced_api && direct_offered
}

#[cfg(test)]
#[path = "cf_range_tests.rs"]
mod tests;
