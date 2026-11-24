//! dc-136c: which names the server can hold are readable here, and the retry of the unreadable ones when the keys
//! change. Driven through the function the sync runs for every listed row (`process_metadata_row`) and the listing
//! the window reads (`drive_v2::model::item_for_entry`), with no server: the address answers nothing and nothing
//! here makes a request.

use super::*;
use beebeeb_core::chunk_stream::{ExpectedChunkFormat, VersionCommitment};
use beebeeb_core::kdf::{MasterKey, derive_file_key, derive_file_subkey};

/// The key the names are sealed under.
const KEY: [u8; 32] = [36u8; 32];
/// A key that opens none of them: this PC before the right key arrives.
const OTHER_KEY: [u8; 32] = [63u8; 32];
const LISTED_AT: i64 = 1_000;

fn state_in(dir: &Path) -> Arc<StateDb> {
    Arc::new(StateDb::open(dir.join("state.db")).unwrap())
}

/// A bridge on `db` holding `key`. Two bridges on one state are this PC before and after a key change.
fn bridge_with(db: &Arc<StateDb>, key: [u8; 32]) -> EngineBridge {
    let api = Arc::new(ApiClient::new("http://127.0.0.1:9".into(), "token".into(), key));
    EngineBridge::new(&crate::identity::TEST, db.clone(), api)
}

/// A file at the top level, as the snapshot lists it, carrying `name_encrypted` as given.
fn node(file_id: &str, name_encrypted: &str, updated_at: i64) -> serde_json::Value {
    serde_json::json!({
        "id": file_id,
        "parent_id": serde_json::Value::Null,
        "name_encrypted": name_encrypted,
        "size_bytes": 3,
        "is_folder": false,
        "content_hash": serde_json::Value::Null,
        "version_number": 1,
        "is_trashed": false,
        "is_starred": false,
        "updated_at": updated_at,
    })
}

/// One listed row through the sync, as a snapshot applies it.
fn list(bridge: &EngineBridge, row: &serde_json::Value) {
    process_metadata_row(bridge, row, "", LISTED_AT, RowSource::Snapshot, &mut Vec::new()).unwrap();
}

/// The row as the window shows it: (name, whether it says its name could not be read).
fn shown(db: &StateDb, file_id: &str) -> (String, bool) {
    let row = db.get_file(file_id).unwrap().expect("the row is stored");
    let item = crate::drive_v2::model::item_for_entry(db, &row).unwrap();
    (item.name, item.name_unreadable == Some(true))
}

/// Whether the server's list was asked for again since the last call.
#[cfg(not(target_os = "macos"))]
fn list_asked_again(db: &StateDb) -> bool {
    db.take_needs_resnapshot().unwrap()
}

fn master(key: [u8; 32]) -> MasterKey {
    MasterKey::from_bytes(key)
}

/// The sealers below are fixtures for names other clients wrote, never a write of this desktop.
#[allow(deprecated)]
fn sealed_by_this_core(key: [u8; 32], file_id: &str, name: &str) -> String {
    beebeeb_core::encrypt::encrypt_name(&master(key), file_id, name, Some("text/plain")).unwrap()
}

/// A v1 envelope under the file's own key (no name subkey, no chunk format), in the native shape.
#[allow(deprecated)]
fn v1_native(file_key: &beebeeb_core::kdf::FileKey, plaintext: &str) -> String {
    serde_json::to_string(&beebeeb_core::encrypt::encrypt_metadata(file_key, plaintext).unwrap()).unwrap()
}

/// The same envelope in the web shape: `{nonce, ciphertext}` in base64, no `cipher_suite`.
#[allow(deprecated)]
fn v1_web(file_key: &beebeeb_core::kdf::FileKey, plaintext: &str) -> String {
    let blob = beebeeb_core::encrypt::encrypt_metadata(file_key, plaintext).unwrap();
    beebeeb_core::metadata_wire::serialize_encrypted_metadata(&blob.nonce, &blob.ciphertext)
}

fn name_json(name: &str) -> String {
    serde_json::json!({ "name": name, "mime_type": "text/plain" }).to_string()
}

/// Red test 1: a file from before names were encrypted carries its name as plain text. It is listed under that
/// name, like on the web, and says nothing about an unreadable name. dc-136b's check marked it and showed
/// "Unavailable file …".
#[test]
fn name_unreadable_never_marks_a_legacy_plaintext_name_file() {
    let dir = tempfile::tempdir().unwrap();
    let db = state_in(dir.path());
    let bridge = bridge_with(&db, KEY);
    let file = "13600000-0000-4000-8000-000000000001";

    list(&bridge, &node(file, "holiday 2019.jpg", LISTED_AT));

    assert_eq!(shown(&db, file), ("holiday 2019.jpg".to_string(), false));
    assert!(!db.name_unreadable(file).unwrap());
    assert_eq!(db.unreadable_name_count().unwrap(), 0);
    #[cfg(not(target_os = "macos"))]
    assert!(!list_asked_again(&db), "a readable name asks for no new list");

    // Listed again (any later snapshot): the same plain name is still the name.
    list(&bridge, &node(file, "holiday 2019.jpg", LISTED_AT + 1));
    assert_eq!(shown(&db, file), ("holiday 2019.jpg".to_string(), false));
}

/// Every shape of `name_encrypted` a client has written reads as the file's name: none is "unreadable".
#[test]
fn name_unreadable_marks_no_format_the_server_can_hold() {
    let dir = tempfile::tempdir().unwrap();
    let db = state_in(dir.path());
    let bridge = bridge_with(&db, KEY);
    let id = |n: u8| format!("13600000-0000-4000-8000-0000000000{n:02x}");
    let text_key = |file_id: &str| derive_file_key(&master(KEY), file_id.as_bytes());
    let version = VersionCommitment::new([7u8; beebeeb_core::chunk_stream::VERSION_ID_LEN], 1).unwrap();

    let mut cases: Vec<(String, String, &str)> = Vec::new();
    let mut add = |n: u8, what: &'static str, seal: &dyn Fn(&str) -> String| {
        let file_id = id(n);
        let sealed = seal(&file_id);
        cases.push((file_id, sealed, what));
    };
    add(0x10, "this core's writer", &|file_id| {
        sealed_by_this_core(KEY, file_id, "this core's writer")
    });
    add(0x11, "v1 native, id as text", &|file_id| {
        v1_native(&text_key(file_id), &name_json("v1 native, id as text"))
    });
    add(0x12, "v1 native, id as 16 bytes", &|file_id| {
        let uuid: uuid::Uuid = file_id.parse().unwrap();
        v1_native(
            &derive_file_key(&master(KEY), uuid.as_bytes()),
            &name_json("v1 native, id as 16 bytes"),
        )
    });
    add(0x13, "v1 web base64", &|file_id| {
        v1_web(&text_key(file_id), &name_json("v1 web base64"))
    });
    add(0x14, "v1 bare name", &|file_id| {
        v1_native(&text_key(file_id), "v1 bare name")
    });
    add(0x15, "v2 without a commitment", &|file_id| {
        beebeeb_core::encrypt::encrypt_name_for(
            &text_key(file_id),
            "v2 without a commitment",
            None,
            ExpectedChunkFormat::V2,
        )
        .unwrap()
    });
    add(0x16, "v2 with a commitment", &|file_id| {
        beebeeb_core::encrypt::encrypt_name_for_version(
            &text_key(file_id),
            "v2 with a commitment",
            None,
            ExpectedChunkFormat::V2,
            version,
        )
        .unwrap()
    });
    // A newer client's chunk format, authenticated under the name subkey: the content is refused elsewhere, the
    // name reads. dc-136b's check (the content pin's reader) marked it.
    add(0x17, "a newer chunk format", &|file_id| {
        let name_key = derive_file_subkey(&text_key(file_id), beebeeb_core::encrypt::NAME_KEY_INFO);
        v1_native(
            &name_key,
            &serde_json::json!({ "name": "a newer chunk format", "chunk_format": 9 }).to_string(),
        )
    });

    for (file_id, sealed, what) in &cases {
        list(&bridge, &node(file_id, sealed, LISTED_AT));
        assert_eq!(shown(&db, file_id), ((*what).to_string(), false), "{what}");
    }
    assert_eq!(db.unreadable_name_count().unwrap(), 0);
    assert_eq!(db.retry_unreadable_names().unwrap(), 0);
    #[cfg(not(target_os = "macos"))]
    assert!(
        !list_asked_again(&db),
        "with no marked row a key change asks for no new list"
    );
}

/// Only what no key opens is unreadable: an envelope sealed under another key, and a damaged one. The row is kept
/// (visible, deletable) under the stable label, marked, and the list is asked for once.
#[test]
fn name_unreadable_marks_only_an_envelope_no_key_opens() {
    let dir = tempfile::tempdir().unwrap();
    let db = state_in(dir.path());
    let bridge = bridge_with(&db, KEY);
    let other = "13600000-0000-4000-8000-000000000021";
    let damaged = "13600000-0000-4000-8000-000000000022";

    list(
        &bridge,
        &node(other, &sealed_by_this_core(OTHER_KEY, other, "secret.txt"), LISTED_AT),
    );
    list(
        &bridge,
        &node(
            damaged,
            r#"{"nonce":"AAAAAAAAAAAAAAAA","ciphertext":"AAAAAAAAAAAAAAAAAAAAAAAAAAAA"}"#,
            LISTED_AT,
        ),
    );

    for file in [other, damaged] {
        let (name, marked) = shown(&db, file);
        assert!(name.starts_with("Unavailable file "), "{name}");
        assert!(marked);
        assert_eq!(
            db.get_file(file).unwrap().unwrap().path,
            "",
            "a blob never becomes a path"
        );
    }
    assert_eq!(db.unreadable_name_count().unwrap(), 2);
    #[cfg(not(target_os = "macos"))]
    assert!(list_asked_again(&db));
}

/// Red test 2: a row listed before its key was here is unreadable; when the key arrives the retry asks for the
/// list again and the row gets its name and loses its mark.
#[test]
fn name_unreadable_row_becomes_readable_after_its_key_arrives() {
    let dir = tempfile::tempdir().unwrap();
    let db = state_in(dir.path());
    let file = "13600000-0000-4000-8000-000000000031";
    let row = node(file, &sealed_by_this_core(KEY, file, "report.pdf"), LISTED_AT);

    list(&bridge_with(&db, OTHER_KEY), &row);
    assert!(shown(&db, file).1, "listed without its key: unreadable");
    #[cfg(not(target_os = "macos"))]
    assert!(list_asked_again(&db));

    // The key arrives (unlock, rotation, an accepted share's key): the marked rows are tried again.
    assert_eq!(db.retry_unreadable_names().unwrap(), 1);
    #[cfg(not(target_os = "macos"))]
    assert!(
        list_asked_again(&db),
        "a key change with a marked row asks for the list again"
    );
    list(&bridge_with(&db, KEY), &row);

    assert_eq!(shown(&db, file), ("report.pdf".to_string(), false));
    assert_eq!(db.get_file(file).unwrap().unwrap().path, "report.pdf");
    assert_eq!(db.retry_unreadable_names().unwrap(), 0, "nothing is left to retry");
    #[cfg(not(target_os = "macos"))]
    assert!(!list_asked_again(&db));
}

/// The same for a file this PC holds under its last readable name: a rename it could not read is read once the key
/// is here, although the list's times for the row have not changed since (the snapshot's unchanged-row skip would
/// have left the old name and the mark for ever).
#[test]
fn name_unreadable_kept_name_is_replaced_after_its_key_arrives() {
    let dir = tempfile::tempdir().unwrap();
    let db = state_in(dir.path());
    let file = "13600000-0000-4000-8000-000000000041";
    let with_key = bridge_with(&db, KEY);
    list(
        &with_key,
        &node(file, &sealed_by_this_core(KEY, file, "notes.txt"), LISTED_AT),
    );
    db.set_status(file, FileStatus::Local).unwrap();

    // Renamed elsewhere; this PC reads the list without the key that opens the new name.
    let renamed = node(file, &sealed_by_this_core(KEY, file, "notes-2026.txt"), LISTED_AT + 500);
    list(&bridge_with(&db, OTHER_KEY), &renamed);
    assert_eq!(
        shown(&db, file),
        ("notes.txt".to_string(), true),
        "the last readable name is kept"
    );
    let held = db.get_file(file).unwrap().unwrap();
    assert_eq!(held.status, FileStatus::Local);
    assert_eq!(held.remote_updated_at, LISTED_AT + 500);

    assert_eq!(db.retry_unreadable_names().unwrap(), 1);
    list(&with_key, &renamed);

    assert_eq!(shown(&db, file), ("notes-2026.txt".to_string(), false));
    let healed = db.get_file(file).unwrap().unwrap();
    assert_eq!(healed.path, "notes-2026.txt");
    assert_eq!(healed.status, FileStatus::Local);
}

/// dc-136b's guarantee stays: nothing authenticates a plain value, so it never renames a file this PC already
/// names. The file keeps its name and is marked; a sealed name that opens is applied and clears the mark.
#[test]
fn name_unreadable_plaintext_never_renames_a_named_file() {
    let dir = tempfile::tempdir().unwrap();
    let db = state_in(dir.path());
    let bridge = bridge_with(&db, KEY);
    let file = "13600000-0000-4000-8000-000000000051";
    list(
        &bridge,
        &node(file, &sealed_by_this_core(KEY, file, "notes.txt"), LISTED_AT),
    );

    list(&bridge, &node(file, "bm90LWEtbmFtZQ==", LISTED_AT + 1));
    assert_eq!(shown(&db, file), ("notes.txt".to_string(), true));
    assert_eq!(db.get_file(file).unwrap().unwrap().path, "notes.txt");

    list(
        &bridge,
        &node(file, &sealed_by_this_core(KEY, file, "notes-fixed.txt"), LISTED_AT + 2),
    );
    assert_eq!(shown(&db, file), ("notes-fixed.txt".to_string(), false));
}
