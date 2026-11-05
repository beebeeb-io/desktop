//! LD-0105 S0, the Windows half: report a fetch's real byte progress to Cloud Files while it
//! downloads ([`keep_fetch_alive`]), by the rule in [`crate::cf_keepalive`]. Split out of
//! `callbacks.rs` (the file-size ratchet).

use windows::Win32::Storage::CloudFilters::*;

/// LD-0105 S0: report `meter` (this fetch's body bytes, per read) to Cloud Files for the pending
/// request `transfer_key`, by [`crate::cf_keepalive::KeepAlive`]'s rule, until the future is dropped:
/// the caller selects it against the download, so it ends with the download. Each report resets
/// Windows' 60 s timer for every pending request of the connection (Microsoft Learn,
/// `CfReportProviderProgress`). A failed report is only logged: the request still has the timer.
pub(crate) async fn keep_fetch_alive(
    connection_key: CF_CONNECTION_KEY,
    transfer_key: i64,
    total: i64,
    meter: &crate::transfer_progress::ReceiveMeter,
) -> std::convert::Infallible {
    let mut keep = crate::cf_keepalive::KeepAlive::new(std::time::Instant::now());
    let mut tick = tokio::time::interval(crate::cf_keepalive::TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let Some(received) = keep.tick(std::time::Instant::now(), meter.received()) else {
            continue;
        };
        let completed = crate::cf_keepalive::completed(received, total);
        // SAFETY: the keys of a live FETCH_DATA request; the callback that owns them is blocked on this
        // future, so the request is still pending while we report.
        if let Err(e) = unsafe { CfReportProviderProgress(connection_key, transfer_key, total, completed) } {
            tracing::debug!(error = %e, "CfReportProviderProgress failed");
        }
    }
}
