//! S1's native tests of the sync start (contract `docs/superpowers/specs/2026-11-12-native-sync-start-and-sign-out.md`,
//! §8 S1): the words for the view, one start worker at a time, a refusal on the b and the status line, and the reads
//! on the UI thread. No engine is started here: the view is written by hand, the way the handle writes it.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use beebeeb_engine::app::SyncStartRefused;
use beebeeb_engine::engine_status::{EngineStatusView, State as Reported};
use beebeeb_engine::signin::gate::GateState as EngineGate;
use beebeeb_engine::sync_view::{Explorer, Start, SyncView};

use super::*;
use crate::shell::tray::{EngineState, tray_model};
use crate::ui::status_line as words;

const KINDS: [SyncStartRefused; 9] = [
    SyncStartRefused::FolderOfAnotherProvider,
    SyncStartRefused::FolderUnreadable,
    SyncStartRefused::FolderMissing,
    SyncStartRefused::RegistrationFailed { code: Some(5) },
    SyncStartRefused::OtherAccountsData,
    SyncStartRefused::IdentityUnknown,
    SyncStartRefused::StopUnconfirmed,
    SyncStartRefused::StateInUse,
    SyncStartRefused::Other,
];

fn ready(start: Start) -> SyncView {
    SyncView {
        gate: EngineGate::Ready,
        start,
        attempt: 1,
        ..SyncView::default()
    }
}

/// Every view in which no engine runs: never started, starting, and each refusal; each paused or not, and each with
/// a finished check and the engine's "idle" still in the view (what tempts an "Up to date").
fn without_an_engine() -> Vec<SyncView> {
    let mut starts = vec![Start::NotAsked, Start::Starting];
    starts.extend(KINDS.map(Start::Refused));
    let mut views = Vec::new();
    for start in starts {
        for paused in [false, true] {
            views.push(SyncView {
                paused,
                reported: Some(Reported::Idle),
                last_check_at: Some(1_794_000_000),
                explorer: Explorer::Added,
                ..ready(start)
            });
        }
    }
    views
}

/// Invariant I12: without a running engine the status line never says "Up to date", "Syncing" or "Paused" (nor any
/// other word of a running one): "Sync is off", or "Starting…" while a start runs. Signed out, it says nothing.
#[test]
fn the_status_is_never_up_to_date_without_an_engine() {
    let running_words = [
        words::UP_TO_DATE,
        words::SYNCING,
        words::PAUSED,
        words::CHECKING,
        words::SYNC_STOPPED,
        OFFLINE,
    ];
    let views = without_an_engine();
    assert_eq!(views.len(), 22);
    for view in &views {
        let word = sync_word(view).expect("signed in and unlocked: the line says something");
        assert!(!running_words.contains(&word), "{:?} says {word}", view.start);
        let expected = if view.start == Start::Starting {
            words::STARTING
        } else {
            words::SYNC_IS_OFF
        };
        assert_eq!(word, expected, "{:?}", view.start);
    }
    for gate in [
        EngineGate::SignedOut,
        EngineGate::NeedsRecoveryPhrase,
        EngineGate::Locked,
    ] {
        assert_eq!(
            sync_word(&SyncView {
                gate,
                ..ready(Start::Running)
            }),
            None
        );
    }
    // A running engine says what it last reported, and "Up to date" only after a finished check.
    let running = |reported: Option<Reported>, checked: bool| SyncView {
        reported,
        last_check_at: checked.then_some(1_794_000_000),
        ..ready(Start::Running)
    };
    assert_eq!(sync_word(&running(Some(Reported::Idle), true)), Some(words::UP_TO_DATE));
    assert_eq!(sync_word(&running(Some(Reported::Idle), false)), Some(words::CHECKING));
    assert_eq!(sync_word(&running(None, false)), Some(words::CHECKING));
    assert_eq!(
        sync_word(&running(Some(Reported::Running), false)),
        Some(words::CHECKING)
    );
    assert_eq!(sync_word(&running(Some(Reported::Syncing), true)), Some(words::SYNCING));
    assert_eq!(sync_word(&running(Some(Reported::Offline), true)), Some(OFFLINE));
    assert_eq!(
        sync_word(&running(Some(Reported::Error), true)),
        Some(words::SYNC_STOPPED)
    );
    let paused = SyncView {
        paused: true,
        ..running(Some(Reported::Idle), true)
    };
    assert_eq!(sync_word(&paused), Some(words::PAUSED));
    let ended = SyncView {
        session_ended: true,
        ..running(Some(Reported::Idle), true)
    };
    assert_eq!(sync_word(&ended), Some(words::SIGN_IN_AGAIN));
}

/// Ten asks in a row while a start runs make one start; once it is done, the next ask makes the next.
#[test]
fn one_start_worker_at_a_time() {
    let worker = StartWorker::default();
    let starts = Arc::new(AtomicU32::new(0));
    let (release, released) = mpsc::channel::<()>();
    let released = Arc::new(std::sync::Mutex::new(released));
    let (finished, done) = mpsc::channel::<()>();
    let mut began = 0;
    for _ in 0..10 {
        let (starts, released, finished) = (starts.clone(), released.clone(), finished.clone());
        let asked = worker
            .ask(move || {
                starts.fetch_add(1, Ordering::SeqCst);
                let _ = released.lock().unwrap().recv_timeout(Duration::from_secs(5));
                let _ = finished.send(());
            })
            .expect("a thread");
        began += u32::from(asked);
    }
    assert_eq!(began, 1, "one ask of ten started a worker");
    release.send(()).unwrap();
    done.recv_timeout(Duration::from_secs(5)).expect("the worker ends");
    // The flag clears when the worker's thread ends, a moment after its last line.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let next = loop {
        let starts = starts.clone();
        let start = move || {
            starts.fetch_add(1, Ordering::SeqCst);
        };
        if worker.ask(start).unwrap() {
            break true;
        }
        if std::time::Instant::now() > deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(next, "a start after the first one ended makes a worker");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while starts.load(Ordering::SeqCst) < 2 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(starts.load(Ordering::SeqCst), 2, "1 start per worker, 2 workers");
}

/// A refused start reaches the b (not syncing, never "Up to date", no File Explorer entry to open) and the status
/// line ("Sync is off"), for every kind; the modal gets the kind, the attempt and the folder's name.
#[test]
fn a_refusal_reaches_the_tray_and_the_status_line() {
    let reported = EngineStatusView::default();
    for kind in KINDS {
        let view = SyncView {
            explorer: Explorer::Added,
            ..ready(Start::Refused(kind))
        };
        let inputs = tray_inputs_of(&view, &reported);
        assert_eq!(inputs.engine, EngineState::NotRunning, "{kind:?}");
        assert!(
            !inputs.explorer_ready,
            "{kind:?}: no engine, nothing to open in File Explorer"
        );
        assert!(inputs.signed_in && !inputs.paused);
        let label = tray_model(&inputs).label;
        assert!(!label.contains("Up to date"), "{kind:?}: {label}");
        assert_eq!(sync_word(&view), Some(words::SYNC_IS_OFF));
        let refusal = sync_refusal(&view, Some("Beebeeb Test".into())).expect("the modal's refusal");
        assert_eq!((refusal.attempt, refusal.kind), (1, refused_of(kind)));
        assert_eq!(refusal.folder.as_deref(), Some("Beebeeb Test"));
    }
    assert_eq!(
        sync_refusal(&ready(Start::Running), None),
        None,
        "no refusal while sync runs"
    );
    // Running, with the Explorer entry: the b may open File Explorer.
    let running = SyncView {
        explorer: Explorer::Added,
        ..ready(Start::Running)
    };
    assert!(tray_inputs_of(&running, &reported).explorer_ready);
}

/// The reads the UI thread makes on every frame go to the view only: no `engine.gate()` (it can read the credential
/// store), no `is_syncing()` or `is_paused()` (the first waits for the engine slot, which a start holds while it
/// registers the folder).
#[test]
fn the_ui_thread_reads_the_view_only() {
    let read = |file: &str| {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(file);
        std::fs::read_to_string(path).unwrap().replace("\r\n", "\n")
    };
    let body = |source: &str, from: &str| -> String {
        let start = source.find(from).unwrap_or_else(|| panic!("no {from}"));
        let end = source[start..]
            .find("\n    }\n")
            .unwrap_or_else(|| panic!("{from} does not close"));
        source[start..start + end].to_string()
    };
    let host = read("engine_host.rs");
    let tray = read("engine_host/tray.rs");
    let reads = [
        body(&host, "    fn session(&self) -> Session {"),
        body(&host, "    fn tray_inputs(&self) -> TrayInputs {"),
        body(&host, "    fn start_sync(&self) -> Result<(), actions::ActionError> {"),
        body(&read("engine_host/signin.rs"), "    fn gate(&self) -> GateState {"),
        body(&host, "    fn sync_state(&self) -> Option<String> {"),
        body(&host, "    fn can_add(&self, _folder: Option<&str>) -> bool {"),
        body(&tray, "    pub fn facts(&self) -> Facts {"),
    ];
    for read in &reads {
        for waits in ["engine.gate()", ".is_syncing()", ".is_paused()"] {
            assert!(!read.contains(waits), "a UI-thread read calls {waits}:\n{read}");
        }
        assert!(read.contains(".view()"), "it reads the view:\n{read}");
    }
}

fn source(file: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(file);
    std::fs::read_to_string(path).unwrap().replace("\r\n", "\n")
}

/// Invariant I1: no launch argument can carry a secret. Every flag the launch parser (and the app's own autostart
/// entry) knows is listed in its source; none names a phrase, a password, a code, a key or a token, so none of those
/// is ever read from argv (where any program of the same user can see it).
#[test]
fn no_launch_argument_can_carry_a_secret() {
    let mut flags = Vec::new();
    for file in ["shell/instance.rs", "shell/autostart.rs"] {
        let text = source(file);
        for literal in text.split('"').skip(1).step_by(2) {
            if literal.starts_with("--") && literal.len() > 2 {
                flags.push(literal.to_string());
            }
        }
    }
    assert!(
        flags.len() >= 25,
        "the parser's flags were found ({} of them): {flags:?}",
        flags.len()
    );
    assert!(flags.iter().any(|flag| flag == "--api-base") && flags.iter().any(|flag| flag == "--autostart"));
    for flag in &flags {
        for secret in [
            "phrase",
            "password",
            "passphrase",
            "words",
            "code",
            "key",
            "token",
            "secret",
            "otp",
        ] {
            assert!(!flag.contains(secret), "the launch flag {flag} would carry a {secret}");
        }
    }
}

/// Invariant I13: in `open()`, the test build's server binding (PR #335) runs before the engine handle exists, and
/// nothing before it opens `state.db` or reads a credential entry's value (the binding counts the entries only).
#[test]
fn nothing_reads_local_state_before_the_server_binding() {
    let open = source("engine_host/open.rs");
    let from = open.find("pub fn open<C: CredentialStores>(").expect("open()");
    let body = &open[from..from + open[from..].find("\n}\n").expect("open() closes")];
    let bind = body
        .find("bind_to_server(&state_dir, entries, api_base)?;")
        .expect("the binding");
    let engine = body.find("Engine::open(").expect("the handle");
    let restore = body.find(".restore()").expect("the stored session read");
    assert!(
        bind < engine && engine < restore,
        "binding, then the handle, then the stored session"
    );
    let before = &body[..bind];
    for reads in [
        "StateDb",
        "state_db",
        "load_session_token",
        "load_wrapped_master_key",
        "load_account_email",
        "session_token(",
        "restore(",
    ] {
        assert!(!before.contains(reads), "open() reads {reads} before the binding");
    }
    let from = open.find("pub fn stored_account_entries<").expect("the entries' count");
    let entries = &open[from..from + open[from..].find("\n}\n").unwrap()];
    assert!(entries.contains("holds_session_token()"), "{entries}");
    assert!(!entries.contains("load_"), "the count reads a value:\n{entries}");
}

/// Sync starts with the keys in memory, a locked screen included, and under no other gate.
#[test]
fn sync_may_start_behind_a_locked_screen_and_never_without_the_keys() {
    let may: Vec<EngineGate> = [
        EngineGate::Ready,
        EngineGate::ScreenLocked,
        EngineGate::Locked,
        EngineGate::SignedOut,
        EngineGate::NeedsRecoveryPhrase,
        EngineGate::NoPlan,
        EngineGate::KeyStoreUnavailable,
    ]
    .into_iter()
    .filter(|gate| may_start(*gate))
    .collect();
    assert_eq!(may, [EngineGate::Ready, EngineGate::ScreenLocked]);
}

/// B-R6: the session watcher starts once, where the real app opens its engine, and no test helper starts it:
/// the tests open engines through `engine_host::open::open` and some end a session on purpose.
#[test]
fn the_session_watcher_starts_once_where_the_real_app_opens_its_engine() {
    let shell = source("shell/mod.rs");
    assert_eq!(shell.matches("watch_session()").count(), 1, "shell/mod.rs");
    let at = shell.find("fn open_engine(").expect("open_engine");
    let end = shell[at..].find("\n}\n").expect("end of open_engine") + at;
    assert!(shell[at..end].contains("watch_session()"), "inside open_engine's body");
    assert_eq!(
        source("engine_host/open.rs").matches("watch_session").count(),
        0,
        "open.rs"
    );
}
