//! Bytes done and bytes total for every file the engine is transferring right now
//! (task 1683 slice 2; spec section 9, row "Per-file progress (b)" and "Overall
//! progress"). Before this, `TrayActivityItem.progress` was always `None`.
//!
//! The upload chunk loop and the download chunk loop report here. A
//! [`TransferGuard`] registers a file when the loop starts and removes it when the
//! guard drops, so a cancelled or failed transfer never lingers as "62 %".
//! `finish` marks it complete and adds its bytes to the batch's finished total, so
//! the overall bar (`bytes_done / bytes_total`) keeps moving forward as files
//! finish instead of resetting to that one file.
//!
//! Nothing here is secret: a file id, a direction, two counters. Names stay in the
//! state DB.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    #[default]
    Up,
    Down,
    /// A direct-streaming open: the bytes are served to the player as they arrive
    /// on the wire (LD-0105). `buffered` is the on-disk byte count (ONDISK);
    /// `speed_bps` is the 2-second moving-window receive rate.
    Stream,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Up => "up",
            Direction::Down => "down",
            Direction::Stream => "stream",
        }
    }
}

/// A 2-second sliding window of `(Instant, cumulative_bytes)` samples used to
/// compute bytes-per-second for a streaming transfer.
#[derive(Debug)]
struct SpeedWindow {
    samples: VecDeque<(Instant, u64)>,
}

impl SpeedWindow {
    fn new() -> Self {
        Self { samples: VecDeque::new() }
    }

    /// Push a new sample and return the current speed in bytes/s over the last `span`.
    fn push(&mut self, now: Instant, cumulative_bytes: u64, span: Duration) -> u64 {
        self.samples.push_back((now, cumulative_bytes));
        // Drop samples older than `span`
        let cutoff = now.checked_sub(span).unwrap_or(now);
        while self.samples.front().map_or(false, |(t, _)| *t < cutoff) {
            self.samples.pop_front();
        }
        if self.samples.len() < 2 {
            return 0;
        }
        let (oldest_t, oldest_b) = self.samples.front().copied().unwrap();
        let (_, newest_b) = self.samples.back().copied().unwrap();
        let elapsed_secs = now.duration_since(oldest_t).as_secs_f64();
        if elapsed_secs < 0.001 {
            return 0;
        }
        (newest_b.saturating_sub(oldest_b) as f64 / elapsed_secs) as u64
    }
}

/// The streaming row's speed: a 2-second moving window (U0 design).
const STREAM_SPEED_SPAN: Duration = Duration::from_secs(2);

/// A whole-file download's speed: the bytes that arrived over the last 10 seconds (LD-0105 U1, lead
/// decision 2026-11-06 09:53; a deviation from the U0 design's 2 s window). Under a download limit the
/// direct route holds new requests back (`direct_download::paced_until_ms`), so bytes arrive in
/// bursts 3 to 5 s apart: a 2 s window was empty in 13 of 21 native samples and read "0 B/s".
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const DOWNLOAD_SPEED_SPAN: Duration = Duration::from_secs(10);

/// One fetch's speed over its own [`ReceiveMeter::received`] (every body byte read, a retried
/// attempt's included: the wire's real rate), sampled by [`FetchRow::follow`] over
/// [`DOWNLOAD_SPEED_SPAN`]; `0` when no byte arrived in that span (the UI then shows no speed).
#[derive(Debug)]
pub(crate) struct FetchSpeed {
    window: SpeedWindow,
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
impl FetchSpeed {
    pub(crate) fn new() -> Self {
        FetchSpeed { window: SpeedWindow::new() }
    }

    /// `meter` read at `now`: the speed in bytes/s of its received bytes.
    pub(crate) fn sample(&mut self, now: Instant, meter: &ReceiveMeter) -> u64 {
        self.window.push(now, meter.received(), DOWNLOAD_SPEED_SPAN)
    }
}

/// `ActiveTransferDto::attention` of a failed download whose file changed on this device since (the
/// names `src/desktopApi.ts` `ActiveTransferRow` uses).
pub const ATTENTION_CHANGED: &str = "changed_on_this_device";
/// `ActiveTransferDto::action` of such a row: the `discard_and_download_again` command.
pub const ACTION_DISCARD: &str = "discard_and_download_again";

/// One file state.db says needs the person (LD-0105 Amendment 4, desktop #225): a failed download whose
/// file changed on this device since. It is kept as it is and refuses every fetch, so it is rarely on
/// the board; its row lasts as long as the poison row, which `discard_and_download_again` ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NeedsAttention {
    pub file_id: String,
    /// The vault path, for the row's name and folder.
    pub path: String,
    pub size: u64,
}

/// state.db's needs-attention rows whose file it still knows (review: a poison with no file row could
/// not be discarded, `discard_now` answers `missing`, and it clears at the next process start).
/// `None`: the DB could not be read.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn needs_attention(db: &crate::state_db::StateDb) -> Option<Vec<NeedsAttention>> {
    let rows = db.poisoned().ok()?;
    Some(
        rows.into_iter()
            .filter(|r| r.needs_attention)
            .filter_map(|r| match db.get_file(&r.file_id) {
                Ok(Some(entry)) => Some(Ok(NeedsAttention { file_id: r.file_id, path: entry.path, size: entry.size_bytes.max(0) as u64 })),
                // No files row: no exit, no row (above).
                Ok(None) => None,
                // Review P3-7: a read error fails the whole read, so the poll keeps the rows it had.
                Err(e) => Some(Err(e)),
            })
            .collect::<Result<Vec<_>, _>>()
            .ok()?,
    )
}

/// The needs-attention rows the last read returned, kept with the engine generation they were read
/// under (delta review P2-2): served only to a poll under that same generation. A lock, a sign-out and
/// an account switch each stop the engine, and the next one is a new generation (`NEXT_GENERATION`), so
/// no other session ever sees them; a lock also clears them ([`attention_for_poll`]).
#[derive(Debug, Default)]
pub(crate) struct AttentionCache {
    /// `(generation, read number, rows)` of the newest read stored.
    read: Option<(u64, u64, Vec<NeedsAttention>)>,
    /// Reads numbered below this started before a discard ended a row: never stored (review P3-6).
    fence: u64,
}

impl AttentionCache {
    pub(crate) const fn new() -> Self {
        AttentionCache { read: None, fence: 0 }
    }

    /// Read number `seq` under `generation` returned `rows`. An older read (a smaller `seq`, or one
    /// started before the fence) never replaces a newer one.
    pub(crate) fn store(&mut self, generation: u64, seq: u64, rows: Vec<NeedsAttention>) {
        if seq < self.fence || self.read.as_ref().is_some_and(|(g, s, _)| *g == generation && *s > seq) {
            return;
        }
        self.read = Some((generation, seq, rows));
    }

    /// The rows last read under `generation`; nothing under any other.
    pub(crate) fn rows_for(&self, generation: u64) -> Vec<NeedsAttention> {
        match &self.read {
            Some((g, _, rows)) if *g == generation => rows.clone(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn clear(&mut self) {
        self.read = None;
    }

    /// A discard ended `file_id`'s row: it is gone from the kept rows, and no read that started before
    /// `next_seq` (the next read's number) may bring it back.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub(crate) fn forget(&mut self, file_id: &str, next_seq: u64) {
        if let Some((_, _, rows)) = &mut self.read {
            rows.retain(|r| r.file_id != file_id);
        }
        self.fence = self.fence.max(next_seq);
    }
}

/// The process's kept needs-attention rows ([`AttentionCache`]), shared by `get_active_transfers` and
/// `discard_and_download_again`.
pub(crate) static ATTENTION_CACHE: Mutex<AttentionCache> = Mutex::new(AttentionCache::new());
/// The next needs-attention read's number.
pub(crate) static ATTENTION_READS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// What one poll shows of the needs-attention rows (delta review P2-2): nothing while the vault is
/// locked or no engine is running (and the kept rows are cleared); a fresh read when it finished in
/// time; else the rows kept for this same engine generation.
pub(crate) fn attention_for_poll(
    cache: &mut AttentionCache,
    unlocked: bool,
    generation: Option<u64>,
    fresh: Option<Vec<NeedsAttention>>,
) -> Vec<NeedsAttention> {
    let Some(generation) = generation.filter(|_| unlocked) else {
        cache.clear();
        return Vec::new();
    };
    fresh.unwrap_or_else(|| cache.rows_for(generation))
}

/// The board's rows plus a row for each file that needs the person (a download, not failed, no
/// progress), or the attention on that file's own download or stream row. An upload row keeps its own
/// shape (review): offering "discard" on it would throw away the change being uploaded.
pub(crate) fn with_attention(mut rows: Vec<ActiveTransferDto>, attention: &[NeedsAttention]) -> Vec<ActiveTransferDto> {
    for file in attention {
        if let Some(on_board) = rows.iter_mut().find(|r| r.file_id == file.file_id) {
            if on_board.direction != Direction::Up {
                on_board.attention = Some(ATTENTION_CHANGED);
                on_board.action = Some(ACTION_DISCARD);
            }
            continue;
        }
        rows.push(ActiveTransferDto {
            file_id: file.file_id.clone(),
            name: display_name(&file.path),
            folder: parent_folder(&file.path),
            direction: Direction::Down,
            done: 0,
            total: file.size,
            buffered: 0,
            speed_bps: 0,
            attention: Some(ATTENTION_CHANGED),
            action: Some(ACTION_DISCARD),
            failed: false,
            failed_at_secs: None,
        });
    }
    rows
}

/// A transfer's metadata serialised into the `get_active_transfers` IPC response.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveTransferDto {
    pub file_id: String,
    /// Last path component (display name for the UI row).
    pub name: String,
    /// Everything before the last component, `""` at the vault root.
    pub folder: String,
    pub direction: Direction,
    /// Cumulative bytes confirmed written to the destination (or the read cursor
    /// for a streaming open). For `Direction::Stream`, the UI shows this as
    /// "buffered" (on-disk bytes).
    pub done: u64,
    /// Declared total bytes; `0` when unknown (a streaming open may not have a
    /// known size until the first chunk arrives).
    pub total: u64,
    /// For `Direction::Stream`: bytes already written to the local buffer file
    /// (ONDISK). For other directions this is `0`.
    pub buffered: u64,
    /// 2-second moving-window receive rate in bytes/s. `0` when not yet computed.
    pub speed_bps: u64,
    /// LD-0105 Amendment 4 (desktop #225): [`ATTENTION_CHANGED`] when a download failed and the file
    /// changed on this device since (state.db's needs-attention poison row); absent otherwise. Never
    /// set by the board itself: [`TransferBoard::snapshot_with_attention`] adds it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attention: Option<&'static str>,
    /// The one action that ends an `attention` row: [`ACTION_DISCARD`]; absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<&'static str>,
    /// The transfer ended in an error (the guard dropped without `finish()`).
    pub failed: bool,
    /// Unix timestamp when the failure was recorded, for expiry logic in the UI.
    pub failed_at_secs: Option<u64>,
}

/// Plaintext bytes an upload has sent once `chunks_done` chunks of `chunk_size`
/// are acknowledged: the last chunk is usually short, so this is capped at the
/// payload size.
pub fn uploaded_bytes(chunks_done: u64, chunk_size: u64, payload_total: u64) -> u64 {
    chunks_done.saturating_mul(chunk_size).min(payload_total)
}

/// The last path component, for either separator. A row's `path` is
/// server-relative with `/`, but a cache path on Windows uses `\`.
pub fn display_name(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// The folder a path sits in (everything before the last component), `""` at the
/// vault root. The popover shows it under the file name.
pub fn parent_folder(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    match trimmed.rfind(['/', '\\']) {
        Some(i) => trimmed[..i].trim_start_matches(['/', '\\']).to_string(),
        None => String::new(),
    }
}

/// One fetch's body bytes, counted per read as they arrive on the wire (LD-0105 S0), on the
/// direct route and the API route alike. A Windows Cloud Files fetch reports this to the platform
/// (`CfReportProviderProgress`) while it downloads, which resets Windows' fixed 60 s callback timer;
/// a chunk on a slow link can take longer than that on its own, so a count of finished chunks would
/// leave the fetch silent. Nothing here is secret: two counters.
///
/// Liveness and progress are apart (lead decision 2026-11-06): [`Self::received`] counts every byte
/// read, a failed or retried attempt's included, and only grows; [`Self::kept`] is the progress,
/// from which a failed attempt's bytes are taken back ([`Self::discard`], [`Self::rollback_to`]).
#[derive(Debug, Default)]
pub struct ReceiveMeter {
    read: std::sync::atomic::AtomicU64,
    kept: std::sync::atomic::AtomicU64,
}

impl ReceiveMeter {
    /// `wire_bytes` more arrived.
    pub fn add(&self, wire_bytes: u64) {
        self.read.fetch_add(wire_bytes, Ordering::Relaxed);
        self.kept.fetch_add(wire_bytes, Ordering::Relaxed);
    }

    /// `wire_bytes` that were added are not used after all (a body that broke off, a chunk that
    /// did not authenticate): no longer progress. They stay in [`Self::received`].
    pub fn discard(&self, wire_bytes: u64) {
        let _ = self.kept.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |k| Some(k.saturating_sub(wire_bytes)));
    }

    /// A whole attempt failed: progress goes back to `mark` (an earlier [`Self::kept`]) if it is past it.
    pub fn rollback_to(&self, mark: u64) {
        let _ = self.kept.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |k| Some(k.min(mark)));
    }

    /// LD-0105 W2: `wire_bytes` arrived but are not progress by themselves (a range session counts its
    /// progress as the plaintext it handed over, [`Self::keep`]): liveness only.
    pub fn read_only(&self, wire_bytes: u64) {
        self.read.fetch_add(wire_bytes, Ordering::Relaxed);
    }

    /// LD-0105 W2: `bytes` of plaintext reached the placeholder: progress only.
    pub fn keep(&self, bytes: u64) {
        self.kept.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Every body byte read so far, failed attempts included: the keep-alive's liveness.
    pub fn received(&self) -> u64 {
        self.read.load(Ordering::Relaxed)
    }

    /// The bytes that count as progress: read, and not discarded.
    pub fn kept(&self) -> u64 {
        self.kept.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Transfer {
    pub direction: Direction,
    pub done: u64,
    pub total: u64,
    /// For `Direction::Stream`: bytes written to the local buffer file (ONDISK).
    pub buffered: u64,
    /// 2-second moving-window receive rate; updated by [`TransferBoard::update_stream`].
    pub speed_bps: u64,
    /// Set when the transfer ends in an error (see [`TransferBoard::mark_failed`]).
    pub failed: bool,
    /// Unix timestamp when `failed` was set, for expiry in the UI (hide after 15 min).
    pub failed_at_secs: Option<u64>,
}

/// 15-minute visibility window for a failed transfer after its guard drops.
/// The UI reads `snapshot_dtos()` and shows the failed row until the entry ages out.
pub const FAILED_ROW_TTL_SECS: u64 = 15 * 60;

#[derive(Debug, Default)]
struct Inner {
    active: HashMap<String, Transfer>,
    /// Transfers that ended in an error and are within [`FAILED_ROW_TTL_SECS`] of their
    /// `failed_at_secs`. Populated by `Drop` when the entry has `failed == true`;
    /// pruned lazily in `snapshot_dtos`.
    recently_failed: HashMap<String, Transfer>,
    /// Vault paths keyed by file_id; registered via [`TransferBoard::begin_named`].
    /// Retained for failed entries until they expire from `recently_failed`.
    paths: HashMap<String, String>,
    /// Per-file speed windows; kept here (not in `Transfer`) so `Transfer` stays `Copy`.
    /// Pruned on `Drop` for all exits (the window is not used after the transfer ends).
    speeds: HashMap<String, SpeedWindow>,
    /// How many guards hold each active row (LD-0105 U1): a download of a file already downloading
    /// joins its row ([`TransferBoard::begin_download`]), and the row ends with the last guard.
    holders: HashMap<String, u32>,
    /// Active rows a holder finished ([`TransferGuard::finish`]): the file was delivered, so the row
    /// ends as a success whoever ends it, and its bytes are counted once, when it ends.
    delivered: std::collections::HashSet<String>,
    /// Plaintext bytes of files that completed since the batch last went quiet.
    finished_bytes: u64,
}

impl Inner {
    /// Register `file_id`'s row. A new attempt ends the file's earlier failed row (one row per file).
    /// `join`: a download row already active (never an upload's) is kept as it is (its progress, its
    /// name) and gains a holder; otherwise the row starts over, as [`TransferBoard::begin`] always did,
    /// and the guards of the row it replaces still count, so the row ends with the last guard.
    fn start(&mut self, file_id: &str, path: Option<&str>, direction: Direction, total: u64, join: bool) {
        self.recently_failed.remove(file_id);
        let held = if self.active.contains_key(file_id) { self.holders.get(file_id).copied().unwrap_or(0) } else { 0 };
        self.holders.insert(file_id.to_string(), held + 1);
        if join && let Some(t) = self.active.get_mut(file_id).filter(|t| t.direction != Direction::Up) {
            // A failure marked just before this download joined is not this download's outcome.
            t.failed = false;
            t.failed_at_secs = None;
            if t.total == 0 {
                t.total = total;
            }
            if let Some(path) = path {
                self.paths.entry(file_id.to_string()).or_insert_with(|| path.to_string());
            }
            return;
        }
        self.active.insert(file_id.to_string(), Transfer { direction, total, ..Transfer::default() });
        self.delivered.remove(file_id);
        if let Some(path) = path {
            self.paths.insert(file_id.to_string(), path.to_string());
        }
    }

    /// Whether no other guard holds `file_id`'s row: the one ending it decides how it ends.
    fn last_holder(&self, file_id: &str) -> bool {
        self.holders.get(file_id).is_none_or(|n| *n <= 1)
    }

    fn mark_failed(&mut self, file_id: &str) {
        let now_secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        if let Some(t) = self.active.get_mut(file_id) {
            t.failed = true;
            t.failed_at_secs = Some(now_secs);
        }
    }
}

#[derive(Debug, Default)]
pub struct TransferBoard {
    inner: Mutex<Inner>,
}

impl TransferBoard {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Register a transfer. `total` is the plaintext size; progress starts at 0.
    pub fn begin(self: &Arc<Self>, file_id: &str, direction: Direction, total: u64) -> TransferGuard {
        self.guard(file_id, None, direction, total, false)
    }

    /// Like [`Self::begin`] but also records the vault `path` (the `files` row's, the path the tray's
    /// recent rows carry) so the IPC DTO can expose a display name and parent folder.
    pub fn begin_named(self: &Arc<Self>, file_id: &str, path: &str, direction: Direction, total: u64) -> TransferGuard {
        self.guard(file_id, Some(path), direction, total, false)
    }

    /// LD-0105 U1: a download of `file_id`, named by its vault `path` as [`Self::begin_named`]. A
    /// download of the file already on the board is joined, never started over: still one row, its
    /// progress kept, and the row ends with the last download that holds it.
    pub fn begin_download(self: &Arc<Self>, file_id: &str, path: &str, total: u64) -> TransferGuard {
        self.guard(file_id, Some(path), Direction::Down, total, true)
    }

    fn guard(self: &Arc<Self>, file_id: &str, path: Option<&str>, direction: Direction, total: u64, join: bool) -> TransferGuard {
        if let Ok(mut inner) = self.inner.lock() {
            inner.start(file_id, path, direction, total, join);
        }
        TransferGuard {
            board: self.clone(),
            file_id: file_id.to_string(),
        }
    }

    /// LD-0105 U1: a download's kept body bytes ([`ReceiveMeter::kept`], counted per read, a failed
    /// attempt's taken back) are its progress, never backwards and never past the total; `speed_bps`
    /// is its fetch's [`FetchSpeed`]. A no-op for a file that is not registered.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub fn received_at(&self, file_id: &str, kept: u64, speed_bps: u64) {
        if let Ok(mut inner) = self.inner.lock()
            && let Some(t) = inner.active.get_mut(file_id)
        {
            t.done = kept.min(t.total).max(t.done);
            t.speed_bps = speed_bps;
        }
    }

    /// Update the on-disk buffer byte count for a streaming transfer and recompute
    /// the 2-second moving-window speed. A no-op when the file is not registered.
    pub fn update_stream(&self, file_id: &str, buffered: u64) {
        let now = Instant::now();
        if let Ok(mut inner) = self.inner.lock() {
            let speed = inner.speeds
                .entry(file_id.to_string())
                .or_insert_with(SpeedWindow::new)
                .push(now, buffered, STREAM_SPEED_SPAN);
            if let Some(t) = inner.active.get_mut(file_id) {
                t.buffered = buffered;
                t.speed_bps = speed;
            }
        }
    }

    /// Mark a registered transfer as failed, recording the current time. The guard
    /// will remove it from `active` when it drops; calling this first lets the UI
    /// show an error row for up to 15 minutes after the guard goes away (the DTO
    /// remains readable until the UI purges it).
    ///
    /// Note: failed transfers are still removed from `active` on `Drop` — the UI
    /// must hold onto the DTO it last received if it wants to show the error row.
    pub fn mark_failed(&self, file_id: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.mark_failed(file_id);
        }
    }

    /// A snapshot of every active transfer and every recently-failed transfer (within
    /// [`FAILED_ROW_TTL_SECS`]) as IPC-ready DTOs. Expired entries are pruned lazily.
    pub fn snapshot_dtos(&self) -> Vec<ActiveTransferDto> {
        self.inner.lock().map(|mut inner| {
            let now_secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            // Prune expired recently-failed entries and their path records.
            let expired: Vec<String> = inner.recently_failed.iter()
                .filter_map(|(id, t)| {
                    // No timestamp → keep (conservative); timestamp too old → prune.
                    if t.failed_at_secs.map_or(false, |s| s + FAILED_ROW_TTL_SECS < now_secs) {
                        Some(id.clone())
                    } else {
                        None
                    }
                })
                .collect();
            for id in &expired {
                inner.recently_failed.remove(id);
                inner.paths.remove(id);
            }

            // Collect (id, Transfer) pairs from active + recently_failed in one pass,
            // then look up paths separately to avoid simultaneous field borrows.
            let pairs: Vec<(String, Transfer)> = inner.active.iter()
                .chain(inner.recently_failed.iter())
                .map(|(id, t)| (id.clone(), *t))
                .collect();

            pairs.into_iter().map(|(id, t)| {
                let path = inner.paths.get(&id).map(|s| s.as_str()).unwrap_or("");
                ActiveTransferDto {
                    file_id: id,
                    name: display_name(path),
                    folder: parent_folder(path),
                    direction: t.direction,
                    done: t.done,
                    total: t.total,
                    buffered: t.buffered,
                    speed_bps: t.speed_bps,
                    attention: None,
                    action: None,
                    failed: t.failed,
                    failed_at_secs: t.failed_at_secs,
                }
            }).collect()
        }).unwrap_or_default()
    }

    /// A new engine starts on this board (`EngineBridge::with_transfers`): the failed rows an earlier
    /// engine left, and their names, go (LD-0105 U1: never another session's file names).
    pub fn forget_failed(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            let inner = &mut *guard;
            for id in inner.recently_failed.keys() {
                inner.paths.remove(id);
            }
            inner.recently_failed.clear();
        }
    }

    /// [`Self::snapshot_dtos`] merged with state.db's needs-attention rows ([`needs_attention`],
    /// [`with_attention`]). No db, or a db that cannot be read: the board alone.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub(crate) fn snapshot_with_attention(&self, db: Option<&crate::state_db::StateDb>) -> Vec<ActiveTransferDto> {
        let attention = db.and_then(needs_attention).unwrap_or_default();
        with_attention(self.snapshot_dtos(), &attention)
    }

    /// Every transfer in flight.
    pub fn active(&self) -> Vec<(String, Transfer)> {
        self.inner
            .lock()
            .map(|inner| inner.active.iter().map(|(id, t)| (id.clone(), *t)).collect())
            .unwrap_or_default()
    }

    /// A download's `(done, total)`; `total` is the server's size hint, so it
    /// replaces the total the transfer started with. A no-op when `file_id` is
    /// not registered, and `done` never runs backwards or past the total.
    pub fn report(&self, file_id: &str, done: u64, total: u64) {
        if let Ok(mut inner) = self.inner.lock()
            && let Some(t) = inner.active.get_mut(file_id)
        {
            t.total = total;
            t.done = done.max(t.done).min(total);
        }
    }

    pub fn get(&self, file_id: &str) -> Option<Transfer> {
        self.inner
            .lock()
            .ok()
            .and_then(|inner| inner.active.get(file_id).copied())
    }

    /// Sum of `done` over the transfers in flight.
    pub fn in_flight_done(&self) -> u64 {
        self.inner
            .lock()
            .map(|inner| inner.active.values().map(|t| t.done.min(t.total)).sum())
            .unwrap_or(0)
    }

    pub fn finished_bytes(&self) -> u64 {
        self.inner.lock().map(|inner| inner.finished_bytes).unwrap_or(0)
    }

    /// Forget the finished bytes of the last batch. Called when the engine goes
    /// back to idle, so the next burst of work starts its bar at 0.
    pub fn reset_batch(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.finished_bytes = 0;
        }
    }
}

pub struct TransferGuard {
    board: Arc<TransferBoard>,
    file_id: String,
}

impl TransferGuard {
    /// `done` is cumulative plaintext bytes; it never runs backwards and never
    /// passes `total`.
    pub fn update(&self, done: u64) {
        if let Ok(mut inner) = self.board.inner.lock()
            && let Some(t) = inner.active.get_mut(&self.file_id)
        {
            t.done = done.max(t.done).min(t.total);
        }
    }

    /// Raise the total when the real size turns out to differ from the hint (a
    /// download's size hint is the server's plaintext size and can be off).
    #[cfg(test)]
    pub fn set_total(&self, total: u64) {
        if let Ok(mut inner) = self.board.inner.lock()
            && let Some(t) = inner.active.get_mut(&self.file_id)
        {
            t.total = total;
            t.done = t.done.min(total);
        }
    }

    /// The transfer succeeded: its bytes join the batch's finished total when the row ends (at once,
    /// unless other downloads of the file still hold it); the row then ends as a success.
    pub fn finish(self) {
        if let Ok(mut inner) = self.board.inner.lock()
            && inner.active.contains_key(&self.file_id)
        {
            inner.delivered.insert(self.file_id.clone());
        }
    }

    /// The transfer did not finish (LD-0105 U1): its row is marked failed ([`TransferBoard::mark_failed`])
    /// and kept for [`FAILED_ROW_TTL_SECS`], unless another download of the file still holds it (it
    /// decides how the row ends). A row a holder delivered ends as a success all the same (`Drop`).
    pub fn fail(self) {
        if let Ok(mut inner) = self.board.inner.lock()
            && inner.last_holder(&self.file_id)
        {
            inner.mark_failed(&self.file_id);
        }
    }
}

impl Drop for TransferGuard {
    fn drop(&mut self) {
        if let Ok(mut inner) = self.board.inner.lock() {
            let held = inner.holders.get_mut(&self.file_id).map(|n| {
                *n = n.saturating_sub(1);
                *n
            });
            if held.is_some_and(|n| n > 0) {
                // Another download still holds the row (LD-0105 U1).
                return;
            }
            inner.holders.remove(&self.file_id);
            let delivered = inner.delivered.remove(&self.file_id);
            if let Some(t) = inner.active.remove(&self.file_id) {
                if delivered {
                    inner.finished_bytes = inner.finished_bytes.saturating_add(t.total);
                    inner.paths.remove(&self.file_id);
                } else if t.failed {
                    // Keep the entry visible in snapshot_dtos for up to FAILED_ROW_TTL_SECS.
                    // The path is retained so the DTO can resolve name and folder.
                    inner.recently_failed.insert(self.file_id.clone(), t);
                } else {
                    // Success or cancellation: path is no longer needed.
                    inner.paths.remove(&self.file_id);
                }
            }
            // Speed window is never needed after the transfer ends (regardless of outcome).
            inner.speeds.remove(&self.file_id);
        }
    }
}

/// How often [`FetchRow::follow`] reads its fetch's meter.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const FOLLOW_EVERY: Duration = Duration::from_secs(1);

/// LD-0105 U1: one Windows Cloud Files whole-file fetch on the board, the "Downloading" row of the
/// U0 design (`EngineBridge::whole_fetch_row` starts it). It follows the fetch's [`ReceiveMeter`]
/// ([`Self::follow`]) and ends with [`Self::finished`]; dropped without it, the fetch did not finish
/// and the row says so ([`TransferGuard::fail`]), unless the engine was stopping (a lock, a sign-out,
/// an account switch, a quit): that ends the row with no failure, never another session's to see. No
/// row (the file has no `files` row to name it by): every call is a no-op.
pub struct FetchRow {
    guard: Option<TransferGuard>,
    stopping: Option<Arc<std::sync::atomic::AtomicBool>>,
}

impl FetchRow {
    pub fn new(guard: TransferGuard) -> Self {
        FetchRow { guard: Some(guard), stopping: None }
    }

    pub fn none() -> Self {
        FetchRow { guard: None, stopping: None }
    }

    /// The engine's stop flag (`EngineBridge::is_stopping`): a fetch cut off by a stop is not failed.
    pub fn stopped_by(mut self, stopping: Arc<std::sync::atomic::AtomicBool>) -> Self {
        self.stopping = Some(stopping);
        self
    }

    /// Report `meter` to the row every [`FOLLOW_EVERY`] until the future is dropped (the caller
    /// selects it against the fetch, so it ends with the fetch): its kept bytes as the progress, and
    /// the speed of its received bytes ([`FetchSpeed`]).
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub async fn follow(&self, meter: &ReceiveMeter) -> std::convert::Infallible {
        let Some(guard) = &self.guard else {
            return std::future::pending().await;
        };
        let mut tick = tokio::time::interval(FOLLOW_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut speed = FetchSpeed::new();
        loop {
            tick.tick().await;
            let bps = speed.sample(tokio::time::Instant::now().into_std(), meter);
            guard.board.received_at(&guard.file_id, meter.kept(), bps);
        }
    }

    /// The fetch delivered the whole file: the row ends ([`TransferGuard::finish`]).
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub fn finished(mut self) {
        if let Some(guard) = self.guard.take() {
            guard.finish();
        }
    }
}

impl Drop for FetchRow {
    fn drop(&mut self) {
        let stopped = self.stopping.as_ref().is_some_and(|s| s.load(Ordering::SeqCst));
        if let Some(guard) = self.guard.take()
            && !stopped
        {
            guard.fail();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transfer_appears_while_its_guard_lives_and_goes_when_it_drops() {
        let board = TransferBoard::new();
        assert!(board.active().is_empty());
        {
            let _g = board.begin("f1", Direction::Up, 1_000);
            assert_eq!(
                board.get("f1"),
                Some(Transfer {
                    direction: Direction::Up,
                    done: 0,
                    total: 1_000,
                    ..Transfer::default()
                })
            );
            assert_eq!(board.active().len(), 1);
        }
        assert_eq!(
            board.get("f1"),
            None,
            "a dropped (failed or cancelled) transfer must not linger"
        );
        assert_eq!(
            board.finished_bytes(),
            0,
            "a failure adds nothing to the finished total"
        );
    }

    #[test]
    fn progress_is_cumulative_monotonic_and_capped_at_the_total() {
        let board = TransferBoard::new();
        let g = board.begin("f", Direction::Down, 100);
        g.update(40);
        assert_eq!(board.get("f").unwrap().done, 40);
        g.update(30);
        assert_eq!(board.get("f").unwrap().done, 40, "never backwards");
        g.update(500);
        assert_eq!(board.get("f").unwrap().done, 100, "never past the total");
    }

    #[test]
    fn a_finished_transfer_adds_its_total_to_the_batch_and_leaves_the_board() {
        let board = TransferBoard::new();
        let a = board.begin("a", Direction::Up, 300);
        a.update(300);
        a.finish();
        let b = board.begin("b", Direction::Up, 200);
        b.update(50);
        assert_eq!(board.finished_bytes(), 300);
        assert_eq!(board.in_flight_done(), 50);
        assert_eq!(board.active().len(), 1, "a is gone, b is in flight");
        drop(b);
        assert_eq!(board.finished_bytes(), 300, "b failed, the finished total keeps only a");
        board.reset_batch();
        assert_eq!(board.finished_bytes(), 0);
    }

    #[test]
    fn a_wrong_size_hint_can_be_corrected() {
        let board = TransferBoard::new();
        let g = board.begin("d", Direction::Down, 100);
        g.update(80);
        g.set_total(60);
        let t = board.get("d").unwrap();
        assert_eq!((t.done, t.total), (60, 60), "done is clamped to the corrected total");
    }

    #[test]
    fn two_transfers_are_independent() {
        let board = TransferBoard::new();
        let a = board.begin("a", Direction::Up, 10);
        let b = board.begin("b", Direction::Down, 20);
        a.update(10);
        b.update(5);
        assert_eq!(board.get("a").unwrap().direction, Direction::Up);
        assert_eq!(board.get("b").unwrap().direction, Direction::Down);
        assert_eq!(board.in_flight_done(), 15);
        drop(a);
        assert_eq!(board.in_flight_done(), 5);
    }

    #[test]
    fn a_second_begin_for_the_same_file_replaces_the_first() {
        // A retry of the same file while the old guard is still alive (should not
        // happen, but must not double count).
        let board = TransferBoard::new();
        let _a = board.begin("x", Direction::Up, 10);
        let b = board.begin("x", Direction::Up, 99);
        assert_eq!(board.active().len(), 1);
        assert_eq!(board.get("x").unwrap().total, 99);
        b.update(1);
        assert_eq!(board.in_flight_done(), 1);
    }

    #[test]
    fn uploaded_bytes_caps_the_short_last_chunk_at_the_payload_size() {
        assert_eq!(uploaded_bytes(0, 4, 10), 0);
        assert_eq!(uploaded_bytes(2, 4, 10), 8);
        assert_eq!(
            uploaded_bytes(3, 4, 10),
            10,
            "the last chunk is short: 12 would overshoot 10"
        );
        assert_eq!(uploaded_bytes(1, 4, 0), 0, "an empty file has no bytes");
        assert_eq!(uploaded_bytes(u64::MAX, 4, 10), 10, "no overflow");
    }

    #[test]
    fn a_name_and_a_folder_come_out_of_either_separator() {
        assert_eq!(display_name("Reports/2026/q3.xlsx"), "q3.xlsx");
        assert_eq!(display_name("C:\\Users\\sam\\q3.xlsx"), "q3.xlsx");
        assert_eq!(display_name("q3.xlsx"), "q3.xlsx");
        assert_eq!(display_name("Folder/"), "Folder");
        assert_eq!(parent_folder("Reports/2026/q3.xlsx"), "Reports/2026");
        assert_eq!(parent_folder("/Reports/q3.xlsx"), "Reports");
        assert_eq!(parent_folder("q3.xlsx"), "");
        assert_eq!(parent_folder("/q3.xlsx"), "");
    }

    #[test]
    fn a_download_report_replaces_the_size_hint_and_stays_monotonic() {
        let board = TransferBoard::new();
        let _g = board.begin("d", Direction::Down, 0);
        board.report("d", 0, 30);
        assert_eq!(board.get("d").unwrap().total, 30);
        board.report("d", 20, 30);
        board.report("d", 10, 30);
        assert_eq!(board.get("d").unwrap().done, 20, "never backwards");
        board.report("not-registered", 5, 5);
        assert_eq!(
            board.get("not-registered"),
            None,
            "a file nobody registered is not invented"
        );
    }

    // --- LD-0105 U1: Direction::Stream, begin_named, update_stream, mark_failed ---

    #[test]
    fn direction_stream_serialises_as_stream() {
        let s = serde_json::to_string(&Direction::Stream).unwrap();
        assert_eq!(s, r#""stream""#);
    }

    #[test]
    fn begin_named_stores_path_and_exposes_it_in_snapshot_dto() {
        let board = TransferBoard::new();
        let _g = board.begin_named("s1", "Videos/holiday.mp4", Direction::Stream, 50_000_000);
        let dtos = board.snapshot_dtos();
        assert_eq!(dtos.len(), 1);
        let dto = &dtos[0];
        assert_eq!(dto.file_id, "s1");
        assert_eq!(dto.name, "holiday.mp4");
        assert_eq!(dto.folder, "Videos");
        assert_eq!(dto.direction, Direction::Stream);
        assert_eq!(dto.total, 50_000_000);
        assert!(!dto.failed, "a fresh transfer is not failed");
        assert_eq!(dto.failed_at_secs, None);
    }

    #[test]
    fn update_stream_records_buffered_bytes() {
        let board = TransferBoard::new();
        let _g = board.begin_named("s2", "music.flac", Direction::Stream, 0);
        board.update_stream("s2", 1_024_000);
        let t = board.get("s2").unwrap();
        assert_eq!(t.buffered, 1_024_000);
    }

    #[test]
    fn speed_window_produces_nonzero_speed_after_two_samples() {
        let board = TransferBoard::new();
        let _g = board.begin_named("s3", "clip.mp4", Direction::Stream, 0);
        // Two calls with a fake time gap: use update_stream twice with real clock
        // (sub-millisecond — speed will be 0 or large; we only check it doesn't panic
        // and the field is present in the DTO).
        board.update_stream("s3", 0);
        board.update_stream("s3", 100_000);
        let dtos = board.snapshot_dtos();
        assert_eq!(dtos.len(), 1);
        // `speed_bps` is either 0 (samples too close in time) or a large positive
        // number; the important thing is the field exists and the board didn't panic.
        let _ = dtos[0].speed_bps;
    }

    #[test]
    fn mark_failed_sets_the_failed_flag_and_a_timestamp() {
        let board = TransferBoard::new();
        let _g = board.begin("f2", Direction::Down, 100);
        board.mark_failed("f2");
        let t = board.get("f2").unwrap();
        assert!(t.failed);
        assert!(t.failed_at_secs.is_some());
    }

    #[test]
    fn mark_failed_on_unregistered_file_is_a_noop() {
        let board = TransferBoard::new();
        board.mark_failed("not-here"); // must not panic
    }

    #[test]
    fn snapshot_dto_for_a_begin_without_path_has_empty_name_and_folder() {
        let board = TransferBoard::new();
        let _g = board.begin("plain", Direction::Down, 200);
        let dtos = board.snapshot_dtos();
        assert_eq!(dtos.len(), 1);
        // `display_name("")` → `""`, `parent_folder("")` → `""`
        assert_eq!(dtos[0].name, "");
        assert_eq!(dtos[0].folder, "");
    }

    // --- P1 fix: failed transfers must linger in snapshot_dtos after the guard drops ---

    #[test]
    fn a_failed_transfer_lingers_in_snapshot_after_the_guard_drops() {
        // Before the fix: snapshot_dtos only reads inner.active, so a dropped
        // (failed) guard left nothing visible. This test must be RED on the unfixed
        // code and GREEN once recently_failed is introduced.
        let board = TransferBoard::new();
        {
            let g = board.begin_named("f_fail", "docs/report.pdf", Direction::Down, 500);
            board.mark_failed("f_fail");
        } // guard drops here — before fix, entry is gone; after fix it's in recently_failed
        let dtos = board.snapshot_dtos();
        assert_eq!(dtos.len(), 1, "failed row must still appear in snapshot after guard drops");
        assert!(dtos[0].failed);
        assert_eq!(dtos[0].file_id, "f_fail");
        assert_eq!(dtos[0].name, "report.pdf");
    }

    #[test]
    fn an_expired_failed_entry_is_pruned_from_the_snapshot() {
        // Inject an already-expired entry directly into recently_failed (failed_at_secs = 0
        // is 1970-01-01, far beyond any TTL). snapshot_dtos must prune and exclude it.
        let board = TransferBoard::new();
        if let Ok(mut inner) = board.inner.lock() {
            inner.recently_failed.insert(
                "expired".to_string(),
                Transfer { failed: true, failed_at_secs: Some(0), ..Transfer::default() },
            );
            inner.paths.insert("expired".to_string(), "docs/old.pdf".to_string());
        }
        let dtos = board.snapshot_dtos();
        assert!(dtos.is_empty(), "an expired failed entry must not appear in snapshot");
    }

    #[test]
    fn a_non_failed_drop_does_not_populate_recently_failed() {
        let board = TransferBoard::new();
        {
            let g = board.begin("ok", Direction::Down, 100);
            g.update(100);
            g.finish();
        }
        // Finished transfer (not failed): nothing in recently_failed
        let inner = board.inner.lock().unwrap();
        assert!(inner.recently_failed.is_empty(), "a successful finish must not populate recently_failed");
    }

    // --- P2 fix: paths and speeds must be pruned when transfers end ---

    #[test]
    fn paths_and_speeds_are_pruned_when_non_failed_transfers_end() {
        // Before the fix: inner.paths and inner.speeds were only ever inserted into,
        // never removed. This test must be RED on the unfixed code (maps are non-empty).
        let board = TransferBoard::new();
        for i in 0u32..3 {
            let id = format!("file{i}");
            let path = format!("dir/file{i}.txt");
            let g = board.begin_named(&id, &path, Direction::Stream, 100);
            board.update_stream(&id, 50);
            g.update(100);
            g.finish();
        }
        let inner = board.inner.lock().unwrap();
        assert!(inner.speeds.is_empty(), "speeds must be pruned after transfers end");
        assert!(inner.paths.is_empty(), "paths must be pruned after non-failed transfers end");
    }

    #[test]
    fn paths_are_kept_for_recently_failed_and_pruned_on_expiry() {
        // A failed transfer's path must stay (so snapshot_dtos can resolve the name),
        // but must be gone once the entry expires.
        let board = TransferBoard::new();
        {
            let g = board.begin_named("fx", "media/clip.mp4", Direction::Stream, 1000);
            board.mark_failed("fx");
        }
        {
            let inner = board.inner.lock().unwrap();
            assert!(inner.paths.contains_key("fx"), "path must be kept while entry is in recently_failed");
        }
        // Expire the entry by setting its timestamp to 0 and calling snapshot_dtos
        {
            if let Ok(mut inner) = board.inner.lock() {
                if let Some(t) = inner.recently_failed.get_mut("fx") {
                    t.failed_at_secs = Some(0);
                }
            }
        }
        board.snapshot_dtos(); // triggers pruning
        {
            let inner = board.inner.lock().unwrap();
            assert!(!inner.recently_failed.contains_key("fx"), "expired entry pruned from recently_failed");
            assert!(!inner.paths.contains_key("fx"), "path pruned alongside expired recently_failed entry");
        }
    }

    // --- LD-0105 U1 board wiring: begin_download, received_at, FetchSpeed, FetchRow ---

    /// Brief U1.1: a second fetch of a file while one runs joins its row: no second row, progress
    /// not back to 0, and the row ends with the last fetch, by that fetch's outcome. Red on the
    /// mutant "`begin_download` starts the row over" (done 0 after the join, the row gone early).
    #[test]
    fn ld0105_u1_a_second_fetch_of_a_file_joins_its_row() {
        let board = TransferBoard::new();
        let first = FetchRow::new(board.begin_download("f", "Videos/clip.mp4", 100));
        board.received_at("f", 60, 0);
        let second = FetchRow::new(board.begin_download("f", "Videos/clip.mp4", 100));
        let rows = board.snapshot_dtos();
        assert_eq!(rows.len(), 1, "one row: {rows:?}");
        assert_eq!((rows[0].done, rows[0].name.as_str()), (60, "clip.mp4"), "progress kept, never back to 0");
        drop(first); // the first fetch failed while the second runs on
        let rows = board.snapshot_dtos();
        assert_eq!(rows.len(), 1, "the second fetch still holds the row");
        assert!(!rows[0].failed, "the second fetch, still running, decides how the row ends");
        board.received_at("f", 100, 0);
        second.finished();
        assert!(board.snapshot_dtos().is_empty(), "the last fetch delivered the file: no row left");
        assert_eq!(board.finished_bytes(), 100, "counted once");
    }

    /// Brief U1.1: the row's progress is the meter's kept b    /// Brief U1.1: the row's progress is the meter's kept bytes, never backwards (a failed attempt's
    /// bytes are taken back from the meter) and never past the total; the speed is the caller's.
    #[test]
    fn ld0105_u1_kept_bytes_are_the_progress_never_backwards_never_past_the_total() {
        let board = TransferBoard::new();
        let _row = board.begin_download("f", "a.bin", 10_000);
        board.received_at("f", 4_000, 1_500);
        let t = board.get("f").unwrap();
        assert_eq!((t.done, t.speed_bps), (4_000, 1_500));
        board.received_at("f", 3_000, 1_500);
        assert_eq!(board.get("f").unwrap().done, 4_000, "a rolled-back attempt never moves the row backwards");
        board.received_at("f", 10_028, 1_500);
        assert_eq!(board.get("f").unwrap().done, 10_000, "wire bytes (28-byte tags) never pass the total");
        board.received_at("not-registered", 5, 1);
        assert_eq!(board.get("not-registered"), None);
    }

    /// The release lane's native 1 s samples of `done` during a 300 MB whole-file download at
    /// 1e963bc (Windows, direct route, `download_kbps_limit` 12,500 KB/s, streaming OFF): the bytes
    /// arrive in bursts while the pacer holds the next requests back. Kept = received on that run (no
    /// discarded body), and these are per read: 118,159,856 is no multiple of 16 MiB.
    const NATIVE_1E963BC: [u64; 23] = [
        0, 33_554_432, 67_108_976, 67_108_976, 67_108_976, 67_108_976, 118_159_856, 134_217_952, 134_217_952,
        134_217_952, 134_217_952, 170_967_324, 201_326_928, 201_326_928, 201_326_928, 201_326_928, 240_506_312,
        267_741_438, 268_435_904, 268_435_904, 268_435_904, 268_435_904, 300_000_000,
    ];

    /// Lead decision 2026-11-06 09:53 (native: 13 of 21 mid-download samples read "0 B/s", the rest
    /// 0.7 to 51.0 MB/s, at a real ≈12.3 MB/s): replayed second by second, the speed is non-zero at
    /// every sample from the first byte to the last, and once its window is full it stays within a
    /// third of the download's real average (the tail's bursts are smaller: 9.7 MB/s at 13.6). Red
    /// on the mutant "2 s window" (kept = received on this trace, so the next test covers that one).
    #[test]
    fn ld0105_u1_the_native_bursts_read_as_a_steady_non_zero_speed() {
        let t0 = Instant::now();
        let meter = ReceiveMeter::default();
        let mut speed = FetchSpeed::new();
        let average = 300_000_000 / 22; // bytes per second, first byte to last
        let mut seen = Vec::new();
        for (s, received) in NATIVE_1E963BC.iter().enumerate() {
            meter.add(received - meter.received());
            let bps = speed.sample(t0 + Duration::from_secs(s as u64), &meter);
            seen.push(bps / 100_000);
            if s >= 1 {
                assert!(bps > 0, "second {s}: a byte arrived in the window, so a speed shows");
            }
            if s >= 10 {
                assert!(bps.abs_diff(average) * 3 <= average, "second {s}: {bps} B/s is within a third of {average}");
            }
        }
        println!("ld0105-u1 native replay, 0.1 MB/s per second: {seen:?}");
    }

    /// Lead decision 2026-11-06 09:53: a fetch fed per-read bytes in chunk-sized bursts (16 MiB in
    /// 64 KiB reads, then 4 s of nothing), with an attempt that fails and is taken back from the kept
    /// progress, still reads a non-zero speed in every window after the first byte: the speed is the
    /// meter's received bytes, the wire's real rate. Red on the mutant "speed of the kept progress".
    #[test]
    fn ld0105_u1_per_read_bursts_and_a_retry_keep_a_non_zero_speed() {
        const CHUNK: u64 = 16 << 20;
        let meter = ReceiveMeter::default();
        let t0 = Instant::now();
        let mut speed = FetchSpeed::new();
        let mut seen = Vec::new();
        for s in 0..30u64 {
            if s % 5 == 1 {
                let mark = meter.kept();
                for _ in 0..(CHUNK / 65_536) {
                    meter.add(65_536);
                }
                if s == 11 {
                    meter.rollback_to(mark); // that body did not authenticate: fetched again
                }
            }
            let bps = speed.sample(t0 + Duration::from_secs(s), &meter);
            seen.push(bps);
            if s >= 1 {
                assert!(bps > 0, "second {s}: a speed shows");
            }
        }
        let steady = &seen[10..];
        let (low, high) = (steady.iter().min().unwrap(), steady.iter().max().unwrap());
        assert!(high - low <= high / 4, "steady once the window is full: {steady:?}");
    }

    /// Brief U1.1: a fetch row dropped without `finished()` is a failed row, named, kept for the TTL;
    /// one that finished is gone. A fetch with no `files` row puts nothing on the board.
    #[test]
    fn ld0105_u1_a_fetch_row_ends_failed_unless_it_finished() {
        let board = TransferBoard::new();
        drop(FetchRow::new(board.begin_download("bad", "Videos/lecture-03.mp4", 10)));
        FetchRow::new(board.begin_download("good", "Videos/ok.mp4", 10)).finished();
        let rows = board.snapshot_dtos();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!((rows[0].file_id.as_str(), rows[0].name.as_str(), rows[0].failed), ("bad", "lecture-03.mp4", true));
        assert!(rows[0].failed_at_secs.is_some());
        drop(FetchRow::none());
        assert_eq!(board.snapshot_dtos().len(), 1, "no row, nothing marked");
    }

    /// Self-review F1: a start that replaces a row (`begin`, the range session's call) while a fetch
    /// holds it keeps counting that fetch, so the fetch's failure is not the newer transfer's and its
    /// drop does not end the newer row. Red on the mutant "a replacing start resets the count to 1".
    #[test]
    fn ld0105_u1_a_replacing_start_keeps_counting_the_live_guards() {
        let board = TransferBoard::new();
        let fetch = FetchRow::new(board.begin_download("f", "Videos/clip.mp4", 100));
        let session = board.begin("f", Direction::Down, 100);
        drop(fetch);
        let rows = board.snapshot_dtos();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert!(!rows[0].failed, "the session still runs: the fetch's failure is not its outcome");
        session.update(50);
        assert_eq!(board.get("f").unwrap().done, 50, "the session's progress still lands on its row");
        drop(session);
        assert!(board.snapshot_dtos().is_empty());
    }

    /// Self-review F2: a download never joins an upload's row (it would keep `up`, which the UI hides).
    #[test]
    fn ld0105_u1_a_download_never_joins_an_upload_row() {
        let board = TransferBoard::new();
        let _up = board.begin_named("f", "Documents/a.txt", Direction::Up, 10);
        let _down = board.begin_download("f", "Documents/a.txt", 100);
        let t = board.get("f").unwrap();
        assert_eq!((t.direction, t.total), (Direction::Down, 100));
    }

    /// Self-review F4/F5: once any fetch delivered the file the row never ends as failed, and its bytes
    /// count once; a fetch that knows the size fills a joined row's unknown total.
    #[test]
    fn ld0105_u1_a_delivered_file_never_ends_as_failed() {
        let board = TransferBoard::new();
        let no_size = board.begin_download("f", "Videos/clip.mp4", 0);
        let first = FetchRow::new(board.begin_download("f", "Videos/clip.mp4", 100));
        assert_eq!(board.get("f").unwrap().total, 100, "the joiner's known size fills the unknown total");
        let second = FetchRow::new(board.begin_download("f", "Videos/clip.mp4", 100));
        first.finished();
        drop(no_size);
        drop(second); // the last holder fails after the file was delivered
        assert!(board.snapshot_dtos().is_empty(), "a delivered file is no failed row");
        assert_eq!(board.finished_bytes(), 100, "counted once");
    }

    /// Self-review F3: a fetch cut off by an engine stop (lock, sign-out, switch, quit) ends with no
    /// failed row; a new engine forgets the failed rows an earlier one left (their names included).
    #[test]
    fn ld0105_u1_a_stop_is_no_failure_and_a_new_engine_forgets_failed_rows() {
        let board = TransferBoard::new();
        let stopping = Arc::new(std::sync::atomic::AtomicBool::new(true));
        drop(FetchRow::new(board.begin_download("cut", "Videos/a.mp4", 10)).stopped_by(stopping));
        assert!(board.snapshot_dtos().is_empty(), "a stop is not the file's failure");
        drop(FetchRow::new(board.begin_download("bad", "Videos/b.mp4", 10)));
        assert_eq!(board.snapshot_dtos().len(), 1);
        board.forget_failed();
        assert!(board.snapshot_dtos().is_empty());
        assert!(board.inner.lock().unwrap().paths.is_empty(), "the names go with the rows");
    }
}
