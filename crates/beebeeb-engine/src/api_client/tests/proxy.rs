//! The proxy rule of the clients for the API base (`api_client::proxy`, #335 review P2-1): a plain-http base on this
//! computer skips every proxy, any other base keeps what it was given. Each client here is built with an explicit
//! proxy, a loopback listener that answers 502, so where a request went shows whether the proxy was kept. The
//! environment is never read or changed (an explicit proxy turns reqwest's own proxy lookup off).

use std::io::{Read, Write as _};
use std::net::TcpListener;
use std::sync::mpsc;

use super::*;
use crate::api_client::proxy::{is_plain_http_loopback, with_api_base_proxy_rule};

/// A loopback listener that answers its first connection with `status` and sends back the request's first line.
fn listener(status: &'static str) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (first_line, read) = mpsc::channel();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n])
                .lines()
                .next()
                .unwrap_or_default()
                .to_string();
            let _ = first_line.send(head);
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
        }
    });
    (address, read)
}

/// A client for `api_base` with the rule applied over an explicit proxy at `proxy`.
fn client_over_proxy(api_base: &str, proxy: &str) -> Client {
    let builder = Client::builder()
        .proxy(reqwest::Proxy::all(format!("http://{proxy}")).unwrap())
        .timeout(Duration::from_secs(10));
    with_api_base_proxy_rule(builder, api_base).build().unwrap()
}

#[test]
fn a_plain_http_loopback_base_is_reached_directly_never_through_the_proxy() {
    let (server, server_saw) = listener("200 OK");
    let (proxy, proxy_saw) = listener("502 Bad Gateway");
    let base = format!("http://{server}");
    let client = client_over_proxy(&base, &proxy);
    let status = run_async(async {
        client
            .get(format!("{base}/api/v1/probe"))
            .send()
            .await
            .map(|r| r.status())
    });
    assert_eq!(
        status.map(|s| s.as_u16()).ok(),
        Some(200),
        "the server answered, not the proxy"
    );
    assert_eq!(
        server_saw.recv_timeout(Duration::from_secs(5)).unwrap(),
        "GET /api/v1/probe HTTP/1.1"
    );
    assert!(proxy_saw.try_recv().is_err(), "nothing reached the proxy");
}

#[test]
fn any_other_base_keeps_its_proxy() {
    // Plain http elsewhere: the proxy is asked for the whole URL (the host is never resolved here).
    let (proxy, proxy_saw) = listener("502 Bad Gateway");
    let client = client_over_proxy("http://api.example.invalid", &proxy);
    let status = run_async(async {
        client
            .get("http://api.example.invalid/api/v1/probe")
            .send()
            .await
            .map(|r| r.status())
    });
    assert_eq!(status.map(|s| s.as_u16()).ok(), Some(502), "the proxy answered");
    assert_eq!(
        proxy_saw.recv_timeout(Duration::from_secs(5)).unwrap(),
        "GET http://api.example.invalid/api/v1/probe HTTP/1.1"
    );
    // https, elsewhere and on this computer: a tunnel through the proxy, as before.
    for (base, tunnel) in [
        (
            "https://api.example.invalid",
            "CONNECT api.example.invalid:443 HTTP/1.1",
        ),
        ("https://127.0.0.1:8443", "CONNECT 127.0.0.1:8443 HTTP/1.1"),
    ] {
        let (proxy, proxy_saw) = listener("502 Bad Gateway");
        let client = client_over_proxy(base, &proxy);
        let sent = run_async(async { client.get(format!("{base}/api/v1/probe")).send().await });
        assert!(sent.is_err(), "{base}: the proxy refused the tunnel");
        assert_eq!(
            proxy_saw.recv_timeout(Duration::from_secs(5)).unwrap(),
            tunnel,
            "{base}"
        );
    }
}

#[test]
fn only_plain_http_to_this_computer_skips_the_proxy() {
    for (base, skips) in [
        ("http://127.0.0.1:9", true),
        ("http://127.0.0.1", true),
        ("http://[::1]:3001", true),
        ("http://localhost:3001", true),
        ("http://LOCALHOST:3001/", true),
        ("https://127.0.0.1:8443", false),
        ("https://localhost", false),
        ("https://api.beebeeb.io", false),
        ("http://api.example.invalid", false),
        ("http://127.0.0.2:9", false),
        ("http://localhost.example.com", false),
        ("http://evil.localhost", false),
        ("not a url", false),
    ] {
        assert_eq!(is_plain_http_loopback(base), skips, "{base}");
    }
}

/// Every client the engine builds for the API base comes from `api_base_client_builder`: the three of `ApiClient`,
/// the auth transport's, and `Engine::open` hands the auth transport its own base.
#[test]
fn every_api_base_client_takes_the_proxy_rule() {
    let squeezed = |text: &str| text.split_whitespace().collect::<String>();
    let client_rs = include_str!("../client.rs");
    let auth = include_str!("../../auth_transport.rs");
    let app = squeezed(include_str!("../../app.rs"));
    assert_eq!(
        client_rs.matches("Client::builder()").count(),
        0,
        "client.rs builds no client of its own"
    );
    assert_eq!(client_rs.matches("api_base_client_builder(&").count(), 3, "client.rs");
    assert_eq!(
        auth.matches("Client::builder()").count(),
        0,
        "auth_transport.rs builds no client of its own"
    );
    assert_eq!(
        auth.matches("api_client::api_base_client_builder(api_base)").count(),
        1,
        "auth_transport.rs"
    );
    assert!(app.contains(&squeezed("let client = auth_transport::client_for(&config.api_base)?;")));
}
