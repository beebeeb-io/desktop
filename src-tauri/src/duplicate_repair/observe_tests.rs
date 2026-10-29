//! Task 2274, plan Task 2 on Windows: the list's platform read (`list::observe`) on a real sync root, the 2270
//! fixture's (`windows_cf::finalization_tests`): a FETCH_DATA callback that serves known bytes and counts every
//! fetch by identity. Spec phase 1 fetches nothing: every test ends with no fetch for its files.
use super::*;
use crate::duplicate_repair::list::{Observed, list_pairs, observe};
use crate::duplicate_repair::model::Class;
use crate::state_db::{FileEntry, FileStatus, ItemKind};
use crate::unknown_check::{OnDisk, PlatformView};
use std::os::windows::fs::MetadataExt;

fn row(db: &crate::state_db::StateDb, id: &str, path: &str, local_hash: Option<&str>) {
    db.upsert_file(&FileEntry {
        file_id: id.into(),
        path: path.into(),
        status: if local_hash.is_some() { FileStatus::Local } else { FileStatus::CloudOnly },
        size_bytes: 10,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    if let Some(hash) = local_hash {
        db.record_local_baseline(id, 1, hash).unwrap();
    }
}

#[test]
fn a_read_only_cloud_only_placeholder_is_observed_without_a_fetch() {
    let root = Root::new();
    let path = root.path.join("a.bin");
    let bytes = b"only on the server".repeat(4000);
    serve("o1-a", &bytes);
    cloud_only(&path, &bytes, "o1-a");
    make_read_only(&path);
    let metadata = std::fs::symlink_metadata(&path).unwrap();
    assert_eq!(
        observe(&path),
        Observed {
            view: placeholder(OnDisk::Nothing, true, "o1-a"),
            size: Some(bytes.len() as u64),
            modified: Some(metadata.last_write_time()),
            read_only: Some(true),
        }
    );
    settle();
    assert_eq!(fetches("o1-"), 0, "phase 1 fetches nothing");
}

#[test]
fn a_placeholder_wholly_on_disk_and_a_plain_file_are_observed_as_they_are() {
    let root = Root::new();
    let held = root.path.join("held.txt");
    std::fs::write(&held, b"bytes on this PC").unwrap();
    placeholders::convert_to_in_sync_placeholder(&held, "o2-held").unwrap();
    let observed = observe(&held);
    assert_eq!(observed.view, placeholder(OnDisk::Wholly, true, "o2-held"));
    assert_eq!((observed.size, observed.read_only), (Some(16), Some(false)));
    let plain = root.path.join("plain.txt");
    std::fs::write(&plain, b"never a placeholder").unwrap();
    let observed = observe(&plain);
    assert_eq!(observed.view, PlatformView::PlainFile);
    assert_eq!((observed.size, observed.read_only), (Some(19), Some(false)));
    settle();
    assert_eq!(fetches("o2-"), 0);
}

#[test]
fn nothing_at_the_path_is_observed_as_missing() {
    let root = Root::new();
    assert_eq!(
        observe(&root.path.join("gone.txt")),
        Observed { view: PlatformView::Missing, size: None, modified: None, read_only: None }
    );
}

/// The list end to end on a real root: two rows at one path, the file there a cloud-only placeholder bound to the
/// row with known content. Listed for the server read (L10 pending), and nothing fetched.
#[test]
fn the_list_reads_a_real_pair_without_a_fetch() {
    let root = Root::new();
    let db = root.db();
    std::fs::create_dir(root.path.join("Docs")).unwrap();
    let bytes = b"the copy this PC is bound to".repeat(500);
    serve("o4-new", &bytes);
    cloud_only(&root.path.join("Docs").join("a.txt"), &bytes, "o4-new");
    row(&db, "o4-old", "Docs/a.txt", None);
    row(&db, "o4-new", "Docs/a.txt", Some(&sha256(&bytes)));
    let listing = list_pairs(&db, &root.path, &observe).unwrap();
    assert_eq!(listing.pairs.len(), 1);
    let pair = &listing.pairs[0];
    assert_eq!((pair.a.file_id.as_str(), pair.b.file_id.as_str(), pair.class), ("o4-new", "o4-old", Class::Unknown));
    assert_eq!((pair.view.kind.as_str(), pair.view.on_disk.as_deref()), ("placeholder", Some("nothing")));
    settle();
    assert_eq!(fetches("o4-"), 0, "phase 1 fetches nothing");
}
