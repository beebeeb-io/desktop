//! HTTP client for the Beebeeb API, used by the desktop sync daemon.
//!
//! Wraps `reqwest::Client` with the bits the daemon actually needs:
//! `list_files` for the periodic sync sweep, `download_chunk` for the
//! cloud-only → local materialisation path, `upload_chunk` for local →
//! cloud propagation, plus the per-file URL helper.
//!
//! Master key + session token are owned by the client so call sites
//! don't have to thread them through every method. Both arrive via
//! `apply_session` (the browser-login handoff).
//!
//! Spec: `docs/superpowers/plans/2026-05-07-desktop-sync-client.md`
//! Phase 1 Task 2.
//!
//! ## Why a fresh client (vs reusing the CLI crate)
//!
//! The CLI's `repos/cli/src/api.rs` is great prior art but lives in a
//! separate crate that pulls in clap, indicatif, dirs::config_dir, and
//! a config-file loader the daemon doesn't want. Easier to lift just
//! the HTTP primitives than to refactor the CLI into a library.
//!
//! ## Layout
//!
//! `client` owns the `ApiClient` value and the request core (construction, the
//! URLs, the bounded 429 retry, the status mapping); `auth`, `files`, `shares`,
//! `search` and `upload` add the request methods their domain names; `error`
//! the typed failure; `ids` the typed ids; `types` the wire DTOs; `headers` the
//! writer-provenance headers and 429 retry policy; `tests/` the unit tests,
//! split by area. This file re-exports the surface `api_client.rs` had before
//! the split.
use reqwest::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use urlencoding::encode;
use zeroize::Zeroize;

mod direct;
// LD-0105 W2: the range session's transport (`beebeeb_transfer::DirectIo`).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) mod range_io;
pub(crate) use direct::{BodyBytes, ChunkApiError, GrantReply, OnBody, fetch_from_storage, now_ms};
mod headers;
use headers::*;
pub(crate) use headers::{provenance_headers};

mod error;
pub use error::{DesktopApiError, Listing};
pub(crate) use error::{ApiResult, ErrorChain};

mod ids;
pub use ids::{FileId, FolderId, InviteId, SharedFolderId, UploadSessionId, VersionId};

mod types;
use types::*;
pub use types::{ListFilesScope, SyncSnapshot, SyncOp, SyncOps, DesktopUploadInitRequest, DesktopUploadInitResponse, InitConflictClass, classify_init_conflict, InitConflict, ProtectedFolderRefused, UploadRefusalCode, UploadRefused, UploadChunkResponse, ParentChange, SearchShardRef, MAX_SEARCH_SHARD_BLOB_BYTES, RegisteredDevice, CreatedSession, HeartbeatBody};
/// Tests only: the engine's tests provoke and pin the server's two 409 init messages by name.
#[cfg(test)]
pub use types::{IN_PROGRESS_MESSAGE, STALE_BASE_MESSAGE};
pub(crate) use types::{UpgradeRequired, upgrade_required, custody_unavailable};

mod client;
pub use client::{ApiClient};
mod auth;
// LD-0113: the Backups root endpoints.
mod backups;
mod files;
mod search;
mod shares;
mod upload;


















































#[cfg(test)]
mod region_set_fixture {
    use super::{ApiResult, DesktopApiError};
    use crate::account_dto::SetPreferredRegionResponse;
    use serde_json::Value;
    use std::collections::{HashMap, VecDeque};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(super) struct RegionRequest {
        pub method: String,
        pub path: String,
        pub authorization: Option<String>,
        pub body: String,
    }

    #[derive(Debug, Clone)]
    struct RegionFixtureResponse {
        status_line: String,
        body: Value,
    }

    #[derive(Debug)]
    struct RegionFixture {
        responses: VecDeque<RegionFixtureResponse>,
        requests: Vec<RegionRequest>,
    }

    pub(super) struct RegionFixtureHandle {
        id: String,
        pub base_url: String,
    }

    static FIXTURE_ID: AtomicUsize = AtomicUsize::new(1);
    static FIXTURES: OnceLock<Mutex<HashMap<String, Arc<Mutex<RegionFixture>>>>> = OnceLock::new();

    impl RegionFixtureHandle {
        pub fn start(responses: Vec<(String, Value)>) -> Self {
            let id = format!("set-region-fixture-{}", FIXTURE_ID.fetch_add(1, Ordering::Relaxed));
            let fixture = RegionFixture {
                responses: responses
                    .into_iter()
                    .map(|(status_line, body)| RegionFixtureResponse { status_line, body })
                    .collect(),
                requests: Vec::new(),
            };
            registry()
                .lock()
                .unwrap()
                .insert(id.clone(), Arc::new(Mutex::new(fixture)));
            Self {
                base_url: format!("fixture://{id}"),
                id,
            }
        }

        pub fn finish(self) -> Vec<RegionRequest> {
            let fixture = registry()
                .lock()
                .unwrap()
                .remove(&self.id)
                .expect("region fixture should still be registered");
            fixture.lock().unwrap().requests.clone()
        }
    }

    /// Answer a `set_preferred_region` call from a fixture, or `None` when the
    /// client is not pointed at one (every real build).
    pub(super) fn try_handle(
        base_url: &str,
        token: &str,
        body: &Value,
    ) -> ApiResult<Option<SetPreferredRegionResponse>> {
        let Some(id) = base_url.strip_prefix("fixture://") else {
            return Ok(None);
        };
        let fixture = registry()
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .unwrap_or_else(|| panic!("region fixture {id} missing"));
        let mut fixture = fixture.lock().unwrap();
        fixture.requests.push(RegionRequest {
            method: "PUT".to_string(),
            path: "/api/v1/me/region".to_string(),
            authorization: Some(format!("Bearer {token}")),
            body: body.to_string(),
        });

        let response = fixture
            .responses
            .pop_front()
            .expect("region fixture has no response left");
        if !response.status_line.starts_with('2') {
            let snippet: String = response.body.to_string().chars().take(300).collect();
            let code = response
                .status_line
                .split_whitespace()
                .next()
                .and_then(|code| code.parse::<u16>().ok())
                .expect("the fixture's status line starts with a status code");
            return Err(DesktopApiError::HttpStatus {
                status: reqwest::StatusCode::from_u16(code).expect("a known status code"),
                snippet,
            });
        }

        Ok(Some(
            serde_json::from_value(response.body).expect("a SetPreferredRegionResponse body"),
        ))
    }

    fn registry() -> &'static Mutex<HashMap<String, Arc<Mutex<RegionFixture>>>> {
        FIXTURES.get_or_init(|| Mutex::new(HashMap::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use region_set_fixture::RegionFixtureHandle as RegionMockServer;
    use std::future::Future;

    fn run_async<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    // ── link monitor (task 1683 slice 2) ─────────────────────────────────────────
    //
    // `send_with_retry` is the one path every engine request that is retried on 429
    // goes through (`sync_ops`, `sync_snapshot`, `list_files`, every typed GET). It
    // must record what happened to the request, because `sync_tick` swallows a failed
    // `/sync/ops` and returns `Ok`.

    /// Serve ONE request on loopback with the given raw HTTP response (or hang up
    /// without answering when `None`), then stop. Returns the base URL.
    fn one_shot_server(raw_response: Option<&'static str>) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                let _ = stream.read(&mut buf);
                if let Some(raw) = raw_response {
                    let _ = stream.write_all(raw.as_bytes());
                }
            }
        });
        url
    }

    fn link_client(base: String) -> ApiClient {
        ApiClient::new(base, "tok".into(), [0u8; 32])
    }

    mod errors;
    mod ids;
    mod link;
    mod urls;
    mod region;
    mod serialization;
    mod retry;
    mod provenance;
    mod upload;








}
