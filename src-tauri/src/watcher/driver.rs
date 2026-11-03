//! `driver` items split out of `watcher.rs`.

use super::*;

/// An event delivered by a Windows Cloud Files NOTIFY callback. Push-only from
/// the callback side ([`crate::windows_cf::callbacks`]); consumed by
/// [`debounce_loop`].
#[derive(Debug, Clone)]
pub enum NotifyEvent {
    /// A handle that may have written a new/modified file has closed
    /// (`NOTIFY_FILE_CLOSE_COMPLETION`). The create/modify trigger. Debounced
    /// per path, then classified + uploaded if it is a genuinely-new user file.
    CloseCompletion(PathBuf),
    /// A file/dir was deleted (`NOTIFY_DELETE_COMPLETION`). Point-in-time — the
    /// op already happened, so no debounce: dispatched immediately.
    Delete(PathBuf),
    /// A file/dir was renamed or moved (`NOTIFY_RENAME_COMPLETION`). `source`
    /// is the old absolute path, `target` the new absolute path. Point-in-time,
    /// dispatched immediately.
    Rename { source: PathBuf, target: PathBuf },
}

/// Owned handle to the running upload driver. Dropping it signals BOTH the
/// debounce task and the enumeration scan task to exit; once the debounce task
/// exits the registered NOTIFY sender is closed, so the Cloud Files callbacks
/// find no receiver and drop events harmlessly.
pub struct WatcherHandle {
    // Dropping these senders closes their oneshot channels, which ends the
    // debounce loop and the scan loop respectively.
    pub(super) _shutdown_debounce: tokio::sync::oneshot::Sender<()>,
    pub(super) _shutdown_scan: tokio::sync::oneshot::Sender<()>,
}

/// Start the upload driver for `sync_root`: register the NOTIFY-event sender so
/// the Cloud Files callbacks can push create/modify/delete/rename events, and
/// spawn the debounce + classify + enqueue loop.
///
/// Returns `Some(WatcherHandle)`. (It is infallible today — there is no OS
/// watcher to fail — but the `Option` return is kept so the call site in
/// `runner::run` is unchanged and a future fallible setup step can short-circuit
/// to `None`.)
pub fn spawn(bridge: Arc<EngineBridge>, sync_root: PathBuf) -> Option<WatcherHandle> {
    let (event_tx, event_rx) = mpsc::unbounded_channel::<NotifyEvent>();
    let (shutdown_debounce_tx, shutdown_debounce_rx) = tokio::sync::oneshot::channel::<()>();
    let (shutdown_scan_tx, shutdown_scan_rx) = tokio::sync::oneshot::channel::<()>();

    // Hand the sender to the Cloud Files callback layer. The `extern "system"`
    // NOTIFY callbacks can't capture state, so they reach this sender through a
    // OnceLock (set here). Windows-only; a no-op on other platforms.
    #[cfg(target_os = "windows")]
    crate::windows_cf::set_notify_sender(event_tx.clone());

    // Keep `event_tx` alive for the lifetime of the loop on every platform so
    // the channel doesn't close immediately on non-Windows builds (where the
    // sender is never registered). The debounce loop owns the receiver.
    let _keep_tx = event_tx;

    tracing::info!(
        sync_root = %sync_root.display(),
        scan_interval_secs = SCAN_INTERVAL.as_secs(),
        "sync-root upload driver started (CF NOTIFY dispatch + enumeration scan)"
    );

    tokio::spawn(debounce_loop(
        bridge.clone(),
        sync_root.clone(),
        event_rx,
        shutdown_debounce_rx,
        _keep_tx,
    ));
    tokio::spawn(scan_loop(bridge, sync_root, shutdown_scan_rx));

    Some(WatcherHandle {
        _shutdown_debounce: shutdown_debounce_tx,
        _shutdown_scan: shutdown_scan_tx,
    })
}

/// Drain NOTIFY events: debounce close-completions per path and dispatch
/// deletes/renames immediately. Once a close-completion path has settled, run
/// the shared classifier and (if it survives) enqueue an encrypted upload.
pub(super) async fn debounce_loop(
    bridge: Arc<EngineBridge>,
    sync_root: PathBuf,
    mut event_rx: mpsc::UnboundedReceiver<NotifyEvent>,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    // Held so the channel stays open for the whole loop lifetime even if no
    // callback sender is registered (non-Windows / pre-callback startup).
    _keep_tx: mpsc::UnboundedSender<NotifyEvent>,
) {
    // path → last time we saw a close-completion for it. We flush a path only
    // once its last event is older than DEBOUNCE (it has stopped changing).
    let mut pending: HashMap<PathBuf, Instant> = HashMap::new();
    let mut tick = tokio::time::interval(DEBOUNCE_TICK);

    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown_rx => {
                tracing::debug!("sync-root upload driver shutting down");
                break;
            }
            maybe_event = event_rx.recv() => {
                match maybe_event {
                    Some(NotifyEvent::CloseCompletion(path)) => {
                        // Debounce: record/refresh the settle timer for this path.
                        pending.insert(path, Instant::now());
                    }
                    Some(NotifyEvent::Delete(path)) => {
                        // The file is already gone — handle immediately. Drop any
                        // pending close-completion for the same path so a stale
                        // settle doesn't try to upload a now-deleted file.
                        pending.remove(&path);
                        // task 0806: if the ENGINE just removed this placeholder to
                        // reconcile a REMOTE deletion, this NOTIFY is an echo of OUR
                        // own delete — drop it (consuming the suppression entry) so we
                        // never queue a redundant server trash. A genuine user delete
                        // has no entry and falls through to handle_delete as before.
                        if take_engine_delete_suppressed(&path) {
                            tracing::debug!("upload driver: dropping engine-originated delete (remote-deletion reconcile)");
                        } else {
                            handle_delete(&bridge, &sync_root, &path).await;
                        }
                    }
                    Some(NotifyEvent::Rename { source, target }) => {
                        // A rename invalidates a pending close-completion for the
                        // OLD path; the NEW path's close (if any) will arrive on
                        // its own event.
                        pending.remove(&source);
                        handle_rename(&bridge, &sync_root, &source, &target).await;
                    }
                    // Sender dropped (handle gone) — exit.
                    None => break,
                }
            }
            _ = tick.tick() => {
                // task 0806: sweep stale engine-delete suppressions so a registered
                // delete whose NOTIFY never arrived can't shadow a later user delete.
                prune_stale_engine_suppressions();
                let now = Instant::now();
                let ready: Vec<PathBuf> = pending
                    .iter()
                    .filter(|(_, seen)| now.duration_since(**seen) >= DEBOUNCE)
                    .map(|(p, _)| p.clone())
                    .collect();
                for path in ready {
                    pending.remove(&path);
                    // Task 1640 (review P2-5): classifying a tracked file reads and hashes it.
                    let (settle_bridge, settle_root) = (bridge.clone(), sync_root.clone());
                    if let Err(e) =
                        tokio::task::spawn_blocking(move || handle_settled_path(&settle_bridge, &settle_root, &path)).await
                    {
                        tracing::warn!(error = %e, "upload driver: settling a closed file panicked; continuing");
                    }
                }
            }
        }
    }
}
