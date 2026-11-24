//! Whether a file's bytes are all on this PC, read from its Windows attributes only: a Cloud Files placeholder whose
//! bytes are still in the cloud carries `RECALL_ON_DATA_ACCESS`, `RECALL_ON_OPEN` or `OFFLINE`, and OPENING it fetches
//! them. Asking the attributes fetches nothing. One mask for every reader that must never hydrate (the found-while-
//! paused walk, the photo-preview maker).

/// `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`, `FILE_ATTRIBUTE_RECALL_ON_OPEN`, `FILE_ATTRIBUTE_OFFLINE`.
pub const NOT_HERE: u32 = 0x0040_0000 | 0x0004_0000 | 0x0000_1000;

/// Whether attributes say some of the file's bytes are not on this PC.
pub fn bytes_not_here(attributes: u32) -> bool {
    attributes & NOT_HERE != 0
}

/// Whether `path` is a file whose bytes are all here (never a folder, never a link). Windows: its attributes, so
/// nothing is fetched to answer. Other systems have no placeholders: a plain file is all here.
pub fn file_bytes_here(path: &std::path::Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.is_file() && !bytes_not_here(metadata.file_attributes())
    }
    #[cfg(not(target_os = "windows"))]
    {
        metadata.is_file()
    }
}
