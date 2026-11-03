//! A19: an owner move that takes an F item (or a folder with F descendants)
//! out of its shared root adopts every foreign item first, confirmed by
//! read-back, or the move is refused.

use super::*;
use crate::state_db::{FileEntry, FileStatus, ItemKind};
use beebeeb_core::chunk_stream::ExpectedChunkFormat;
use beebeeb_core::kdf::{FileKey, MasterKey};
use beebeeb_sync::key_resolution::{wrap_child_key, wrap_folder_custody};

const ROOT: &str = "10000000-0000-4000-8000-0000000000c1";
const OWNER_FOLDER: &str = "40000000-0000-4000-8000-0000000000c2";
const GUEST: &str = "20000000-0000-8000-8000-0000000000c3";

fn seed(bridge: &crate::engine_bridge::EngineBridge, id: &str, kind: ItemKind, parent: Option<&str>) {
    let entry = FileEntry {
        file_id: id.into(),
        path: id.into(),
        status: FileStatus::Local,
        size_bytes: 0,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: kind.clone(),
    };
    bridge
        .db()
        .upsert_file_with_contract(&entry, |contract| {
            contract.item_kind = kind;
            contract.parent_id = parent.map(str::to_string);
        })
        .unwrap();
}

/// The shared root `ROOT`, an owner folder inside it, and one F file under
/// that folder. `poison` makes the server answer the adoption POST with an
/// envelope that does not open to our key, so the read-back cannot confirm it.
fn world(poison: bool) -> (Fixture, Arc<Mutex<Vec<String>>>) {
    let master = MasterKey::from_bytes([7; 32]);
    let folder = FileKey::from_bytes([55; 32]);
    let guest = FileKey::from_bytes([66; 32]);
    let guest_name =
        beebeeb_core::encrypt::encrypt_name_for(&guest, "added.txt", None, ExpectedChunkFormat::Unrecorded).unwrap();
    let wrap = STANDARD.encode(wrap_child_key(&folder, ROOT, GUEST, 1, &guest).unwrap());
    let custody = STANDARD.encode(wrap_folder_custody(&master, ROOT, &folder).unwrap());
    let served = Arc::new(Mutex::new(json!({
        "id": GUEST, "name_encrypted": guest_name, "parent_id": OWNER_FOLDER,
        "is_folder": false, "version_number": 1, "size_bytes": 4, "updated_at": 1,
        "acted_via_share_id": "fixture-share", "uploaded_by": null,
        "owner_wrapped_file_key": null,
    })));
    let row = served.clone();
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let body = if req.path == "/api/v1/shares/invites/sent" {
            json!({"invites": []})
        } else if req.path == format!("/api/v1/shared-folders/{ROOT}") {
            json!({"folder_id": ROOT, "role": "owner", "key_version": 1,
                   "owner_wrapped_folder_key": custody})
        } else if req.path.starts_with(&format!("/api/v1/shared-folders/{ROOT}/keys")) {
            json!({"keys": [{"file_id": GUEST, "encrypted_file_key": wrap}], "next_cursor": null})
        } else if req.path == format!("/api/v1/files/{GUEST}/owner-key") {
            let stored = if poison {
                json!("not-a-frame")
            } else {
                req.body["owner_wrapped_file_key"].clone()
            };
            row.lock().unwrap()["owner_wrapped_file_key"] = stored;
            json!({"adopted": true})
        } else if req.path == format!("/api/v1/files/{OWNER_FOLDER}") {
            json!({"id": OWNER_FOLDER, "parent_id": ROOT, "is_folder": true, "version_number": 1,
                   "name_encrypted": "opaque", "size_bytes": 0, "updated_at": 1})
        } else if req.path == format!("/api/v1/files/{ROOT}") {
            json!({"id": ROOT, "parent_id": null, "is_folder": true, "version_number": 1,
                   "name_encrypted": "opaque", "size_bytes": 0, "updated_at": 1})
        } else if req.path == format!("/api/v1/files/{GUEST}") {
            return Some((200, serde_json::to_vec(&*row.lock().unwrap()).unwrap()));
        } else {
            json!({})
        };
        Some((200, serde_json::to_vec(&body).unwrap()))
    });
    (fixture, seen)
}

fn bridge(dir: &std::path::Path, fixture: &Fixture) -> crate::engine_bridge::EngineBridge {
    let db = Arc::new(StateDb::open(dir.join("db")).unwrap());
    let bridge = crate::engine_bridge::EngineBridge::new(db, Arc::new(fixture.api()));
    bridge.db().remember_folder_custody_at_revision(ROOT, 0).unwrap();
    seed(&bridge, ROOT, ItemKind::Folder, None);
    seed(&bridge, OWNER_FOLDER, ItemKind::Folder, Some(ROOT));
    seed(&bridge, GUEST, ItemKind::File, Some(OWNER_FOLDER));
    bridge
}

#[test]
fn a19_a_move_out_adopts_every_foreign_descendant_first() {
    let (fixture, seen) = world(false);
    let temp = tempfile::tempdir().unwrap();
    let bridge = bridge(temp.path(), &fixture);
    assert!(bridge.db().get_owned_key_record(GUEST).unwrap().is_none(), "nothing is adopted yet");
    run(bridge.adopt_foreign_subtree(OWNER_FOLDER, None)).expect("adoption lands before the move");
    let record = bridge.db().get_owned_key_record(GUEST).unwrap().expect("an adopted record");
    assert!(record.owner_wrap.is_some(), "read back: the served envelope opens");
    assert!(record.pending_owner_wrap.is_none());
    // Still inside the root: no adoption, no request.
    let before = count(&seen, "/owner-key");
    run(bridge.adopt_foreign_subtree(OWNER_FOLDER, Some(ROOT))).expect("a move inside the root needs nothing");
    assert_eq!(count(&seen, "/owner-key"), before);
}

#[test]
fn a19_a_failed_adoption_refuses_the_move() {
    let (fixture, seen) = world(true);
    let temp = tempfile::tempdir().unwrap();
    let bridge = bridge(temp.path(), &fixture);
    let error = run(bridge.adopt_foreign_subtree(OWNER_FOLDER, None)).expect_err("refused");
    assert!(format!("{error}").contains("move needs adoption"), "{error}");
    let record = bridge.db().get_owned_key_record(GUEST).unwrap().unwrap();
    assert!(record.owner_wrap.is_none(), "an envelope that does not open is never adopted");
    assert_eq!(count(&seen, "/owner-key"), 1, "one attempt, then the refusal: {seen:?}");
}
