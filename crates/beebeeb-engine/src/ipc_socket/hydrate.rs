//! `hydrate` items split out of `ipc_socket.rs`.

use super::*;

pub(super) enum HydrateOutcome {
    Reply(IpcResponse),
    ClientGone,
}

/// Whether the client asked for `HydrateProgress` frames before the final reply
/// (task 1670 issue 3). An enum rather than a `bool`, so every call site says
/// what the flag means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProgressReporting {
    /// Old extensions read exactly one reply; a progress frame would be
    /// mistaken for it.
    Off,
    On,
}

impl ProgressReporting {
    pub(super) fn from_wire(progress: bool) -> Self {
        if progress { Self::On } else { Self::Off }
    }

    pub(super) fn wanted(self) -> bool {
        matches!(self, Self::On)
    }
}

/// Serve one `HydrateFile` request: validate, hydrate, and (when the client
/// opted in) stream `HydrateProgress` frames while the download + decrypt run.
///
/// While the hydrate future runs, this also watches the read half: if the
/// client hangs up (Finder cancelled the transfer, the extension was killed)
/// the future is dropped — the download stops and `hydrate_file`'s guard puts
/// the row's status back — instead of decrypting a file nobody will collect.
pub(super) async fn hydrate_over_ipc(
    frames: &mut FrameReader<tokio::net::unix::OwnedReadHalf>,
    write_half: &mut tokio::net::unix::OwnedWriteHalf,
    bridge: &crate::engine_bridge::EngineBridge,
    file_id: &ItemId,
    dest_path: &str,
    progress: ProgressReporting,
) -> HydrateOutcome {
    // Task 1670 round 2: `file_id` here is the raw File Provider
    // item identifier straight off the wire — reject it before it
    // is trusted for anything else in this arm. See
    // `macos_validate_hydrate_item_identifier`'s doc comment.
    #[cfg(target_os = "macos")]
    let identifier_check = macos_validate_hydrate_item_identifier(file_id.as_str());
    #[cfg(not(target_os = "macos"))]
    let identifier_check: Result<(), &'static str> = Ok(());

    if let Err(msg) = identifier_check {
        // The identifier is untrusted wire input: never logged.
        tracing::warn!(reason = "invalid_identifier", "Finder hydrate failed");
        return HydrateOutcome::Reply(IpcResponse::Error {
            message: msg.to_string(),
        });
    }
    // `dest_path` arrives straight off the wire (untrusted). Bound
    // it to the caller's legitimate destinations before handing it
    // to `hydrate_file`, which decrypts vault plaintext to disk
    // (task 1247). The only real IPC caller (the File Provider
    // extension / FUSE mount) writes either under the sync root or
    // into the per-file temp cache, so both are allowed roots; if
    // no sync root is configured yet, only the temp dir is.
    // Resolve sync_root from config (the way the retired `SetRecursivePin`
    // handler used to); on unix the value is only compared against, never
    // dereferenced by Windows-specific code.
    //
    // Task 1670: on macOS, `temp_root` above is THIS (sandboxed)
    // process's own private container temp dir — the File Provider
    // extension is a DIFFERENT sandboxed process with its own
    // separate container temp dir, so a real destination it builds
    // can never be inside `temp_root` (see `macos_hydrate_cache_dir`'s
    // doc comment for the full root-cause). Add the shared App
    // Group hydrate-cache directory as a third allowed root, and
    // make sure it exists (owner-only, backup-excluded — round 2)
    // before the containment check runs. Round 4: the Swift side
    // copies its staging file into the SYSTEM's own temp
    // directory and deletes THIS copy immediately after (see
    // `MACOS_HYDRATE_CACHE_TTL`'s doc comment), so this and the
    // periodic sweep in `runner.rs` are now a crash backstop —
    // they only ever find staging orphaned by a crash between
    // decrypt and that copy, not a file the system still holds.
    let sync_root = crate::config::DesktopConfig::load_for(bridge.identity)
        .ok()
        .and_then(|cfg| cfg.sync_root);
    let temp_root = bridge.identity.scratch_dir_in(&std::env::temp_dir());
    let dest = std::path::Path::new(dest_path);
    #[cfg(target_os = "macos")]
    let macos_hydrate_dir = {
        let dir = macos_hydrate_cache_dir(bridge.identity);
        if let Err(e) = macos_ensure_hydrate_cache_dir(&dir) {
            tracing::warn!(error = %e, dir = %dir.display(), "could not create macOS hydrate-cache dir");
        }
        if let Err(e) =
            macos_sweep_stale_hydrate_cache_entries(&dir, MACOS_HYDRATE_CACHE_TTL, std::time::SystemTime::now())
        {
            tracing::warn!(error = %e, dir = %dir.display(), "hydrate-cache TTL sweep failed (best-effort)");
        }
        dir
    };
    let mut allowed_roots: Vec<&std::path::Path> = Vec::new();
    if let Some(root) = &sync_root {
        allowed_roots.push(root.as_path());
    }
    allowed_roots.push(temp_root.as_path());
    #[cfg(target_os = "macos")]
    allowed_roots.push(macos_hydrate_dir.as_path());

    // The engine calls `report` synchronously from inside the download loop;
    // hop through an unbounded channel so this task does the socket writes.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(u64, u64)>();
    let report = move |done: u64, total: u64| {
        let _ = tx.send((done, total));
    };
    let progress_cb: Option<&(dyn Fn(u64, u64) + Send + Sync)> = if progress.wanted() { Some(&report) } else { None };
    // macOS: through the alias, and from the queue while the held write is queued (spec §7.4).
    // Other unix builds hydrate from the server as before.
    #[cfg(target_os = "macos")]
    let hydrate = async {
        bridge
            .serve_hydrate(file_id.as_str(), dest, &allowed_roots, progress_cb)
            .await
            .map(|_| ())
    };
    #[cfg(not(target_os = "macos"))]
    let hydrate = bridge.hydrate_file_with_progress(file_id.as_str(), dest, &allowed_roots, progress_cb);
    tokio::pin!(hydrate);

    let result = loop {
        tokio::select! {
            biased;
            result = &mut hydrate => break Some(result),
            Some((done, total)) = rx.recv() => {
                if write_frame(write_half, &IpcResponse::HydrateProgress { done, total }).await.is_err() {
                    break None;
                }
            }
            read = frames.fill_more() => {
                // 0 / error = the peer closed. Bytes = a pipelined request,
                // which stays buffered for the next `next_frame()`; the
                // extension never pipelines during a hydrate, so a flood
                // beyond the request cap is treated as a hang-up.
                match read {
                    Ok(n) if n > 0 && frames.buffered_len() <= MAX_REQUEST_BYTES => {}
                    _ => break None,
                }
            }
        }
    };

    match result {
        Some(outcome) => {
            // Task 1693 — root cause of the missing terminal progress frame:
            // `report(done, total)` fires SYNCHRONOUSLY inside the hydrate
            // future's last poll, so the final chunk's frame is queued on `rx`
            // BEFORE the future returns Ready; the `biased` select polls the
            // hydrate branch first, completion outranks the queued frame, and
            // the loop above broke without ever writing it. Drain whatever the
            // future queued before answering, so the terminal (done == total)
            // frame always precedes the final reply, as the protocol promises
            // (`docs/IPC_PROTOCOL.md`: progress frames stream BEFORE the reply).
            while let Ok((done, total)) = rx.try_recv() {
                if write_frame(write_half, &IpcResponse::HydrateProgress { done, total })
                    .await
                    .is_err()
                {
                    let (file_id, id_kind) = crate::engine_bridge::wire_id_fields(file_id.as_str());
                    tracing::info!(
                        file_id,
                        id_kind,
                        "IPC client went away while flushing the terminal hydrate progress frames"
                    );
                    return HydrateOutcome::ClientGone;
                }
            }
            match outcome {
                Ok(()) => HydrateOutcome::Reply(IpcResponse::Ok {}),
                Err(e) => HydrateOutcome::Reply(hydrate_failure_reply(file_id.as_str(), e)),
            }
        }
        None => {
            // `hydrate` is dropped when this function returns.
            let (file_id, id_kind) = crate::engine_bridge::wire_id_fields(file_id.as_str());
            tracing::info!(
                file_id,
                id_kind,
                "IPC client went away mid-hydrate; hydration cancelled"
            );
            HydrateOutcome::ClientGone
        }
    }
}

/// The reply to a hydrate that failed: the extension's `fetchContents` fails
/// with it, and the system shows the item as failing to download. It leaves
/// one warning in the daemon log with the item id and a fixed category; the
/// error's text can carry a path or a URL, so it is classified, never logged.
/// The id is the request's, from the wire (M5): on Linux the identifier check is a no-op,
/// so it can be any caller-supplied text.
pub fn hydrate_failure_reply(file_id: &str, error: anyhow::Error) -> IpcResponse {
    let (file_id, id_kind) = crate::engine_bridge::wire_id_fields(file_id);
    tracing::warn!(
        file_id,
        id_kind,
        reason = hydrate_failure_category(&error),
        "Finder hydrate failed"
    );
    IpcResponse::Error {
        message: error.to_string(),
    }
}

/// Fixed category for a failed hydrate, for [`hydrate_failure_reply`].
pub(super) fn hydrate_failure_category(error: &anyhow::Error) -> &'static str {
    if crate::engine_bridge::engine_cause::<crate::engine_bridge::QueuedCopyMissing>(error).is_some() {
        return "queued_copy_missing";
    }
    if let Some(status) = crate::engine_bridge::error_http_status(error) {
        return match status {
            404 => "not_found",
            401 => "unauthorized",
            403 => "forbidden",
            409 => "conflict",
            500..=599 => "server_error",
            _ => "http_other",
        };
    }
    if error
        .chain()
        .any(|cause| cause.downcast_ref::<reqwest::Error>().is_some())
    {
        return "network";
    }
    if error
        .chain()
        .any(|cause| cause.downcast_ref::<std::io::Error>().is_some())
    {
        return "io";
    }
    if error
        .chain()
        .any(|cause| cause.downcast_ref::<rusqlite::Error>().is_some())
    {
        return "database";
    }
    "other"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_reporting_round_trips_the_wire_flag() {
        assert_eq!(ProgressReporting::from_wire(false), ProgressReporting::Off);
        assert_eq!(ProgressReporting::from_wire(true), ProgressReporting::On);
        assert!(!ProgressReporting::Off.wanted());
        assert!(ProgressReporting::On.wanted());
    }
}
