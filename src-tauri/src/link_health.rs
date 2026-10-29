//! Can Beebeeb reach its server, and when did it last get an answer? (task 1683
//! slice 2; spec `docs/specs/2026-09-30-macos-menubar-popover.md` section 9,
//! rows "Offline vs server did not answer" and "Last successful check".)
//!
//! Two different failures need two different words in the popover:
//!
//! - **Offline** (state d2): the request never reached a server. DNS, no route,
//!   connection refused. Copy: "You're offline".
//! - **Server did not answer** (state d): the request went out and no usable
//!   answer came back. A timeout, a dropped connection, a 5xx. Copy: "Can't reach
//!   Beebeeb".
//!
//! A 4xx (including 401 and 429) is NOT either: the server answered. 401 has its
//! own path (`AuthHealth`, state e2) and 429 is backed off inside the client.
//!
//! ## Why a monitor and not just the tick's `Err`
//!
//! `sync_tick` swallows a failed `GET /sync/ops` on purpose ("skip this tick and
//! retry next time", engine_bridge.rs) and returns `Ok`. That is the commonest way
//! for a steady-state tick to fail when the network drops, so the tick's `Err`
//! alone would report "Up to date" for as long as the Mac is offline. Every request
//! that goes through `ApiClient::send_with_retry` therefore records its outcome
//! here, and the runner reads the monitor after the tick. A tick is only a
//! successful check (`last_tick_ok_at` advances) when it returned `Ok` AND no
//! request in it failed at the link level.
//!
//! Known limit: requests that bypass `send_with_retry` (chunk PUTs and GETs, file
//! metadata) do not record. The per-tick `sync_ops`/`sync_snapshot` call does, and
//! every tick makes one, so an outage is seen within one tick either way.

use std::sync::Mutex;

/// The per-request timeout `ApiClient` builds its `reqwest::Client` with. One
/// constant so the popover's "timeout after 30 s" cannot drift from the client.
pub const API_REQUEST_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkFailureKind {
    /// The request never reached a server (state d2).
    Offline,
    /// The request went out and no usable answer came back (state d).
    ServerDidNotAnswer,
}

/// Why, in one stable word. The UI never parses error text; it gets this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkReason {
    /// DNS, no route, connection refused.
    Connect,
    /// No answer within [`API_REQUEST_TIMEOUT_SECS`].
    Timeout,
    /// The connection was accepted and then dropped or reset before an answer.
    NoResponse,
    /// The server answered with a 5xx.
    Http5xx(u16),
}

impl LinkReason {
    /// Stable machine code (goes into the event and the snapshot).
    pub fn code(self) -> &'static str {
        match self {
            LinkReason::Connect => "connect",
            LinkReason::Timeout => "timeout",
            LinkReason::NoResponse => "no_response",
            LinkReason::Http5xx(_) => "http_5xx",
        }
    }

    /// The mono line under state d's sentence (spec section 4: it must ADD
    /// information, never restate the sentence, and is at most 30 characters).
    /// `None` when there is nothing to add: state d2 ("You're offline") has no
    /// mono line.
    pub fn detail(self) -> Option<String> {
        match self {
            LinkReason::Connect => None,
            LinkReason::Timeout => Some(format!("timeout after {API_REQUEST_TIMEOUT_SECS} s")),
            LinkReason::NoResponse => Some("connection dropped".to_string()),
            LinkReason::Http5xx(status) => Some(format!("HTTP {status}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkFailure {
    pub kind: LinkFailureKind,
    pub reason: LinkReason,
}

impl LinkFailure {
    const fn offline() -> Self {
        Self {
            kind: LinkFailureKind::Offline,
            reason: LinkReason::Connect,
        }
    }

    const fn no_answer(reason: LinkReason) -> Self {
        Self {
            kind: LinkFailureKind::ServerDidNotAnswer,
            reason,
        }
    }
}

impl From<LinkFailureKind> for crate::surfaces::phase::Connectivity {
    fn from(kind: LinkFailureKind) -> Self {
        match kind {
            LinkFailureKind::Offline => crate::surfaces::phase::Connectivity::Offline,
            LinkFailureKind::ServerDidNotAnswer => crate::surfaces::phase::Connectivity::ServerDidNotAnswer,
        }
    }
}

/// Classify one `io::Error` from the transport's source chain.
fn classify_io(error: &std::io::Error) -> Option<LinkFailure> {
    use std::io::ErrorKind::*;
    match error.kind() {
        ConnectionRefused | NotConnected | AddrNotAvailable | HostUnreachable | NetworkUnreachable | NetworkDown => {
            Some(LinkFailure::offline())
        }
        TimedOut => Some(LinkFailure::no_answer(LinkReason::Timeout)),
        ConnectionReset | ConnectionAborted | BrokenPipe | UnexpectedEof => {
            Some(LinkFailure::no_answer(LinkReason::NoResponse))
        }
        _ => None,
    }
}

/// Classify a `reqwest::Error`. `None` means "not a link failure": the server
/// answered (a decode error, a 4xx turned into an error) or it is ours (a builder
/// error).
pub fn classify_reqwest(error: &reqwest::Error) -> Option<LinkFailure> {
    if let Some(status) = error.status() {
        return status
            .is_server_error()
            .then(|| LinkFailure::no_answer(LinkReason::Http5xx(status.as_u16())));
    }
    if error.is_timeout() {
        return Some(LinkFailure::no_answer(LinkReason::Timeout));
    }
    if error.is_connect() {
        return Some(LinkFailure::offline());
    }
    // Something under reqwest (hyper, the OS) with a recognisable io kind.
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        if let Some(io) = cause.downcast_ref::<std::io::Error>()
            && let Some(failure) = classify_io(io)
        {
            return Some(failure);
        }
        source = cause.source();
    }
    // The request was sent but the exchange broke before a response (for example
    // the peer closed mid-message): the server did not answer.
    if error.is_request() {
        return Some(LinkFailure::no_answer(LinkReason::NoResponse));
    }
    None
}

/// Parse `check_status`'s own text, `HTTP 503 Service Unavailable: <body>`.
fn classify_status_text(message: &str) -> Option<LinkFailure> {
    let rest = message.strip_prefix("HTTP ")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let status: u16 = digits.parse().ok()?;
    (500..=599)
        .contains(&status)
        .then(|| LinkFailure::no_answer(LinkReason::Http5xx(status)))
}

/// Classify an error from anywhere in the engine. Walks the whole chain, so a
/// `reqwest::Error` wrapped by `anyhow::Context` is still found.
pub fn classify_error(error: &anyhow::Error) -> Option<LinkFailure> {
    for cause in error.chain() {
        if let Some(re) = cause.downcast_ref::<reqwest::Error>() {
            return classify_reqwest(re);
        }
        if let Some(io) = cause.downcast_ref::<std::io::Error>()
            && let Some(failure) = classify_io(io)
        {
            return Some(failure);
        }
    }
    classify_status_text(&error.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LinkSnapshot {
    /// When a request last got a usable answer (unix seconds).
    pub last_ok_at: Option<i64>,
    /// The most recent failure and when it happened. Cleared by the next answer.
    pub failure: Option<(LinkFailure, i64)>,
}

/// The outcome of the most recent request, shared between the API client (which
/// writes it) and the runner (which reads it after each tick).
#[derive(Debug, Default)]
pub struct LinkMonitor {
    inner: Mutex<LinkSnapshot>,
}

impl LinkMonitor {
    pub fn new() -> Self {
        Self::default()
    }

    /// A request got an answer the server meant (any non-5xx response).
    pub fn note_ok(&self, now: i64) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.last_ok_at = Some(now);
            inner.failure = None;
        }
    }

    pub fn note_failure(&self, failure: LinkFailure, now: i64) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.failure = Some((failure, now));
        }
    }

    /// Record a finished request: a link failure, or an answer.
    pub fn note_reqwest_error(&self, error: &reqwest::Error, now: i64) {
        match classify_reqwest(error) {
            Some(failure) => self.note_failure(failure, now),
            // Not a link failure (the server answered, or it is a local error): it
            // does not say the link is down, and does not prove it is up either.
            None => {}
        }
    }

    pub fn note_status(&self, status: reqwest::StatusCode, now: i64) {
        if status.is_server_error() {
            self.note_failure(LinkFailure::no_answer(LinkReason::Http5xx(status.as_u16())), now);
        } else {
            self.note_ok(now);
        }
    }

    pub fn snapshot(&self) -> LinkSnapshot {
        self.inner.lock().map(|inner| *inner).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    /// A one-connection-at-a-time HTTP server on an ephemeral loopback port.
    /// `respond` gets the connection and decides what a server would do.
    fn serve(respond: impl Fn(std::net::TcpStream) + Send + 'static) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                respond(stream);
            }
        });
        url
    }

    fn drain_request(stream: &mut std::net::TcpStream) {
        let mut buf = [0u8; 4096];
        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
        let _ = stream.read(&mut buf);
    }

    fn block<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn client(timeout: Duration) -> reqwest::Client {
        // `no_proxy`: a proxy in the environment (CI, this sandbox) would answer for
        // loopback and turn every case below into a proxy's failure, not ours.
        reqwest::Client::builder().no_proxy().timeout(timeout).build().unwrap()
    }

    async fn get(client: &reqwest::Client, url: &str) -> Result<reqwest::Response, reqwest::Error> {
        client.get(url).send().await
    }

    #[test]
    fn a_refused_connection_is_offline() {
        block(async {
            // Bind, note the port, drop the listener: nothing listens there any more.
            let port = {
                let l = TcpListener::bind("127.0.0.1:0").unwrap();
                l.local_addr().unwrap().port()
            };
            let err = get(&client(Duration::from_secs(5)), &format!("http://127.0.0.1:{port}/"))
                .await
                .unwrap_err();
            let failure = classify_reqwest(&err).expect("a refused connection is a link failure");
            assert_eq!(failure.kind, LinkFailureKind::Offline);
            assert_eq!(failure.reason, LinkReason::Connect);
            // And through anyhow, which is how the engine sees it.
            let wrapped = anyhow::Error::new(err).context("sync_ops");
            assert_eq!(classify_error(&wrapped).unwrap().kind, LinkFailureKind::Offline);
        })
    }

    #[test]
    fn an_unresolvable_host_is_offline() {
        block(async {
            // `.invalid` is reserved (RFC 2606): it can never resolve.
            let err = get(&client(Duration::from_secs(10)), "http://nothing.invalid/")
                .await
                .unwrap_err();
            let failure = classify_reqwest(&err).expect("a DNS failure is a link failure");
            assert_eq!(failure.kind, LinkFailureKind::Offline, "{err:?}");
        })
    }

    #[test]
    fn a_server_that_never_answers_is_a_timeout_not_offline() {
        block(async {
            let url = serve(|mut stream| {
                drain_request(&mut stream);
                // Accept, read, say nothing.
                std::thread::sleep(Duration::from_secs(3));
            });
            let err = get(&client(Duration::from_millis(300)), &url).await.unwrap_err();
            assert!(err.is_timeout(), "{err:?}");
            let failure = classify_reqwest(&err).unwrap();
            assert_eq!(failure.kind, LinkFailureKind::ServerDidNotAnswer);
            assert_eq!(failure.reason, LinkReason::Timeout);
        })
    }

    #[test]
    fn a_connection_dropped_before_an_answer_did_not_answer() {
        block(async {
            let url = serve(|mut stream| {
                drain_request(&mut stream);
                drop(stream);
            });
            let err = get(&client(Duration::from_secs(5)), &url).await.unwrap_err();
            let failure = classify_reqwest(&err).expect("a dropped connection is a link failure");
            assert_eq!(failure.kind, LinkFailureKind::ServerDidNotAnswer, "{err:?}");
            assert_eq!(failure.reason, LinkReason::NoResponse);
        })
    }

    #[test]
    fn a_5xx_is_the_server_not_answering_and_a_4xx_is_not_a_link_failure() {
        block(async {
            for (status_line, expect) in [
                ("503 Service Unavailable", Some(503u16)),
                ("500 Internal Server Error", Some(500)),
                ("401 Unauthorized", None),
                ("404 Not Found", None),
                ("429 Too Many Requests", None),
            ] {
                let status_line = status_line.to_string();
                let body = status_line.clone();
                let url = serve(move |mut stream| {
                    drain_request(&mut stream);
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {body}\r\ncontent-length: 2\r\nconnection: close\r\n\r\nno"
                    );
                });
                let resp = get(&client(Duration::from_secs(5)), &url).await.unwrap();
                // Through `error_for_status`, which is what `get_file` does.
                let via_err = resp.error_for_status().unwrap_err();
                let got = classify_reqwest(&via_err).map(|f| match f.reason {
                    LinkReason::Http5xx(s) => s,
                    other => panic!("expected an HTTP status reason, got {other:?}"),
                });
                assert_eq!(got, expect, "{status_line}");
            }
        })
    }

    #[test]
    fn the_clients_own_status_text_is_classified() {
        // `ApiClient::check_status` bails with this text for a non-2xx.
        let e = anyhow::anyhow!("HTTP 502 Bad Gateway: upstream gone");
        let f = classify_error(&e).unwrap();
        assert_eq!(f.kind, LinkFailureKind::ServerDidNotAnswer);
        assert_eq!(f.reason, LinkReason::Http5xx(502));
        for not_a_failure in [
            "HTTP 401 Unauthorized: bad token",
            "HTTP 404 Not Found: nope",
            "HTTP 429 slow down",
        ] {
            assert_eq!(classify_error(&anyhow::anyhow!(not_a_failure)), None, "{not_a_failure}");
        }
        // Text that merely mentions a 5xx is not a status line.
        assert_eq!(classify_error(&anyhow::anyhow!("the file 5xx.txt failed")), None);
        assert_eq!(classify_error(&anyhow::anyhow!("decrypt failed")), None);
    }

    #[test]
    fn io_kinds_map_to_the_two_failures() {
        use std::io::{Error, ErrorKind};
        let offline = [
            ErrorKind::ConnectionRefused,
            ErrorKind::HostUnreachable,
            ErrorKind::NetworkUnreachable,
            ErrorKind::NetworkDown,
            ErrorKind::NotConnected,
            ErrorKind::AddrNotAvailable,
        ];
        for kind in offline {
            let f = classify_error(&anyhow::Error::new(Error::from(kind))).unwrap();
            assert_eq!(f.kind, LinkFailureKind::Offline, "{kind:?}");
        }
        let no_answer = [
            ErrorKind::TimedOut,
            ErrorKind::ConnectionReset,
            ErrorKind::ConnectionAborted,
            ErrorKind::BrokenPipe,
            ErrorKind::UnexpectedEof,
        ];
        for kind in no_answer {
            let f = classify_error(&anyhow::Error::new(Error::from(kind))).unwrap();
            assert_eq!(f.kind, LinkFailureKind::ServerDidNotAnswer, "{kind:?}");
        }
        // A local disk error is not the network.
        assert_eq!(
            classify_error(&anyhow::Error::new(Error::from(ErrorKind::PermissionDenied))),
            None
        );
        assert_eq!(
            classify_error(&anyhow::Error::new(Error::from(ErrorKind::NotFound))),
            None
        );
    }

    #[test]
    fn reason_lines_add_information_and_fit_the_30_character_rule() {
        assert_eq!(LinkReason::Timeout.detail().as_deref(), Some("timeout after 30 s"));
        assert_eq!(LinkReason::Http5xx(503).detail().as_deref(), Some("HTTP 503"));
        assert_eq!(LinkReason::NoResponse.detail().as_deref(), Some("connection dropped"));
        // State d2 ("You're offline") has no mono line.
        assert_eq!(LinkReason::Connect.detail(), None);
        for reason in [
            LinkReason::Timeout,
            LinkReason::Http5xx(599),
            LinkReason::NoResponse,
            LinkReason::Connect,
        ] {
            if let Some(line) = reason.detail() {
                assert!(line.chars().count() <= 30, "{line:?}");
                assert!(!line.contains('\n'));
            }
        }
        let codes: Vec<&str> = [
            LinkReason::Connect,
            LinkReason::Timeout,
            LinkReason::NoResponse,
            LinkReason::Http5xx(500),
        ]
        .iter()
        .map(|r| r.code())
        .collect();
        assert_eq!(codes, ["connect", "timeout", "no_response", "http_5xx"]);
    }

    #[test]
    fn the_shown_timeout_is_the_one_the_client_is_built_with() {
        // The constant feeds both the client and the words "timeout after N s".
        let source = include_str!("api_client.rs").replace("\r\n", "\n");
        assert!(
            source.contains(".timeout(Duration::from_secs(crate::link_health::API_REQUEST_TIMEOUT_SECS))"),
            "ApiClient::new must build its client with link_health::API_REQUEST_TIMEOUT_SECS"
        );
    }

    #[test]
    fn the_latest_outcome_wins_and_an_answer_clears_a_failure() {
        let m = LinkMonitor::new();
        assert_eq!(m.snapshot(), LinkSnapshot::default());

        m.note_ok(100);
        assert_eq!(m.snapshot().last_ok_at, Some(100));
        assert_eq!(m.snapshot().failure, None);

        m.note_failure(LinkFailure::offline(), 200);
        let s = m.snapshot();
        assert_eq!(s.last_ok_at, Some(100), "a failure does not erase when it last worked");
        assert_eq!(s.failure, Some((LinkFailure::offline(), 200)));

        m.note_ok(300);
        let s = m.snapshot();
        assert_eq!(s.last_ok_at, Some(300));
        assert_eq!(s.failure, None, "an answer clears the failure");
    }

    #[test]
    fn a_5xx_status_records_a_failure_and_any_other_status_records_an_answer() {
        let m = LinkMonitor::new();
        m.note_status(reqwest::StatusCode::SERVICE_UNAVAILABLE, 10);
        assert_eq!(
            m.snapshot().failure,
            Some((LinkFailure::no_answer(LinkReason::Http5xx(503)), 10))
        );
        assert_eq!(m.snapshot().last_ok_at, None, "a 503 is not an answer");
        for ok in [
            reqwest::StatusCode::OK,
            reqwest::StatusCode::UNAUTHORIZED,
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            reqwest::StatusCode::NOT_FOUND,
        ] {
            let m = LinkMonitor::new();
            m.note_failure(LinkFailure::offline(), 1);
            m.note_status(ok, 2);
            assert_eq!(m.snapshot().failure, None, "{ok}");
            assert_eq!(m.snapshot().last_ok_at, Some(2), "{ok}");
        }
    }

    #[test]
    fn a_failure_kind_maps_to_the_phase_connectivity() {
        use crate::surfaces::phase::Connectivity;
        assert_eq!(Connectivity::from(LinkFailureKind::Offline), Connectivity::Offline);
        assert_eq!(
            Connectivity::from(LinkFailureKind::ServerDidNotAnswer),
            Connectivity::ServerDidNotAnswer
        );
    }
}
