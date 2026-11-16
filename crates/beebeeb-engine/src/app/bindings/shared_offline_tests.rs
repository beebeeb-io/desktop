//! `shared_kept_in`, the read behind `Engine::shared_kept_of_for`, on a state database of the test's own: it names
//! only the rows that are shared with the account AND kept. Every name is invented.

use super::shared_kept_in;
use crate::state_db::{FileEntry, FileStatus, ItemKind, Namespace, PERMISSION_READ, StateDb};

fn put(db: &StateDb, id: &str, path: &str, shared: bool) {
    let entry = FileEntry {
        file_id: id.into(),
        path: path.into(),
        status: FileStatus::CloudOnly,
        size_bytes: 10,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: ItemKind::File,
    };
    db.upsert_file_with_contract(&entry, |contract| {
        contract.item_kind = ItemKind::File;
        if shared {
            contract.namespace = Namespace::SharedWithMe;
            contract.shared_root_id = Some(id.into());
            contract.permission_bits = PERMISSION_READ;
        }
    })
    .unwrap();
}

/// A kept shared file, a shared file not kept, a kept own file, and an id with no row: only the first is named.
/// The namespace filter and the kept filter each keep one of the others out.
#[test]
fn shared_kept_of_names_only_kept_shared_rows() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    put(&db, "s-kept", "Shared with me/a.txt", true);
    put(&db, "s-free", "Shared with me/b.txt", true);
    put(&db, "own-kept", "c.txt", false);
    db.set_recursive_pin("s-kept", true, 1).unwrap();
    db.set_recursive_pin("own-kept", true, 1).unwrap();
    let ask: Vec<String> = ["s-kept", "s-free", "own-kept", "no-such-row"]
        .iter()
        .map(|id| id.to_string())
        .collect();
    let named = shared_kept_in(&db, &ask).unwrap();
    assert_eq!(named.len(), 1, "{named:?}");
    assert!(named.contains("s-kept"), "{named:?}");
    assert!(shared_kept_in(&db, &[]).unwrap().is_empty());
}
