//! What Settings › Sync and the Windows tray popover read to say what sync is doing (0.8.15).
//!
//! On Guus's PC (0.8.14) the popover showed nothing while File Explorer showed files pending: its only sources were the
//! counts of rows mid-transfer and the recent-files list, so a queue of 2,924 paused changes, 43 that gave up, and an
//! engine stuck in its startup check all looked like "Your files are synced". This is the one read both surfaces poll:
//! the live pause flag, the engine's last real state, what is moving, the queue in counts, and the startup check while
//! it runs. It never carries a name, a path or an error text, and reading it changes nothing.
//!
//! The words are the frontend's (`src/windows/syncActivityModel.ts`); `tests/fixtures/sync-activity-summary.on-hold.json`
//! is the JSON this module's test compares its output to and the TypeScript test parses.

use crate::engine_status::{EngineStatusView, StartupCheck};
use crate::state_db::QueueHold;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SyncActivitySummary {
    /// The live pause flag: what the app menu, the tray menu and Settings › Sync flip.
    pub paused: bool,
    /// The engine's last real state from `engine-status` (`running` before its first check finished, `idle`, `syncing`,
    /// `paused`, `offline`, `error`, `stopped`), or "" before any.
    pub engine_state: String,
    /// Files the engine said were left in its last check, if it said.
    pub files_remaining: Option<u32>,
    /// Files moving right now (the transfer board).
    pub transfers_active: u32,
    /// Unix seconds of the last check that reached the server, if any since the app started.
    pub last_tick_ok_at: Option<i64>,
    /// The queue, or `None` when there is no state DB yet.
    pub queue: Option<QueueHold>,
    /// The startup check of the files on this PC, while it runs.
    pub startup_check: Option<StartupCheck>,
}

pub fn summary(
    paused: bool,
    engine: &EngineStatusView,
    transfers_active: usize,
    queue: Option<QueueHold>,
) -> SyncActivitySummary {
    SyncActivitySummary {
        paused,
        engine_state: engine.state.clone(),
        files_remaining: engine.files_remaining,
        transfers_active: u32::try_from(transfers_active).unwrap_or(u32::MAX),
        last_tick_ok_at: engine.last_tick_ok_at,
        queue,
        startup_check: engine.startup_check,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_db::HeldGroup;
    use serde_json::json;

    /// Guus's PC on 26 Oct, from the copied state DB: 2,832 + 92 changes paused for `auth` on 4 Oct, 43 that gave up,
    /// no transfer, and the engine in its startup check (the engine lane's progress hook).
    fn on_hold() -> SyncActivitySummary {
        let mut engine = EngineStatusView::default();
        engine.apply(&json!({
            "state": "running",
            "last_tick_ok_at": null,
            crate::engine_status::STARTUP_CHECK_KEY: { "done": 12_400, "total": 37_169 },
        }));
        summary(
            false,
            &engine,
            0,
            Some(QueueHold {
                waiting: 0,
                gave_up: 43,
                held: vec![HeldGroup {
                    reason: "auth".into(),
                    count: 2_924,
                    oldest_at: 1_791_115_500,
                    newest_at: 1_791_122_220,
                }],
            }),
        )
    }

    #[test]
    fn the_summary_is_the_json_the_frontend_parses() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/sync-activity-summary.on-hold.json")).unwrap();
        assert_eq!(serde_json::to_value(on_hold()).unwrap(), fixture);
    }

    #[test]
    fn it_reads_the_live_pause_flag_and_the_engine_view_as_they_are() {
        let mut engine = EngineStatusView::default();
        engine.apply(&json!({ "state": "syncing", "files_remaining": 3, "last_tick_ok_at": 1_761_480_000 }));
        let summary = summary(true, &engine, 2, None);
        assert!(summary.paused);
        assert_eq!(summary.engine_state, "syncing");
        assert_eq!((summary.files_remaining, summary.transfers_active), (Some(3), 2));
        assert_eq!(summary.last_tick_ok_at, Some(1_761_480_000));
        assert_eq!((summary.queue, summary.startup_check), (None, None));
    }
}
