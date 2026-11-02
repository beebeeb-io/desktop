//! The process-wide state one popover shares with the commands: the transfer board
//! the engine writes and the last `engine-status` view its listener saw.
//!
//! Registered once as Tauri managed state (`lib.rs`); `Clone` because Tauri hands out
//! a handle per command, not because more than one engine runs (decision 0800,
//! N = 1).

use super::*;

/// What the engine and the commands share, registered once as Tauri managed state.
/// One engine runs at a time (decision 0800, N = 1), so this is process-wide.
#[derive(Clone)]
pub struct PopoverRuntime {
    /// Bytes done and total per file in flight (written by the engine's chunk loops).
    pub transfers: Arc<TransferBoard>,
    /// The last `engine-status` payload the listener saw.
    pub status: SharedStatusView,
}

impl Default for PopoverRuntime {
    fn default() -> Self {
        Self {
            transfers: TransferBoard::new(),
            status: Arc::new(Mutex::new(EngineStatusView::default())),
        }
    }
}
