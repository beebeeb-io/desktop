//! New file, listed at once (w36): the file `file_ops::create_file` made is given its row and its upload now, under
//! the signed-in session, not when the folder scan next finds it.

use std::sync::Arc;

use super::{CredentialStores, Engine};
use crate::api_client::ApiClient;
use crate::engine_bridge::EngineBridge;

impl<C: CredentialStores> Engine<C> {
    /// List and queue the new file at `rel` (a state DB path `file_ops::create_file` returned) now, and ask the
    /// runner to send it. `false`: not done here (no session, no sync root, or the file was not queued); the folder
    /// scan finds the file as before.
    pub fn list_new_file(&self, rel: &str) -> bool {
        let Ok(state) = self.local_state_for_changes() else {
            return false;
        };
        let Ok((token, key)) = self.edit_credentials(state.session()) else {
            return false;
        };
        let Ok(sync_root) = self.sync_root() else {
            return false;
        };
        let queued = state
            .with_shared(|db| {
                let api = Arc::new(ApiClient::new(self.config.api_base.clone(), token, *key));
                let bridge = EngineBridge::new(self.identity, db, api);
                crate::file_ops::list_new_file(&bridge, &sync_root, rel)
            })
            .unwrap_or(false);
        if queued {
            self.nudge_runner();
        }
        queued
    }
}
