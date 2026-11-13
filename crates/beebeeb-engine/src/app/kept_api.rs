//! What this PC keeps (Available offline), for a host without a UI toolkit (the native This PC section and Keep on
//! this PC). One change: [`Engine::set_kept`], the same pin change the 0.8 app's "Available offline" toggle makes
//! (`EngineBridge::set_recursive_pin`: the state database, the queued pin for other devices, and on Windows the
//! placeholder's pin state).
//!
//! BLOCKS (the state database, and on Windows a Cloud Files call): call it from a worker thread, never from the UI
//! thread. Must not: log a name or a path.

use std::sync::Arc;

use super::{CredentialStores, Engine};
use crate::api_client::ApiClient;
use crate::engine_bridge::EngineBridge;

impl<C: CredentialStores> Engine<C> {
    /// Keep `file_id` (a file, or a folder with everything in it, also what is added later) on this PC, or stop
    /// keeping it: it stays in Beebeeb, and its downloaded copies may leave this PC again. `Err` in fixed words: no
    /// sync folder set, nothing synced yet, or the change was refused.
    pub fn set_kept(&self, file_id: &str, kept: bool) -> Result<(), String> {
        let config = crate::config::DesktopConfig::load_for(self.identity)?;
        let sync_root = config
            .sync_root
            .ok_or_else(|| "no sync folder is set on this computer".to_string())?;
        let db = Arc::new(self.state_db()?);
        // The pin change reads and writes the state database only (and the OS pin state): no request, so the
        // client carries no token and no key, as the 0.8 app's command builds it.
        let api = Arc::new(ApiClient::new(self.config.api_base.clone(), String::new(), [0u8; 32]));
        EngineBridge::new(self.identity, db, api)
            .set_recursive_pin(&sync_root, file_id, kept)
            .map(|_| ())
            .map_err(|_| "could not change what is kept on this computer".to_string())
    }
}
