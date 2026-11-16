//! The file id a placeholder's identity names, read by its place on disk: the prune of two server items with one name
//! (`drive_v2::case_place::removable_by_identity`) removes the entry only when it is the pruned row's own.

use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;

use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE,
};

/// The file id the placeholder at `path` (a file or a folder) names (`placeholders::handle_identity`), read through a
/// handle opened for its attributes only, on the entry itself (never a recall), sharing every access. `None`: it is not
/// a placeholder, it names no id, or it could not be opened.
pub fn identity_at(path: &Path) -> Option<String> {
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0 | FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(path)
        .ok()?;
    super::placeholders::handle_identity(&file).1
}
