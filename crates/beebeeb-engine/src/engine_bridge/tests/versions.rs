//! `versions` engine tests, split out of `engine_bridge.rs`.

use super::*;
pub(super) fn versioned_server_answer(request: &RecordedRequest, s: &mut VersionedServerState) -> String {
    let method = request.method.as_str();
    let path = request.path.as_str();
    if method == "POST" && path == "/api/v1/uploads/init" {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        if let Some(status) = body["file_id"].as_str().and_then(|id| s.refuse_init.get(id)).copied() {
            let code = status.split(' ').next().and_then(|code| code.parse().ok()).unwrap_or(0);
            s.inits.push((body, code));
            return http_json(status, serde_json::json!({ "error": "refused" }));
        }
        if let Some((status, refusal)) = body["file_id"]
            .as_str()
            .and_then(|id| s.refuse_init_with.get(id))
            .cloned()
        {
            let code = status.split(' ').next().and_then(|code| code.parse().ok()).unwrap_or(0);
            s.inits.push((body, code));
            return http_json(status, refusal);
        }
        if body["file_id"].as_str().is_some_and(|id| s.trashed_files.contains(id)) {
            s.inits.push((body, 409));
            return http_json("409 Conflict", serde_json::json!({ "error": "file is in trash" }));
        }
        if let Some(message) = body["file_id"].as_str().and_then(|id| s.conflict_init_once.remove(id)) {
            s.inits.push((body, 409));
            return http_json("409 Conflict", serde_json::json!({ "error": message }));
        }
        let file_id = match body["file_id"].as_str() {
            Some(id) => {
                if let Some(file) = s.files.get(id)
                    && let Some(base) = body["base_version_number"].as_i64()
                    && base != file.versions.len() as i64
                {
                    s.inits.push((body, 409));
                    return http_json(
                        "409 Conflict",
                        serde_json::json!({ "error": "stale base version for replacement upload" }),
                    );
                }
                s.files.entry(id.to_string()).or_default();
                id.to_string()
            }
            None => {
                s.next_file += 1;
                let id = if s.uuid_ids {
                    format!("5e7e0000-0000-4000-8000-{:012}", s.next_file)
                } else {
                    format!("server-file-{}", s.next_file)
                };
                s.files.insert(id.clone(), MockServerFile::default());
                id
            }
        };
        s.next_session += 1;
        let session = format!("session-{}", s.next_session);
        s.sessions.insert(session.clone(), (file_id.clone(), Vec::new()));
        // As on the server: the session's object version is the one `complete` makes current, and
        // the one the file's row then names (`download_version.object_version_id`).
        let next_version = s.files.get(&file_id).map_or(0, |file| file.versions.len()) + 1;
        let response = serde_json::json!({
            "file_id": file_id,
            "tenant_id": "tenant-1",
            "object_version_id": mock_object_version_id(&file_id, next_version),
            "upload_session_id": session,
            "chunk_size_bytes": body["chunk_size_bytes"],
            "chunk_count": body["chunk_count"],
            "storage_format_version": 2,
            "storage_pool_id": "pool-1",
            "region": "local"
        });
        s.inits.push((body, 201));
        return http_json("201 Created", response);
    }
    // A restore appends a copy of version `vid` (the mock's object ids are
    // `object-{id}-v{n}`) and answers as the server's first branch does (VER:613-618).
    if method == "POST"
        && let Some(rest) = path.strip_prefix("/api/v1/files/")
        && rest.ends_with("/restore")
    {
        let parts: Vec<&str> = rest.split('/').collect(); // [id, "versions", vid, "restore"]
        let (file_id, object) = (parts[0].to_string(), parts[2].to_string());
        let index = object
            .rsplit("-v")
            .next()
            .and_then(|n| n.parse::<usize>().ok())
            .unwrap_or(1)
            .saturating_sub(1);
        let file = s.files.entry(file_id.clone()).or_default();
        let chunks = file.versions.get(index).cloned().unwrap_or_default();
        file.versions.push(chunks);
        let version = file.versions.len();
        // With version metadata on, the restore's name becomes the file's name and the new
        // version's metadata in the same step.
        if let Some(name) = serde_json::from_slice::<serde_json::Value>(&request.body)
            .ok()
            .and_then(|body| body["name_encrypted"].as_str().map(str::to_string))
        {
            s.authenticated_names.insert(file_id.clone(), name.clone());
            s.version_metadata.insert((file_id.clone(), version), name);
        }
        s.restores.push((file_id.clone(), object));
        return http_json(
            "200 OK",
            serde_json::json!({
                "message": "version restored",
                "version_number": version,
                "current_object_version_id": format!("object-{file_id}-v{version}"),
            }),
        );
    }
    // `GET /files/{id}/versions`: every version, newest first, with the metadata it was made with.
    if method == "GET"
        && let Some(id) = path
            .strip_prefix("/api/v1/files/")
            .and_then(|rest| rest.strip_suffix("/versions"))
        && let Some(file) = s.files.get(id)
    {
        let prefix = s
            .version_id_prefix
            .get(id)
            .cloned()
            .unwrap_or_else(|| format!("object-{id}"));
        let versions: Vec<serde_json::Value> = (1..=file.versions.len())
            .rev()
            .map(|n| {
                serde_json::json!({
                    "id": format!("{prefix}-v{n}"),
                    "version_number": n,
                    "version_metadata_encrypted": s.version_metadata.get(&(id.to_string(), n)),
                })
            })
            .collect();
        return http_json("200 OK", serde_json::json!({ "file_id": id, "versions": versions }));
    }
    if method == "DELETE"
        && let Some(id) = path.strip_prefix("/api/v1/files/")
    {
        s.trashes.push(id.to_string());
        return http_json("200 OK", serde_json::json!({ "ok": true }));
    }
    if method == "PATCH"
        && let Some(id) = path.strip_prefix("/api/v1/files/")
    {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap_or(serde_json::Value::Null);
        // A server without version metadata has one name per file: the last one written.
        if s.version_metadata_off
            && let Some(name) = body["name_encrypted"].as_str()
        {
            s.authenticated_names.insert(id.to_string(), name.to_string());
        }
        s.patches.push((id.to_string(), body));
        return http_json("200 OK", serde_json::json!({ "ok": true }));
    }
    // `POST /files/{id}/upload/abandon`: keyed by FILE, as on the server. Every open session of the
    // file goes, whoever opened it.
    if method == "POST"
        && let Some(id) = path
            .strip_prefix("/api/v1/files/")
            .and_then(|rest| rest.strip_suffix("/upload/abandon"))
    {
        s.sessions.retain(|_, (file_id, _)| file_id != id);
        return http_json("200 OK", serde_json::json!({ "id": id, "status": "abandoned" }));
    }
    if let Some(rest) = path.strip_prefix("/api/v1/uploads/") {
        let mut parts = rest.split('/');
        let session = parts.next().unwrap_or_default().to_string();
        let action = parts.next().unwrap_or_default();
        if method == "PUT" && action == "chunks" {
            let index: usize = parts.next().unwrap_or("0").parse().unwrap();
            if s.fail_first_chunk_once.remove(&session) {
                return http_json("500 Internal Server Error", serde_json::json!({ "error": "boom" }));
            }
            if let Some((status, refusal)) = s
                .sessions
                .get(&session)
                .and_then(|(file_id, _)| s.refuse_chunk_with.get(file_id))
                .cloned()
            {
                return http_json(status, refusal);
            }
            let Some((_, chunks)) = s.sessions.get_mut(&session) else {
                return http_json("404 Not Found", serde_json::json!({ "error": "no session" }));
            };
            if chunks.len() <= index {
                chunks.resize(index + 1, Vec::new());
            }
            chunks[index] = request.body.clone();
            return http_json(
                "200 OK",
                serde_json::json!({ "index": index, "size": request.body.len(), "skipped": false }),
            );
        }
        // The lease renewal: 200 while the session is open, 404 once it is gone.
        if method == "POST" && action == "heartbeat" {
            return if s.sessions.contains_key(&session) {
                http_json("200 OK", serde_json::json!({ "upload_session_id": session }))
            } else {
                http_json("404 Not Found", serde_json::json!({ "error": "no session" }))
            };
        }
        if method == "POST" && action == "complete" {
            if s.fail_complete_once.remove(&session) {
                return http_json("500 Internal Server Error", serde_json::json!({ "error": "boom" }));
            }
            if let Some((status, refusal)) = s
                .sessions
                .get(&session)
                .and_then(|(file_id, _)| s.refuse_complete_with.get(file_id))
                .cloned()
            {
                return http_json(status, refusal);
            }
            // The server's rule with version metadata on: a completion name is refused when the
            // file's name is no longer the one the session's init saw (a rename in between).
            if s.renamed_since_init.remove(&session) {
                return http_json(
                    "409 Conflict",
                    serde_json::json!({
                        "error": "file_changed_during_upload",
                        "message": "the file was renamed while this upload was in progress"
                    }),
                );
            }
            if let Some(file_id) = s.completed_reply_lost.get(&session).cloned() {
                return http_json(
                    "200 OK",
                    serde_json::json!({ "id": file_id, "status": "completed", "already_completed": true }),
                );
            }
            let Some((file_id, chunks)) = s.sessions.remove(&session) else {
                return http_json("404 Not Found", serde_json::json!({ "error": "no session" }));
            };
            // nonce (12) + tag (16) per chunk.
            let size: usize = chunks.iter().map(|chunk| chunk.len().saturating_sub(28)).sum();
            let file = s.files.entry(file_id.clone()).or_default();
            file.versions.push(chunks);
            let version = file.versions.len();
            // As the server does with version metadata on: the completion name (the upload's sealed
            // name with its version commitment) becomes the file's name in the same step, and the
            // answer carries the name the file now has.
            // Version metadata off: the completion name is ignored, and the answer names the row
            // as it was (the name of the file's last init when nothing else named it).
            let completed_name = if s.version_metadata_off {
                s.authenticated_names.get(&file_id).cloned().or_else(|| {
                    s.inits
                        .iter()
                        .rev()
                        .find(|(body, _)| body["file_id"].as_str() == Some(file_id.as_str()))
                        .and_then(|(body, _)| body["file_name"].as_str().map(str::to_string))
                })
            } else {
                completion_name(request)
            };
            if let Some(name) = completed_name.clone() {
                s.authenticated_names.insert(file_id.clone(), name.clone());
                if !s.version_metadata_off {
                    s.version_metadata.insert((file_id.clone(), version), name);
                }
            }
            if s.complete_reply_lost_once.remove(&session) {
                s.completed_reply_lost.insert(session, file_id);
                return http_json(
                    "500 Internal Server Error",
                    serde_json::json!({ "error": "reply lost" }),
                );
            }
            let mut answer = serde_json::json!({
                "file_id": file_id,
                "version_number": version,
                "current_object_version_id": format!("object-{file_id}-v{version}"),
                "size_bytes": size,
                "mime_type": "text/plain"
            });
            if let Some(name) = completed_name {
                answer["name_encrypted"] = serde_json::json!(name);
            }
            return http_json("200 OK", answer);
        }
    }
    if method == "GET" && path == "/api/v1/shares/invites/incoming" {
        let body = s
            .incoming_invites
            .clone()
            .unwrap_or_else(|| serde_json::json!({ "invites": [] }));
        return http_json("200 OK", body);
    }
    if method == "GET" && path == "/api/v1/sync/snapshot" {
        return match s.snapshots.pop_front() {
            Some((status, body)) => http_json(&status, body),
            None => http_json(
                "503 Service Unavailable",
                serde_json::json!({ "error": "no snapshot queued" }),
            ),
        };
    }
    if method == "GET"
        && let Some(query) = path.strip_prefix("/api/v1/sync/ops")
    {
        let since = query
            .strip_prefix("?since=")
            .and_then(|since| since.parse::<i64>().ok())
            .unwrap_or(0);
        return http_json("200 OK", serde_json::json!({ "ops": [], "since": since }));
    }
    http_json(
        "404 Not Found",
        serde_json::json!({ "error": format!("unexpected {method} {path}") }),
    )
}

/// Run the queue until nothing is due, moving the clock past every
/// backoff so a refused op is retried. Returns the number of passes.
pub(super) async fn drain_upload_queue(bridge: &EngineBridge, sync_root: &Path) -> usize {
    let mut now = now_secs();
    for pass in 1..=12 {
        bridge.process_due_operations(sync_root, now).await.unwrap();
        now = now.saturating_add(10_000);
        if bridge.db.list_due_operations(now).unwrap().is_empty() {
            return pass;
        }
    }
    12
}

/// Run `action` on its own thread and wait for it, at most 5 s, and fail the test if it
/// has not finished. Every seam sits outside every transaction and every lock, so a
/// correct implementation never blocks the action: a hang is a defect, not a timing
/// artefact, and a silent timeout would let the action race the rest of the test.
pub(super) fn run_competing(action: impl FnOnce() + Send + 'static) {
    let (done, wait) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        action();
        let _ = done.send(());
    });
    wait.recv_timeout(Duration::from_secs(5))
        .expect("the competing action did not finish");
}

pub(super) fn finder_file_target(
    file_id: Option<&str>,
    filename: &str,
    contents: &Path,
    base_version_identifier: Option<String>,
) -> FinderWriteTarget {
    FinderWriteTarget {
        file_id: file_id.map(str::to_string),
        parent_id: None,
        filename: filename.to_string(),
        rel_path: None,
        kind: FinderWriteItemKind::File,
        contents_path: Some(contents.to_string_lossy().into_owned()),
        content_type: Some("text/plain".into()),
        base_version_identifier,
    }
}

/// The content version the extension holds for `file_id`, read the way
/// the extension gets it: from the IPC item payload (no IPC socket off
/// unix, so the same formula directly).
pub(super) fn held_content_version(bridge: &EngineBridge, file_id: &str) -> String {
    let entry = bridge.db.get_file(file_id).unwrap().unwrap();
    #[cfg(unix)]
    {
        crate::ipc_socket::file_entry_payload_for_db(&bridge.db, &entry, "root")
            .content_version
            .unwrap()
    }
    #[cfg(not(unix))]
    {
        let contract = bridge.db.get_file_contract_state(file_id).unwrap().unwrap();
        item_content_version(contract.current_version, entry.content_hash.as_deref())
    }
}
