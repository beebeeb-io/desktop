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
        Self {
            samples: VecDeque::new(),
        }
    }

    /// Push a new sample and return the current speed in bytes/s over the last `span`.
    fn push(&mut self, now: Instant, cumulative_bytes: u64, span: Duration) -> u64 {
        self.samples.push_back((now, cumulative_bytes));
        // Drop samples older than `span`
        let cutoff = now.checked_sub(span).unwrap_or(now);
        while self.samples.front().is_some_and(|(t, _)| *t < cutoff) {
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
pub struct FetchSpeed {
    window: SpeedWindow,
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
impl Default for FetchSpeed {
    fn default() -> Self {
        Self::new()
    }
}

impl FetchSpeed {
    pub fn new() -> Self {
        FetchSpeed {
            window: SpeedWindow::new(),
        }
    }

    /// `meter` read at `now`: the speed in bytes/s of its received bytes.
    pub fn sample(&mut self, now: Instant, meter: &ReceiveMeter) -> u64 {
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
pub struct NeedsAttention {
    pub file_id: String,
    /// The vault path, for the row's name and folder.
    pub path: String,
    pub size: u64,
}

/// state.db's needs-attention rows whose file it still knows (review: a poison with no file row could
/// not be discarded, `discard_now` answers `missing`, and it clears at the next process start).
/// `None`: the DB could not be read.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn needs_attention(db: &crate::state_db::StateDb) -> Option<Vec<NeedsAttention>> {
    let rows = db.poisoned().ok()?;
    rows.into_iter()
        .filter(|r| r.needs_attention)
        .filter_map(|r| match db.get_file(&r.file_id) {
            Ok(Some(entry)) => Some(Ok(NeedsAttention {
                file_id: r.file_id,
                path: entry.path,
                size: entry.size_bytes.max(0) as u64,
            })),
            // No files row: no exit, no row (above).
            Ok(None) => None,
            // Review P3-7: a read error fails the whole read, so the poll keeps the rows it had.
            Err(e) => Some(Err(e)),
        })
        .collect::<Result<Vec<_>, _>>()
        .ok()
}

/// The needs-attention rows the last read returned, kept with the engine generation they were read
/// under (delta review P2-2): served only to a poll under that same generation. A lock, a sign-out and
/// an account switch each stop the engine, and the next one is a new generation (`NEXT_GENERATION`), so
/// no other session ever sees them; a lock also clears them ([`attention_for_poll`]).
#[derive(Debug, Default)]
pub struct AttentionCache {
    /// `(generation, read number, rows)` of the newest read stored.
    read: Option<(u64, u64, Vec<NeedsAttention>)>,
    /// Reads numbered below this started before a discard ended a row: never stored (review P3-6).
    fence: u64,
}

impl AttentionCache {
    pub const fn new() -> Self {
        AttentionCache { read: None, fence: 0 }
    }

    /// Read number `seq` under `generation` returned `rows`. An older read (a smaller `seq`, or one
    /// started before the fence) never replaces a newer one.
    pub fn store(&mut self, generation: u64, seq: u64, rows: Vec<NeedsAttention>) {
        if seq < self.fence || self.read.as_ref().is_some_and(|(g, s, _)| *g == generation && *s > seq) {
            return;
        }
        self.read = Some((generation, seq, rows));
    }

    /// The rows last read under `generation`; nothing under any other.
    pub fn rows_for(&self, generation: u64) -> Vec<NeedsAttention> {
        match &self.read {
            Some((g, _, rows)) if *g == generation => rows.clone(),
            _ => Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.read = None;
    }

    /// A discard ended `file_id`'s row: it is gone from the kept rows, and no read that started before
    /// `next_seq` (the next read's number) may bring it back.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub fn forget(&mut self, file_id: &str, next_seq: u64) {
        if let Some((_, _, rows)) = &mut self.read {
            rows.retain(|r| r.file_id != file_id);
        }
        self.fence = self.fence.max(next_seq);
    }
}

/// The process's kept needs-attention rows ([`AttentionCache`]), shared by `get_active_transfers` and
/// `discard_and_download_again`.
pub static ATTENTION_CACHE: Mutex<AttentionCache> = Mutex::new(AttentionCache::new());
/// The next needs-attention read's number.
pub static ATTENTION_READS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// What one poll shows of the needs-attention rows (delta review P2-2): nothing while the vault is
/// locked or no engine is running (and the kept rows are cleared); a fresh read when it finished in
/// time; else the rows kept for this same engine generation.
pub fn attention_for_poll(
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
pub fn with_attention(mut rows: Vec<ActiveTransferDto>, attention: &[NeedsAttention]) -> Vec<ActiveTransferDto> {
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
        let _ = self.kept.try_update(Ordering::Relaxed, Ordering::Relaxed, |k| {
            Some(k.saturating_sub(wire_bytes))
        });
    }

    /// A whole attempt failed: progress goes back to `mark` (an earlier [`Self::kept`]) if it is past it.
    pub fn rollback_to(&self, mark: u64) {
        let _ = self
            .kept
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |k| Some(k.min(mark)));
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
        let held = if self.active.contains_key(file_id) {
            self.holders.get(file_id).copied().unwrap_or(0)
        } else {
            0
        };
        self.holders.insert(file_id.to_string(), held + 1);
        if join && let Some(t) = self.active.get_mut(file_id).filter(|t| t.direction != Direction::Up) {
            // A failure marked just before this download joined is not this download's outcome.
            t.failed = false;
            t.failed_at_secs = None;
            if t.total == 0 {
                t.total = total;
            }
            if let Some(path) = path {
                self.paths
                    .entry(file_id.to_string())
                    .or_insert_with(|| path.to_string());
            }
            return;
        }
        self.active.insert(
            file_id.to_string(),
            Transfer {
                direction,
                total,
                ..Transfer::default()
            },
        );
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
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if let Some(t) = self.active.get_mut(file_id) {
            t.failed = true;
            t.failed_at_secs = Some(now_secs);
        }
    }
}

/// What the board rings when a download row begins, fails or leaves it ([`TransferBoard::set_wake`]).
#[derive(Default)]
struct Wake(Mutex<Option<Arc<dyn Fn() + Send + Sync>>>);

impl std::fmt::Debug for Wake {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Wake")
    }
}

/// dc-163: the least time between two reports of the first-listing count. The ingest reports at most this often
/// (not per row), and every report rings the wake hook, so a window redraws the count about five times a second.
pub const LISTING_REPORT_EVERY: std::time::Duration = std::time::Duration::from_millis(200);

#[derive(Debug, Default)]
pub struct TransferBoard {
    inner: Mutex<Inner>,
    wake: Wake,
    /// (rows ingested, rows in the snapshot) while a snapshot is being applied; `None` otherwise.
    first_listing: Mutex<Option<(u64, u64)>>,
    /// w31: whether the account's file list has ever arrived on this PC ([`FirstList`], as its number).
    first_list: std::sync::atomic::AtomicU8,
    /// w31: a window asked for the pull now (Try again on a first list that failed); the runner's loop waits on it.
    pull_now: tokio::sync::Notify,
}

/// w31: the account's first file list on this PC, as the running engine last said it. Until it arrived the local
/// state holds no server row at all (at most the folders this PC's own sync folder gave it), so a window must not
/// draw that as the account's files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FirstList {
    /// The list arrived (this run or an earlier one), or no engine said anything yet.
    #[default]
    Here,
    /// Never listed, and the engine is getting it.
    Awaited,
    /// Never listed, and the last try failed. The engine tries again at every check.
    Failed,
}

impl TransferBoard {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Ring `wake` whenever a DOWNLOAD row begins, fails or leaves the board (never for an upload or a stream), after
    /// the board's lock is released. For the host's window: while the engine is paused, offline or stopped it sends no
    /// status pulse, and nothing else would tell the window a copy came or went. Replaces an earlier one.
    pub fn set_wake(&self, wake: impl Fn() + Send + Sync + 'static) {
        if let Ok(mut slot) = self.wake.0.lock() {
            *slot = Some(Arc::new(wake));
        }
    }

    /// How far the listing of the account's files has come while a snapshot is ingested (DW-0802b): `(listed, total)`.
    /// `None` when no snapshot is being applied. In memory, never waits on anything but a short lock.
    pub fn first_listing(&self) -> Option<(u64, u64)> {
        self.first_listing.lock().ok().and_then(|slot| *slot)
    }

    /// A snapshot of `total` rows is being ingested and `listed` are in. Rings the wake hook. dc-163: the ingest
    /// calls this at most every [`LISTING_REPORT_EVERY`], never per row, so each call may ring.
    pub fn set_first_listing(&self, listed: u64, total: u64) {
        match self.first_listing.lock() {
            Ok(mut slot) => *slot = Some((listed, total)),
            Err(_) => return,
        }
        self.ring();
    }

    /// The snapshot is whole (or its ingest failed): no listing is in progress. Rings the wake hook.
    pub fn end_first_listing(&self) {
        let had = self
            .first_listing
            .lock()
            .ok()
            .is_some_and(|mut slot| slot.take().is_some());
        if had {
            self.ring();
        }
    }

    /// w31: where the account's first file list is. In memory, never waits.
    pub fn first_list(&self) -> FirstList {
        match self.first_list.load(Ordering::SeqCst) {
            1 => FirstList::Awaited,
            2 => FirstList::Failed,
            _ => FirstList::Here,
        }
    }

    /// w31: the runner says where the first list is. Rings the wake hook when that changed.
    pub fn set_first_list(&self, state: FirstList) {
        let number = match state {
            FirstList::Here => 0,
            FirstList::Awaited => 1,
            FirstList::Failed => 2,
        };
        if self.first_list.swap(number, Ordering::SeqCst) != number {
            self.ring();
        }
    }

    /// w31: ask the running engine for its pull now instead of at its next tick. A permit is kept when it is busy.
    pub fn ask_pull_now(&self) {
        self.pull_now.notify_one();
    }

    /// w31: the runner's wait for [`Self::ask_pull_now`].
    pub async fn pull_asked(&self) {
        self.pull_now.notified().await;
    }

    fn ring(&self) {
        let wake = self.wake.0.lock().ok().and_then(|slot| slot.clone());
        if let Some(wake) = wake {
            wake();
        }
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

    fn guard(
        self: &Arc<Self>,
        file_id: &str,
        path: Option<&str>,
        direction: Direction,
        total: u64,
        join: bool,
    ) -> TransferGuard {
        if let Ok(mut inner) = self.inner.lock() {
            inner.start(file_id, path, direction, total, join);
        }
        if direction == Direction::Down {
            self.ring();
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
            let speed = inner
                .speeds
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
        let down = self.inner.lock().is_ok_and(|mut inner| {
            let down = inner
                .active
                .get(file_id)
                .is_some_and(|t| t.direction == Direction::Down);
            inner.mark_failed(file_id);
            down
        });
        if down {
            self.ring();
        }
    }

    /// A snapshot of every active transfer and every recently-failed transfer (within
    /// [`FAILED_ROW_TTL_SECS`]) as IPC-ready DTOs. Expired entries are pruned lazily.
    pub fn snapshot_dtos(&self) -> Vec<ActiveTransferDto> {
        self.inner
            .lock()
            .map(|mut inner| {
                let now_secs = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                // Prune expired recently-failed entries and their path records.
                let expired: Vec<String> = inner
                    .recently_failed
                    .iter()
                    .filter_map(|(id, t)| {
                        // No timestamp → keep (conservative); timestamp too old → prune.
                        if t.failed_at_secs.is_some_and(|s| s + FAILED_ROW_TTL_SECS < now_secs) {
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
                let pairs: Vec<(String, Transfer)> = inner
                    .active
                    .iter()
                    .chain(inner.recently_failed.iter())
                    .map(|(id, t)| (id.clone(), *t))
                    .collect();

                pairs
                    .into_iter()
                    .map(|(id, t)| {
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
                    })
                    .collect()
            })
            .unwrap_or_default()
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
    pub fn snapshot_with_attention(&self, db: Option<&crate::state_db::StateDb>) -> Vec<ActiveTransferDto> {
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
    #[cfg(any(test, feature = "test-support"))]
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
        let down = self.board.inner.lock().is_ok_and(|mut inner| {
            let last = inner.last_holder(&self.file_id);
            let down = inner
                .active
                .get(&self.file_id)
                .is_some_and(|t| t.direction == Direction::Down);
            if last {
                inner.mark_failed(&self.file_id);
            }
            last && down
        });
        if down {
            self.board.ring();
        }
    }
}

impl Drop for TransferGuard {
    fn drop(&mut self) {
        let mut ended_down = false;
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
                ended_down = t.direction == Direction::Down;
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
        if ended_down {
            self.board.ring();
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
        FetchRow {
            guard: Some(guard),
            stopping: None,
        }
    }

    pub fn none() -> Self {
        FetchRow {
            guard: None,
            stopping: None,
        }
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
            match std::future::pending::<std::convert::Infallible>().await {}
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
#[path = "transfer_progress_tests.rs"]
mod tests;
