//! Insights' kinds and the sidebar's split are one definition (`model::storage_category`): Backups, photos and
//! videos, the rest. Invented names.

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
fn storage_insights_count_backups_photos_and_the_rest_like_the_sidebar() {
    let files = [
        entry("a", "Invented plan.pdf", 500),
        entry("b", "Invented song.mp3", 700),
        entry("c", "Trip/Invented shot.jpg", 30_000),
        entry("d", "Invented clip.mov", 200_000),
        entry("e", "Backups/Invented phone/Invented backed.jpg", 9_000_000),
    ];
    let refs: Vec<&FileEntry> = files.iter().collect();
    let roots = vec!["backups".to_string()];
    let Insights::Ready(data) = insights_of(&refs, &roots, (9_231_200, 10_000_000)) else { panic!("ready") };
    let says = |name: &str| data.kinds.iter().find(|k| k.name == name).map(|k| k.says.clone());
    assert_eq!(says("Backups").as_deref(), Some("9 MB · 1 file"), "backup bytes are not dropped");
    assert_eq!(says("Media").as_deref(), Some("230 KB · 2 files"), "photos and videos only");
    // Audio is a file now, as in the sidebar's split.
    assert_eq!(says("Other").as_deref(), Some("700 bytes · 1 file"));
    assert_eq!(data.totals, "5 files · 9.2 MB");
}
