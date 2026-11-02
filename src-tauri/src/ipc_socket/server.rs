//! `server` items split out of `ipc_socket.rs`.

use super::*;

/// Bind and 0600-harden the daemon's Unix IPC listener at `path`. Split out
/// of `serve_ipc_at` (task 1524) so a bind failure can be tested in
/// isolation and, more importantly, so it no longer panics: this used to be
/// a bare `UnixListener::bind(&path).expect("bind IPC socket")` inside a
/// fire-and-forget `tokio::spawn`ed task, so a failure (e.g. the sandbox
/// refusing a path outside the app's container) silently killed the whole
/// IPC accept loop with no error ever reaching the caller — the readiness
/// probe just timed out with a generic message. Callers now get the real
/// `std::io::Error` back and can log/surface it.
pub(super) fn bind_ipc_listener(path: &std::path::Path) -> std::io::Result<UnixListener> {
    bind_ipc_listener_with(path, |path| UnixListener::bind(path))
}

pub(super) fn bind_ipc_listener_with<T>(
    path: &std::path::Path,
    bind: impl FnOnce(&std::path::Path) -> std::io::Result<T>,
) -> std::io::Result<T> {
    use std::os::unix::fs::PermissionsExt;

    // Binding at the public endpoint exposes its umask-derived mode before
    // chmod. Stage in a fresh owner-only directory instead; never change the
    // process-wide umask in this multithreaded application.
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let staging = tempfile::Builder::new()
        .prefix("")
        .rand_bytes(6)
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir_in(parent)?;
    // Six directory characters + "/s" = eight bytes, matching "ipc.sock":
    // staging also fits the macOS App Group sockaddr_un path budget.
    let staged_path = staging.path().join("s");
    let listener = bind(&staged_path)?;
    // Fail closed: neither chmod nor publication failure may return a listener.
    // TempDir removes the private socket/directory on every error path.
    std::fs::set_permissions(&staged_path, std::fs::Permissions::from_mode(0o600))?;
    std::fs::rename(&staged_path, path)?;
    Ok(listener)
}

pub async fn serve_ipc(
    db: std::sync::Arc<crate::state_db::StateDb>,
    bridge: std::sync::Arc<crate::engine_bridge::EngineBridge>,
    cancel: oneshot::Receiver<()>,
) -> std::io::Result<()> {
    serve_ipc_at(ipc_socket_path(), db, bridge, cancel).await
}

/// Real IPC server, bound to an explicit `path`. `serve_ipc` calls this with the
/// production `ipc_socket_path()`; tests bind it to a throwaway temp socket so
/// they can exercise the real accept/dispatch path without touching the real
/// production socket path (task 1247).
///
/// Returns `Err` if binding, hardening or publication fails (see [`bind_ipc_listener`]) —
/// never panics. A bind failure means the loop below never starts; the
/// caller is responsible for logging/surfacing the error (`runner.rs` logs
/// it and records it for `wait_for_file_provider_ipc_ready` to report
/// verbatim instead of just timing out).
///
/// Write contents follow [`WriteContentsPolicy::platform_default`]: on macOS
/// only the App Group upload-staging directory is accepted.
pub async fn serve_ipc_at(
    path: std::path::PathBuf,
    db: std::sync::Arc<crate::state_db::StateDb>,
    bridge: std::sync::Arc<crate::engine_bridge::EngineBridge>,
    cancel: oneshot::Receiver<()>,
) -> std::io::Result<()> {
    serve_ipc_at_with_ready(path, db, bridge, cancel, None, WriteContentsPolicy::platform_default()).await
}

/// Readiness means the listener is bound, hardened and published. On startup
/// failure the sender is dropped and the server returns the original error.
/// `contents` decides where write requests may take file contents from.
pub(crate) async fn serve_ipc_at_with_ready(
    path: std::path::PathBuf,
    db: std::sync::Arc<crate::state_db::StateDb>,
    bridge: std::sync::Arc<crate::engine_bridge::EngineBridge>,
    mut cancel: oneshot::Receiver<()>,
    ready: Option<oneshot::Sender<()>>,
    contents: WriteContentsPolicy,
) -> std::io::Result<()> {
    let listener = bind_ipc_listener(&path)?;
    if let Some(ready) = ready {
        let _ = ready.send(());
    }
    tracing::info!("IPC socket listening at {:?}", path);
    let mut connections: Vec<JoinHandle<()>> = Vec::new();
    // One table per daemon process, shared by every connection: a retry arrives
    // on a NEW connection (task 1684). It is deliberately in-memory only.
    let write_dedup = WriteDedup::<IpcResponse>::new();

    loop {
        connections.retain(|handle| !handle.is_finished());
        tokio::select! {
            biased;
            _ = &mut cancel => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let db = db.clone();
                    let bridge = bridge.clone();
                    let write_dedup = write_dedup.clone();
                    let contents = contents.clone();
                    connections.push(tokio::spawn(handle_connection(stream, db, bridge, write_dedup, contents)));
                }
                Err(e) => tracing::warn!("IPC accept error: {e}"),
            }
        }
    }
    for handle in connections {
        handle.abort();
    }
    drop(listener);
    let _ = std::fs::remove_file(&path);
    tracing::info!("IPC socket stopped at {:?}", path);
    Ok(())
}

/// Read the connecting peer process's UID from a connected Unix stream.
///
/// Linux uses `SO_PEERCRED` (the kernel fills a `libc::ucred`). macOS/iOS use
/// `getpeereid()` — the sanctioned BSD API for exactly this (simpler than the
/// `LOCAL_PEERCRED` getsockopt: no struct layout to get wrong, libc fills two
/// plain `uid_t`/`gid_t` out-params). Implemented and VERIFIED on real macOS
/// hardware 2026-08-29 (Darwin 25.6, Apple Silicon): with this in place the two
/// real-socket engine_bridge IPC tests (`ipc_allows_hydrate_write_under_allowed_root`,
/// `ipc_rejects_hydrate_write_outside_allowed_roots`) go from failing-closed
/// (connection dropped, client sees EOF/BrokenPipe) to passing — closing the
/// blocking item in the macOS bring-up brief (task 1247). Platforms with
/// neither implementation still fail closed (returns `Err`, so the caller
/// rejects the connection).
#[cfg(target_os = "linux")]
pub(super) fn peer_uid(stream: &UnixStream) -> std::io::Result<u32> {
    let fd = std::os::unix::io::AsRawFd::as_raw_fd(stream);
    // SAFETY: `cred` is a plain-old-data struct the kernel fills; we pass its
    // size in `len` (updated in-place) and only read `cred` after a 0 return.
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut libc::ucred as *mut libc::c_void,
            &mut len,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(cred.uid)
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(super) fn peer_uid(stream: &UnixStream) -> std::io::Result<u32> {
    let fd = std::os::unix::io::AsRawFd::as_raw_fd(stream);
    let mut euid: libc::uid_t = 0;
    let mut egid: libc::gid_t = 0;
    // SAFETY: getpeereid only writes the two out-params on success (rc == 0);
    // both are plain integers we own on the stack. We read them only after
    // checking rc.
    let rc = unsafe { libc::getpeereid(fd, &mut euid, &mut egid) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(euid)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "ios")))]
pub(super) fn peer_uid(_stream: &UnixStream) -> std::io::Result<u32> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "peer credential check not implemented for this platform",
    ))
}

/// Pure authorization predicate, split out so the comparison logic is unit-
/// testable without a real cross-UID connection (which would need root/setuid
/// privileges no sandboxed test runner has). The daemon only serves a peer
/// running as its own OS user.
pub(super) fn is_authorized_peer(daemon_uid: u32, peer_uid: u32) -> bool {
    daemon_uid == peer_uid
}

pub(super) async fn handle_connection(
    stream: UnixStream,
    db: std::sync::Arc<crate::state_db::StateDb>,
    bridge: std::sync::Arc<crate::engine_bridge::EngineBridge>,
    write_dedup: std::sync::Arc<WriteDedup<IpcResponse>>,
    contents_policy: WriteContentsPolicy,
) {
    // Authenticate the peer BEFORE reading or dispatching anything. The daemon
    // holds vault keys in memory and `hydrate_file` is a decrypt oracle, so a
    // connection from any other OS user is refused outright — no request is
    // parsed, no response is written, the connection is just dropped
    // (task 1247). `peer_uid` is implemented for Linux (SO_PEERCRED) and
    // macOS/iOS (getpeereid); any other platform fails closed.
    let daemon_uid = unsafe { libc::getuid() };
    match peer_uid(&stream) {
        Ok(uid) if is_authorized_peer(daemon_uid, uid) => {}
        Ok(uid) => {
            tracing::warn!(
                peer_uid = uid,
                daemon_uid,
                "IPC connection rejected: peer UID does not match daemon"
            );
            return;
        }
        Err(e) => {
            tracing::warn!(error = %e, "IPC connection rejected: could not verify peer credentials");
            return;
        }
    }

    // Framing (task 1670 issue 3): one compact-JSON request per `\n`-terminated
    // line, exactly one delimited reply per request (hydrate may send
    // `HydrateProgress` lines first when asked). See `crate::ipc_frame` and
    // `docs/IPC_PROTOCOL.md`.
    let (read_half, mut write_half) = stream.into_split();
    let mut frames = FrameReader::new(read_half, MAX_REQUEST_BYTES);
    loop {
        let line = match frames.next_frame().await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(FrameError::TooLarge { limit }) => {
                tracing::warn!(limit, "IPC request exceeded the size limit; closing connection");
                let _ = write_frame(
                    &mut write_half,
                    &IpcResponse::Error {
                        message: format!("request exceeds the {limit}-byte limit"),
                    },
                )
                .await;
                break;
            }
            Err(FrameError::Io(_)) => break,
        };
        let req: IpcRequest = match serde_json::from_slice(&line) {
            Ok(r) => r,
            Err(e) => {
                // The stream is still in sync (we consumed exactly one line),
                // so answer and keep serving the connection.
                if write_frame(&mut write_half, &IpcResponse::Error { message: e.to_string() })
                    .await
                    .is_err()
                {
                    break;
                }
                continue;
            }
        };
        let resp = match req {
            IpcRequest::GetFileStatus { file_id } => file_status_response(&db, &file_id),
            IpcRequest::ListFileProviderItems { container_id } => IpcResponse::FileProviderItems {
                items: list_file_provider_items(&db, &container_id),
            },
            IpcRequest::HydrateFile {
                file_id,
                dest_path,
                progress,
            } => {
                match hydrate_over_ipc(&mut frames, &mut write_half, &bridge, &file_id, &dest_path, progress).await {
                    HydrateOutcome::Reply(resp) => resp,
                    // The client hung up mid-hydrate: nobody is left to reply to.
                    HydrateOutcome::ClientGone => break,
                }
            }
            IpcRequest::QueueFinderCreate {
                parent_id,
                filename,
                kind,
                contents_path,
                content_type,
                request_id,
                deletion_conflicted,
                template_identifier,
                template_content_version,
            } => {
                match admit_write_contents(&contents_policy, "create", contents_path) {
                    Err(refusal) => contents_refusal_response(refusal),
                    Ok((_, staged)) if parent_id.as_deref() == Some(NAMESPACE_SHARED_WITH_ME) => {
                        // Refused: the copy it handed over is deleted now.
                        drop(staged);
                        log_refused_write("create", "read_only_namespace");
                        // Answer and keep serving the connection (this used to
                        // `return`, silently dropping the connection).
                        IpcResponse::Error {
                            message: "Shared with me is read-only at the namespace root".into(),
                        }
                    }
                    Ok((contents_path, staged)) => {
                        let target = crate::engine_bridge::FinderWriteTarget {
                            file_id: None,
                            parent_id: normalize_parent_id(parent_id),
                            filename,
                            // OS-extension IPC supplies the leaf name only; keep the
                            // leaf-as-path fallback (queue_finder_create defaults
                            // rel_path → filename). The macOS/Linux extensions thread
                            // their own nesting via parent_id, not a relative path.
                            rel_path: None,
                            kind: parse_write_kind(&kind),
                            contents_path,
                            content_type,
                            base_version_identifier: None,
                        };
                        let fingerprint = format!(
                            "create|{}|{}|{}",
                            target.parent_id.as_deref().unwrap_or(""),
                            target.filename,
                            parse_write_kind_name(&target.kind)
                        );
                        let (work_db, work_bridge) = (db.clone(), bridge.clone());
                        // `staged` rides in the work closure: it is dropped (the
                        // handed-over copy deleted) once the engine has its own
                        // copy or refused, or unrun when a repeat is answered
                        // from the first attempt's result.
                        dedup_write(&write_dedup, &db, request_id, fingerprint, move || {
                            // The engine reads the opened file, never the path.
                            let opened = staged.as_ref().map(StagedContents::file);
                            // Only the macOS File Provider path mints write tokens (spec §8.8),
                            // and only it resolves a deletion-conflicted create (§7.2).
                            #[cfg(target_os = "macos")]
                            let result = if deletion_conflicted {
                                work_bridge.queue_file_provider_deletion_conflicted_create(
                                    target,
                                    template_identifier,
                                    template_content_version,
                                    opened,
                                )
                            } else {
                                work_bridge.queue_file_provider_create_from(target, opened)
                            };
                            #[cfg(not(target_os = "macos"))]
                            let result = {
                                let _ = (deletion_conflicted, template_identifier, template_content_version);
                                work_bridge
                                    .queue_finder_create_from(target, opened)
                                    .map(crate::engine_bridge::FpWrite::plain)
                            };
                            let response = write_outcome_response("create", &work_db, result);
                            drop(staged);
                            response
                        })
                        .await
                    }
                }
            }
            IpcRequest::QueueFinderModify {
                file_id,
                parent_id,
                filename,
                kind,
                contents_path,
                content_type,
                base_version_identifier,
                request_id,
            } => match admit_write_contents(&contents_policy, "modify", contents_path) {
                Err(refusal) => contents_refusal_response(refusal),
                Ok((contents_path, staged)) => {
                    let target = crate::engine_bridge::FinderWriteTarget {
                        file_id: Some(file_id),
                        parent_id: normalize_parent_id(parent_id),
                        filename,
                        // Modify is metadata/version only; no new-file path key.
                        rel_path: None,
                        kind: parse_write_kind(&kind),
                        contents_path,
                        content_type,
                        base_version_identifier,
                    };
                    let fingerprint = format!(
                        "modify|{}|{}|{}",
                        target.file_id.as_deref().unwrap_or(""),
                        target.filename,
                        parse_write_kind_name(&target.kind)
                    );
                    let (work_db, work_bridge) = (db.clone(), bridge.clone());
                    // See the create arm for when `staged` is dropped.
                    dedup_write(&write_dedup, &db, request_id, fingerprint, move || {
                        let opened = staged.as_ref().map(StagedContents::file);
                        // Only the macOS File Provider path mints write tokens (spec §8.8).
                        #[cfg(target_os = "macos")]
                        let result = work_bridge.queue_file_provider_modify_from(target, opened);
                        #[cfg(not(target_os = "macos"))]
                        let result = work_bridge
                            .queue_finder_modify_from(target, opened)
                            .map(crate::engine_bridge::FpWrite::plain);
                        let response = write_outcome_response("modify", &work_db, result);
                        drop(staged);
                        response
                    })
                    .await
                }
            },
            IpcRequest::QueueFinderDelete {
                file_id,
                base_version_identifier,
            } => {
                // The user is deleting this item: no remembered create may hand
                // it back to a later request that happens to carry the same key
                // (same file copied in again). Forget BEFORE queueing, so a
                // create that arrives while the trash is being queued is a new
                // create. The row stays in place until the server trash
                // converges, so the row alone cannot say "deleted" (task 1684).
                forget_dedup_for_item(&write_dedup, &file_id);
                write_outcome_response(
                    "delete",
                    &db,
                    bridge
                        .queue_finder_delete(&file_id, base_version_identifier)
                        .map(crate::engine_bridge::FpWrite::plain),
                )
            }
            // Task 1698: the `SetRecursivePin` IPC RPC is RETIRED — it had no
            // caller (audit G13): the File Provider extension never sent it
            // and the pinning surface is the Tauri `set_recursive_pin`
            // command (Onboarding / Mac Settings), which calls
            // `EngineBridge::set_recursive_pin` directly. Task 1698 replaces
            // the concept with per-item `contentPolicy` (`.pinned` on the
            // payload → `.downloadEagerlyAndKeepDownloaded`); the pin STATE
            // itself stays on `files.pin_state` (task 1683 owns the backend).
            //
            // Task 1699 (audit G13): three more never-called RPCs are
            // RETIRED from the wire — `SetFileStatus` (a no-op stub the
            // extension never sent), `RecordOpenedFile` and
            // `EnforceSmartCache` (smart-cache bookkeeping with no File
            // Provider caller; the daemon enforces the cache limit itself
            // via `EngineBridge::enforce_configured_cache_limit`, and
            // `IpcResponse::CacheCleanup` went with them). Unknown variants
            // are refused per-request by the deserialization error path
            // above without dropping the connection; see
            // `retired_rpcs_are_refused_and_the_connection_survives`.
            IpcRequest::FetchThumbnail {
                file_id,
                dest_path,
                max_dimension,
            } => {
                // `file_id` is a raw item identifier straight off the wire;
                // validate it exactly like the hydrate arm does before it is
                // trusted for anything else.
                #[cfg(target_os = "macos")]
                let identifier_check = macos_validate_hydrate_item_identifier(&file_id);
                #[cfg(not(target_os = "macos"))]
                let identifier_check: Result<(), &'static str> = Ok(());
                let validated = match identifier_check {
                    Err(msg) => Err(IpcResponse::Error {
                        message: msg.to_string(),
                    }),
                    Ok(()) => {
                        // `dest_path` arrives straight off the wire
                        // (untrusted). Bound it to the caller's legitimate
                        // destinations — the same allowed roots as
                        // `HydrateFile` (sync root, temp dir, and on macOS
                        // the shared App Group hydrate-cache directory) —
                        // BEFORE any fetch runs, and keep the plaintext
                        // staging write atomic (`.part` + rename).
                        let allowed_roots = thumbnail_allowed_roots();
                        let dest = std::path::PathBuf::from(&dest_path);
                        match thumbnail_destination_error(&dest, &allowed_roots) {
                            Some(message) => Err(IpcResponse::Error { message }),
                            None => {
                                // `finder_thumbnail` is async (HTTP + decrypt;
                                // through the alias, and an error while the
                                // item's write is queued, spec §5.6); the
                                // staging write that follows is a small
                                // blocking file write, which the other
                                // dispatch arms also do inline.
                                let fetched = bridge
                                    .finder_thumbnail(&file_id, thumbnail_variant(max_dimension))
                                    .await;
                                match fetched {
                                    Ok(plaintext) => {
                                        match persist_thumbnail_plaintext(&dest, &allowed_roots, &plaintext) {
                                            Ok(size_bytes) => Ok(IpcResponse::ThumbnailWritten { size_bytes }),
                                            Err(e) => Ok(IpcResponse::Error {
                                                message: format!("thumbnail fetch failed: {e}"),
                                            }),
                                        }
                                    }
                                    Err(e) => Ok(IpcResponse::Error {
                                        message: format!("thumbnail fetch failed: {e}"),
                                    }),
                                }
                            }
                        }
                    }
                };
                validated.unwrap_or_else(|resp| resp)
            }
            IpcRequest::GetSyncSummary => {
                let syncing = db
                    .count_by_status(crate::state_db::FileStatus::Downloading)
                    .unwrap_or(0);
                let cloud_only = db.count_by_status(crate::state_db::FileStatus::CloudOnly).unwrap_or(0);
                let conflicts = db.count_by_status(crate::state_db::FileStatus::Conflict).unwrap_or(0);
                IpcResponse::SyncSummary {
                    syncing,
                    cloud_only,
                    conflicts,
                }
            }
            IpcRequest::ListChanges { since_anchor, limit } => {
                // Page limit: honor the caller's suggested size, clamped to a
                // 100-change ceiling (Apple caps the system at 100x its
                // suggestion; our ceiling keeps replies well inside the frame
                // budget). Absent -> 100.
                let page = limit.unwrap_or(100).clamp(1, 100) as usize;
                match db.list_file_changes_paged(since_anchor.as_deref().map(str::as_bytes), page) {
                    Ok(Some((changes, anchor))) => IpcResponse::FileProviderChanges {
                        changes: file_provider_change_payloads(&db, changes),
                        next_anchor: anchor.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
                    },
                    Ok(None) => IpcResponse::FileProviderChanges {
                        changes: Vec::new(),
                        next_anchor: None,
                    },
                    // An unparseable anchor means the log no longer contains
                    // that cursor (sweep/gap): the replica must fall back to a
                    // full enumeration, so surface a real error rather than a
                    // silently-wrong empty reply.
                    Err(e) => IpcResponse::Error {
                        message: format!("sync anchor expired: {e}"),
                    },
                }
            }
            IpcRequest::GetSyncAnchor => IpcResponse::FileProviderSyncAnchor {
                // The persistent cursor survives extension process death (it
                // lives in state.db); nil only when nothing ever changed.
                anchor: db
                    .fp_last_anchor()
                    .ok()
                    .flatten()
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
            },
            IpcRequest::ReportMaterialized { container_ids } => match db.set_materialized_containers(&container_ids) {
                Ok(()) => IpcResponse::Ok {},
                Err(e) => IpcResponse::Error { message: e.to_string() },
            },
        };
        if write_frame(&mut write_half, &resp).await.is_err() {
            break;
        }
    }
}
