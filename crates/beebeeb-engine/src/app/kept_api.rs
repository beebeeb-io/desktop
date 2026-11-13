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
        // The account-bound handle (task 2488): the pin is changed in the signed-in account's state alone, and
        // nothing is written into a state before it is proven that account's.
        let state = self.local_state_for_changes().map_err(|refused| {
            if refused.not_this_accounts() {
                NOT_YOURS.to_string()
            } else {
                NO_LOCAL_STATE.to_string()
            }
        })?;
        state
            .with_shared(|db| pin(self.identity, &self.config.api_base, db, &sync_root, file_id, kept))
            .map_err(|_| NOT_SIGNED_IN.to_string())?
    }
}

impl<C: CredentialStores> Engine<C> {
    /// Make every downloaded copy that is not kept online only again, in the signed-in account's local state
    /// (the account-bound handle, task 2488): the bytes freed. Nothing is touched for a state that is not handed
    /// out. BLOCKS: a worker's.
    pub fn free_up_space(&self) -> Result<u64, String> {
        let Some(sync_root) = self.configured_sync_root()? else {
            return Ok(0);
        };
        let state = match self.local_state_for_changes() {
            Ok(state) => state,
            // Nothing synced on this computer: nothing to free (as the unbound reclaim answers a missing database).
            Err(super::LocalStateRefused::NothingSynced) => return Ok(0),
            Err(refused) if refused.not_this_accounts() => return Err(NOT_YOURS.to_string()),
            Err(_) => return Err(NO_LOCAL_STATE.to_string()),
        };
        // The session is asked again before each file: a lock or a sign-out while this runs ends it there. What was
        // freed until then was freed, and is the answer.
        let mut reclaimed = None;
        let whole = state.with_while(|db, go_on| {
            reclaimed = Some(crate::free_up_space::free_up_space_in(
                self.identity,
                db,
                &sync_root,
                go_on,
            ));
        });
        match (whole, reclaimed) {
            (_, Some(reclaimed)) => reclaimed.map(|freed| freed.bytes_freed),
            (_, None) => Err(NOT_SIGNED_IN.to_string()),
        }
    }
}

impl<C: CredentialStores> Engine<C> {
    /// Free up space for a selection: the downloaded copies of `ids` (each a file, or a folder with everything in
    /// it) become online only, in the signed-in account's local state (the account-bound handle, task 2488). Per
    /// id, in the order asked: what the rows say of it after the reclaim (`free_up_space::SelectionAnswer`). A kept file
    /// and a file with a change that is not sent are never touched.
    /// The session is asked again before each file; an answer read across a session change is dropped (`Err`).
    /// BLOCKS: a worker's.
    pub fn free_up_space_of(&self, ids: &[String]) -> Result<Vec<crate::free_up_space::SelectionAnswer>, String> {
        if self.view().gate != GateState::Ready {
            return Err(NOT_SIGNED_IN.to_string());
        }
        let Some(sync_root) = self.configured_sync_root()? else {
            return Err(NO_SYNC_FOLDER.to_string());
        };
        let state = self.local_state_for_changes().map_err(|refused| {
            if refused.not_this_accounts() {
                NOT_YOURS.to_string()
            } else {
                NO_LOCAL_STATE.to_string()
            }
        })?;
        state
            .with_while(|db, go_on| crate::free_up_space::free_up_space_of(self.identity, db, &sync_root, ids, go_on))
            .map_err(|_| NOT_SIGNED_IN.to_string())?
    }
}

pub const NO_SYNC_FOLDER: &str = "no sync folder is set on this computer";

/// `set_kept`'s refusals before anything is read or written.
pub const NOT_SIGNED_IN: &str = "not signed in";
pub const NOT_YOURS: &str = "the local state on this computer is another account's";
pub const NO_LOCAL_STATE: &str = "the local state on this computer could not be opened";

/// The pin change itself, on a state database already open.
fn pin(
    identity: &'static crate::identity::EngineIdentity,
    api_base: &str,
    db: Arc<crate::state_db::StateDb>,
    sync_root: &Path,
    file_id: &str,
    kept: bool,
) -> Result<(), String> {
    // The pin change reads and writes the state database only (and the OS pin state): no request, so the client
    // carries no token and no key, as the 0.8 app's command builds it.
    let api = Arc::new(ApiClient::new(api_base.to_string(), String::new(), [0u8; 32]));
    EngineBridge::new(identity, db, api)
        .set_recursive_pin(sync_root, file_id, kept)
        .map(|_| ())
        .map_err(|_| "could not change what is kept on this computer".to_string())
}
