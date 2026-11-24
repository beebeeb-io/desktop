//! "Sign in with another device" through the real source: one request at a time, nothing of a request after its cancel,
//! the QR code of the address only, and each engine ending drawn as the screen reads it. The engine's request itself
//! (the socket, the countdown, the keys, the install) is tested in the engine (`device_request_tests.rs`). Here the
//! server is a closed port (the request ends at once, offline) or a port that takes the connection and never answers
//! (the request waits). Every name and code is invented.

use std::net::TcpListener;
use std::sync::atomic::AtomicUsize;
use std::time::{Duration, Instant};

use beebeeb_engine::app::EngineConfig;
use beebeeb_engine::app::bindings::device_request::ANOTHER_DEVICE;
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;

use super::*;
use crate::frames::signin::source::SignInSource;

type Source = EngineSignInSource<MemoryCredentials>;

const CODE: &str = "KQ7M-4TXP";
const PAGE: &str = "https://app.example.eu/cli-auth";

/// A source on a fresh signed-out handle whose API base is `base`.
fn source_at(base: &str) -> (Arc<Engine<MemoryCredentials>>, Source) {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "signin-device-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: base.to_string(),
    };
    let engine = Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens"));
    engine.restore();
    let source = EngineSignInSource::new(engine.clone());
    (engine, source)
}

/// An API base nothing listens at: a request to it ends at once, offline.
fn closed_port() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    base
}

/// A server that takes every connection and never answers: a request to it waits. Counts the connections.
struct Silent {
    base: String,
    taken: Arc<AtomicUsize>,
}

fn silent() -> Silent {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let taken = Arc::new(AtomicUsize::new(0));
    let counted = taken.clone();
    thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            counted.fetch_add(1, Ordering::SeqCst);
            // Kept open and never answered.
            held.push(stream);
        }
    });
    Silent { base, taken }
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let began = Instant::now();
    while !done() {
        assert!(
            began.elapsed() < Duration::from_secs(20),
            "timed out waiting for {what}"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

/// M5 (`device_cancel` without its `next_attempt`): an answer of the request is already in the channel when the person
/// leaves; it never reaches the screen. The control first: without the cancel, that same answer is handed out.
#[test]
fn signin_device_cancel_drops_every_later_event() {
    let (_engine, mut control) = source_at(&closed_port());
    control.begin_device_request();
    // The thread clears its seat after its answer is in the channel.
    wait_until("the control's answer is in the channel", || {
        lock(&control.device.0).is_none()
    });
    assert_eq!(
        control.poll(),
        Some(SignInEvent::DeviceRequested(Err(SignInProblem::Offline)))
    );

    let (_engine, mut source) = source_at(&closed_port());
    source.begin_device_request();
    wait_until("the answer is in the channel", || lock(&source.device.0).is_none());
    source.cancel_device_request();
    assert_eq!(
        source.poll(),
        None,
        "no answer of a request that was left reaches the screen"
    );

    // A request that waits: the cancel stops it, and nothing of it arrives after.
    let server = silent();
    let (_engine, mut waiting) = source_at(&server.base);
    waiting.begin_device_request();
    wait_until("the request reached the server", || {
        server.taken.load(Ordering::SeqCst) == 1
    });
    waiting.cancel_device_request();
    thread::sleep(Duration::from_millis(300));
    assert_eq!(waiting.poll(), None);
}

/// One request at a time: a second press while one runs opens no second socket and makes no new attempt. Once the
/// request is left, a press starts the next one.
#[test]
fn signin_device_second_press_starts_nothing() {
    let server = silent();
    let (_engine, mut source) = source_at(&server.base);
    source.begin_device_request();
    wait_until("the first request reached the server", || {
        server.taken.load(Ordering::SeqCst) == 1
    });
    source.begin_device_request();
    thread::sleep(Duration::from_millis(300));
    assert_eq!(server.taken.load(Ordering::SeqCst), 1, "one request, one socket");
    assert_eq!(
        source.attempts[Wait::Device as usize].load(Ordering::SeqCst),
        1,
        "the second press made no new attempt"
    );
    assert_eq!(source.poll(), None, "and said nothing");
    source.cancel_device_request();
    source.begin_device_request();
    wait_until("the next request reached the server", || {
        server.taken.load(Ordering::SeqCst) == 2
    });
    source.cancel_device_request();
}

/// Task 1734: the QR code is the address's, exactly, never one that holds the code; the code itself is shown beside it,
/// with this PC's name as the engine's first frame sends it.
#[test]
fn signin_device_qr_has_no_code() {
    let shown = AtomicBool::new(false);
    let event = on_event(
        DeviceRequestEvent::Code {
            code: CODE.into(),
            verification_uri: PAGE.into(),
            expires_in: 300,
        },
        "Invented-PC",
        &shown,
    );
    let request = match event {
        SignInEvent::DeviceRequested(Ok(request)) => request,
        other => panic!("the code is a request on the screen, got {other:?}"),
    };
    assert!(shown.load(Ordering::SeqCst));
    assert_eq!(
        (request.code.as_str(), request.seconds_left, request.asking_as.as_str()),
        (CODE, 300, "Invented-PC")
    );
    assert!(!request.qr.is_empty());
    assert_eq!(Some(request.qr.clone()), qr_of(PAGE), "the QR code is the address's");
    for with_code in [
        format!("{PAGE}?code={CODE}"),
        format!("{PAGE}#{CODE}"),
        CODE.to_string(),
    ] {
        assert_ne!(Some(request.qr.clone()), qr_of(&with_code), "{with_code}");
    }
    assert_eq!(
        asking_as(),
        device_request_name().unwrap_or_else(|| THIS_COMPUTER.to_string())
    );
    assert_eq!(
        on_event(DeviceRequestEvent::Tick { seconds_left: 9 }, "Invented-PC", &shown),
        SignInEvent::DeviceTick { seconds_left: 9 }
    );
    assert_eq!(
        on_event(DeviceRequestEvent::Connection(false), "Invented-PC", &shown),
        SignInEvent::DeviceConnection { connected: false }
    );
}

/// Each engine ending as the screen reads it: the `Device` endings once a code is shown (the screen reads nothing else
/// there), the form's problems before; a stop says nothing.
#[test]
fn signin_device_endings_are_drawn_as_the_screen_reads_them() {
    use DeviceRequestRefused as Refused;
    let by = || ANOTHER_DEVICE.to_string();
    let device = |status| Some(SignInEvent::Device(status));
    assert_eq!(
        ending(
            Ok(DeviceSignedIn {
                by: by(),
                vault_unlocked: true
            }),
            true
        ),
        device(DeviceStatus::Approved { by: by() })
    );
    assert_eq!(ending(Err(Refused::Stopped), true), None);
    assert_eq!(ending(Err(Refused::Stopped), false), None);
    assert_eq!(
        ending(Err(Refused::Denied { by: by() }), true),
        device(DeviceStatus::Denied { by: by() })
    );
    assert_eq!(
        ending(Err(Refused::KeysNotReceived { by: by() }), true),
        device(DeviceStatus::KeysNotReceived { by: by() })
    );
    // Row 3.4 of checkpoint 2: a lost request is not an expired code, and an approval this PC did not take is not keys
    // that did not arrive. Mutant: `AccountMismatch` drawn as `KeysNotReceived` again.
    assert_eq!(ending(Err(Refused::Expired), true), device(DeviceStatus::Expired));
    for lost in [Refused::ConnectionLost, Refused::Failed, Refused::Offline] {
        assert_eq!(ending(Err(lost.clone()), true), device(DeviceStatus::Lost), "{lost:?}");
    }
    assert_eq!(
        ending(Err(Refused::AccountMismatch), true),
        device(DeviceStatus::OtherAccount)
    );
    for not_taken in [Refused::Overtaken, Refused::NotStored] {
        assert_eq!(
            ending(Err(not_taken.clone()), true),
            device(DeviceStatus::NotSignedIn),
            "{not_taken:?}"
        );
    }
    assert_eq!(
        ending(Err(Refused::Offline), false),
        Some(SignInEvent::DeviceRequested(Err(SignInProblem::Offline)))
    );
    for other in [Refused::Failed, Refused::Overtaken, Refused::Expired] {
        assert_eq!(
            ending(Err(other.clone()), false),
            Some(SignInEvent::DeviceRequested(Err(SignInProblem::Other))),
            "{other:?}"
        );
    }
}
