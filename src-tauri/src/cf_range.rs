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
        (self.next, rx)
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
    /// else any pending request (D-3: a provider may over-serve), else the latest request's transfer
    /// key with no request (read-ahead after every request was met). `None`: nothing was ever asked.
    pub(crate) fn keys_for(&self, offset: u64, len: u64) -> Option<RequestKeys> {
        let end = offset.saturating_add(len);
        let overlapping = self
            .live
            .values()
            .find(|d| d.offset < end && offset < d.offset.saturating_add(d.len));
        overlapping
            .or_else(|| self.live.values().next_back())
            .map(|d| d.keys)
            .or_else(|| {
                self.last_keys.map(|k| RequestKeys {
                    transfer_key: k.transfer_key,
                    request_key: REQUEST_KEY_DEFAULT,
                })
            })
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
    identity_recorded: bool,
    progress: Option<Box<dyn Fn(u64) + Send>>,
}

impl<P: PieceOut, R: IdentityRecords> StreamSink<P, R> {
    pub(crate) fn new(book: Arc<Mutex<DemandBook>>, out: P, records: R, file_size: u64) -> Self {
        StreamSink {
            book,
            out,
            records,
            file_size,
            transferred: 0,
            identity_recorded: false,
            progress: None,
        }
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
        let keys = self
            .book()
            .keys_for(offset, len)
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotConnected))?;
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

#[cfg(test)]
#[path = "cf_range_tests.rs"]
mod tests;
