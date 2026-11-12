//! The binding of a test build's local data to one server (`engine_host::open::bind_to_server`, the lead's rulings
//! inside decision 2462), from the rule itself to `open`. Kept apart from `tests.rs` for its size; the shared start
//! (`started`) and the source helper (`squeezed`) live there.

use beebeeb_engine::config::DesktopConfig;
use beebeeb_engine::keychain::{AuthResult, AuthSecretStore, AuthStoreError, SecretBytes, SessionToken};
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_paths;

use super::open::{self, ApiBase, CredentialsWithoutData, OpenError, OtherServer, Server};
use super::tests::{squeezed, started};
use crate::shell::identity::engine_identity;

/// A custom server for the binding tests. Not `http://127.0.0.1:9`: that is a unit-test build's DEFAULT (the engine's
/// `test-support`), while in a shipped test build it is a custom server (the capture rung's closed port).
fn custom(origin: &str) -> ApiBase {
    let base = open::api_base(Some(origin), true).expect("a custom origin");
    assert_eq!(base.server(), Server::Custom(origin.to_string()), "{origin}");
    base
}

fn record_in(state_dir: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(state_dir.join(open::SERVER_FILE)).ok()
}

fn other_server(result: Result<(), OpenError>) -> OtherServer {
    match result {
        Err(OpenError::OtherServer(other)) => other,
        other => panic!("refused for another server, not {other:?}"),
    }
}

fn without_data(result: Result<(), OpenError>) -> CredentialsWithoutData {
    match result {
        Err(OpenError::CredentialsWithoutData(alone)) => alone,
        other => panic!("refused: stored credentials without local data, not {other:?}"),
    }
}

/// Data signed in on the default server is refused for a custom one, when the record says so, or (made before the
/// record existed) when its files are still in the folder; the default opens that and records itself. Credentials with
/// nothing in the folder are refused for every server (#335 review P1-1): the record went with the folder.
#[test]
fn data_signed_in_on_the_default_server_is_refused_for_a_custom_one() {
    let (default, local) = (ApiBase::engine_default(), custom("http://127.0.0.1:3001"));
    let asked_local = Server::Custom("http://127.0.0.1:3001".to_string());
    // Made after this change: the first open recorded the default, then the engine made its database.
    let dir = tempfile::tempdir().unwrap();
    open::bind_to_server(dir.path(), 0, &default).expect("a first open on the default");
    assert_eq!(record_in(dir.path()).as_deref(), Some("default\n"));
    std::fs::write(dir.path().join("state.db"), b"").unwrap();
    let other = other_server(open::bind_to_server(dir.path(), 1, &local));
    assert_eq!(
        other,
        OtherServer {
            made_for: Some(Server::Default),
            asked: asked_local.clone()
        }
    );
    // A stored session, no record, nothing in the folder: refused for the custom server AND for the default.
    let dir = tempfile::tempdir().unwrap();
    for asked in [&local, &default] {
        assert_eq!(
            without_data(open::bind_to_server(dir.path(), 1, asked)),
            CredentialsWithoutData
        );
    }
    assert_eq!(record_in(dir.path()), None, "a refusal records nothing");
    // Signed in before this change, its database still here: the default's.
    std::fs::write(dir.path().join("state.db"), b"").unwrap();
    let other = other_server(open::bind_to_server(dir.path(), 1, &local));
    assert_eq!((other.made_for, other.asked), (Some(Server::Default), asked_local));
    open::bind_to_server(dir.path(), 1, &default).expect("the default opens its own data");
    assert_eq!(record_in(dir.path()).as_deref(), Some("default\n"));
}

/// The uninstaller's "Delete the application data" removes the state folder (and the record in it) and keeps the
/// credentials and the settings file; a folder can also be removed or emptied by hand. Stored credentials with no
/// local data beside them say nothing about their server, so every server is refused, and nothing is made.
#[test]
fn stored_credentials_without_local_data_are_refused_for_every_server() {
    let base = tempfile::tempdir().unwrap();
    let gone = base.path().join("removed-with-its-record");
    let emptied = tempfile::tempdir().unwrap();
    let half_written = tempfile::tempdir().unwrap();
    std::fs::write(
        half_written.path().join(format!("{}.partial", open::SERVER_FILE)),
        "http://",
    )
    .unwrap();
    for dir in [gone.as_path(), emptied.path(), half_written.path()] {
        for asked in [
            ApiBase::engine_default(),
            custom("http://127.0.0.1:3001"),
            custom("https://staging.example"),
        ] {
            for entries in [1, 3] {
                let refusal = without_data(open::bind_to_server(dir, entries, &asked));
                assert_eq!(refusal, CredentialsWithoutData, "{} {entries}", asked.origin());
            }
        }
        assert_eq!(record_in(dir), None, "{}", dir.display());
    }
    assert!(!gone.exists(), "a refusal makes no folder");
    // Without the credentials it is a clean computer again.
    open::bind_to_server(&gone, 0, &custom("http://127.0.0.1:3001")).expect("nothing here at all");
    assert_eq!(record_in(&gone).as_deref(), Some("http://127.0.0.1:3001\n"));
}

/// Data made for a custom server is refused without the flag (the default) and for another custom server; the record
/// stays as it was.
#[test]
fn data_made_for_a_custom_server_is_refused_for_the_default_and_for_another_custom_one() {
    let dir = tempfile::tempdir().unwrap();
    open::bind_to_server(dir.path(), 0, &custom("http://127.0.0.1:3001")).expect("a clean first open");
    std::fs::write(dir.path().join("state.db"), b"").unwrap();
    let made_for = Some(Server::Custom("http://127.0.0.1:3001".to_string()));
    let other = other_server(open::bind_to_server(dir.path(), 1, &ApiBase::engine_default()));
    assert_eq!(
        other,
        OtherServer {
            made_for: made_for.clone(),
            asked: Server::Default
        }
    );
    let other = other_server(open::bind_to_server(dir.path(), 1, &custom("https://staging.example")));
    assert_eq!(
        (other.made_for, other.asked),
        (made_for, Server::Custom("https://staging.example".into()))
    );
    assert_eq!(record_in(dir.path()).as_deref(), Some("http://127.0.0.1:3001\n"));
    // A record nobody can read opens for no server.
    std::fs::write(dir.path().join(open::SERVER_FILE), "not a server\n").unwrap();
    for asked in [ApiBase::engine_default(), custom("http://127.0.0.1:3001")] {
        assert_eq!(other_server(open::bind_to_server(dir.path(), 1, &asked)).made_for, None);
    }
}

/// Open call (b), the lead's ruling: a record with nothing beside it (no other file in the folder, no credential
/// entry) is replaced by the next run's server, readable or not, because nothing was made for the server it names. A
/// file or a credential beside it keeps it.
#[test]
fn a_record_with_nothing_beside_it_is_replaced_by_the_next_runs_server() {
    let (default, local, staging) = (
        ApiBase::engine_default(),
        custom("http://127.0.0.1:3001"),
        custom("https://staging.example"),
    );
    // A first start that went nowhere (a mistyped origin, a start that failed after the record).
    let dir = tempfile::tempdir().unwrap();
    open::bind_to_server(dir.path(), 0, &local).expect("a clean first open");
    open::bind_to_server(dir.path(), 0, &default).expect("the default replaces a record alone");
    assert_eq!(record_in(dir.path()).as_deref(), Some("default\n"));
    open::bind_to_server(dir.path(), 0, &staging).expect("and another custom server replaces that");
    assert_eq!(record_in(dir.path()).as_deref(), Some("https://staging.example\n"));
    // An unreadable record alone, likewise.
    std::fs::write(dir.path().join(open::SERVER_FILE), "not a server\n").unwrap();
    open::bind_to_server(dir.path(), 0, &local).expect("an unreadable record alone is replaced");
    assert_eq!(record_in(dir.path()).as_deref(), Some("http://127.0.0.1:3001\n"));
    // A credential entry beside it: kept, readable or not.
    let other = other_server(open::bind_to_server(dir.path(), 1, &default));
    assert_eq!(other.made_for, Some(Server::Custom("http://127.0.0.1:3001".into())));
    std::fs::write(dir.path().join(open::SERVER_FILE), "not a server\n").unwrap();
    assert_eq!(
        other_server(open::bind_to_server(dir.path(), 1, &default)).made_for,
        None
    );
    // A file beside it: kept.
    let dir = tempfile::tempdir().unwrap();
    open::bind_to_server(dir.path(), 0, &local).expect("a clean first open");
    std::fs::write(dir.path().join(".device.json"), b"").unwrap();
    let other = other_server(open::bind_to_server(dir.path(), 0, &staging));
    assert_eq!(other.made_for, Some(Server::Custom("http://127.0.0.1:3001".into())));
    assert_eq!(record_in(dir.path()).as_deref(), Some("http://127.0.0.1:3001\n"));
}

/// Local data from before the record existed (anything in the state folder) counts as the default's: a custom server
/// is refused, the default opens it and records itself. A half-written record alone is not data.
#[test]
fn local_data_without_a_record_counts_as_the_default_servers() {
    let local = custom("http://127.0.0.1:3001");
    for leftover in ["state.db", ".beebeeb-sync.lock", ".device.json"] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(leftover), b"").unwrap();
        let other = other_server(open::bind_to_server(dir.path(), 0, &local));
        assert_eq!(other.made_for, Some(Server::Default), "{leftover}");
        assert_eq!(record_in(dir.path()), None, "{leftover}");
        open::bind_to_server(dir.path(), 0, &ApiBase::engine_default()).expect(leftover);
        assert_eq!(record_in(dir.path()).as_deref(), Some("default\n"), "{leftover}");
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(format!("{}.partial", open::SERVER_FILE)), "http://").unwrap();
    open::bind_to_server(dir.path(), 0, &local).expect("a half-written record is not data");
    assert_eq!(record_in(dir.path()).as_deref(), Some("http://127.0.0.1:3001\n"));
}

/// A clean computer (no state folder yet, no credential entry) starts on a custom server, the capture rung's case, and
/// records it before the engine makes anything there.
#[test]
fn a_clean_computer_starts_on_a_custom_server_and_records_it() {
    let base = tempfile::tempdir().unwrap();
    let state_dir = base.path().join("not-made-yet");
    open::bind_to_server(&state_dir, 0, &custom("http://127.0.0.1:3001")).expect("a clean computer starts");
    assert_eq!(record_in(&state_dir).as_deref(), Some("http://127.0.0.1:3001\n"));
    let names: Vec<_> = std::fs::read_dir(&state_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, [open::SERVER_FILE], "the record, and nothing else yet");
}

/// The same server twice starts twice, the default and a custom one. The default's own origin given with
/// `--api-base` is the default server.
#[test]
fn the_same_server_twice_starts_twice() {
    for asked in [
        ApiBase::engine_default(),
        custom("http://127.0.0.1:3001"),
        custom("https://staging.example"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        open::bind_to_server(dir.path(), 0, &asked).expect("the first open");
        std::fs::write(dir.path().join("state.db"), b"").unwrap();
        open::bind_to_server(dir.path(), 1, &asked).expect("the second open");
        open::bind_to_server(dir.path(), 1, &asked).expect("and the third");
    }
    let default_by_flag =
        open::api_base(Some(&format!("{}/", beebeeb_engine::runner::default_api_base())), true).unwrap();
    assert!(default_by_flag.from_flag());
    assert_eq!(default_by_flag.server(), Server::Default);
}

/// A store that cannot answer whether it holds anything.
struct Unanswering;

impl AuthSecretStore for Unanswering {
    fn save_session_token(&self, _: &SessionToken) -> AuthResult<()> {
        Err(AuthStoreError::Backend("down".into()))
    }
    fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
        Err(AuthStoreError::Backend("down".into()))
    }
    fn delete_session_token(&self) -> AuthResult<()> {
        Err(AuthStoreError::Backend("down".into()))
    }
    fn save_wrapped_master_key(&self, _: SecretBytes) -> AuthResult<()> {
        Err(AuthStoreError::Backend("down".into()))
    }
    fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
        Err(AuthStoreError::Backend("down".into()))
    }
    fn delete_wrapped_master_key(&self) -> AuthResult<()> {
        Err(AuthStoreError::Backend("down".into()))
    }
}

/// A stored session is counted through the engine's own probes of the account's store, never read out; a store that
/// cannot answer counts as holding something.
#[test]
fn a_stored_session_is_counted_the_engines_way_and_a_silent_store_counts_as_one() {
    let stores = MemoryCredentials::default();
    assert_eq!(open::stored_account_entries(&stores.store_for("slot-a")), 0);
    stores
        .store_for("slot-a")
        .save_session_token(&SessionToken::new("an-invented-token").unwrap())
        .unwrap();
    assert_eq!(open::stored_account_entries(&stores.store_for("slot-a")), 1);
    assert_eq!(
        open::stored_account_entries(&stores.store_for("slot-b")),
        0,
        "another slot's"
    );
    assert!(open::stored_account_entries(&Unanswering) > 0, "fail closed");
}

/// Each refusal names kinds and states only: the two servers' kinds, or "stored credentials without local data"; no
/// origin, no token, no account.
#[test]
fn the_refusal_names_the_two_servers_kinds_and_nothing_else() {
    let (a, b) = (
        Server::Custom("http://127.0.0.1:3001".to_string()),
        Server::Custom("https://staging.example".to_string()),
    );
    let line = |made_for: Option<Server>, asked: Server| OtherServer { made_for, asked }.to_string();
    let start = "this test build's data on this computer was made for ";
    for (text, made, asked) in [
        (
            line(Some(Server::Default), a.clone()),
            "the default server",
            "a custom server (--api-base)",
        ),
        (
            line(Some(a.clone()), Server::Default),
            "a custom server (--api-base)",
            "the default server",
        ),
        (
            line(Some(a.clone()), b.clone()),
            "a custom server (--api-base)",
            "a different custom server (--api-base)",
        ),
        (
            line(None, b.clone()),
            "a server its record no longer names",
            "a custom server (--api-base)",
        ),
    ] {
        assert_eq!(
            text,
            format!("{start}{made}, and this run asks for {asked}. The app did not start.")
        );
        assert!(
            !text.contains("127.0.0.1") && !text.contains("staging") && !text.contains('\n'),
            "{text}"
        );
    }
    assert_eq!(
        CredentialsWithoutData.to_string(),
        "this test build has stored credentials without local data on this computer: nothing says which server they \
         were made for, so no server gets them. The app did not start."
    );
}

/// The start path binds the test build's data: the shared start (`started`, a stored session and the default) left
/// the default's record, and the binding sits after the account slot and before the engine (which takes the stores
/// and the state folder, so it cannot come later). How `run` says a refusal: `shell::start_refusal_tests`.
#[test]
fn the_start_path_binds_the_test_builds_data_before_the_engine_reads_it() {
    let started = started();
    let state_dir = state_paths::beebeeb_state_dir_for(started.opened.identity).unwrap();
    assert_eq!(record_in(&state_dir).as_deref(), Some("default\n"));
    let open_rs = squeezed(include_str!("open.rs"));
    let body = &open_rs[open_rs.find(&squeezed("pub fn open<C: CredentialStores>(")).unwrap()..];
    let at = |step: &str| body.find(&squeezed(step)).unwrap_or_else(|| panic!("open has {step}"));
    let order = [
        "DesktopConfig::ensure_account_id_for(identity)?;",
        "if !identity.is_real() {",
        "let entries = stored_account_entries(&stores.for_account(identity, &account_id));",
        "bind_to_server(&state_dir, entries, api_base)?;",
        "Engine::open(identity, config, host, stores)?",
        "engine.restore()",
    ]
    .map(at);
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{order:?}");
}

/// Runs a future that waits on nothing (a synchronous `open` inside the engine's state-folder scope) to its end.
fn run_now<F: std::future::Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    match std::future::Future::poll(future.as_mut(), &mut context) {
        std::task::Poll::Ready(output) => output,
        std::task::Poll::Pending => panic!("the future waited on something"),
    }
}

/// `open` itself for `asked`, in `state_dir` (the engine's test scope; the process folder stays `started`'s), on
/// credential stores whose account slot holds a stored session. Only a refusal is expected: an opened engine panics.
fn open_signed_in(state_dir: &std::path::Path, asked: &ApiBase) -> OpenError {
    let base = &started().base;
    let stores = MemoryCredentials::default();
    let slot = DesktopConfig::ensure_account_id_for(engine_identity()).expect("the sandboxed settings file");
    stores
        .store_for(&slot)
        .save_session_token(&SessionToken::new("an-invented-token").unwrap())
        .unwrap();
    let result = run_now(state_paths::STATE_DIR_OVERRIDE.scope(state_dir.to_path_buf(), async {
        open::open(base, asked, stores, || {})
    }));
    match result {
        Err(error) => error,
        Ok(_) => panic!("the engine opened a test build's signed-in data for {}", asked.origin()),
    }
}

/// Through `open` itself: the account slot's stored session with nothing in the folder is refused for every server,
/// and with the folder's database (made before the record existed) for a custom one; before the engine opens, and
/// nothing is recorded or made.
#[test]
fn open_refuses_a_test_build_signed_in_on_the_default_for_a_custom_server() {
    let state_dir = tempfile::tempdir().unwrap();
    for asked in [custom("http://127.0.0.1:3001"), ApiBase::engine_default()] {
        match open_signed_in(state_dir.path(), &asked) {
            OpenError::CredentialsWithoutData(alone) => assert_eq!(alone, CredentialsWithoutData),
            other => panic!(
                "{}: refused for credentials without local data, not {other:?}",
                asked.origin()
            ),
        }
    }
    assert_eq!(
        std::fs::read_dir(state_dir.path()).unwrap().count(),
        0,
        "nothing made in the folder"
    );
    std::fs::write(state_dir.path().join("state.db"), b"").unwrap();
    match open_signed_in(state_dir.path(), &custom("http://127.0.0.1:3001")) {
        OpenError::OtherServer(other) => assert_eq!(
            other,
            OtherServer {
                made_for: Some(Server::Default),
                asked: Server::Custom("http://127.0.0.1:3001".to_string())
            }
        ),
        other => panic!("refused for another server, not {other:?}"),
    }
    assert_eq!(record_in(state_dir.path()), None);
}
