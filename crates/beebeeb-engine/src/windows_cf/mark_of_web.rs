//! Mark of the Web for files that landed from a share someone else owns: an NTFS `Zone.Identifier` stream with
//! `ZoneId=3` (Internet), so SmartScreen and Office Protected View treat them as downloads. Own files never get it.

#![cfg(target_os = "windows")]

use std::path::{Path, PathBuf};

use crate::state_db::{Namespace, StateDb};

/// The stream's content: the zone the file came from (3 = Internet).
pub const ZONE_STREAM_CONTENT: &str = "[ZoneTransfer]\r\nZoneId=3\r\n";

/// `<file>:Zone.Identifier`, the alternate data stream that holds the mark.
pub fn zone_stream_path(file: &Path) -> PathBuf {
    let mut os = file.as_os_str().to_os_string();
    os.push(":Zone.Identifier");
    PathBuf::from(os)
}

/// Whether the item belongs to a share another account owns (the shared-with-me namespace).
pub fn is_foreign_share(db: &StateDb, file_id: &str) -> bool {
    matches!(db.get_file_contract_state(file_id), Ok(Some(c)) if c.namespace == Namespace::SharedWithMe)
}

/// Write the mark on `file`. Best-effort for the caller: a failure is returned so it can be logged.
pub fn mark(file: &Path) -> std::io::Result<()> {
    std::fs::write(zone_stream_path(file), ZONE_STREAM_CONTENT)
}

/// Mark the file when, and only when, it came from a share someone else owns.
pub fn mark_if_foreign_share(db: &StateDb, file_id: &str, file: &Path) -> std::io::Result<bool> {
    if !is_foreign_share(db, file_id) {
        return Ok(false);
    }
    mark(file)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_writes_zone_id_3_on_the_stream_only() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.html");
        std::fs::write(&file, b"<html>").unwrap();
        mark(&file).unwrap();
        let stream = std::fs::read_to_string(zone_stream_path(&file)).unwrap();
        assert!(stream.contains("ZoneId=3"));
        assert_eq!(std::fs::read(&file).unwrap(), b"<html>");
        let own = dir.path().join("own.txt");
        std::fs::write(&own, b"x").unwrap();
        assert!(std::fs::read_to_string(zone_stream_path(&own)).is_err(), "an own file carries no mark");
    }

    #[test]
    fn stream_path_appends_the_stream_name() {
        assert_eq!(
            zone_stream_path(Path::new(r"C:\r\a.pyw")),
            PathBuf::from(r"C:\r\a.pyw:Zone.Identifier")
        );
    }
}
