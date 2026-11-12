//! Backups on the real engine: the one rule that needs the app's identity.
//!
//! There is no engine-backed `BackupsSource` yet, on purpose: the engine handle has no backup reads (devices,
//! folders, the last pass), and the trait's `Status` can only say "all backed up" or "backing up", so a source
//! built today would have to claim one of them. With a real engine the window shows Backups as "not built yet"
//! (`BackupsUi::new(None)`), never the board's sample devices. What IS decided here, for both apps, is
//! [`may_back_up`]: the sync folder is never a backup folder, and whose sync folder that is comes from the
//! identity (the fixture source compares with one fixed name, which is wrong for the test build). The
//! engine-backed source's `add_folder` is this function.

use std::path::{Component, Path};

use beebeeb_engine::identity::EngineIdentity;

/// Whether `path` may be chosen as a backup folder by the app `identity`, whose sync folder is `sync_root`.
/// Refused: the sync folder itself, anything inside it, and any folder with the identity's own sync-folder name
/// in its path (the suggested location, also before sync was ever started). Names compare without case, as
/// Windows does.
#[cfg_attr(not(test), allow(dead_code))] // its caller is the engine-backed source (module comment)
pub fn may_back_up(identity: &EngineIdentity, sync_root: Option<&Path>, path: &Path) -> bool {
    let same =
        |a: &std::ffi::OsStr, b: &std::ffi::OsStr| a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy());
    let own_name = std::ffi::OsStr::new(identity.sync_folder_name());
    let names_own_folder = path
        .components()
        .any(|part| matches!(part, Component::Normal(name) if same(name, own_name)));
    let inside_root = sync_root.is_some_and(|root| {
        let (root, path): (Vec<_>, Vec<_>) = (root.components().collect(), path.components().collect());
        !root.is_empty()
            && path.len() >= root.len()
            && root.iter().zip(&path).all(|(a, b)| same(a.as_os_str(), b.as_os_str()))
    });
    !names_own_folder && !inside_root
}
