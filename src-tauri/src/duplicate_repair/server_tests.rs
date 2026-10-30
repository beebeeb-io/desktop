//! Task 2274, plan Task 3: the server read (spec phase 1b, rows L4-L9, the L8 amendment) against a loopback server
//! that records every request line. Spec I3/T14: the read sends only GETs, and the invites list is read once a run.
use super::model::{Class, Pair, RowFacts, ServerFile, ViewFacts};
use super::server::{RepairApi, server_read};
use crate::api_client::ApiClient;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// A loopback HTTP server: answers each `METHOD /path?query` it was given, 404 to anything else, and records every
/// request line in order. A request listed more than once gets its answers in order, the last one from then on (a
/// file read before and after a trash).
pub(super) struct RepairMockServer {
    pub base_url: String,
    requests: Arc<Mutex<Vec<String>>>,
    shutdown: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl RepairMockServer {
    pub fn start(routes: Vec<(String, u16, Vec<u8>)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let (seen, stop) = (Arc::clone(&requests), Arc::clone(&shutdown));
        let handle = thread::spawn(move || {
            let started = std::time::Instant::now();
            while !stop.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(30) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                stream.set_nonblocking(false).unwrap();
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap() == 1 {
                    head.push(byte[0]);
                }
                let line = String::from_utf8_lossy(&head).lines().next().unwrap_or_default().to_string();
                let request = line.rsplit_once(' ').map_or(line.clone(), |(request, _)| request.to_string());
                let mut seen = seen.lock().unwrap();
                let asked = seen.iter().filter(|line| **line == request).count();
                let answers: Vec<_> = routes.iter().filter(|(route, _, _)| *route == request).collect();
                let (status, body) = answers
                    .get(asked.min(answers.len().saturating_sub(1)))
                    .map_or((404, br#"{"error":"not found"}"#.to_vec()), |(_, status, body)| (*status, body.clone()));
                seen.push(request);
                drop(seen);
                let head = format!("HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(&body));
            }
        });
        Self { base_url, requests, shutdown, handle: Some(handle) }
    }

    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for RepairMockServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub(super) fn get(path: &str, status: u16, body: String) -> (String, u16, Vec<u8>) {
    (format!("GET {path}"), status, body.into_bytes())
}

pub(super) fn file(parent: &str, size: i64, trashed: bool, version: i64) -> String {
    serde_json::json!({
        "id": "ignored",
        "parent_id": parent,
        "size_bytes": size,
        "is_trashed": trashed,
        "trashed_at": if trashed { serde_json::json!("2026-10-29T10:00:00+00:00") } else { serde_json::Value::Null },
        "updated_at": "2026-10-29T09:00:00+00:00",
        "chunk_count": 1,
        "download_version": { "version_number": version },
        "acted_via_share_id": null,
        "is_folder": false,
    })
    .to_string()
}

pub(super) fn links(expires: &[Option<&str>]) -> String {
    let shares: Vec<_> = expires.iter().map(|at| serde_json::json!({ "id": "s", "expires_at": at })).collect();
    serde_json::json!({ "shares": shares, "count": shares.len() }).to_string()
}

pub(super) fn invites(of: &[(&str, &str, Option<&str>)]) -> String {
    let rows: Vec<_> = of
        .iter()
        .map(|(file_id, status, expires)| {
            serde_json::json!({ "id": "i", "file_id": file_id, "status": status, "expires_at": expires,
                                "recipient_email": "someone@example.org" })
        })
        .collect();
    serde_json::json!({ "invites": rows }).to_string()
}

pub(super) fn row(id: &str) -> RowFacts {
    RowFacts {
        file_id: id.into(),
        status: "local".into(),
        size_bytes: 10,
        remote_updated_at: 1,
        local_hash: Some("h".into()),
        current_version: 1,
        shared: false,
    }
}

pub(super) fn pair(n: u32, a: &str, b: &str) -> Pair {
    Pair {
        n,
        path: format!("p{n}.txt"),
        a: row(a),
        b: row(b),
        view: ViewFacts {
            kind: "placeholder".into(),
            on_disk: Some("wholly".into()),
            in_sync: Some(true),
            identity_names: Some(a.into()),
            size: Some(10),
            modified: Some(1),
            read_only: Some(true),
        },
        class: Class::Unknown,
        server: None,
        action: super::model::Action::None,
        applied: None,
        seal: String::new(),
    }
}

async fn read(mock: &RepairMockServer, pairs: &mut [Pair]) -> anyhow::Result<()> {
    server_read(&RepairApi::new(ApiClient::new(mock.base_url.clone(), "test-token".into(), [0; 32])), pairs).await
}

/// L4: A missing or trashed on the server. B is not read.
#[tokio::test]
async fn a_missing_or_trashed_a_is_a_gone() {
    let mock = RepairMockServer::start(vec![
        get("/api/v1/files/a2", 200, file("p", 10, true, 1)),
        get("/api/v1/shares/invites/sent", 200, invites(&[])),
    ]);
    let mut pairs = vec![pair(1, "a1", "b1"), pair(2, "a2", "b2")];
    read(&mock, &mut pairs).await.unwrap();
    assert_eq!((pairs[0].class, pairs[1].class), (Class::AGone, Class::AGone));
    assert_eq!(pairs[0].server.as_ref().unwrap().a, ServerFile::Gone);
    assert!(pairs[1].server.as_ref().unwrap().b.is_none(), "B is not read once A is gone");
    assert!(!mock.requests().iter().any(|line| line.contains("/b1") || line.contains("/b2")), "{:?}", mock.requests());
}

/// L5 and L6.
#[tokio::test]
async fn a_missing_b_is_b_gone_and_a_trashed_b_is_in_the_trash() {
    let mock = RepairMockServer::start(vec![
        get("/api/v1/files/a1", 200, file("p", 10, false, 1)),
        get("/api/v1/files/a2", 200, file("p", 10, false, 1)),
        get("/api/v1/files/b2", 200, file("p", 10, true, 1)),
        get("/api/v1/shares/invites/sent", 200, invites(&[])),
    ]);
    let mut pairs = vec![pair(1, "a1", "b1"), pair(2, "a2", "b2")];
    read(&mock, &mut pairs).await.unwrap();
    assert_eq!((pairs[0].class, pairs[1].class), (Class::BGone, Class::BInTrash));
    assert_eq!(pairs[0].server.as_ref().unwrap().b, Some(ServerFile::Gone));
}

/// L7.
#[tokio::test]
async fn b_under_another_parent_is_moved() {
    let mock = RepairMockServer::start(vec![
        get("/api/v1/files/a1", 200, file("p", 10, false, 1)),
        get("/api/v1/files/b1", 200, file("q", 10, false, 1)),
        get("/api/v1/shares/invites/sent", 200, invites(&[])),
    ]);
    let mut pairs = vec![pair(1, "a1", "b1")];
    read(&mock, &mut pairs).await.unwrap();
    assert_eq!(pairs[0].class, Class::Moved);
}

/// L8 (amended): a share on the PC, a share the server names, a live link, a live invite, or a second version.
#[tokio::test]
async fn a_shared_or_versioned_b_is_your_call() {
    let mut routes = vec![get("/api/v1/shares/invites/sent", 200, invites(&[
        ("b4", "approved", None),
        ("b5", "withdrawn", None),
        ("b6", "invited", Some("2020-01-01T00:00:00+00:00")),
        ("elsewhere", "approved", None),
    ]))];
    for n in 1..=8 {
        routes.push(get(&format!("/api/v1/files/a{n}"), 200, file("p", 10, false, 1)));
        let version = if n == 3 { 2 } else { 1 };
        let mut b: serde_json::Value = serde_json::from_str(&file("p", 10, false, version)).unwrap();
        if n == 2 {
            b["acted_via_share_id"] = serde_json::json!("share-1");
        }
        routes.push(get(&format!("/api/v1/files/b{n}"), 200, b.to_string()));
        let link: &[Option<&str>] = match n {
            7 => &[None],
            8 => &[Some("2020-01-01T00:00:00+00:00")],
            _ => &[],
        };
        routes.push(get(&format!("/api/v1/shares/by-file/b{n}"), 200, links(link)));
    }
    let mock = RepairMockServer::start(routes);
    let mut pairs: Vec<Pair> = (1..=8).map(|n| pair(n, &format!("a{n}"), &format!("b{n}"))).collect();
    pairs[0].b.shared = true;
    read(&mock, &mut pairs).await.unwrap();
    let classes: Vec<Class> = pairs.iter().map(|pair| pair.class).collect();
    use Class::{Unknown, YourCall};
    // 1 local share, 2 via a share, 3 a second version, 4 a live invite, 5 a withdrawn invite, 6 an expired invite,
    // 7 a live link, 8 an expired link.
    assert_eq!(classes, [YourCall, YourCall, YourCall, YourCall, Unknown, Unknown, YourCall, Unknown]);
    let counts: Vec<(Option<usize>, Option<usize>)> =
        pairs.iter().map(|pair| (pair.server.as_ref().unwrap().live_links, pair.server.as_ref().unwrap().live_invites)).collect();
    assert_eq!(counts[3], (Some(0), Some(1)));
    assert_eq!(counts[6], (Some(1), Some(0)));
    assert_eq!(counts[7], (Some(0), Some(0)));
    let invites_read = mock.requests().iter().filter(|line| line.contains("/shares/invites/sent")).count();
    assert_eq!(invites_read, 1, "the invites list is read once a run");
}

/// Review P2: `GET /shares/by-file` lists only the owner's links and answers 404 for a B in a folder shared with this
/// account (server `shares.rs` ~1848). A B the read already makes your call (a share on this PC or on the server, a
/// second version) is decided before that call, which is never sent for it, so one such B no longer stops the read.
#[tokio::test]
async fn a_b_already_your_call_is_decided_without_the_owner_only_links_call() {
    let mut routes = vec![get("/api/v1/shares/invites/sent", 200, invites(&[]))];
    for n in 1..=3 {
        routes.push(get(&format!("/api/v1/files/a{n}"), 200, file("p", 10, false, 1)));
        let mut b: serde_json::Value = serde_json::from_str(&file("p", 10, false, if n == 3 { 2 } else { 1 })).unwrap();
        if n == 2 {
            b["acted_via_share_id"] = serde_json::json!("share-1");
        }
        routes.push(get(&format!("/api/v1/files/b{n}"), 200, b.to_string()));
    }
    let mock = RepairMockServer::start(routes);
    let mut pairs: Vec<Pair> = (1..=3).map(|n| pair(n, &format!("a{n}"), &format!("b{n}"))).collect();
    pairs[0].b.shared = true;
    read(&mock, &mut pairs).await.unwrap();
    assert!(pairs.iter().all(|pair| pair.class == Class::YourCall), "{:?}", pairs.iter().map(|p| p.class).collect::<Vec<_>>());
    assert!(!mock.requests().iter().any(|line| line.contains("/shares/by-file/")), "{:?}", mock.requests());
    assert!(pairs.iter().all(|pair| pair.server.as_ref().unwrap().live_links.is_none()));
}

/// Review P2: A or B a folder on the server is never a file pair, and no action is allowed for it: a trash of a folder
/// trashes everything under it. Neither the links nor the invites are read for it.
#[tokio::test]
async fn a_folder_on_the_server_is_not_a_file_pair() {
    let folder = || {
        let mut folder: serde_json::Value = serde_json::from_str(&file("p", 0, false, 0)).unwrap();
        folder["is_folder"] = serde_json::json!(true);
        folder["chunk_count"] = serde_json::json!(0);
        folder.to_string()
    };
    let mock = RepairMockServer::start(vec![
        get("/api/v1/files/a1", 200, folder()),
        get("/api/v1/files/b1", 200, file("p", 0, false, 1)),
        get("/api/v1/files/a2", 200, file("p", 0, false, 1)),
        get("/api/v1/files/b2", 200, folder()),
        get("/api/v1/shares/by-file/b1", 200, links(&[])),
        get("/api/v1/shares/by-file/b2", 200, links(&[])),
        get("/api/v1/shares/invites/sent", 200, invites(&[])),
    ]);
    let mut pairs = vec![pair(1, "a1", "b1"), pair(2, "a2", "b2")];
    read(&mock, &mut pairs).await.unwrap();
    assert_eq!((pairs[0].class, pairs[1].class), (Class::NotAFile, Class::NotAFile));
    use super::model::Action;
    assert!(!Class::NotAFile.allows(Action::TrashB) && !Class::NotAFile.allows(Action::ForgetB));
    assert_eq!(Class::NotAFile.proposed(), Action::None);
    assert!(!mock.requests().iter().any(|line| line.contains("/shares/")), "{:?}", mock.requests());
}

/// L9, and L10 kept for the compare: equal size stays unknown, with the server's facts recorded.
#[tokio::test]
async fn sizes_that_differ_are_listed_and_equal_sizes_wait_for_the_compare() {
    let mock = RepairMockServer::start(vec![
        get("/api/v1/files/a1", 200, file("p", 10, false, 1)),
        get("/api/v1/files/b1", 200, file("p", 11, false, 1)),
        get("/api/v1/files/a2", 200, file("p", 10, false, 1)),
        get("/api/v1/files/b2", 200, file("p", 10, false, 1)),
        get("/api/v1/shares/by-file/b1", 200, links(&[])),
        get("/api/v1/shares/by-file/b2", 200, links(&[])),
        get("/api/v1/shares/invites/sent", 200, invites(&[])),
    ]);
    let mut pairs = vec![pair(1, "a1", "b1"), pair(2, "a2", "b2")];
    read(&mock, &mut pairs).await.unwrap();
    assert_eq!((pairs[0].class, pairs[1].class), (Class::DiffersBySize, Class::Unknown));
    let ServerFile::Found(b) = pairs[1].server.as_ref().unwrap().b.clone().unwrap() else { panic!("B found") };
    assert_eq!((b.size_bytes, b.version_number, b.parent_id.as_deref()), (10, 1, Some("p")));
    assert_eq!(b.updated_at, "2026-10-29T09:00:00+00:00");
}

/// Rows L1-L3b decided locally: no server call for those pairs. Every call is a GET (I3).
#[tokio::test]
async fn only_pairs_the_local_rows_passed_are_read_and_only_with_gets() {
    let mock = RepairMockServer::start(vec![
        get("/api/v1/files/a1", 200, file("p", 10, false, 1)),
        get("/api/v1/files/b1", 200, file("p", 10, false, 1)),
        get("/api/v1/shares/by-file/b1", 200, links(&[])),
        get("/api/v1/shares/invites/sent", 200, invites(&[])),
    ]);
    let mut pairs = vec![pair(1, "a1", "b1"), pair(2, "a2", "b2"), pair(3, "a3", "b3")];
    pairs[1].class = Class::ABusy;
    pairs[2].class = Class::Unbound;
    read(&mock, &mut pairs).await.unwrap();
    assert_eq!((pairs[1].class, pairs[2].class), (Class::ABusy, Class::Unbound));
    assert!(pairs[1].server.is_none() && pairs[2].server.is_none());
    let requests = mock.requests();
    assert!(requests.iter().all(|line| line.starts_with("GET ")), "{requests:?}");
    assert!(!requests.iter().any(|line| line.contains("a2") || line.contains("a3")), "{requests:?}");
}

/// A server error stops the run. The error names the pair and the status, never an id or the address (I5).
#[tokio::test]
async fn a_server_error_stops_the_run_without_naming_an_id() {
    let mock = RepairMockServer::start(vec![
        get("/api/v1/files/a-secret-id", 200, file("p", 10, false, 1)),
        get("/api/v1/files/b-secret-id", 500, r#"{"error":"boom"}"#.into()),
        get("/api/v1/shares/invites/sent", 200, invites(&[])),
    ]);
    let mut pairs = vec![pair(1, "a-secret-id", "b-secret-id")];
    let error = format!("{:#}", read(&mock, &mut pairs).await.unwrap_err());
    assert!(error.contains("pair 1") && error.contains("500"), "{error}");
    assert!(!error.contains("secret-id") && !error.contains("127.0.0.1"), "{error}");
}
