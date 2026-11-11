//! Review test 4: the real `HttpsClient` and the machine against a TLS server on this PC. The certificate and its
//! key are made inside the test; nothing leaves the machine and no key is committed.

use std::cell::Cell;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use crate::machine::{Config, Event, Failure, State, Updater};
use crate::manifest::{Channel, MAX_MANIFEST_BYTES};
use crate::net::HttpsClient;
use crate::policy::Refusal;
use crate::testkit::{
    FakeApply, FakeClock, MemStore, NOW, Publish, TestKey, artifact_bytes, artifact_name, trust, version,
};

const SIZE: usize = 200_000;

/// What the server sends for one request: the whole response, head included.
type Route = Box<dyn Fn(&str, Option<u64>) -> Vec<u8> + Send>;

/// Every request: path and the `Range` start, if any.
type RequestLog = Arc<Mutex<Vec<(String, Option<u64>)>>>;

struct Server {
    port: u16,
    certificate_pem: String,
    log: RequestLog,
}

fn respond(status: &str, headers: &[(&str, String)], body: &[u8]) -> Vec<u8> {
    let mut head = format!("HTTP/1.1 {status}\r\nConnection: close\r\n");
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    [head.as_bytes(), b"\r\n", body].concat()
}

fn ok(body: &[u8]) -> Vec<u8> {
    respond("200 OK", &[("Content-Length", body.len().to_string())], body)
}

impl Server {
    fn start(route: Route) -> Self {
        let made = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).expect("a test certificate");
        let certificate_pem = made.cert.pem();
        let identity =
            native_tls::Identity::from_pkcs8(certificate_pem.as_bytes(), made.key_pair.serialize_pem().as_bytes())
                .expect("a test identity");
        let acceptor = native_tls::TlsAcceptor::new(identity).expect("a TLS acceptor");
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("a local port");
        let port = listener.local_addr().expect("address").port();
        let log: RequestLog = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&log);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let Ok(mut tls) = acceptor.accept(stream) else { continue };
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") && tls.read(&mut byte).is_ok_and(|read| read == 1) {
                    head.push(byte[0]);
                }
                let head = String::from_utf8_lossy(&head).into_owned();
                let path = head.split(' ').nth(1).unwrap_or_default().to_string();
                let range = head
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("range: bytes=")
                            .map(str::to_string)
                    })
                    .and_then(|range| range.trim_end_matches('-').parse().ok());
                seen.lock().expect("log").push((path.clone(), range));
                let _ = tls.write_all(&route(&path, range));
                let _ = tls.shutdown();
            }
        });
        Self {
            port,
            certificate_pem,
            log,
        }
    }

    fn base(&self) -> String {
        format!("https://localhost:{}", self.port)
    }

    fn paths(&self) -> Vec<String> {
        self.log
            .lock()
            .expect("log")
            .iter()
            .map(|(path, _)| path.clone())
            .collect()
    }

    /// An install of 0.9.0 whose one trusted host is this server.
    fn updater(
        &self,
        key: &TestKey,
        staging: &std::path::Path,
    ) -> Updater<HttpsClient, FakeClock, MemStore, FakeApply> {
        let mut trust = trust(key);
        trust.allowed_hosts = vec![format!("localhost:{}", self.port)];
        let config = Config {
            trust,
            base_url: self.base(),
            installed: version("0.9.0"),
            staging_dir: staging.to_path_buf(),
        };
        Updater::new(
            config,
            HttpsClient::trusting(self.certificate_pem.as_bytes()),
            FakeClock(Cell::new(NOW)),
            MemStore::default(),
            FakeApply::default(),
        )
    }
}

fn settle(updater: &mut Updater<HttpsClient, FakeClock, MemStore, FakeApply>, event: Event) -> State {
    let mut state = updater.step(event).clone();
    for _ in 0..10_000 {
        if !matches!(
            state,
            State::Checking { .. } | State::Downloading { .. } | State::Verifying { .. }
        ) {
            return state;
        }
        state = updater.step(Event::Tick).clone();
    }
    panic!("the updater did not settle");
}

const CHECK: Event = Event::Check {
    channel: Channel::Stable,
    intent: crate::machine::Intent::Manual,
};

/// One honest release of 0.9.1 at this server's own address, and how the test wants each path answered.
struct Release {
    manifest: Vec<u8>,
    signature: String,
    artifact: Vec<u8>,
}

fn release(key: &TestKey, base: &str) -> Release {
    let mut publish = Publish::new(Channel::Stable, "0.9.1", artifact_bytes(SIZE));
    publish.url = format!("{base}/desktop/artifacts/0.9.1/{}", artifact_name("0.9.1"));
    let (manifest, signature) = publish.manifest(key);
    Release {
        manifest,
        signature,
        artifact: publish.artifact,
    }
}

/// Starts a server that knows its own port (the manifest must name it) and answers through `route`.
fn serve(key: &TestKey, route: impl Fn(&Release, &str, Option<u64>) -> Vec<u8> + Send + 'static) -> Server {
    // The port is only known once the listener exists, and the manifest must carry it: the release is built on
    // the first request.
    let port = Arc::new(Mutex::new(0u16));
    let (port_for_route, key_text) = (Arc::clone(&port), key.clone_for_thread());
    let made: Mutex<Option<Release>> = Mutex::new(None);
    let server = Server::start(Box::new(move |path, range| {
        let mut made = made.lock().expect("release");
        let base = format!("https://localhost:{}", *port_for_route.lock().expect("port"));
        route(made.get_or_insert_with(|| release(&key_text, &base)), path, range)
    }));
    *port.lock().expect("port") = server.port;
    server
}

fn honest(release: &Release, path: &str, range: Option<u64>) -> Vec<u8> {
    match (path, range) {
        ("/desktop/v2/stable.json", _) => ok(&release.manifest),
        ("/desktop/v2/stable.json.sig", _) => ok(release.signature.as_bytes()),
        (path, None) if path.ends_with(".exe") => ok(&release.artifact),
        (path, Some(from)) if path.ends_with(".exe") => {
            let (from, total) = (from as usize, release.artifact.len());
            respond(
                "206 Partial Content",
                &[
                    ("Content-Length", (total - from).to_string()),
                    ("Content-Range", format!("bytes {from}-{}/{total}", total - 1)),
                ],
                &release.artifact[from..],
            )
        }
        _ => respond("404 Not Found", &[("Content-Length", "0".to_string())], b""),
    }
}

#[test]
fn the_real_client_downloads_and_resumes_over_tls() {
    let key = TestKey::generate();
    let server = serve(&key, honest);
    let dir = tempfile::tempdir().expect("temp dir");
    let mut updater = server.updater(&key, dir.path());
    assert!(matches!(settle(&mut updater, CHECK), State::Available { .. }));
    // 5000 bytes are already on disk: the client asks for the rest and the server answers 206 from there.
    fs::write(
        dir.path().join(format!("{}.part", artifact_name("0.9.1"))),
        &artifact_bytes(SIZE)[..5_000],
    )
    .expect("part");
    let State::ReadyToRestart { installer, .. } = settle(&mut updater, Event::Download) else {
        panic!("expected a staged update")
    };
    assert_eq!(fs::read(installer).expect("staged"), artifact_bytes(SIZE));
    assert_eq!(
        server.log.lock().expect("log").last().expect("a request").1,
        Some(5_000)
    );
}

#[test]
fn a_redirect_is_not_followed() {
    let key = TestKey::generate();
    let server = serve(&key, |release, path, range| match path {
        // Whoever follows this gets an honest manifest: only not following it keeps the test green.
        "/desktop/v2/stable.json" => respond(
            "302 Found",
            &[
                ("Location", "/elsewhere/stable.json".to_string()),
                ("Content-Length", "0".to_string()),
            ],
            b"",
        ),
        "/elsewhere/stable.json" => ok(&release.manifest),
        _ => honest(release, path, range),
    });
    let dir = tempfile::tempdir().expect("temp dir");
    let mut updater = server.updater(&key, dir.path());
    assert_eq!(
        settle(&mut updater, CHECK),
        State::Failed {
            failure: Failure::ServerStatus(302),
            resume: None
        }
    );
    assert_eq!(
        server.paths(),
        ["/desktop/v2/stable.json"],
        "the redirect target was never asked"
    );
}

#[test]
fn a_range_answer_without_content_range_is_refused() {
    let key = TestKey::generate();
    let server = serve(&key, |release, path, range| match range {
        // The rest of the file, honestly, but without saying where it starts.
        Some(from) if path.ends_with(".exe") => respond(
            "206 Partial Content",
            &[("Content-Length", (release.artifact.len() - from as usize).to_string())],
            &release.artifact[from as usize..],
        ),
        _ => honest(release, path, range),
    });
    let dir = tempfile::tempdir().expect("temp dir");
    let mut updater = server.updater(&key, dir.path());
    assert!(matches!(settle(&mut updater, CHECK), State::Available { .. }));
    let part = dir.path().join(format!("{}.part", artifact_name("0.9.1")));
    fs::write(&part, &artifact_bytes(SIZE)[..5_000]).expect("part");
    assert!(matches!(
        settle(&mut updater, Event::Download),
        State::Failed {
            failure: Failure::ServerStatus(206),
            resume: Some(_)
        }
    ));
    assert_eq!(
        fs::metadata(&part).expect("part").len(),
        5_000,
        "not a byte was appended"
    );
}

#[test]
fn the_size_caps_hold_against_a_server_that_sends_more() {
    let key = TestKey::generate();
    // A manifest that never ends (2 MiB of it): refused at the cap, not read to the end.
    let server = serve(&key, |release, path, range| match path {
        "/desktop/v2/stable.json" => respond("200 OK", &[], &vec![b' '; 2 * MAX_MANIFEST_BYTES]),
        _ => honest(release, path, range),
    });
    let dir = tempfile::tempdir().expect("temp dir");
    let mut updater = server.updater(&key, dir.path());
    assert_eq!(
        settle(&mut updater, CHECK),
        State::Failed {
            failure: Failure::Refused(Refusal::ManifestUnreadable),
            resume: None
        }
    );

    // An installer followed by more bytes, with no stated length: only the manifest's length is read and staged.
    let server = serve(&key, |release, path, range| match path {
        path if path.ends_with(".exe") => {
            respond("200 OK", &[], &[&release.artifact[..], &[0xAA; 50_000][..]].concat())
        }
        _ => honest(release, path, range),
    });
    let dir = tempfile::tempdir().expect("temp dir");
    let mut updater = server.updater(&key, dir.path());
    assert!(matches!(settle(&mut updater, CHECK), State::Available { .. }));
    let State::ReadyToRestart { installer, .. } = settle(&mut updater, Event::Download) else {
        panic!("expected a staged update")
    };
    assert_eq!(fs::metadata(installer).expect("staged").len(), SIZE as u64);

    // The same with a stated length that is not the manifest's: refused before a byte is kept.
    let server = serve(&key, |release, path, range| match path {
        path if path.ends_with(".exe") => ok(&[&release.artifact[..], &[0xAA; 50_000][..]].concat()),
        _ => honest(release, path, range),
    });
    let dir = tempfile::tempdir().expect("temp dir");
    let mut updater = server.updater(&key, dir.path());
    assert!(matches!(settle(&mut updater, CHECK), State::Available { .. }));
    assert_eq!(
        settle(&mut updater, Event::Download),
        State::Failed {
            failure: Failure::WrongSize,
            resume: None
        }
    );
    assert!(fs::read_dir(dir.path()).expect("staging").next().is_none());
}
