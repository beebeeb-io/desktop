//! The slot the Cloud Files NOTIFY callbacks reach the upload driver through, split out of `windows_cf/mod.rs`
//! (file-size ratchet). Must stay re-pointable: see `NOTIFY_TX`.

use std::sync::RwLock;

/// Sender into the upload driver's debounce loop (`crate::watcher`). The
/// `extern "system"` NOTIFY callbacks can't capture state, so they reach the
/// live channel through this slot. Overwritten by [`set_notify_sender`] from
/// `watcher::spawn`; read by [`notify_sender`] in the callback module.
///
/// **Must be RE-POINTABLE, not set-once.** On a re-login or sync-root change the
/// runner DROPS the old runner and re-spawns IN THE SAME PROCESS (see
/// `runner::run` lifecycle), so `watcher::spawn` runs again and creates a BRAND
/// NEW channel. The previous debounce loop has ended, so the old channel's
/// receiver is gone and every `send` into it errors. A set-once `OnceLock` would
/// keep the dead first sender forever → local uploads would silently stop after
/// the first logout→login. So we store the sender in a `RwLock<Option<…>>` and
/// OVERWRITE it on every (re)spawn, always pointing at the live debounce loop.
///
pub(super) static NOTIFY_TX: RwLock<Option<tokio::sync::mpsc::UnboundedSender<crate::watcher::NotifyEvent>>> =
    RwLock::new(None);

/// Register (or RE-register) the upload driver's event sender so the Cloud Files
/// NOTIFY callbacks ([`super::callbacks::notify_file_close_completion_callback`] and
/// friends) push create/modify/delete/rename events into the LIVE debounce loop.
/// Called from `watcher::spawn` on every (re)spawn — it OVERWRITES any prior
/// sender so a re-login installs the new channel and uploads keep working.
pub fn set_notify_sender(tx: tokio::sync::mpsc::UnboundedSender<crate::watcher::NotifyEvent>) {
    if let Ok(mut slot) = NOTIFY_TX.write() {
        *slot = Some(tx);
    }
}

/// Internal accessor for the callback module: a clone of the CURRENT NOTIFY-event
/// sender, or `None` if `watcher::spawn` hasn't registered one yet (e.g. a NOTIFY
/// callback raced engine startup, or a logout cleared it). The callback drops the
/// event in that case — a missed startup event is reconciled by the next
/// `sync_tick`.
pub(crate) fn notify_sender() -> Option<tokio::sync::mpsc::UnboundedSender<crate::watcher::NotifyEvent>> {
    NOTIFY_TX.read().ok().and_then(|slot| slot.clone())
}
