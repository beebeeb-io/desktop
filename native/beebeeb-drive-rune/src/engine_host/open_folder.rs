//! The Beebeeb folder on this PC as a path: the one place the menu's and the tray's "Open in File Explorer" ask.

use std::path::PathBuf;

use beebeeb_engine::app::{CredentialStores, Engine};

/// Why there is no folder to open: sync is not running for this handle, or no sync root is set yet. The Files
/// source's own refusals for the same read (`local`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct NoFolder;

/// The sync root, read as the Files source's `local` reads it: refused while sync is not running.
pub(super) fn beebeeb_folder<C: CredentialStores>(engine: &Engine<C>) -> Result<PathBuf, NoFolder> {
    if !engine.is_syncing() {
        return Err(NoFolder);
    }
    engine.sync_root().map_err(|_| NoFolder)
}
