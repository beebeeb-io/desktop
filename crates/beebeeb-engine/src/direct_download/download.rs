//! The direct-download loop: windowed storage fetches, in-order decryption into one zeroizing
//! buffer, grant renewal, the pinned per-chunk API fallback, and the decision to hand the file
//! back to the API path (`Outcome::UseApi`).
//!
//! Owns pacing (`paced_until_ms`), the event loop and the version-pin rules.
//! Must not log or return a URL: a storage failure is a status number.
//! Tests: `crate::direct_download::tests`.

use super::*;

/// When the user's download ceiling lets the next request start: `wire_bytes` received since
/// `start_ms` at `kbps` KB/s. The API path sleeps after each chunk; with up to four requests in
/// flight the direct path holds back new ones instead, so the average rate is the same.
pub fn paced_until_ms(start_ms: i64, wire_bytes: u64, kbps: u64) -> i64 {
    if kbps == 0 {
        return i64::MIN;
    }
    let ms = u128::from(wire_bytes) * 1000 / (u128::from(kbps) * 1024);
    start_ms.saturating_add(i64::try_from(ms).unwrap_or(i64::MAX))
}

/// Download `request.file_id` over the direct transport when the server offers it.
///
/// `Ok(UseApi)`: not used or abandoned; the caller downloads the file through the API.
/// `Err`: a failure the API path would not fix.
pub async fn download(
    api: &ApiClient,
    request: Request<'_>,
    transport: Transport,
    storage_override: Option<Storage<'_>>,
    progress: &(dyn Progress + Sync),
) -> Result<Outcome, DirectError> {
    if transport == Transport::Api || request.chunk_count == 0 {
        return Ok(Outcome::UseApi);
    }
    let Some(discovery) = api.direct_download_offer().await else {
        return Ok(Outcome::UseApi);
    };
    let storage = match storage_override {
        Some(storage) => storage,
        None => match api.storage_client() {
            Some(client) => Storage::Client(client),
            None => return Ok(Outcome::UseApi),
        },
    };
    let file_id = request.file_id;
    progress.decrypted(0);
    let config = dt::PlannerConfig::from_discovery(&discovery, rand::random());
    let mut planner = dt::DownloadPlanner::new(config, discovery, api.api_base(), file_id);
    let mut tasks: FuturesUnordered<Task<'_>> = FuturesUnordered::new();
    let mut ready: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    let mut plaintext: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::with_capacity(request.approx_size));
    let mut reader = beebeeb_core::chunk_stream::ChunkDecryptor::for_push_with_file_key(
        beebeeb_core::kdf::FileKey::from_bytes(*request.file_key.as_bytes()),
        request.file_id,
        request.expected_format,
    );
    let mut geometry_checked = false;
    // Chunks that came through the pinned API route instead of storage.
    let mut api_chunks: u32 = 0;
    let started_ms = crate::api_client::now_ms();
    let mut wire_bytes: u64 = 0;

    loop {
        // Everything the planner has for us right now, unless the user's ceiling holds new
        // requests back (the ones in flight still finish).
        let paced = paced_until_ms(started_ms, wire_bytes, request.download_kbps_limit);
        let wake: Option<i64> = if crate::api_client::now_ms() < paced {
            Some(paced)
        } else {
            loop {
                match planner.poll(crate::api_client::now_ms()) {
                    dt::Poll::RequestWindow(window) => {
                        tracing::debug!(
                            start = window.start_index,
                            count = window.count,
                            "direct download: window request"
                        );
                        tasks.push(Box::pin(async move {
                            Event::Window(api.download_grants(file_id, &window).await)
                        }));
                    }
                    dt::Poll::Fetch { index, grant } => {
                        tasks.push(Box::pin(async move {
                            Event::Storage {
                                index,
                                result: storage.fetch(grant, &|b| report_body(progress, b)).await,
                            }
                        }));
                    }
                    dt::Poll::FetchViaApi {
                        index,
                        object_version_id,
                    } => {
                        let Some(length) = planner.geometry().and_then(|g| g.chunk_length(index)) else {
                            return Ok(Outcome::UseApi);
                        };
                        tracing::debug!(chunk = index, "direct download: chunk through the pinned API route");
                        tasks.push(Box::pin(async move {
                            Event::Api {
                                index,
                                result: api
                                    .download_chunk_pinned(file_id, index, &object_version_id, length, &|b| {
                                        report_body(progress, b)
                                    })
                                    .await,
                            }
                        }));
                    }
                    dt::Poll::Wait { until_ms } => break until_ms,
                    dt::Poll::Done => {
                        let transport = if api_chunks > 0 {
                            FileTransport::Mixed
                        } else {
                            FileTransport::Direct
                        };
                        reader.finish().map_err(|_| DirectError::Corrupt)?;
                        return Ok(Outcome::Done { plaintext, transport });
                    }
                    dt::Poll::Stop(verdict) => {
                        return match verdict {
                            dt::Verdict::ApiForFile => {
                                tracing::debug!("direct download: falling back to the API for this file");
                                Ok(Outcome::UseApi)
                            }
                            dt::Verdict::Restart => Err(DirectError::VersionChanged),
                            dt::Verdict::Terminal(t) => Err(DirectError::Refused(t.status)),
                            dt::Verdict::Corrupt => Err(DirectError::Corrupt),
                        };
                    }
                }
            }
        };

        // Wait for something to finish, or for the next backoff (or the ceiling) to end.
        let event = if tasks.is_empty() {
            match wake {
                Some(at) => {
                    tokio::time::sleep(until(at)).await;
                    None
                }
                None => {
                    // Nothing outstanding and nothing scheduled: the planner is stuck. Never hang a
                    // download on that; the API path is always correct.
                    tracing::debug!("direct download: planner stalled; falling back to the API");
                    return Ok(Outcome::UseApi);
                }
            }
        } else if let Some(at) = wake {
            tokio::select! {
                ev = tasks.next() => ev,
                _ = tokio::time::sleep(until(at)) => None,
            }
        } else {
            tasks.next().await
        };

        let now = crate::api_client::now_ms();
        match event {
            None => {}
            Some(Event::Window(GrantReply::Window(response))) => match planner.window_answered(response, now) {
                Ok(()) => {
                    if !geometry_checked {
                        geometry_checked = true;
                        if let Some(pin) = request.pin {
                            let landed = planner
                                .pinned_version()
                                .and_then(|v| uuid::Uuid::parse_str(v).ok())
                                .map(|v| v.to_string());
                            if landed.as_deref() != Some(pin.object_version_id.as_str()) {
                                tracing::debug!("direct download: the grant names another version than the metadata");
                                return Err(DirectError::VersionChanged);
                            }
                            // The same version: the grant's sizes are its manifest's, or the API path
                            // (which checks every chunk against the manifest) serves the file.
                            let manifest = dt::Geometry {
                                chunk_count: pin.chunk_count,
                                chunk_size: pin.chunk_size_bytes,
                                encrypted_size: pin.ciphertext_size_bytes,
                                original_size: pin.plaintext_size_bytes,
                            };
                            if planner.geometry() != Some(manifest) {
                                tracing::debug!(
                                    "direct download: the grant's sizes differ from the version's manifest"
                                );
                                return Ok(Outcome::UseApi);
                            }
                        }
                        if planner.geometry().map(|g| g.chunk_count) != Some(request.chunk_count) {
                            // The same version (or a legacy file) whose manifest and metadata
                            // disagree: the API path, which has its own checks, serves it.
                            tracing::debug!("direct download: manifest chunk count differs from the metadata");
                            return Ok(Outcome::UseApi);
                        }
                    }
                }
                Err(violation) => tracing::debug!(%violation, "direct download: window refused"),
            },
            Some(Event::Window(GrantReply::Malformed)) => {
                tracing::debug!("direct download: window answer could not be decoded");
                planner.window_malformed();
            }
            Some(Event::Window(GrantReply::Failed(outcome))) => {
                tracing::debug!(?outcome, "direct download: window failed");
                planner.window_failed(outcome, now);
            }
            Some(Event::Storage { index, result }) => match result {
                Ok(bytes) => {
                    // Its body was reported to `progress` read by read as it arrived (LD-0105 S0).
                    wire_bytes += bytes.len() as u64;
                    if let Err(e) = planner.fetch_ok(index) {
                        tracing::debug!(error = %e, "direct download: planner error");
                        return Ok(Outcome::UseApi);
                    }
                    ready.insert(index, bytes);
                }
                Err(status) => {
                    tracing::debug!(chunk = index, status, "direct download: chunk from storage failed");
                    if let Err(e) = planner.fetch_failed(index, now) {
                        tracing::debug!(error = %e, "direct download: planner error");
                        return Ok(Outcome::UseApi);
                    }
                }
            },
            Some(Event::Api { index, result }) => match result {
                Ok(bytes) => {
                    // Its body was reported to `progress` read by read as it arrived (LD-0105 S0).
                    wire_bytes += bytes.len() as u64;
                    if let Err(e) = planner.fetch_ok(index) {
                        tracing::debug!(error = %e, "direct download: planner error");
                        return Ok(Outcome::UseApi);
                    }
                    api_chunks += 1;
                    ready.insert(index, bytes);
                }
                Err(ChunkApiError::Restart) => return Err(DirectError::VersionChanged),
                Err(ChunkApiError::Terminal(status)) => return Err(DirectError::Refused(status)),
                // The pinned route failed: nothing per chunk is left to try. The planner ends the
                // file and the API path takes it from the start.
                Err(ChunkApiError::Failed(message) | ChunkApiError::Unreachable(message)) => {
                    tracing::debug!(chunk = index, %message, "direct download: pinned API route failed");
                    if let Err(e) = planner.api_fetch_failed(index) {
                        tracing::debug!(error = %e, "direct download: planner error");
                        return Ok(Outcome::UseApi);
                    }
                }
            },
        }

        // Decrypt and append what is ready, strictly in order.
        loop {
            let next = planner.consumed_count();
            let Some(frame) = ready.remove(&next) else { break };
            match crate::engine_bridge::content_frame(&frame)
                .and_then(|frame| reader.push_frame(&frame).map(|opened| opened.data).map_err(Into::into))
            {
                Ok(mut chunk) => {
                    plaintext.extend_from_slice(&chunk);
                    chunk.zeroize();
                    progress.decrypted(plaintext.len() as u64);
                    if let Err(e) = planner.consumed(next) {
                        tracing::debug!(error = %e, "direct download: planner error");
                        return Ok(Outcome::UseApi);
                    }
                }
                Err(_) => {
                    tracing::debug!(chunk = next, "direct download: chunk did not authenticate");
                    // Its bytes were reported as they arrived; it is fetched again, so they are not progress.
                    progress.discarded(frame.len() as u64);
                    if let Err(e) = planner.chunk_unauthentic(next) {
                        tracing::debug!(error = %e, "direct download: planner error");
                        return Ok(Outcome::UseApi);
                    }
                    break;
                }
            }
        }
    }
}

/// One body report (LD-0105 S0) to `progress`.
fn report_body(progress: &dyn super::Progress, bytes: crate::api_client::BodyBytes) {
    match bytes {
        crate::api_client::BodyBytes::Read(n) => progress.received(n),
        crate::api_client::BodyBytes::Discarded(n) => progress.discarded(n),
    }
}

pub(super) fn until(at_ms: i64) -> Duration {
    Duration::from_millis(u64::try_from(at_ms.saturating_sub(crate::api_client::now_ms())).unwrap_or(0))
}
