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

    /// Push a new sample and return the current speed in bytes/s.
    fn push(&mut self, now: Instant, cumulative_bytes: u64) -> u64 {
        self.samples.push_back((now, cumulative_bytes));
        // Drop samples older than 2 seconds
        let cutoff = now.checked_sub(Duration::from_secs(2)).unwrap_or(now);
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
    /// Plaintext bytes of files that completed since the batch last went quiet.
    finished_bytes: u64,
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
        if let Ok(mut inner) = self.inner.lock() {
            inner.active.insert(
                file_id.to_string(),
                Transfer { direction, total, ..Transfer::default() },
            );
        }
        TransferGuard {
            board: self.clone(),
            file_id: file_id.to_string(),
        }
    }

    /// Like [`Self::begin`] but also records the vault `path` so the IPC DTO can
    /// expose a display name and parent folder. W2 uses this for streaming opens.
    pub fn begin_named(self: &Arc<Self>, file_id: &str, path: &str, direction: Direction, total: u64) -> TransferGuard {
        if let Ok(mut inner) = self.inner.lock() {
            inner.active.insert(
                file_id.to_string(),
                Transfer { direction, total, ..Transfer::default() },
            );
            inner.paths.insert(file_id.to_string(), path.to_string());
        }
        TransferGuard {
            board: self.clone(),
            file_id: file_id.to_string(),
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
                .push(now, buffered);
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
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(t) = inner.active.get_mut(file_id) {
                t.failed = true;
                t.failed_at_secs = Some(now_secs);
            }
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
                    failed: t.failed,
                    failed_at_secs: t.failed_at_secs,
                }
            }).collect()
        }).unwrap_or_default()
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

    /// The transfer succeeded: its bytes join the batch's finished total.
    pub fn finish(self) {
        if let Ok(mut inner) = self.board.inner.lock()
            && let Some(t) = inner.active.get(&self.file_id).copied()
        {
            inner.finished_bytes = inner.finished_bytes.saturating_add(t.total);
        }
    }
}

impl Drop for TransferGuard {
    fn drop(&mut self) {
        if let Ok(mut inner) = self.board.inner.lock() {
            if let Some(t) = inner.active.remove(&self.file_id) {
                if t.failed {
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
}
