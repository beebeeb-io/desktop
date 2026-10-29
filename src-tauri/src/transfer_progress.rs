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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Up,
    Down,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Up => "up",
            Direction::Down => "down",
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transfer {
    pub direction: Direction,
    pub done: u64,
    pub total: u64,
}

#[derive(Debug, Default)]
struct Inner {
    active: HashMap<String, Transfer>,
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
                Transfer {
                    direction,
                    done: 0,
                    total,
                },
            );
        }
        TransferGuard {
            board: self.clone(),
            file_id: file_id.to_string(),
        }
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
            inner.active.remove(&self.file_id);
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
                    total: 1_000
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
}
