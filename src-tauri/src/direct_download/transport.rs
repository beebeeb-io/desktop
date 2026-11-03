//! Transport selection and the one storage fetch: `Transport` (auto/api, from `BB_TRANSPORT` or
//! `desktop.toml`), `FileTransport` (what a finished download used) and `Storage` (the
//! presigned-link fetch, with its test-only failure mode).
//!
//! The selection rules live in `beebeeb_core::direct_transfer`; this module maps them to the
//! desktop's names and performs a single fetch.
//! Must not renew grants or decide a fallback: see `download`.
//! Tests: `crate::direct_download::tests`.

use super::*;

/// Which transport a download may use. `Auto` (the default) uses direct storage transfers when the
/// server offers them and the handshake succeeds; `Api` forces today's API transport, for support
/// and the compatibility matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Transport {
    Auto,
    Api,
}

impl Transport {
    /// The environment variable that forces a transport (`api` or `auto`). It wins over
    /// `desktop.toml`'s `transport` key.
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) const ENV: &'static str = "BB_TRANSPORT";

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "api" => Some(Transport::Api),
            "auto" => Some(Transport::Auto),
            _ => None,
        }
    }

    /// The environment's value wins over the config file's; an unrecognised value is ignored.
    pub(crate) fn resolve(env: Option<&str>, config: Option<&str>) -> Self {
        env.and_then(Self::parse)
            .or_else(|| config.and_then(Self::parse))
            .unwrap_or(Transport::Auto)
    }

    /// The transport this process selects, given `desktop.toml`'s `transport` value. (A test bridge
    /// takes its test's transport instead, so a test never reads the developer's environment.)
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) fn selected(config: Option<&str>) -> Self {
        Self::resolve(std::env::var(Self::ENV).ok().as_deref(), config)
    }
}

/// Which transport a whole-file download ended up using.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileTransport {
    /// The API's per-chunk route, as before task 2173.
    Api,
    /// Every chunk straight from object storage.
    Direct,
    /// Direct, with some chunks through the pinned per-chunk API route. Still one version.
    Mixed,
}

impl FileTransport {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            FileTransport::Api => "api",
            FileTransport::Direct => "direct",
            FileTransport::Mixed => "mixed",
        }
    }
}

/// How a chunk is fetched from its presigned link.
#[derive(Clone, Copy)]
pub(crate) enum Storage<'a> {
    Client(&'a reqwest::Client),
    /// Tests only: every storage fetch fails with this status, without a request, so a test
    /// against a real server can force the renewals and the pinned-route fallback.
    #[cfg(test)]
    Fail(u16),
}

impl Storage<'_> {
    pub(super) async fn fetch(self, grant: dt::ChunkGrant) -> Result<Vec<u8>, u16> {
        match self {
            Storage::Client(client) => crate::api_client::fetch_from_storage(client, &grant).await,
            #[cfg(test)]
            Storage::Fail(status) => Err(status),
        }
    }
}
