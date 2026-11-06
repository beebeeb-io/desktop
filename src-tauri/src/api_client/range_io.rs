//! LD-0105 W2: the desktop's transport for core's range session (`beebeeb_transfer::DirectIo`).
//!
//! The API half uses [`ApiClient`]'s client (its `X-Beebeeb-*` headers and the bearer token); the
//! capability header goes only on the calls that ask for it (the grant requests). The storage half
//! uses the separate storage client (no default headers, no redirects, https unless the API is on
//! loopback http), and its errors are status numbers only: reqwest's text holds the signed URL.
//!
//! LD-0105 S0: every body read is counted as it arrives, so a fetch that waits on a slow chunk still
//! shows Windows it is alive ([`crate::transfer_progress::ReceiveMeter::read_only`]). The progress
//! Cloud Files is told is the plaintext the sink handed over, never wire bytes, so a failed or
//! refetched body is liveness, not progress.

use std::future::Future;
use std::time::{Duration, Instant};

use beebeeb_core::direct_transfer as dt;
use beebeeb_transfer::{ApiCall, ApiMethod, ApiReply, DirectIo, SignedUrl, StorageReply};

use super::ApiClient;

/// One range session's transport: the account's client, the session's meter, and the configured
/// download limit (`desktop.toml` `download_kbps_limit`, 0 = none), shared by the session's GETs.
pub(crate) struct RangeIo<'a> {
    api: &'a ApiClient,
    meter: &'a crate::transfer_progress::ReceiveMeter,
    pace: Pace,
}

impl<'a> RangeIo<'a> {
    pub(crate) fn new(api: &'a ApiClient, meter: &'a crate::transfer_progress::ReceiveMeter, kbps_limit: u64) -> Self {
        RangeIo {
            api,
            meter,
            pace: Pace::new(kbps_limit),
        }
    }

    /// Count `n` body bytes that just arrived, then wait as long as the download limit asks.
    async fn arrived(&self, n: u64) {
        self.meter.read_only(n);
        if let Some(wait) = self.pace.after(n) {
            tokio::time::sleep(wait).await;
        }
    }
}

/// The download limit over all of a session's GETs: after `bytes` in total, at least
/// `bytes / (kbps × 1000)` seconds have passed since the first one.
struct Pace {
    bytes_per_s: u64,
    state: std::sync::Mutex<Option<(Instant, u64)>>,
}

impl Pace {
    fn new(kbps: u64) -> Self {
        Pace {
            bytes_per_s: kbps.saturating_mul(1000),
            state: std::sync::Mutex::new(None),
        }
    }

    fn after(&self, n: u64) -> Option<Duration> {
        if self.bytes_per_s == 0 {
            return None;
        }
        let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (start, total) = state.get_or_insert((Instant::now(), 0));
        *total += n;
        let due = Duration::from_secs_f64(*total as f64 / self.bytes_per_s as f64);
        due.checked_sub(start.elapsed())
    }
}

/// The process-wide monotonic epoch `mono_ms` counts from (the `DirectIo` contract: one clock for
/// every transport instance).
fn mono_epoch() -> Instant {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

impl DirectIo for RangeIo<'_> {
    fn api(&self, call: ApiCall) -> impl Future<Output = ApiReply> + Send {
        async move {
            let url = self.api.url(&call.path);
            let mut request = match call.method {
                ApiMethod::Get => self.api.client.get(url),
                ApiMethod::Post => self.api.client.post(url),
            }
            .header("Authorization", format!("Bearer {}", self.api.token))
            .timeout(call.timeout);
            if let Some(capability) = call.capabilities {
                request = request.header(dt::CAPABILITIES_HEADER, capability);
            }
            if let Some(body) = call.body {
                request = request.header("Content-Type", "application/json").body(body);
            }
            let Ok(mut resp) = request.send().await else {
                return ApiReply::default();
            };
            let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
            let (content_type, retry_after) = (header("content-type"), header("retry-after"));
            let status = resp.status().as_u16();
            // A pinned chunk's body is a download (S0: counted per read); a JSON answer is not.
            let counted = call.path.contains("/chunks/");
            let mut body = Vec::new();
            while body.len() < call.max_body {
                match resp.chunk().await {
                    Ok(Some(piece)) => {
                        if counted {
                            self.arrived(piece.len() as u64).await;
                        }
                        let room = call.max_body - body.len();
                        body.extend_from_slice(&piece[..piece.len().min(room)]);
                    }
                    Ok(None) => break,
                    // A body that broke off: what arrived is not the answer.
                    Err(_) => return ApiReply::default(),
                }
            }
            ApiReply {
                status,
                content_type,
                retry_after,
                body,
            }
        }
    }

    fn storage_get(
        &self,
        url: &SignedUrl,
        expected_len: u64,
        total: Duration,
        idle: Duration,
        _connect: Duration,
    ) -> impl Future<Output = StorageReply> + Send {
        let link = url.expose().to_string();
        async move {
            // The storage client's own connect timeout applies (`build_storage_client`).
            let Some(client) = self.api.storage_client() else {
                return StorageReply::default();
            };
            let Ok(mut resp) = client.get(&link).timeout(total).send().await else {
                return StorageReply::default();
            };
            let status = resp.status().as_u16();
            let content_length = resp.content_length();
            if status != 200 {
                return StorageReply {
                    status,
                    content_length,
                    body: None,
                };
            }
            // At most `expected_len + 1` bytes, so an overlong body shows as one byte too many.
            let cap = usize::try_from(expected_len.saturating_add(1)).unwrap_or(usize::MAX);
            let mut body = Vec::with_capacity(cap.min(content_length.map_or(0, |n| n as usize)).min(256 << 20));
            loop {
                match tokio::time::timeout(idle, resp.chunk()).await {
                    Ok(Ok(Some(piece))) => {
                        self.arrived(piece.len() as u64).await;
                        let room = cap - body.len();
                        body.extend_from_slice(&piece[..piece.len().min(room)]);
                        if body.len() >= cap {
                            break;
                        }
                    }
                    Ok(Ok(None)) => break,
                    // A broken or silent body: no HTTP answer to use (status 0, never reqwest's text).
                    Ok(Err(_)) | Err(_) => return StorageReply::default(),
                }
            }
            StorageReply {
                status,
                content_length,
                body: Some(bytes::Bytes::from(body)),
            }
        }
    }

    fn storage_put(
        &self,
        _url: &SignedUrl,
        _body: bytes::Bytes,
        _headers: &[(&'static str, String)],
        _total: Duration,
    ) -> impl Future<Output = u16> + Send {
        // Downloads only: a range session never uploads.
        async { 0 }
    }

    fn sleep(&self, d: Duration) -> impl Future<Output = ()> + Send {
        tokio::time::sleep(d)
    }

    fn wall_ms(&self) -> i64 {
        super::direct::now_ms()
    }

    fn mono_ms(&self) -> i64 {
        mono_epoch().elapsed().as_millis() as i64
    }
}

#[cfg(test)]
mod tests {
    use super::Pace;

    /// The download limit (`download_kbps_limit`) holds over a session's GETs: 1,000 KB/s means
    /// 500,000 bytes are due after 0.5 s. Red on the mutant "no pacing" (`after` always `None`).
    #[test]
    fn ld0105_w2_the_download_limit_paces_the_session() {
        let pace = Pace::new(1_000);
        let wait = pace.after(500_000).expect("a wait");
        assert!(wait > std::time::Duration::from_millis(400) && wait <= std::time::Duration::from_millis(500), "{wait:?}");
        assert_eq!(Pace::new(0).after(1 << 30), None, "no limit, no wait");
    }
}
