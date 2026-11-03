//! T3 review fixes: a block persisted by any build excludes a row, and a
//! foreign row whose key does not resolve is shown as unavailable, never under
//! a server-supplied plaintext name.

use super::*;
use beebeeb_core::chunk_stream::ExpectedChunkFormat;

const FOREIGN: &str = "20000000-0000-8000-8000-0000000000f1";
const OWNER: &str = "10000000-0000-4000-8000-0000000000b2";

#[test]
fn review_p3_1_a_persisted_block_matches_after_the_fingerprint_grew() {
    let master = MasterKey::from_bytes([7; 32]);
    let key = beebeeb_core::kdf::derive_file_key(&master, OWNER.as_bytes());
    let name = beebeeb_core::encrypt::encrypt_name_for(&key, "notes.txt", None, ExpectedChunkFormat::Unrecorded).unwrap();
    let record = OwnedKeyRecord {
        file_id: OWNER.into(),
        name_encrypted: name,
        // A fingerprint from before `request_row` joined the record: it can
        // never equal today's fingerprint, yet the block must still hold.
        blocked_fingerprint: Some("pre-upgrade".into()),
        ..Default::default()
    };
    assert!(
        record_key(&master, &record, beebeeb_core::owned_keys::ItemKind::File).is_err(),
        "any persisted block hides the row"
    );
}

#[test]
fn review_p2_9_a_foreign_row_without_a_key_is_unavailable() {
    let temp = tempfile::tempdir().unwrap();
    let db = StateDb::open(temp.path().join("db")).unwrap();
    db.put_owned_key_record(&OwnedKeyRecord {
        file_id: FOREIGN.into(),
        name_encrypted: r#"{"plaintext":"server-chosen.txt"}"#.into(),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        stored_name(&db, FOREIGN, &[7; 32]).unwrap(),
        Some(blocked_name(FOREIGN)),
        "a foreign id never falls back to a derived key and the server's plaintext name"
    );
}

#[test]
fn review_p3_4_a_plaintext_folder_row_is_not_read_as_a_file() {
    use crate::state_db::{FileEntry, FileStatus, ItemKind};
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(StateDb::open(temp.path().join("db")).unwrap());
    let folder = "40000000-0000-4000-8000-0000000000d4";
    let entry = FileEntry {
        file_id: folder.into(),
        path: "Shared".into(),
        status: FileStatus::Local,
        size_bytes: 0,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: ItemKind::Folder,
    };
    db.upsert_file_with_contract(&entry, |contract| {
        contract.item_kind = ItemKind::Folder;
    })
    .unwrap();
    db.put_owned_key_record(&OwnedKeyRecord {
        file_id: folder.into(),
        name_encrypted: "Shared".into(),
        ..Default::default()
    })
    .unwrap();
    let fixture = Fixture::new(|_| None);
    let bridge = crate::engine_bridge::EngineBridge::new(db, Arc::new(fixture.api()));
    let err = match run(bridge.resolved_owned_key(folder)) {
        Ok(_) => panic!("a plaintext owner name never resolves"),
        Err(err) => err.to_string(),
    };
    assert!(
        !err.contains("identified by its first content chunk"),
        "the stored kind is Folder, so core must not read the row as a file: {err}"
    );
}

#[test]
fn review_p3_5_equal_keys_compare_equal_and_one_flipped_byte_does_not() {
    let master = MasterKey::from_bytes([7; 32]);
    let left = beebeeb_core::kdf::derive_file_key(&master, b"left");
    let right = beebeeb_core::kdf::derive_file_key(&master, b"left");
    assert!(same_key(&left, &right), "the same key is the same key");
    let mut bytes = *right.as_bytes();
    bytes[17] ^= 1;
    let other = beebeeb_core::kdf::FileKey::from_bytes(bytes);
    assert!(!same_key(&left, &other), "one flipped byte is a different key");
}
