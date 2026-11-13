//! What this PC keeps (Available offline), for a host without a UI toolkit (the native This PC section and Keep on
//! this PC). One change: [`Engine::set_kept`], the same pin change the 0.8 app's "Available offline" toggle makes
//! (`EngineBridge::set_recursive_pin`: the state database, the queued pin for other devices, and on Windows the
//! placeholder's pin state).
//!
//! BLOCKS (the state database, and on Windows a Cloud Files call): call it from a worker thread, never from the UI
//! thread. Must not: log a name or a path.

use std::path::Path;
use std::sync::Arc;

use super::{CredentialStores, Engine};
use crate::api_client::ApiClient;
use crate::engine_bridge::EngineBridge;
use crate::signin::gate::GateState;

#[cfg(test)]
#[path = "kept_api_tests.rs"]
mod tests;

impl<C: CredentialStores> Engine<C> {
    /// Keep `file_id` (a file, or a folder with everything in it, also what is added later) on this PC, or stop
    /// keeping it: it stays in Beebeeb, and its downloaded copies may leave this PC again. `Err` in fixed words: no
    /// one signed in with the vault open, the local state is another account's (task 2488), no sync folder set,
    /// nothing synced yet, or the change was refused.
    pub fn set_kept(&self, file_id: &str, kept: bool) -> Result<(), String> {
        if self.view().gate != GateState::Ready {
            return Err(NOT_SIGNED_IN.to_string());
        }
        let config = crate::config::DesktopConfig::load_for(self.identity)?;
        let sync_root = config
            .sync_root
            .ok_or_else(|| "no sync folder is set on this computer".to_string())?;
        let db = self.state_db()?;
        if !self.owns_local_state(&db) {
            return Err(NOT_YOURS.to_string());
        }
        pin(self.identity, &self.config.api_base, db, &sync_root, file_id, kept)
    }
}

/// `set_kept`'s refusals before anything is read or written.
pub const NOT_SIGNED_IN: &str = "not signed in";
pub const NOT_YOURS: &str = "the local state on this computer is another account's";

/// The pin change itself, on a state database already open.
fn pin(
    identity: &'static crate::identity::EngineIdentity,
    api_base: &str,
    db: crate::state_db::StateDb,
    sync_root: &Path,
    file_id: &str,
    kept: bool,
) -> Result<(), String> {
    // The pin change reads and writes the state database only (and the OS pin state): no request, so the client
    // carries no token and no key, as the 0.8 app's command builds it.
    let api = Arc::new(ApiClient::new(api_base.to_string(), String::new(), [0u8; 32]));
    EngineBridge::new(identity, Arc::new(db), api)
        .set_recursive_pin(sync_root, file_id, kept)
        .map(|_| ())
        .map_err(|_| "could not change what is kept on this computer".to_string())
}
