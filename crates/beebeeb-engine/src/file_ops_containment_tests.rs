//! Tests that `file_ops::full_path` keeps every sync-root path inside the root. Ported from
//! `src-tauri/src/drive_v2/tests.rs` (`drive_v2_open_and_reveal_stay_inside_the_sync_root`).

use super::*;
use crate::drive_v2::DriveError;

fn root() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("Docs")).unwrap();
    dir
}

#[test]
fn drive_v2_open_and_reveal_stay_inside_the_sync_root() {
    let dir = root();
    let root = dir.path();
    assert!(full_path(root, "Docs/notes.txt").is_ok());
    assert!(
        full_path(root, "/etc/passwd").unwrap().starts_with(root),
        "a state DB path's leading slash stays inside"
    );
    for bad in ["../outside.txt", "Docs/../../x", "C:\\x", "a\\..\\..\\x"] {
        assert!(full_path(root, bad).is_err(), "{bad}");
    }
    assert_eq!(full_path(root, "../outside.txt").unwrap_err(), DriveError::NotFound);
}

/// Not in the Tauri file: the empty path is the root itself, and a symlink inside the root that points
/// outside is refused (the containment check follows links).
#[test]
fn the_empty_path_is_the_root_and_a_link_out_of_it_is_refused() {
    let dir = root();
    let root = dir.path();
    assert_eq!(full_path(root, "").unwrap(), root);
    assert_eq!(full_path(root, "/").unwrap(), root);
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("link")).unwrap();
        assert_eq!(full_path(root, "link/secret.txt").unwrap_err(), DriveError::NotFound);
    }
}
