//! Explorer's "Free up space" command (the Windows Cloud Files status flyout, `windows_cf::status_ui`), decided here
//! on every OS so the rule is tested everywhere (task 2490).
//!
//! The command acts on the signed-in account's local state alone. Whether it may is asked of the engine handle that
//! [`serve`] was given, at the moment Explorer asks (its `State`) and again when it is clicked (its `Invoke`), never
//! when the flyout was built: Explorer may keep a command object across a lock or a sign-out. The engine answers
//! through the account-bound handle (`app::owned_state`): a signed-in, unlocked session whose account owns the local
//! state. Anything else (signed out, locked, a session the server ended, another account, no engine handle, an
//! engine handle that is gone) is Disabled, and a click frees nothing and changes no row.
//!
//! The binding Explorer finds the command by is removed by the engine at every stop, lock and sign-out
//! ([`ExplorerBinding`]); a later engine start registers it again when it connects the sync folder.
//!
//! Must not: hold the slot's lock while the engine is asked; open the state database itself; log a name or a path.

use std::path::Path;
use std::sync::{Arc, Mutex, Weak};

use crate::app::LocalStateRefused;
use crate::identity::EngineIdentity;

/// What the command asks of the engine handle. The engine's answers are the account-bound handle's.
pub trait ReclaimOwner: Send + Sync {
    /// `None` while a signed-in, unlocked session's account owns the local state.
    fn refusal(&self) -> Option<LocalStateRefused>;
    /// Make the downloaded copies that are not kept online only again, in that account's local state: the bytes freed.
    fn free_up_space(&self) -> Result<u64, String>;
}

/// The command's state, as Explorer shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandState {
    Enabled,
    Disabled,
}

/// What a click came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invoked {
    /// Nothing was freed and no row changed; the refusal's fixed word.
    Refused(&'static str),
    Freed(u64),
    /// The reclaim was asked and did not finish (its error is not passed on: it may name a file).
    Failed,
}

/// The engine handle the command asks: a weak reference, so the command never keeps an engine alive, and an engine
/// that is gone answers as none.
static SERVED: Mutex<Option<Weak<dyn ReclaimOwner>>> = Mutex::new(None);

/// The command asks `owner` from now on (the last one served wins).
pub fn serve(owner: Weak<dyn ReclaimOwner>) {
    *SERVED.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(owner);
}

/// The engine handle served, while it lives. The slot's lock is released before the caller asks it anything.
pub fn served() -> Option<Arc<dyn ReclaimOwner>> {
    SERVED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .and_then(Weak::upgrade)
}

/// Enabled only while `owner` hands out the local state.
pub fn state_of(owner: Option<&dyn ReclaimOwner>) -> CommandState {
    match owner {
        Some(owner) if owner.refusal().is_none() => CommandState::Enabled,
        _ => CommandState::Disabled,
    }
}

/// A click: asked again, then the reclaim through the engine handle (which proves the owner once more).
pub fn invoke(owner: Option<&dyn ReclaimOwner>) -> Invoked {
    let Some(owner) = owner else {
        return Invoked::Refused("no_engine");
    };
    if let Some(refused) = owner.refusal() {
        return Invoked::Refused(refused.kind());
    }
    match owner.free_up_space() {
        Ok(freed) => Invoked::Freed(freed),
        Err(_) => Invoked::Failed,
    }
}

/// Where Explorer finds the command for a sync folder. The engine removes it at a stop, a lock and a sign-out.
pub trait ExplorerBinding: Send + Sync {
    fn remove(&self, sync_root: &Path);
}

/// The OS's binding: the status flyout's registry binding on Windows; no other OS has one.
pub struct OsExplorerBinding {
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    identity: &'static EngineIdentity,
}

impl OsExplorerBinding {
    pub fn new(identity: &'static EngineIdentity) -> Self {
        OsExplorerBinding { identity }
    }
}

impl ExplorerBinding for OsExplorerBinding {
    #[cfg(target_os = "windows")]
    fn remove(&self, sync_root: &Path) {
        crate::windows_cf::status_ui::remove_for_sync_root(self.identity, sync_root);
    }

    #[cfg(not(target_os = "windows"))]
    fn remove(&self, _sync_root: &Path) {}
}

#[cfg(test)]
#[path = "explorer_command_tests.rs"]
mod tests;
