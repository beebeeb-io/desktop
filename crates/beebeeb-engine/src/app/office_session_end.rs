//! The one seam between the engine's session end and the Office host (which the engine cannot name): the host
//! registers ONE hook per state dir, and a vault lock, a server-ended close and a sign-out call it BEFORE the key
//! leaves memory, the recovery blobs are wiped and the scratch key ends ([`Engine::office_before_session_end`]).
//!
//! - [`SessionEnding::Lock`]: the host seals a recovery snapshot of every open edited document (it puts them through
//!   [`Engine::put_office_snapshot_for`], which needs the session still in memory), within a hard bound it owns.
//! - [`SessionEnding::SignOut`]: the host ends its helpers and drops what they hold, so no snapshot lands after the
//!   wipe that follows. Nothing is snapshotted: sign-out deletes every blob.
//!
//! The hook can neither refuse nor fail the step; a panic in it is caught.
//!
//! Must not: hold a state of its own beyond the registered hook; log a name, a path or a byte.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::{CredentialStores, Engine};

/// Which end of the session is coming.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionEnding {
    /// The key is about to leave memory (a vault lock, or the server ended the session).
    Lock,
    /// The account is about to be signed out.
    SignOut,
}

/// What the host registers.
pub type SessionEndHook = Arc<dyn Fn(SessionEnding) + Send + Sync>;

static HOOKS: Mutex<Option<HashMap<PathBuf, SessionEndHook>>> = Mutex::new(None);

fn set_in(state_dir: &Path, hook: SessionEndHook) {
    let mut all = HOOKS.lock().unwrap_or_else(|p| p.into_inner());
    all.get_or_insert_with(HashMap::new).insert(state_dir.to_path_buf(), hook);
}

fn run_in(state_dir: &Path, ending: SessionEnding) {
    // Cloned out: the hook runs with no lock of this module held.
    let hook = HOOKS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .and_then(|all| all.get(state_dir).cloned());
    if let Some(hook) = hook {
        let _ = catch_unwind(AssertUnwindSafe(|| hook(ending)));
    }
}

impl<C: CredentialStores> Engine<C> {
    /// Register the host's hook for this engine's state dir (a second call replaces the first).
    pub fn set_office_session_end_hook(&self, hook: SessionEndHook) {
        set_in(&self.config.state_dir, hook);
    }

    /// Called by the lock and sign-out paths before anything of the session ends. Blocks as long as the hook does.
    pub(super) fn office_before_session_end(&self, ending: SessionEnding) {
        run_in(&self.config.state_dir, ending);
    }
}

#[cfg(test)]
mod office_session_end_tests {
    use super::*;
    use crate::account::{SessionGeneration, SessionTransition};
    use crate::host::ChannelHost;
    use crate::identity::TEST;
    use crate::session::Session;
    use crate::signin::memory_store::MemoryCredentials;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use zeroize::Zeroizing;

    /// The tests share one state dir (and so one hook slot and one scratch folder): one at a time.
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

    const FILE: &str = "11111111-1111-4111-8111-111111111111";

    #[test]
    fn the_hook_of_a_state_dir_runs_with_the_ending_and_a_panic_is_caught() {
        let dir = PathBuf::from("/nonexistent/session-end-a");
        let seen = Arc::new(AtomicUsize::new(0));
        let s = seen.clone();
        set_in(
            &dir,
            Arc::new(move |e| {
                s.fetch_add(if e == SessionEnding::Lock { 1 } else { 10 }, Ordering::SeqCst);
            }),
        );
        run_in(&dir, SessionEnding::Lock);
        run_in(&dir, SessionEnding::SignOut);
        run_in(Path::new("/nonexistent/other"), SessionEnding::Lock);
        assert_eq!(seen.load(Ordering::SeqCst), 11);
        set_in(&dir, Arc::new(|_| panic!("boom")));
        run_in(&dir, SessionEnding::Lock);
    }

    fn engine_with_a_session(account: &str) -> Arc<Engine<MemoryCredentials>> {
        let (host, _messages) = ChannelHost::new(|| {});
        let config = super::super::EngineConfig {
            account_id: account.into(),
            state_dir: crate::state_paths::init_for_test(),
            api_base: "http://127.0.0.1:9".into(),
        };
        let engine =
            Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the handle opens"));
        {
            let _writes = engine.writes.write().unwrap();
            engine
                .acct
                .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
            *engine.acct.session.lock().unwrap() = Some(Session {
                token: "an-invented-token".into(),
                master_key: [7; 32],
                email: None,
            });
        }
        engine
    }

    /// A package the recovery store's kind check accepts.
    fn package() -> Zeroizing<Vec<u8>> {
        let mut v = b"the last words".to_vec();
        v.extend_from_slice(b"word/document.xml");
        Zeroizing::new(v)
    }

    fn files_under(dir: &Path) -> usize {
        std::fs::read_dir(dir).map(|rd| rd.count()).unwrap_or(0)
    }

    /// A vault lock calls the hook while the session is still in memory (the put works), keeps the blob, and ends the
    /// scratch key and folder right after.
    #[test]
    fn a_lock_calls_the_hook_with_the_session_still_in_memory_and_ends_the_scratch() {
        let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
        let engine = engine_with_a_session("an-invented-slot-lock");
        let session: SessionGeneration = engine.acct.session_generation();
        let scratch = engine.scratch_for(session).expect("a scratch store");
        let spilled = scratch.put(&vec![9u8; 4096]).expect("spilled");
        let state_dir = engine.config.state_dir.clone();
        let sealed_in_the_hook = Arc::new(AtomicUsize::new(0));
        let (probe, seen) = (engine.clone(), sealed_in_the_hook.clone());
        engine.set_office_session_end_hook(Arc::new(move |ending| {
            assert_eq!(ending, SessionEnding::Lock);
            // The spilled copy still opens, and the blob goes in: the session is still in memory.
            assert!(spilled.read().is_ok(), "scratch not ended yet");
            if probe.put_office_snapshot_for(session, FILE, 3, &package()).is_ok() {
                seen.fetch_add(1, Ordering::SeqCst);
            }
        }));
        engine.lock_vault();
        assert_eq!(sealed_in_the_hook.load(Ordering::SeqCst), 1, "the snapshot was sealed before the key left");
        assert!(engine.office_snapshots_for(session).is_err(), "locked: the key is gone, nothing opens");
        assert!(
            files_under(&state_dir.join("office-recovery")) > 0,
            "the sealed blob stays through a lock, for the offer after unlock"
        );
        assert!(scratch.put(b"x").is_err(), "the scratch key ended with the lock");
        assert_eq!(files_under(&state_dir.join("office-scratch")), 0);
    }

    /// A sign-out calls the hook first, and then no recovery blob and no scratch file is left.
    #[test]
    fn a_sign_out_leaves_no_recovery_blob_and_no_scratch_file() {
        let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
        let engine = engine_with_a_session("an-invented-slot-out");
        let session = engine.acct.session_generation();
        let state_dir = engine.config.state_dir.clone();
        engine.put_office_snapshot_for(session, FILE, 3, &package()).expect("kept");
        let blobs_before = files_under(&state_dir.join("office-recovery"));
        assert!(blobs_before > 0);
        let scratch = engine.scratch_for(session).expect("a scratch store");
        let _spilled = scratch.put(&vec![9u8; 4096]).expect("spilled");
        let called = Arc::new(AtomicUsize::new(0));
        let c = called.clone();
        engine.set_office_session_end_hook(Arc::new(move |ending| {
            assert_eq!(ending, SessionEnding::SignOut);
            c.fetch_add(1, Ordering::SeqCst);
        }));
        engine.sign_out_keeping_local_data().expect("signed out");
        assert_eq!(called.load(Ordering::SeqCst), 1);
        assert!(
            files_under(&state_dir.join("office-recovery")) < blobs_before,
            "the account's folder of blobs is gone"
        );
        assert!(engine.office_snapshots_for(session).is_err());
        assert_eq!(files_under(&state_dir.join("office-scratch")), 0, "no scratch file");
    }
}
