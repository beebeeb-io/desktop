//! Stub: domain removal (dm-23): `remove_keeping_unsynced`. Owned by a later task; nothing here yet.

use beebeeb_engine::finder_setup::error::{app_code, FpError};

/// Removes the Finder location, keeping what never reached the server. Not built yet: answers an app error.
#[allow(dead_code)]
pub(crate) fn remove_keeping_unsynced() -> Result<(), FpError> {
    Err(FpError::app(
        app_code::UNEXPECTED_BRIDGE_RETURN,
        "removing the Finder location is not built yet",
    ))
}
