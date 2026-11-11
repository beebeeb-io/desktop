//! Tests of `flow`: which answers of the signed feed's address fall back to the unsigned feed (a 404 for the
//! manifest, nothing else), the record written before an install, and the hand-off note. The feed is a local
//! server; the keys are made in the test.

use super::flow::*;
use super::trust::TrustedKeys;
use super::trust_tests::kit::{SIGNED_AT, TestKey, URL_090, manifest};
use super::*;
use std::io::{Read as _, Write as _};
use std::sync::{Arc, Mutex};

/// Serves `routes` (path, status, body) and 404 for anything else; records the paths asked for.
fn feed_server(routes: Vec<(&'static str, u16, Vec<u8>)>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let asked = Arc::new(Mutex::new(Vec::new()));
    let seen = asked.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]).to_string();
            let path = head.split_whitespace().nth(1).unwrap_or_default().to_string();
            let (status, body) = routes
                .iter()
                .find(|(route, _, _)| *route == path)
                .map_or((404, b"not found".to_vec()), |(_, status, body)| {
                    (*status, body.clone())
                });
            seen.lock().unwrap().push(path);
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
        }
    });
    (base, asked)
}

struct World {
    old: TestKey,
    next: TestKey,
    keys: TrustedKeys,
    good: Vec<u8>,
}

fn world() -> World {
    let (old, next) = (TestKey::generate(), TestKey::generate());
    let keys = TrustedKeys::new(&old.tauri_pubkey(), &next.tauri_pubkey()).unwrap();
    let artifact = next.sign(b"installer", "Beebeeb_0.9.0_x64-setup.exe");
    let good = manifest("0.9.0", "desktop", "stable", URL_090, &artifact, Some(9_000_001));
    World { old, next, keys, good }
}

async fn read(
    world: &World,
    windows: bool,
    routes: Vec<(&'static str, u16, Vec<u8>)>,
) -> (Result<Option<String>, String>, usize) {
    let (base, asked) = feed_server(routes);
    let result = signed_manifest_for(windows, &base, &world.keys, ReleaseChannel::Stable, SIGNED_AT, None)
        .await
        .map(|signed| signed.map(|manifest| manifest.version));
    let count = asked.lock().unwrap().len();
    (result, count)
}

const MANIFEST: &str = "/desktop/v2/stable.json";
const SIGNATURE: &str = "/desktop/v2/stable.json.sig";

#[tokio::test]
async fn only_a_missing_v2_manifest_falls_back_to_the_unsigned_feed() {
    let w = world();
    let sig = |bytes: &[u8], file: &str| w.next.sign(bytes, file).into_bytes();
    // 404 for the manifest: no native release yet. The one fallback.
    assert_eq!(read(&w, true, vec![]).await, (Ok(None), 1));
    // Present and signed for this channel.
    let good = vec![
        (MANIFEST, 200, w.good.clone()),
        (SIGNATURE, 200, sig(&w.good, "stable.json")),
    ];
    assert_eq!(read(&w, true, good).await, (Ok(Some("0.9.0".into())), 2));
    // Windows only: every other host keeps today's feed and asks nothing.
    assert_eq!(
        read(&w, false, vec![(MANIFEST, 200, w.good.clone())]).await,
        (Ok(None), 0)
    );

    // Everything else is an error, never the unsigned feed.
    let stranger = TestKey::generate();
    let other_product = manifest("0.9.0", "pass", "stable", URL_090, "x", Some(1));
    let hard: Vec<(&str, Vec<(&'static str, u16, Vec<u8>)>, &str)> = vec![
        (
            "a signature by another key",
            vec![
                (MANIFEST, 200, w.good.clone()),
                (SIGNATURE, 200, stranger.sign(&w.good, "stable.json").into_bytes()),
            ],
            "update_manifest_not_signed",
        ),
        (
            "a signature by the legacy key",
            vec![
                (MANIFEST, 200, w.good.clone()),
                (SIGNATURE, 200, w.old.sign(&w.good, "stable.json").into_bytes()),
            ],
            "update_key_not_for_this_line",
        ),
        (
            "a signature of other bytes",
            vec![
                (MANIFEST, 200, w.good.clone()),
                (SIGNATURE, 200, sig(b"other", "stable.json")),
            ],
            "update_manifest_not_signed",
        ),
        (
            "no signature file",
            vec![(MANIFEST, 200, w.good.clone())],
            "update_manifest_not_signed",
        ),
        (
            "an error page as the signature",
            vec![(MANIFEST, 200, w.good.clone()), (SIGNATURE, 200, b"<html>".to_vec())],
            "update_manifest_not_signed",
        ),
        (
            "another channel's manifest",
            vec![
                (MANIFEST, 200, w.good.clone()),
                (SIGNATURE, 200, sig(&w.good, "beta.json")),
            ],
            "update_manifest_wrong_channel",
        ),
        (
            "another product",
            vec![
                (MANIFEST, 200, other_product.clone()),
                (SIGNATURE, 200, sig(&other_product, "stable.json")),
            ],
            "update_manifest_wrong_channel",
        ),
        (
            "a server error",
            vec![(MANIFEST, 500, Vec::new())],
            "update_manifest_unavailable",
        ),
        (
            "a redirect",
            vec![(MANIFEST, 302, Vec::new())],
            "update_manifest_unavailable",
        ),
        (
            "a forbidden",
            vec![(MANIFEST, 403, Vec::new())],
            "update_manifest_unavailable",
        ),
    ];
    for (what, routes, code) in hard {
        let (result, _) = read(&w, true, routes).await;
        let error = result.expect_err(what);
        assert!(error.starts_with(&format!("{code}: ")), "{what}: {error}");
    }
    // A host that does not answer at all.
    let refused = signed_manifest_for(
        true,
        "http://127.0.0.1:1",
        &w.keys,
        ReleaseChannel::Stable,
        SIGNED_AT,
        None,
    )
    .await;
    assert!(refused.is_err_and(|error| error.starts_with("update_manifest_unavailable: ")));
}

#[test]
fn the_updater_target_is_os_and_arch_in_the_updater_s_names() {
    let rows = [
        ("windows", "x86_64", "windows-x86_64"),
        ("windows", "aarch64", "windows-aarch64"),
        ("windows", "x86", "windows-i686"),
        ("linux", "arm", "linux-armv7"),
        ("macos", "aarch64", "darwin-aarch64"),
    ];
    for (os, arch, want) in rows {
        assert_eq!(updater_target(os, arch), want);
    }
}

#[test]
fn the_v2_manifest_addresses_are_one_per_channel() {
    let urls: Vec<_> = [ReleaseChannel::Stable, ReleaseChannel::Beta, ReleaseChannel::Alpha]
        .into_iter()
        .map(|channel| v2_manifest_url(UPDATE_MANIFEST_BASE_URL, channel))
        .collect();
    assert_eq!(
        urls,
        [
            "https://releases.beebeeb.io/desktop/v2/stable.json",
            "https://releases.beebeeb.io/desktop/v2/beta.json",
            "https://releases.beebeeb.io/desktop/v2/alpha.json",
        ]
    );
}

#[test]
fn the_record_is_written_before_the_install_and_taken_back_when_it_fails() {
    let log = Mutex::new(Vec::new());
    let note = |what: &'static str| log.lock().unwrap().push(what);
    let ok = record_then_install(
        || Ok(note("record")),
        || note("take back"),
        || Ok::<(), String>(note("install")),
    );
    assert_eq!(ok, Ok(()));
    assert_eq!(
        *log.lock().unwrap(),
        ["record", "install"],
        "the install may never return: the record comes first"
    );
    log.lock().unwrap().clear();
    let failed = record_then_install(
        || Ok(note("record")),
        || note("take back"),
        || {
            note("install");
            Err("installer refused")
        },
    );
    assert_eq!(failed, Err("installer refused".to_string()));
    assert_eq!(*log.lock().unwrap(), ["record", "install", "take back"]);
    log.lock().unwrap().clear();
    let unrecorded = record_then_install(
        || Err("read-only".to_string()),
        || note("take back"),
        || Ok::<(), String>(note("install")),
    );
    assert_eq!(unrecorded, Err("read-only".to_string()));
    assert!(log.lock().unwrap().is_empty(), "no record, no install");
}

#[test]
fn the_hand_off_note_round_trips_and_leaves_no_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    write_update_handoff(dir.path(), "0.8.25", "0.9.0", ReleaseChannel::Beta, 1_800_000_000).unwrap();
    let note: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join(UPDATE_HANDOFF_FILE)).unwrap()).unwrap();
    assert_eq!(
        note,
        serde_json::json!({"from": "0.8.25", "to": "0.9.0", "channel": "beta", "at": 1_800_000_000u64})
    );
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, [std::ffi::OsString::from(UPDATE_HANDOFF_FILE)]);
}

#[tokio::test]
async fn an_accepted_manifest_becomes_the_replay_mark_and_a_refused_one_never_does() {
    let w = world();
    let dir = tempfile::tempdir().unwrap();
    let channel = ReleaseChannel::Stable;
    let now = SIGNED_AT;
    let serve = |signed_at: u64| {
        let signature = w.next.sign_at(&w.good, "stable.json", &signed_at.to_string());
        vec![
            (MANIFEST, 200, w.good.clone()),
            (SIGNATURE, 200, signature.into_bytes()),
        ]
    };
    let read_at = async |signed_at: u64, now: u64| {
        let (base, _) = feed_server(serve(signed_at));
        signed_manifest_for(true, &base, &w.keys, channel, now, Some(dir.path()))
            .await
            .map(|signed| signed.map(|manifest| manifest.signed_at))
    };
    assert_eq!(replay_mark(dir.path(), channel), None);
    // More than 24 h ahead of this PC's clock: refused, and NOT recorded.
    let ahead = read_at(now + 86_401, now).await.unwrap_err();
    assert!(ahead.starts_with("update_manifest_signed_in_future: "), "{ahead}");
    assert_eq!(
        replay_mark(dir.path(), channel),
        None,
        "a refused manifest leaves no mark"
    );
    // Accepted: recorded.
    assert_eq!(read_at(now, now).await, Ok(Some(now)));
    assert_eq!(replay_mark(dir.path(), channel), Some(now));
    // An older manifest, honestly signed, replayed: refused; the mark stays.
    let stale = read_at(now - 60, now).await.unwrap_err();
    assert!(stale.starts_with("update_manifest_stale: "), "{stale}");
    assert_eq!(replay_mark(dir.path(), channel), Some(now));
    // A newer one moves it forward; the other channels have their own mark.
    assert_eq!(read_at(now + 60, now).await, Ok(Some(now + 60)));
    assert_eq!(replay_mark(dir.path(), channel), Some(now + 60));
    assert_eq!(replay_mark(dir.path(), ReleaseChannel::Beta), None);
    // Stored: the larger of the two, whatever the clock says now.
    record_replay_mark(dir.path(), channel, now - 500).unwrap();
    assert_eq!(replay_mark(dir.path(), channel), Some(now + 60));
    record_replay_mark(dir.path(), ReleaseChannel::Beta, 5).unwrap();
    assert_eq!(
        (
            replay_mark(dir.path(), channel),
            replay_mark(dir.path(), ReleaseChannel::Beta)
        ),
        (Some(now + 60), Some(5))
    );
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, [std::ffi::OsString::from(UPDATE_SEEN_FILE)], "no temp file left");
}

/// The order a native install needs: sync quiet first, then the record and the hand-off note, then the installer.
#[test]
fn a_native_install_makes_sync_quiet_before_the_note_and_the_installer() {
    // A Windows checkout may hold CRLF line endings.
    let source = include_str!("flow.rs").replace("\r\n", "\n");
    let body = &source[source.find("pub(crate) async fn download_record_install(").unwrap()..];
    let at = |needle: &str| body.find(needle).unwrap_or_else(|| panic!("{needle} is gone"));
    let order = [
        at("update.download("),
        at("trust::is_native_line(&to)"),
        at("preflight::quiet_engine("),
        at("record_then_install("),
        at("if trust::is_native_line(&to)\n                && let Some(dir) = handoff_dir.as_deref()"),
        at("write_update_handoff("),
        at("update.install(bytes)"),
    ];
    assert!(order.is_sorted(), "{order:?}");
}
