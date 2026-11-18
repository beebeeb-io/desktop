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
#[path = "office_session_end_tests.rs"]
mod office_session_end_tests;
