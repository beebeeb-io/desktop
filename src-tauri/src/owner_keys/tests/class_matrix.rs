//! L12/L14 class matrix: A2, A3, A5, A24.
//!
//! Owns: a real name under a foreign-looking listing, an ignored L8 envelope on
//! an owner id, one verified adoption under proven custody, and a second wrap
//! that must block the adopting device.
//! Must not: run the index repair, mutate the mirror, or touch The Finder queue.
//! Tests: this whole file.

use super::*;
use beebeeb_core::kdf::{FileKey, MasterKey};

fn master() -> MasterKey {
    MasterKey::from_bytes([7; 32])
}

fn name_under(key: &FileKey, name: &str) -> String {
    beebeeb_core::encrypt::encrypt_name_for(
        key,
        name,
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap()
}

fn bridge_for(fixture: &Fixture) -> crate::engine_bridge::EngineBridge {
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    std::mem::forget(temp);
    bridge
}

/// A proven custody root `root` holding `row`, one inventory wrap, and an item
/// read that only carries the winner envelope after an adoption POST.
fn world(
    root: &'static str,
    row: &Value,
    folder: &FileKey,
    inventory: Value,
    winner: Vec<u8>,
) -> (Fixture, Arc<Mutex<Vec<String>>>) {
    let item = row["id"].as_str().unwrap().to_string();
    let custody = json!({
        "folder_id":root,"role":"owner","key_version":1,
        "owner_wrapped_folder_key":STANDARD.encode(
            beebeeb_sync::key_resolution::wrap_folder_custody(&master(), root, folder).unwrap()
        )
    });
    let served = row.clone();
    let posted = Arc::new(Mutex::new(false));
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let (log, adopted) = (seen.clone(), posted.clone());
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let body = if req.path == "/api/v1/shares/invites/sent" {
            json!({"invites":[]})
        } else if req.path == format!("/api/v1/shared-folders/{root}") {
            custody.clone()
        } else if req.path.starts_with(&format!("/api/v1/shared-folders/{root}/keys")) {
            inventory.clone()
        } else if req.path == format!("/api/v1/files/{item}/owner-key") && req.method == "POST" {
            *adopted.lock().unwrap() = true;
            json!({"adopted":true})
        } else if req.path == format!("/api/v1/files/{item}") {
            let mut over = served.clone();
            if *adopted.lock().unwrap() {
                over["owner_wrapped_file_key"] = Value::String(STANDARD.encode(&winner));
            }
            over
        } else if req.path == format!("/api/v1/files/{root}") {
            json!({"id":root,"parent_id":null,"is_folder":true})
        } else if req.path.starts_with("/api/v1/files?") || req.path.starts_with("/api/v1/files/") {
            json!({"files":[],"next_cursor":null})
        } else {
            json!({})
        };
        Some((200, serde_json::to_vec(&body).unwrap()))
    });
    (fixture, seen)
}

#[test]
fn a2_owner_row_with_a_member_wrap_but_the_real_name_is_derived_and_not_adopted() {
    let owner = "22222222-2222-4222-8222-22222222222a";
    let root = "10000000-0000-4000-8000-0000000000a1";
    let folder = FileKey::from_bytes([56; 32]);
    let member = FileKey::from_bytes([0x52; 32]);
    let derived = beebeeb_core::kdf::derive_file_key(&master(), owner.as_bytes());
    let member_wrap =
        beebeeb_sync::key_resolution::wrap_owner_file_key(&master(), "20000000-0000-8000-8000-00000000002a", &member)
            .unwrap();
    let row = json!({"id":owner,"name_encrypted":name_under(&derived,"real.txt"),"parent_id":root,
        "is_folder":false,"version_number":1,"size_bytes":4,"updated_at":1,
        "acted_via_share_id":"fixture-share","uploaded_by":"member",
        "owner_wrapped_file_key":STANDARD.encode(&member_wrap)});
    // A real custody root whose inventory holds an L7 wrap of the member key for
    // this owner id: a resolver that applies the L7 reader to an owner id would
    // open it, so the test must fail if that reader is ever reached.
    let inventory = json!({"keys":[{
        "file_id":owner,
        "encrypted_file_key":STANDARD.encode(
            beebeeb_sync::key_resolution::wrap_child_key(&folder, root, owner, 1, &member).unwrap()
        )
    }],"next_cursor":null});
    // An owner id cannot carry an L8 winner (core refuses to mint one), and the
    // served row's envelope is the member's; this is never handed back.
    let (fixture, seen) = world(root, &row, &folder, inventory, member_wrap.clone());
    let bridge = bridge_for(&fixture);
    let prepared = run(bridge.prepare_owned_rows(&[row])).expect("A2 must not abort the snapshot");
    assert_eq!(prepared.len(), 1);
    assert!(prepared[0].readable, "the real name opens under the derived key");
    assert!(!prepared[0].record.foreign_added);
    assert!(prepared[0].record.owner_wrap.is_none());
    assert!(prepared[0].record.pending_owner_wrap.is_none(), "no adoption was prepared");
    assert_eq!(count(&seen, "/owner-key"), 0, "{seen:?}");
    assert_eq!(count(&seen, "/shared-folders"), 0, "the owner path never walks custody: {seen:?}");
}

#[test]
fn a3_l8_envelope_served_on_an_owner_id_is_ignored() {
    let owner = "22222222-2222-4222-8222-22222222222b";
    let derived = beebeeb_core::kdf::derive_file_key(&master(), owner.as_bytes());
    // A well-formed L8 frame, but bound to a foreign id: replaying it on an
    // owner id must never select a key.
    let envelope = beebeeb_sync::key_resolution::wrap_owner_file_key(
        &master(),
        "20000000-0000-8000-8000-00000000002b",
        &derived,
    )
    .unwrap();
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        Some((404, serde_json::to_vec(&json!({})).unwrap()))
    });
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(StateDb::open(temp.path().join("db")).unwrap());
    let bridge = crate::engine_bridge::EngineBridge::new(db.clone(), Arc::new(fixture.api()));
    let row = json!({"id":owner,"name_encrypted":name_under(&derived,"enveloped.txt"),"parent_id":null,
        "is_folder":false,"version_number":1,"size_bytes":4,"updated_at":1,
        "acted_via_share_id":null,"uploaded_by":null,
        "owner_wrapped_file_key":STANDARD.encode(&envelope)});
    let prepared = run(bridge.prepare_owned_rows(&[row])).expect("A3 must not abort the snapshot");
    assert_eq!(prepared.len(), 1);
    assert!(prepared[0].readable);
    assert!(!prepared[0].record.foreign_added, "the id class, not the served envelope");
    assert!(prepared[0].record.owner_wrap.is_none(), "the L8 envelope is ignored");
    assert!(prepared[0].record.pending_owner_wrap.is_none());
    assert_eq!(count(&seen, "/owner-key"), 0, "{seen:?}");
    assert_eq!(count(&seen, "/shared-folders"), 0, "no custody walk for an owner id: {seen:?}");
    // A3: beyond the record fields, the persisted row still resolves to the
    // derived key and lists its real name.
    db.put_owned_key_record(&prepared[0].record).unwrap();
    assert_eq!(run(bridge.resolved_owned_key(owner)).unwrap().as_bytes(), derived.as_bytes());
    assert_eq!(
        crate::owner_keys::stored_name(&db, owner, &master().to_bytes()).unwrap().as_deref(),
        Some("enveloped.txt")
    );
}

#[test]
fn a5_foreign_wrap_under_proven_custody_is_adopted_once_and_verified() {
    let root = "10000000-0000-4000-8000-0000000000d1";
    let item = "20000000-0000-8000-8000-0000000000d2";
    let folder = FileKey::from_bytes([55; 32]);
    let key = FileKey::from_bytes([0x66; 32]);
    let winner = beebeeb_sync::key_resolution::wrap_owner_file_key(&master(), item, &key).unwrap();
    let row = json!({"id":item,"name_encrypted":name_under(&key,"shared.txt"),"parent_id":root,
        "is_folder":false,"version_number":1,"size_bytes":4,"updated_at":1,
        "acted_via_share_id":"fixture-share","uploaded_by":null,
        "owner_wrapped_file_key":null});
    let inventory = json!({"keys":[{
        "file_id":item,
        "encrypted_file_key":STANDARD.encode(
            beebeeb_sync::key_resolution::wrap_child_key(&folder, root, item, 1, &key).unwrap()
        )
    }],"next_cursor":null});
    let (fixture, seen) = world(root, &row, &folder, inventory, winner.clone());
    let bridge = bridge_for(&fixture);
    let prepared = run(bridge.prepare_owned_rows(&[row.clone()])).expect("A5 prepares the row");
    assert_eq!(prepared.len(), 1);
    assert!(prepared[0].readable);
    assert_eq!(prepared[0].record.owner_wrap.as_ref(), Some(&winner), "the verified winner is pinned");
    assert!(prepared[0].record.pending_owner_wrap.is_none());
    assert_eq!(exact(&seen, &format!("POST /api/v1/files/{item}/owner-key")), 1, "{seen:?}");
    // Second pass: already adopted, so never adopted again.
    let again = run(bridge.prepare_owned_rows(&[row])).expect("A5 is stable");
    assert!(again[0].readable);
    assert_eq!(exact(&seen, &format!("POST /api/v1/files/{item}/owner-key")), 1, "{seen:?}");
}

#[test]
fn a24_a_second_wrap_after_another_device_adopted_blocks_this_device() {
    let root = "10000000-0000-4000-8000-0000000000e1";
    let item = "20000000-0000-8000-8000-0000000000e2";
    let folder = FileKey::from_bytes([55; 32]);
    let mine = FileKey::from_bytes([0x71; 32]);
    let theirs = FileKey::from_bytes([0x72; 32]);
    let second = beebeeb_sync::key_resolution::wrap_owner_file_key(&master(), item, &theirs).unwrap();
    let row = json!({"id":item,"name_encrypted":name_under(&mine,"contested.txt"),"parent_id":root,
        "is_folder":false,"version_number":1,"size_bytes":4,"updated_at":1,
        "acted_via_share_id":"fixture-share","uploaded_by":null,
        "owner_wrapped_file_key":null});
    let inventory = json!({"keys":[{
        "file_id":item,
        "encrypted_file_key":STANDARD.encode(
            beebeeb_sync::key_resolution::wrap_child_key(&folder, root, item, 1, &mine).unwrap()
        )
    }],"next_cursor":null});
    let (fixture, seen) = world(root, &row, &folder, inventory, second);
    let bridge = bridge_for(&fixture);
    let prepared = run(bridge.prepare_owned_rows(&[row])).expect("A24 must not abort the snapshot");
    assert_eq!(prepared.len(), 1);
    assert!(!prepared[0].readable, "the conflicting winner never resolves");
    assert!(prepared[0].record.blocked_fingerprint.is_some());
    assert!(run(bridge.resolved_owned_key(item)).is_err());
    // One attempt, then the conflict is recorded rather than retried forever.
    let again = run(bridge.prepare_owned_rows(&[prepared[0].row.clone()])).expect("A24 is stable");
    assert!(!again[0].readable);
    assert_eq!(exact(&seen, &format!("POST /api/v1/files/{item}/owner-key")), 1, "{seen:?}");
}
