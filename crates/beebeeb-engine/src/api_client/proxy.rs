//! The proxy rule of every client the engine builds for the API base.
//!
//! Owns: whether a client that talks to the API base may go through a configured proxy. Must not: build a client for
//! anything but the API base (the storage links of `direct.rs` keep their own client), or read the environment itself
//! (reqwest does, for the proxies it takes by default). Tests: `api_client/tests/proxy.rs`.

/// [`reqwest::Client::builder`] for a client that talks to `api_base` (an origin, no slash at the end), with
/// [`with_api_base_proxy_rule`] applied. Every client the engine builds for the API base starts here.
pub fn api_base_client_builder(api_base: &str) -> reqwest::ClientBuilder {
    with_api_base_proxy_rule(reqwest::Client::builder(), api_base)
}

/// `builder` for `api_base`. A plain `http://` base on this computer (`127.0.0.1`, `[::1]` or `localhost`, any port: a
/// test server) is reached directly, never through a proxy. reqwest takes the environment's and the system's proxies
/// by default, with no exception for loopback, so through a proxy the session token, the sign-in messages and the
/// request bodies would cross to the proxy host unencrypted, and a proxy on another computer would reach its own
/// loopback, not this one's. Every other base, `https://` to this computer included, keeps the default.
pub(crate) fn with_api_base_proxy_rule(builder: reqwest::ClientBuilder, api_base: &str) -> reqwest::ClientBuilder {
    if is_plain_http_loopback(api_base) {
        builder.no_proxy()
    } else {
        builder
    }
}

/// `http://` to `127.0.0.1`, `[::1]` or `localhost`, as the URL parser reads `api_base`.
pub(crate) fn is_plain_http_loopback(api_base: &str) -> bool {
    reqwest::Url::parse(api_base)
        .is_ok_and(|url| url.scheme() == "http" && matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost")))
}
