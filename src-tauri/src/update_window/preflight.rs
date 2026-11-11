//! What must be quiet before a native installer (0.9.0 or later) replaces this app in place: uploads in flight
//! finished, the sync engine stopped, the Cloud Files provider disconnected (bridge plan 2026-11-11, item 2).
//!
//! Owns: the bounded wait for uploads in flight; the engine stop through the app's one stop helper
//! (`stop_engine_in_slot`, which on Windows disconnects the Cloud Files provider before and after the stop); the
//! engine slot kept held until the installer has taken over, so nothing starts an engine again in between.
//! Must not: sign out, lock the vault, delete a file or a placeholder, unregister the sync root, or touch the pause
//! (the pause flag always says what the config file says, `config_write_pin_tests`; a pause written for the update
//! would also be read by the next version). So nothing holds new uploads back while it waits: a busy queue puts
//! the update off until it has drained.
//! Known limit: an upload that starts after the board stayed empty for [`QUIET_LOOKS`] looks (about 2 s) gets only
//! the engine's own graceful stop before it is cut; the file stays queued and is sent again by the next version.
//! Tests: `preflight_tests`.

use super::*;
use crate::transfer_progress::{Direction, TransferBoard};
use std::time::Duration;

/// The code an error leads with when the installer was not started because sync could not be made quiet.
pub(crate) const UPDATE_SYNC_NOT_QUIET_CODE: &str = "update_sync_not_quiet";
/// How long uploads in flight get to finish before the update is put off.
pub(crate) const UPLOAD_WAIT: Duration = Duration::from_secs(120);
const UPLOAD_POLL: Duration = Duration::from_millis(250);
/// How many looks in a row must find no upload before sync counts as quiet. A step that is sending several files
/// starts the next one a moment after the last ended: one empty look can fall in that gap.
pub(crate) const QUIET_LOOKS: u32 = 8;

/// Uploads in flight on the board (a failed row is not in flight).
pub(crate) fn uploads_in_flight(board: &TransferBoard) -> usize {
    board
        .active()
        .iter()
        .filter(|(_, transfer)| transfer.direction == Direction::Up && !transfer.failed)
        .count()
}

/// Waits until `in_flight` reports none `quiet_looks` times in a row, at most `limit`. `Err(n)`: `n` were still in
/// flight at the end (at least 1: a board that never stayed empty is not quiet).
pub(crate) async fn wait_for_uploads(
    in_flight: impl Fn() -> usize,
    quiet_looks: u32,
    limit: Duration,
    poll: Duration,
) -> Result<(), usize> {
    let deadline = tokio::time::Instant::now() + limit;
    let mut quiet = 0;
    let mut last_seen = 0;
    loop {
        let left = in_flight();
        if left == 0 {
            quiet += 1;
            if quiet >= quiet_looks {
                return Ok(());
            }
        } else {
            quiet = 0;
            last_seen = left;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(left.max(last_seen).max(1));
        }
        tokio::time::sleep(poll).await;
    }
}

/// The engine slot, held: nothing starts an engine while this lives. Dropping it frees the slot; a stopped engine
/// is not started again (the caller's error says to restart Beebeeb).
pub(crate) struct QuietEngine<'a> {
    _slot: tokio::sync::MutexGuard<'a, Option<runner::EngineRunner>>,
    /// Whether an engine was running and was stopped here.
    pub(crate) stopped_an_engine: bool,
}

/// Makes `acct` quiet for a native installer. In order: wait at most `limit` for the uploads in flight; take the
/// engine out of its slot and stop it (on Windows the stop disconnects the Cloud Files provider; the sync root
/// stays registered and no placeholder is touched). An upload that does not finish in time, or a stop that is not
/// confirmed, is an error with a plain sentence: nothing is installed.
pub(crate) async fn quiet_engine<'a>(
    acct: &'a account::AccountRuntime,
    board: Option<&TransferBoard>,
    limit: Duration,
) -> Result<QuietEngine<'a>, String> {
    let in_flight = || board.map_or(0, uploads_in_flight);
    if let Err(left) = wait_for_uploads(in_flight, QUIET_LOOKS, limit, UPLOAD_POLL).await {
        tracing::warn!(
            left,
            "update pre-flight: uploads still in flight; the update is put off"
        );
        return Err(format!(
            "{UPDATE_SYNC_NOT_QUIET_CODE}: Beebeeb is still uploading {left} file(s), so the update was not installed. Nothing was interrupted. Try again when the upload has finished."
        ));
    }
    let mut slot = acct.engine.lock().await;
    let mut stopped_an_engine = false;
    if let Some(engine) = slot.take() {
        let outcome = stop_engine_in_slot(acct, engine).await;
        if !outcome.is_stopped() {
            tracing::error!(
                task_confirmed = outcome.task_confirmed(),
                "update pre-flight: the engine stop was not clean"
            );
            return Err(format!(
                "{UPDATE_SYNC_NOT_QUIET_CODE}: sync could not be stopped cleanly, so the update was not installed. Restart Beebeeb and try again."
            ));
        }
        stopped_an_engine = true;
    }
    // No engine was running (a locked vault, no sync folder): make sure the provider is disconnected all the same.
    #[cfg(target_os = "windows")]
    if !stopped_an_engine && let Err(error) = crate::windows_cf::revoke_callbacks().await {
        tracing::error!(%error, "update pre-flight: the Cloud Files provider did not disconnect");
        return Err(format!(
            "{UPDATE_SYNC_NOT_QUIET_CODE}: the Beebeeb folder could not be disconnected from Explorer, so the update was not installed. Restart Beebeeb and try again."
        ));
    }
    tracing::info!(stopped_an_engine, "update pre-flight: sync is quiet");
    Ok(QuietEngine {
        _slot: slot,
        stopped_an_engine,
    })
}
