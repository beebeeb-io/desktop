//! macOS: the File Provider bridge (`packaging/macos/FileProviderBridge.m`, compiled by `build.rs`) and its Rust
//! layer. One file per owner: `bridge` the C calls and their decoding, `ports` the Finder-setup ports, `removal` the
//! domain removal, `open` Open/Reveal in Finder, `materialise` the one call that makes Finder's copy of an item
//! present (dm-75), `lifecycle_log` the lifecycle log. Must not decide what Finder
//! shows (the engine does). File Provider logs only through `ExtensionLog`; nothing here logs a path, a name or OS
//! error text.

pub(crate) mod bridge;
pub(crate) mod lifecycle_log;
pub(crate) mod materialise;
pub(crate) mod open;
pub(crate) mod ports;
pub(crate) mod removal;

use beebeeb_engine::file_provider_port::{FileProviderPort, WorkingSetSignalOutcome};
use beebeeb_engine::finder_setup::error::FpError;

/// The engine's port over the bridge: one working-set signal, through the one bridge gate, and (dm-72) the read of
/// Finder's copy of an item.
pub struct FileProviderBridgePort;

impl FileProviderPort for FileProviderBridgePort {
    fn signal_working_set(&self) -> Result<WorkingSetSignalOutcome, FpError> {
        bridge::signal_working_set()
    }

    fn with_finder_copy(&self, item_id: &str, read: &mut dyn FnMut(&std::path::Path)) -> bool {
        bridge::with_finder_copy(item_id, read)
    }
}
