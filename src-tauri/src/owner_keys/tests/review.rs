//! T3 review fixes: a block persisted by any build excludes a row, and a
//! foreign row whose key does not resolve is shown as unavailable, never under
//! a server-supplied plaintext name.

use super::*;
use beebeeb_core::chunk_stream::ExpectedChunkFormat;

const FOREIGN: &str = "20000000-0000-8000-8000-0000000000f1";
const OWNER: &str = "10000000-0000-4000-8000-0000000000r2";

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
