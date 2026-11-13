//! Explorer's "Free up space" command against a real engine handle (task 2490): the signed-in owner frees as before;
//! signed out keeping the local data, locked, a session the server ended and another account each answer Disabled,
//! and a click frees 0 bytes and changes no row. A stop, a lock and a sign-out remove the command's binding (a test
//! double for the OS's). The state database and the sync folder are each test's own; the shared settings are never
//! written. Every name is invented.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::super::{Engine, EngineConfig};
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::explorer_command::{CommandState, ExplorerBinding, Invoked, ReclaimOwner, invoke, served, state_of};
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};

type TestEngine = Engine<MemoryCredentials>;

const HELD: u64 = 1_000;

/// The binding removals a stop asked for.
#[derive(Default)]
struct Removed(Mutex<Vec<PathBuf>>);

impl ExplorerBinding for Removed {
    fn remove(&self, sync_root: &std::path::Path) {
        self.0.lock().unwrap().push(sync_root.to_path_buf());
    }
}

impl Removed {
    fn take(&self) -> Vec<PathBuf> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

struct Fixture {
    engine: Arc<TestEngine>,
    removed: Arc<Removed>,
    db: PathBuf,
    root: PathBuf,
    file: PathBuf,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

/// A handle whose local state is account A's, with one downloaded file that is not kept (its bytes in the sync
/// folder): what "Free up space" frees.
fn fixture() -> Fixture {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-slot".into(),
        state_dir,
        api_base: "http://127.0.0.1:9".into(),
    };
    let engine = Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"));
    let dirs = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let db_path = dirs.0.path().join("state.db");
    let root = dirs.1.path().canonicalize().unwrap();
    let file = root.join("held.bin");
    std::fs::write(&file, vec![7u8; HELD as usize]).unwrap();
    let db = StateDb::open(&db_path).unwrap();
    let entry = FileEntry {
        file_id: "f-1".into(),
        path: "/held.bin".into(),
        status: FileStatus::Local,
        size_bytes: HELD as i64,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: None,
        item_kind: ItemKind::File,
    };
    db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
        .unwrap();
    db.mark_cached("f-1", file.to_str().unwrap(), HELD as i64, 1).unwrap();
    db.set_owner(&Identity::new(Some("user-a"), Some("a@example.eu")))
        .unwrap();
    drop(db);
    engine.use_local_state_for_test(db_path.clone());
    engine.use_sync_root_for_test(root.clone());
    let removed = Arc::new(Removed::default());
    engine.use_explorer_binding_for_test(removed.clone());
    Fixture {
        engine,
        removed,
        db: db_path,
        root,
        file,
        _dirs: dirs,
    }
}

/// A session in memory, as a sign-in and an unlock leave it.
fn signed_in(engine: &TestEngine, user_id: &str, email: &str) {
    let _writes = engine.writes.write().unwrap();
    engine
        .acct
        .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
    *engine.acct.session.lock().unwrap() = Some(Session {
        token: "an-invented-token".into(),
        master_key: [7; 32],
        email: Some(email.to_string()),
    });
    engine.remember_session_user_id(Some(user_id.to_string()));
    engine.set_auth_present(true);
}

fn owner(engine: &TestEngine) -> Option<&dyn ReclaimOwner> {
    Some(engine as &dyn ReclaimOwner)
}

fn status(f: &Fixture) -> FileStatus {
    StateDb::open(&f.db).unwrap().get_file("f-1").unwrap().unwrap().status
}

/// The command is Disabled, a click is refused with `kind` and frees nothing: the bytes and the row are as they were.
fn refused_and_untouched(f: &Fixture, kind: &'static str) {
    assert_eq!(state_of(owner(&f.engine)), CommandState::Disabled, "{kind}");
    assert_eq!(invoke(owner(&f.engine)), Invoked::Refused(kind), "{kind}");
    assert_eq!(
        std::fs::metadata(&f.file).unwrap().len(),
        HELD,
        "{kind}: the bytes stay"
    );
    assert_eq!(status(f), FileStatus::Local, "{kind}: the row is unchanged");
}

/// The control: the signed-in owner's click frees the file as before (on Windows the fixture holds no placeholder,
/// so the reclaim is reached and frees nothing).
#[test]
fn the_signed_in_owner_frees_as_before() {
    let f = fixture();
    signed_in(&f.engine, "user-a", "a@example.eu");
    assert_eq!(state_of(owner(&f.engine)), CommandState::Enabled);
    let invoked = invoke(owner(&f.engine));
    #[cfg(not(target_os = "windows"))]
    {
        assert_eq!(invoked, Invoked::Freed(HELD));
        assert!(!f.file.exists(), "the downloaded copy left this computer");
        assert_eq!(status(&f), FileStatus::CloudOnly);
    }
    #[cfg(target_os = "windows")]
    assert!(
        matches!(invoked, Invoked::Freed(_)),
        "the reclaim was reached: {invoked:?}"
    );
}

#[test]
fn signed_out_keeping_the_local_data_the_command_frees_nothing_and_its_binding_is_removed() {
    let f = fixture();
    signed_in(&f.engine, "user-a", "a@example.eu");
    assert_eq!(state_of(owner(&f.engine)), CommandState::Enabled, "the control");
    f.engine.sign_out_keeping_local_data().expect("the sign-out");
    assert_eq!(f.removed.take(), [f.root.clone()], "the sign-out removed the binding");
    refused_and_untouched(&f, "signed_out");
}

#[test]
fn locked_the_command_frees_nothing_and_its_binding_is_removed() {
    let f = fixture();
    signed_in(&f.engine, "user-a", "a@example.eu");
    f.engine.lock();
    assert_eq!(f.removed.take(), [f.root.clone()], "the lock removed the binding");
    refused_and_untouched(&f, "locked");
}

#[test]
fn a_session_the_server_ended_frees_nothing() {
    let f = fixture();
    signed_in(&f.engine, "user-a", "a@example.eu");
    for _ in 0..3 {
        let refused = anyhow::anyhow!("401 Unauthorized");
        f.engine.acct.auth_health.note_result(Some(&refused));
    }
    assert!(
        f.engine.acct.auth_health.is_expired(),
        "precondition: the session ended"
    );
    refused_and_untouched(&f, "session_ended");
}

#[test]
fn another_account_signed_in_frees_nothing_of_the_owners() {
    let f = fixture();
    signed_in(&f.engine, "user-b", "b@example.eu");
    refused_and_untouched(&f, "other_account");
}

/// A stop removes the binding for the folder the engine started in and for the configured one, also when no engine
/// ran (the binding outlives a restart of the app).
#[test]
fn a_stop_removes_the_binding_of_the_started_and_the_configured_folder() {
    let f = fixture();
    let started = f.root.join("started-elsewhere");
    f.engine.note_explorer_root(&started);
    f.engine.stop();
    assert_eq!(f.removed.take(), [started, f.root.clone()]);
    f.engine.stop();
    assert_eq!(
        f.removed.take(),
        [f.root.clone()],
        "nothing ran: the configured folder's binding still goes"
    );
}

/// The one test of the process's slot: the served handle is asked at each call, and a dropped handle answers as
/// none (the command keeps no engine alive).
#[test]
fn the_served_handle_is_asked_each_time_and_a_dropped_one_disables_the_command() {
    let f = fixture();
    f.engine.serve_explorer_command();
    assert_eq!(state_of(served().as_deref()), CommandState::Disabled, "signed out");
    signed_in(&f.engine, "user-a", "a@example.eu");
    assert_eq!(state_of(served().as_deref()), CommandState::Enabled, "the owner");
    let Fixture { engine, .. } = f;
    drop(engine);
    assert!(served().is_none(), "the slot keeps no engine alive");
    assert_eq!(invoke(served().as_deref()), Invoked::Refused("no_engine"));
}
