//! Insights' largest files carry their file id, so a press can show the file in Files (audit S4).

use super::*;
use crate::frames::settings::drive::Insights;
use beebeeb_engine::state_db::{FileEntry, FileStatus, ItemKind};

fn entry(id: &str, path: &str, size: i64) -> FileEntry {
    FileEntry {
        file_id: id.into(),
        path: path.into(),
        status: FileStatus::CloudOnly,
        size_bytes: size,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: ItemKind::File,
    }
}

#[test]
fn the_largest_files_carry_their_file_id() {
    let (a, b) = (entry("f-big", "Backups/Archive.zip", 900), entry("f-small", "Notes.txt", 5));
    let Insights::Ready(data) = insights_of(&[&b, &a], (905, 1000)) else { panic!("ready") };
    assert_eq!(data.largest[0].id, "f-big");
    assert_eq!(data.largest[1].id, "f-small");
}

