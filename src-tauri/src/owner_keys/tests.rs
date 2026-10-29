use super::*;
use crate::state_db::{FileEntry, FileStatus, ItemKind};
use serde_json::json;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

#[derive(Clone)]
struct Request {
    path: String,
    method: String,
    bytes: Vec<u8>,
    body: Value,
}
struct Fixture {
    base: String,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Fixture {
    fn new(mut answer: impl FnMut(Request) -> Option<Value> + Send + 'static) -> Self {
        Self::wire(move |request| answer(request).map(|body| (200, serde_json::to_vec(&body).unwrap())))
    }
    fn wire(mut answer: impl FnMut(Request) -> Option<(u16, Vec<u8>)> + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let thread = thread::spawn(move || {
            while !signal.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // macOS inherits the listener's nonblocking mode.
                        stream.set_nonblocking(false).unwrap();
                        let request = read_request(&mut stream);
                        if let Some((status, bytes)) = answer(request) {
                            let header = format!(
                                "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                bytes.len()
                            );
                            stream.write_all(header.as_bytes()).unwrap();
                            stream.write_all(&bytes).unwrap();
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(2)),
                    Err(e) => panic!("fixture accept: {e}"),
                }
            }
        });
        Self {
            base,
            stop,
            thread: Some(thread),
        }
    }
    fn api(&self) -> ApiClient {
        ApiClient::new(self.base.clone(), "fixture-session".into(), [7; 32])
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let result = self.thread.take().unwrap().join();
        if !thread::panicking() {
            result.expect("fixture server panicked");
        }
    }
}
fn read_request(stream: &mut TcpStream) -> Request {
    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut bytes = Vec::new();
    let end = loop {
        let mut byte = [0];
        assert_eq!(stream.read(&mut byte).unwrap(), 1);
        bytes.push(byte[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            break bytes.len();
        }
    };
    let header = String::from_utf8(bytes.clone()).unwrap();
    let length = header
        .lines()
        .find_map(|line| {
            let (name, val) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| val.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    bytes.resize(end + length, 0);
    stream.read_exact(&mut bytes[end..]).unwrap();
    let path = header
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();
    let body = serde_json::from_slice(&bytes[end..]).unwrap_or(Value::Null);
    let method = header.split_whitespace().next().unwrap().to_string();
    Request {
        path,
        method,
        bytes: bytes[end..].to_vec(),
        body,
    }
}
fn run<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}
fn record() -> OwnedKeyRecord {
    let master = beebeeb_core::kdf::MasterKey::from_bytes([7; 32]);
    let key = beebeeb_core::kdf::FileKey::from_bytes([0x9d; 32]);
    let envelope = beebeeb_sync::key_resolution::wrap_owner_file_key(&master, "item", &key).unwrap();
    OwnedKeyRecord {
        file_id: "item".into(),
        name_encrypted: "opaque-name".into(),
        pending_owner_wrap: Some(envelope),
        foreign_added: true,
        ..Default::default()
    }
}

#[test]
fn sf2237d_t3_migration_preserves_contracts() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    db.upsert_file(&FileEntry {
        file_id: "legacy".into(),
        path: "legacy.txt".into(),
        status: FileStatus::Local,
        size_bytes: 4,
        modified_at: 1,
        content_hash: Some("hash".into()),
        remote_updated_at: 1,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    let before = db.get_file_contract_state("legacy").unwrap().unwrap();
    // Emulate a pre-2237d database: the legacy file/contract survives migration.
    drop(db);
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute("DROP TABLE IF EXISTS owned_key_records", [])
        .unwrap();
    let db = StateDb::open(&path).unwrap();
    db.put_owned_key_record(&record()).unwrap();
    drop(db);
    let db = StateDb::open(&path).unwrap();
    let after = db.get_file_contract_state("legacy").unwrap().unwrap();
    assert_eq!(
        (after.local_base_version, after.local_hash),
        (before.local_base_version, before.local_hash)
    );
    let reopened_records = usize::from(db.get_owned_key_record("item").unwrap().is_some());
    assert_eq!(reopened_records, 1);
}

#[test]
fn sf2237d_t3_opaque_roundtrip_and_purge() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    let row = record();
    db.put_owned_key_record(&row).unwrap();
    assert_eq!(db.get_owned_key_record("item").unwrap(), Some(row.clone()));
    let conn = rusqlite::Connection::open(&path).unwrap();
    let columns: Vec<String> = conn
        .prepare("PRAGMA table_info(owned_key_records)")
        .unwrap()
        .query_map([], |r| r.get(1))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        columns
            .iter()
            .filter(|c| matches!(c.as_str(), "raw_key" | "file_key" | "folder_key" | "master_key"))
            .count(),
        0
    );
    let serialized: String = conn
        .query_row("SELECT record FROM owned_key_records", [], |r| r.get(0))
        .unwrap();
    assert!(!serialized.contains(&serde_json::to_string(&vec![0x9du8; 32]).unwrap()));
    assert_eq!(row.fingerprint(), row.fingerprint());
    let mut changed = row.clone();
    changed.name_encrypted.push('x');
    assert_ne!(row.fingerprint(), changed.fingerprint());
    db.clear_account_data(false).unwrap();
    assert_eq!(db.get_owned_key_record("item").unwrap(), None);
    assert!(!db.has_account_data().unwrap());
    db.put_owned_key_record(&row).unwrap();
    db.delete_owned_key_record("item").unwrap();
    assert_eq!(db.get_owned_key_record("item").unwrap(), None);
    db.put_owned_key_record(&row).unwrap();
    let api = ApiClient::new("http://127.0.0.1:9".into(), "fixture".into(), [7; 32]);
    let stop = AtomicBool::new(false);
    let before_purge = AccountScope::new(&db, &api, &stop);
    db.purge_all_local_state().unwrap();
    assert_eq!(db.get_owned_key_record("item").unwrap(), None);
    assert!(before_purge.put(&row).is_err());
}

#[test]
fn sf2237d_t3_wire_array_and_base64() {
    let frame = vec![1u8; 60];
    assert_eq!(normalize_wrap(&Value::Null).unwrap(), None);
    assert_eq!(
        normalize_wrap(&json!(STANDARD.encode(&frame))).unwrap(),
        Some(frame.clone())
    );
    assert_eq!(normalize_wrap(&json!(frame)).unwrap(), Some(vec![1; 60]));
    for bad in [
        json!([256]),
        json!([-1]),
        json!([1.5]),
        json!(["1"]),
        json!(true),
        json!("!"),
        json!([1]),
    ] {
        assert!(normalize_wrap(&bad).is_err());
    }
}

#[test]
fn sf2237d_t3_inventory_cursor_ties_and_cycle() {
    let fixture = Fixture::new(|req| {
        assert!(req.path.contains("limit=500"));
        if req.path.contains("cursor=tied%2B%2F%3D") {
            Some(json!({"keys":[{"file_id":"last","encrypted_file_key":"opaque"}],"next_cursor":null}))
        } else {
            Some(
                json!({"keys":(0..500).map(|n|json!({"file_id":n.to_string(),"created_at":1})).collect::<Vec<_>>(),"next_cursor":"tied+/="}),
            )
        }
    });
    let api = fixture.api();
    let db = StateDb::open(":memory:").unwrap();
    let stop = AtomicBool::new(false);
    let scope = AccountScope::new(&db, &api, &stop);
    let inventory = run(scope.inventory("root")).unwrap();
    assert_eq!(inventory.len(), 501);
    let cyclic = Fixture::new(|_| Some(json!({"keys":[],"next_cursor":"same"})));
    let api = cyclic.api();
    assert!(run(AccountScope::new(&db, &api, &stop).inventory("root")).is_err());
}

#[test]
fn sf2237d_t3_write_once_false_and_timeout() {
    let db = Arc::new(StateDb::open(":memory:").unwrap());
    let observed = Arc::new(Mutex::new(Vec::new()));
    let obs = observed.clone();
    let target = db.clone();
    let fixture = Fixture::new(move |req| {
        assert!(req.path.ends_with("/item/owner-key"));
        assert!(req.body["owner_wrapped_file_key"].is_string());
        obs.lock()
            .unwrap()
            .push(target.get_owned_key_record("item").unwrap().is_some());
        if obs.lock().unwrap().len() == 1 {
            Some(json!({"adopted":false}))
        } else {
            None
        }
    });
    let api = fixture.api();
    let stop = AtomicBool::new(false);
    let row = record();
    let scope = AccountScope::new(&db, &api, &stop);
    assert!(!run(scope.adopt(&row)).unwrap());
    assert!(run(scope.adopt(&row)).is_err());
    assert_eq!(
        *observed.lock().unwrap(),
        vec![true, true],
        "persist before POST, including ambiguous failure"
    );
    assert_eq!(
        db.get_owned_key_record("item").unwrap().unwrap().pending_owner_wrap,
        row.pending_owner_wrap
    );
}

#[test]
fn sf2237d_t3_account_switch_cancels() {
    let db = Arc::new(StateDb::open(":memory:").unwrap());
    let target = db.clone();
    let fixture = Fixture::new(move |_| {
        target.clear_account_data(false).unwrap();
        Some(json!({"keys":[{"file_id":"previous-account"}],"next_cursor":null}))
    });
    let api = fixture.api();
    let stop = AtomicBool::new(false);
    let scope = AccountScope::new(&db, &api, &stop);
    assert!(run(scope.inventory("root")).is_err());
    let cross_account_writes = usize::from(scope.put(&record()).is_ok());
    assert_eq!(cross_account_writes, 0);
    let scope = AccountScope::new(&db, &api, &stop);
    stop.store(true, Ordering::Release);
    assert!(scope.put(&record()).is_err());
    assert_eq!(db.get_owned_key_record("item").unwrap(), None);
}

mod ingestion;

#[test]
fn sf2237d_l8_old_owner_wrap_is_rejected_without_fallback() {
    use beebeeb_core::{
        chunk_stream::ExpectedChunkFormat,
        kdf::{FileKey, MasterKey},
    };
    let master = MasterKey::from_bytes([7; 32]);
    let id = "old-owner-format";
    let key = FileKey::from_bytes([91; 32]);
    // The removed format was an empty-AAD frame under derive_file_key(master,id).
    // Build the historical wire bytes through core, never a compatibility reader.
    let derived = beebeeb_core::kdf::derive_file_key(&master, id.as_bytes());
    let old = beebeeb_core::encrypt::encrypt_chunk_raw(&derived, key.as_bytes()).unwrap();
    let encrypted =
        beebeeb_core::encrypt::encrypt_name_for(&key, "legacy.txt", None, ExpectedChunkFormat::Unrecorded).unwrap();
    for pending in [false, true] {
        for foreign in [false, true] {
            let record = OwnedKeyRecord {
                file_id: id.into(),
                name_encrypted: encrypted.clone(),
                foreign_added: foreign,
                owner_wrap: (!pending).then(|| old.clone()),
                pending_owner_wrap: pending.then(|| old.clone()),
                ..Default::default()
            };
            assert!(
                record_key(&master, &record).is_err(),
                "old owner envelopes must never open or select derived-key fallback"
            );
        }
    }
}

#[test]
fn sf2265_forged_name_blob_owner_copy_is_refused() {
    use beebeeb_core::kdf::MasterKey;
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000001";
    // The folder's own 32-byte name JSON, sealed under the legacy folder key.
    // A server can replay this as an "owner copy" after answering 404.
    let name_json = br#"{"mime_type":null,"name":"Docs"}"#;
    assert_eq!(name_json.len(), 32);
    let derived = beebeeb_core::kdf::derive_file_key(&master, root.as_bytes());
    let forged = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&derived, name_json).unwrap());
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let row = forged.clone();
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let (status, body) = if req.path == "/api/v1/shares/invites/sent" {
            (
                200,
                json!({"invites":[{
                    "id":"inv-1","file_id":root,"is_folder_share":true,"status":"approved",
                    "created_at":"2026-10-28T00:00:00Z","encrypted_owner_folder_key":forged
                }]}),
            )
        } else if req.path == "/api/v1/shares/invites/inv-1/folder-keys" {
            (200, json!({"keys":[{"file_id":root,"encrypted_file_key":row}]}))
        } else {
            (404, json!({}))
        };
        Some((status, serde_json::to_vec(&body).unwrap()))
    });
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    let error = run(bridge.shared_folder_custody_key(root))
        .err()
        .expect("forged copy must not open");
    assert!(error.to_string().contains("Can't verify"), "{error}");
    assert!(!bridge.db().has_seen_folder_custody(root).unwrap());
    let seen = seen.lock().unwrap().clone();
    assert!(
        seen.iter()
            .any(|line| line == "GET /api/v1/shares/invites/inv-1/folder-keys"),
        "proof rows were read: {seen:?}"
    );
    assert_eq!(
        seen.iter()
            .filter(|line| line.contains("/key") || line.contains("/owner-key"))
            .count(),
        0,
        "no key may be written or pinned: {seen:?}"
    );
}

type OwnerCopyObservation = (Fixture, Arc<Mutex<Vec<String>>>, Arc<Mutex<Vec<Value>>>);

fn owner_copy_test(root: &'static str, invites: Value, folder_keys: Value) -> OwnerCopyObservation {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let bodies = Arc::new(Mutex::new(Vec::<Value>::new()));
    let stored = Arc::new(Mutex::new(None::<String>));
    let log = seen.clone();
    let body_log = bodies.clone();
    let store = stored.clone();
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let (status, body) = if req.path == "/api/v1/shares/invites/sent" {
            (200, invites.clone())
        } else if req.path.ends_with("/folder-keys") {
            (200, folder_keys.clone())
        } else if req.path == format!("/api/v1/shared-folders/{root}/key") && req.method == "PUT" {
            *store.lock().unwrap() = req.body["owner_wrapped_folder_key"].as_str().map(str::to_string);
            body_log.lock().unwrap().push(req.body.clone());
            (200, json!({"created":true,"key_version":1}))
        } else if req.path == format!("/api/v1/shared-folders/{root}") {
            match store.lock().unwrap().clone() {
                Some(wrap) => (
                    200,
                    json!({"folder_id":root,"role":"owner","key_version":1,"owner_wrapped_folder_key":wrap}),
                ),
                None => (404, json!({})),
            }
        } else {
            (404, json!({}))
        };
        Some((status, serde_json::to_vec(&body).unwrap()))
    });
    (fixture, seen, bodies)
}

#[test]
fn sf2265_purpose_bound_owner_copy_opens_and_registers_l7_custody() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000002";
    let folder = FileKey::from_bytes([55; 32]);
    let blob = beebeeb_core::kdf::wrap_invite_owner_copy(&master, root.as_bytes(), &folder).unwrap();
    let mut copy = blob.nonce;
    copy.extend_from_slice(&blob.ciphertext);
    let invites = json!({"invites":[{
        "id":"inv-2","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-28T00:00:00Z","encrypted_owner_folder_key":STANDARD.encode(&copy)
    }]});
    let (fixture, seen, bodies) = owner_copy_test(root, invites, json!({"keys":[]}));
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    // The fixture answers 404 for the custody GET, then accepts the write.
    let key = run(bridge.shared_folder_custody_key(root));
    assert!(
        key.is_ok(),
        "purpose-bound copy must open: {}",
        key.as_ref().err().map(|e| e.to_string()).unwrap_or_default()
    );
    assert_eq!(key.unwrap().as_bytes(), folder.as_bytes());
    let bodies = bodies.lock().unwrap().clone();
    assert_eq!(bodies.len(), 1, "exactly one custody write");
    assert_eq!(bodies[0]["format"], "l7");
    assert_eq!(bodies[0]["key_version"], 1);
    assert!(bridge.db().has_seen_folder_custody(root).unwrap());
    let seen = seen.lock().unwrap().clone();
    assert!(
        !seen.iter().any(|line| line.ends_with("/folder-keys")),
        "a purpose-bound copy needs no proof rows: {seen:?}"
    );
}

#[test]
fn sf2265_legacy_copy_proves_batch_key_but_never_custody() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000003";
    let item = "20000000-0000-4000-8000-000000000001";
    // A real key is not valid UTF-8; core refuses a text candidate (L9 rule 1).
    let folder = FileKey::from_bytes([0x8d; 32]);
    let derived = beebeeb_core::kdf::derive_file_key(&master, root.as_bytes());
    let copy = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&derived, folder.as_bytes()).unwrap());
    let child = beebeeb_core::kdf::derive_file_key(&master, item.as_bytes());
    let wrap = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&folder, child.as_bytes()).unwrap());
    let invites = json!({"invites":[{
        "id":"inv-3","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-28T00:00:00Z","encrypted_owner_folder_key":copy
    }]});
    let (fixture, _seen, bodies) = owner_copy_test(
        root,
        invites,
        json!({"keys":[{"file_id":item,"encrypted_file_key":wrap}]}),
    );
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    // Review P2-1: the proven key is batch-only. It is not custody, so no
    // caller may treat it as one (it would seal index names under a key the
    // server could have chosen).
    let error = run(bridge.shared_folder_custody_key(root))
        .err()
        .expect("a proven legacy key must never be custody");
    assert!(error.to_string().contains("no custody"), "{error}");
    assert_eq!(
        bodies.lock().unwrap().len(),
        0,
        "a legacy key is never registered (L11)"
    );
    assert!(!bridge.db().has_seen_folder_custody(root).unwrap());
}

#[test]
fn sf2265_legacy_copy_with_forged_rows_is_refused() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000004";
    let item = "20000000-0000-4000-8000-000000000002";
    // A real key is not valid UTF-8; core refuses a text candidate (L9 rule 1).
    let folder = FileKey::from_bytes([0x8d; 32]);
    let derived = beebeeb_core::kdf::derive_file_key(&master, root.as_bytes());
    let copy = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&derived, folder.as_bytes()).unwrap());
    // A row that opens under the candidate but carries the wrong child key.
    let wrap = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&folder, &[9u8; 32]).unwrap());
    let invites = json!({"invites":[{
        "id":"inv-4","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-28T00:00:00Z","encrypted_owner_folder_key":copy
    }]});
    let (fixture, _seen, bodies) = owner_copy_test(
        root,
        invites,
        json!({"keys":[{"file_id":item,"encrypted_file_key":wrap}]}),
    );
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    let error = run(bridge.shared_folder_custody_key(root))
        .err()
        .expect("forged rows must not prove");
    assert!(error.to_string().contains("Can't verify"), "{error}");
    assert_eq!(bodies.lock().unwrap().len(), 0);
    assert!(!bridge.db().has_seen_folder_custody(root).unwrap());
}

#[test]
fn sf2265_non_approved_invite_is_never_read() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000005";
    let item = "20000000-0000-4000-8000-000000000003";
    // A real key is not valid UTF-8; core refuses a text candidate (L9 rule 1).
    let folder = FileKey::from_bytes([0x8d; 32]);
    let derived = beebeeb_core::kdf::derive_file_key(&master, root.as_bytes());
    let copy = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&derived, folder.as_bytes()).unwrap());
    let child = beebeeb_core::kdf::derive_file_key(&master, item.as_bytes());
    let wrap = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&folder, child.as_bytes()).unwrap());
    // The pending invite is newer and would open, but only approved copies count.
    let invites = json!({"invites":[
        {"id":"inv-approved","file_id":root,"is_folder_share":true,"status":"approved",
         "created_at":"2026-10-20T00:00:00Z","encrypted_owner_folder_key":copy},
        {"id":"inv-pending","file_id":root,"is_folder_share":true,"status":"invited",
         "created_at":"2026-10-28T00:00:00Z","encrypted_owner_folder_key":copy}
    ]});
    let (fixture, seen, _bodies) = owner_copy_test(
        root,
        invites,
        json!({"keys":[{"file_id":item,"encrypted_file_key":wrap}]}),
    );
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    // The approved legacy copy proves the key for a batch only; the pending,
    // newer invite is never read and never becomes custody (P2-1).
    let error = run(bridge.shared_folder_custody_key(root))
        .err()
        .expect("a legacy key is not custody");
    assert!(error.to_string().contains("no custody"), "{error}");
    let seen = seen.lock().unwrap().clone();
    assert!(
        seen.iter()
            .any(|line| line == "GET /api/v1/shares/invites/inv-approved/folder-keys"),
        "{seen:?}"
    );
    assert!(
        !seen.iter().any(|line| line.contains("inv-pending")),
        "a non-approved invite is never read: {seen:?}"
    );
}

#[test]
fn sf2258_upgrade_required_is_its_own_state_with_no_retry() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000006";
    let folder = FileKey::from_bytes([55; 32]);
    let blob = beebeeb_core::kdf::wrap_invite_owner_copy(&master, root.as_bytes(), &folder).unwrap();
    let mut copy = blob.nonce;
    copy.extend_from_slice(&blob.ciphertext);
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let bodies = Arc::new(Mutex::new(Vec::<Value>::new()));
    let log = seen.clone();
    let body_log = bodies.clone();
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let (status, body) = if req.path == "/api/v1/shares/invites/sent" {
            (
                200,
                json!({"invites":[{
                    "id":"inv-6","file_id":root,"is_folder_share":true,"status":"approved",
                    "created_at":"2026-10-28T00:00:00Z","encrypted_owner_folder_key":STANDARD.encode(&copy)
                }]}),
            )
        } else if req.path == format!("/api/v1/shared-folders/{root}/key") && req.method == "PUT" {
            body_log.lock().unwrap().push(req.body.clone());
            (
                409,
                json!({"error":"upgrade_required","message":"This folder's keys must be upgraded first; legacy key writes are refused."}),
            )
        } else {
            (404, json!({}))
        };
        Some((status, serde_json::to_vec(&body).unwrap()))
    });
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    let error = run(bridge.shared_folder_custody_key(root))
        .err()
        .expect("upgrade_required must not be swallowed");
    assert!(error.to_string().contains("must upgrade"), "{error}");
    assert!(
        !crate::api_client::upgrade_required(&error),
        "the caller sees the folder state, not the raw transport error"
    );
    let bodies = bodies.lock().unwrap().clone();
    assert_eq!(bodies.len(), 1);
    assert_eq!(bodies[0]["format"], "l7", "a custody-folder write declares L7");
    let seen = seen.lock().unwrap().clone();
    assert_eq!(
        seen.iter().filter(|line| line.ends_with("/key")).count(),
        1,
        "the custody write is never retried: {seen:?}"
    );
    assert!(
        !seen.iter().any(|line| line.contains("/folder-keys")),
        "it is never retried as a legacy read: {seen:?}"
    );
    assert!(!bridge.db().has_seen_folder_custody(root).unwrap());
}

#[test]
fn sf2258_key_writes_name_their_format_only_with_custody() {
    assert_eq!(custody_write_format(true), Some("l7"));
    assert_eq!(custody_write_format(false), None);
}

fn foreign_child(id: &str, root: &str) -> Value {
    json!({"id":id,"name_encrypted":"opaque-name","parent_id":root,"is_folder":false,
           "version_number":1,"size_bytes":4,"updated_at":1,
           "acted_via_share_id":"fixture-share","uploaded_by":null,
           "owner_wrapped_file_key":null})
}

/// The path the original bug used: `prepare_owned_rows`, not the custody
/// helper (review P2-3). The custody GET answers 404 (record, then winner),
/// the invite list and one invite's rows are served, and the custody PUT
/// answers `put`.
fn prepare_world(
    root: &'static str,
    invites: Value,
    folder_keys: Value,
    inventory: Value,
    put: (u16, Value),
    winner: Option<Value>,
) -> (Fixture, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen_put = Arc::new(Mutex::new(false));
    let (log, put_seen) = (seen.clone(), seen_put.clone());
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let (status, body) = if req.path == "/api/v1/shares/invites/sent" {
            (200, invites.clone())
        } else if req.path.ends_with("/folder-keys") {
            (200, folder_keys.clone())
        } else if req.path == format!("/api/v1/shared-folders/{root}/key") && req.method == "PUT" {
            *put_seen.lock().unwrap() = true;
            put.clone()
        } else if req.path == format!("/api/v1/shared-folders/{root}") {
            // The record is absent until the write is attempted; a winner is
            // only visible on the re-read after a conflict.
            match &winner {
                Some(row) if *put_seen.lock().unwrap() => (200, row.clone()),
                _ => (404, json!({})),
            }
        } else if req.path.starts_with(&format!("/api/v1/shared-folders/{root}/keys")) {
            (200, inventory.clone())
        } else {
            (404, json!({}))
        };
        Some((status, serde_json::to_vec(&body).unwrap()))
    });
    (fixture, seen)
}

fn count(seen: &Arc<Mutex<Vec<String>>>, needle: &str) -> usize {
    seen.lock().unwrap().iter().filter(|line| line.contains(needle)).count()
}

/// `contains("/shared-folders/{id}")` would also match `/keys`, so exact match
/// the request line when counting a specific endpoint.
fn exact(seen: &Arc<Mutex<Vec<String>>>, line: &str) -> usize {
    seen.lock().unwrap().iter().filter(|seen| seen.as_str() == line).count()
}

fn forged_name_blob(master: &beebeeb_core::kdf::MasterKey, root: &str) -> String {
    // The folder's own 32-byte name JSON, sealed under the legacy folder key:
    // a server can replay it as an "owner copy" after answering 404.
    let name_json = br#"{"mime_type":null,"name":"Docs"}"#;
    assert_eq!(name_json.len(), 32);
    let derived = beebeeb_core::kdf::derive_file_key(master, root.as_bytes());
    STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&derived, name_json).unwrap())
}

fn legacy_copy(master: &beebeeb_core::kdf::MasterKey, root: &str, folder: &beebeeb_core::kdf::FileKey) -> String {
    let derived = beebeeb_core::kdf::derive_file_key(master, root.as_bytes());
    STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&derived, folder.as_bytes()).unwrap())
}

fn purpose_bound_copy(
    master: &beebeeb_core::kdf::MasterKey,
    root: &str,
    folder: &beebeeb_core::kdf::FileKey,
) -> String {
    let blob = beebeeb_core::kdf::wrap_invite_owner_copy(master, root.as_bytes(), folder).unwrap();
    let mut copy = blob.nonce;
    copy.extend_from_slice(&blob.ciphertext);
    STANDARD.encode(&copy)
}

#[test]
fn sf2265_prepare_blocks_foreign_row_under_forged_folder() {
    use beebeeb_core::kdf::MasterKey;
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000011";
    let item = "20000000-0000-4000-8000-000000000011";
    let invites = json!({"invites":[{
        "id":"inv-11","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-28T00:00:00Z",
        "encrypted_owner_folder_key":forged_name_blob(&master, root)
    }]});
    let (fixture, seen) = prepare_world(
        root,
        invites,
        json!({"keys":[]}),
        json!({"keys":[],"next_cursor":null}),
        (200, json!({})),
        None,
    );
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    let prepared = run(bridge.prepare_owned_rows(&[foreign_child(item, root)]))
        .expect("a forged folder must not fail the snapshot");
    assert_eq!(prepared.len(), 1);
    assert!(!prepared[0].readable);
    assert!(prepared[0].record.blocked_fingerprint.is_some());
    assert!(prepared[0].record.pending_owner_wrap.is_none());
    assert_eq!(count(&seen, "/owner-key"), 0, "{seen:?}");
    assert_eq!(count(&seen, &format!("/shared-folders/{root}/key")), 0, "{seen:?}");
    assert!(!bridge.db().has_seen_folder_custody(root).unwrap());
}

#[test]
fn sf2265_prepare_never_pins_l8_under_a_legacy_proven_key() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000012";
    let item = "20000000-0000-4000-8000-000000000012";
    let folder = FileKey::from_bytes([0x8d; 32]);
    // A real key is not valid UTF-8 (L9 rule 1); the child wrap is the proof.
    let child = beebeeb_core::kdf::derive_file_key(&master, item.as_bytes());
    let wrap = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&folder, child.as_bytes()).unwrap());
    // Review 2276 item 1: the fixture must be able to tell the two behaviours
    // apart. The folder's L7 inventory really holds a child wrap bound to this
    // item and this folder key, and the item's name opens under that child key,
    // so a legacy-proven folder key that is (wrongly) treated as custody would
    // resolve the item and pin an L8 envelope under it (5f3fa53).
    let child_wrap =
        STANDARD.encode(beebeeb_sync::key_resolution::wrap_child_key(&folder, root, item, 1, &child).unwrap());
    let sealed = beebeeb_core::encrypt::encrypt_name_for(
        &child,
        "shared.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let invites = json!({"invites":[{
        "id":"inv-12","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-28T00:00:00Z",
        "encrypted_owner_folder_key":legacy_copy(&master, root, &folder)
    }]});
    let (fixture, seen) = prepare_world(
        root,
        invites,
        json!({"keys":[{"file_id":item,"encrypted_file_key":wrap}]}),
        json!({"keys":[{"file_id":item,"encrypted_file_key":child_wrap}],"next_cursor":null}),
        (200, json!({})),
        None,
    );
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    let mut row = foreign_child(item, root);
    row["name_encrypted"] = json!(sealed);
    let prepared = run(bridge.prepare_owned_rows(&[row])).expect("a legacy-proven key must not fail the snapshot");
    assert_eq!(prepared.len(), 1);
    assert!(prepared[0].record.blocked_fingerprint.is_some());
    assert!(prepared[0].record.owner_wrap.is_none());
    assert!(
        prepared[0].record.pending_owner_wrap.is_none(),
        "no L8 envelope under a legacy key"
    );
    assert_eq!(count(&seen, "/owner-key"), 0, "{seen:?}");
    assert_eq!(count(&seen, &format!("/shared-folders/{root}/key")), 0, "{seen:?}");
    assert_eq!(
        count(&seen, &format!("/shared-folders/{root}/keys")),
        0,
        "no L7 child resolution under a legacy key: {seen:?}"
    );
    let error = run(bridge.shared_folder_custody_key(root))
        .err()
        .expect("a legacy-proven key is not custody");
    assert!(error.to_string().contains("no custody"), "{error}");
}

#[test]
fn sf2265_prepare_swallows_a_501_from_the_custody_put() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000013";
    let item = "20000000-0000-4000-8000-000000000013";
    let folder = FileKey::from_bytes([55; 32]);
    let invites = json!({"invites":[{
        "id":"inv-13","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-28T00:00:00Z",
        "encrypted_owner_folder_key":purpose_bound_copy(&master, root, &folder)
    }]});
    // Exactly the server's feature-gate refusal (feature_gate.rs).
    let (fixture, seen) = prepare_world(
        root,
        invites,
        json!({"keys":[]}),
        json!({"keys":[],"next_cursor":null}),
        (
            501,
            json!({"error":"shared_folders_v1","message":"This server does not support shared folders yet."}),
        ),
        None,
    );
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    let prepared = run(bridge.prepare_owned_rows(&[foreign_child(item, root)]))
        .unwrap_or_else(|error| panic!("a 501 from the custody PUT must not fail the snapshot: {error}; {seen:?}"));
    assert_eq!(prepared.len(), 1);
    assert!(prepared[0].record.blocked_fingerprint.is_some());
    assert_eq!(
        count(&seen, &format!("PUT /api/v1/shared-folders/{root}/key")),
        1,
        "{seen:?}"
    );
    assert_eq!(
        count(&seen, "/folder-keys"),
        0,
        "never retried as a legacy read: {seen:?}"
    );
    assert_eq!(count(&seen, "/owner-key"), 0, "{seen:?}");
    assert!(!bridge.db().has_seen_folder_custody(root).unwrap());
}

#[test]
fn sf2265_prepare_rereads_the_winner_after_key_version_conflict() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000014";
    let item = "20000000-0000-4000-8000-000000000014";
    let folder = FileKey::from_bytes([55; 32]);
    let winner_key = FileKey::from_bytes([0x44; 32]);
    let invites = json!({"invites":[{
        "id":"inv-14","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-28T00:00:00Z",
        "encrypted_owner_folder_key":purpose_bound_copy(&master, root, &folder)
    }]});
    let winner = json!({
        "folder_id":root,"role":"owner","key_version":2,
        "owner_wrapped_folder_key":STANDARD.encode(
            beebeeb_sync::key_resolution::wrap_folder_custody(&master, root, &winner_key).unwrap()
        )
    });
    let (fixture, seen) = prepare_world(
        root,
        invites,
        json!({"keys":[]}),
        json!({"keys":[],"next_cursor":null}),
        (
            409,
            json!({"error":"key_version_conflict","message":"The folder key was re-wrapped at a newer version; re-read and retry."}),
        ),
        Some(winner),
    );
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    let prepared = run(bridge.prepare_owned_rows(&[foreign_child(item, root)]))
        .expect("a lost registration race is re-read, not fatal");
    assert_eq!(prepared.len(), 1);
    assert_eq!(
        count(&seen, &format!("PUT /api/v1/shared-folders/{root}/key")),
        1,
        "{seen:?}"
    );
    assert_eq!(
        exact(&seen, &format!("GET /api/v1/shared-folders/{root}")),
        2,
        "the stale record, then the winner: {seen:?}"
    );
    assert!(bridge.db().has_seen_folder_custody(root).unwrap());
    let custody = run(bridge.shared_folder_custody_key(root)).unwrap();
    assert_eq!(custody.as_bytes(), winner_key.as_bytes());
}

#[test]
fn sf2265_prepare_asks_custody_once_per_folder_not_once_per_row() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000015";
    let folder = FileKey::from_bytes([55; 32]);
    let invites = json!({"invites":[{
        "id":"inv-15","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-28T00:00:00Z",
        "encrypted_owner_folder_key":purpose_bound_copy(&master, root, &folder)
    }]});
    // A typed failure to remember (review P3-4): without the per-folder cache
    // the batch repeats the GET and the 501 PUT once per row.
    let (fixture, seen) = prepare_world(
        root,
        invites,
        json!({"keys":[]}),
        json!({"keys":[],"next_cursor":null}),
        (
            501,
            json!({"error":"shared_folders_v1","message":"This server does not support shared folders yet."}),
        ),
        None,
    );
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    let rows: Vec<Value> = (0..3)
        .map(|n| foreign_child(&format!("20000000-0000-4000-8000-00000000002{n}"), root))
        .collect();
    let prepared = run(bridge.prepare_owned_rows(&rows)).unwrap();
    assert_eq!(prepared.len(), 3);
    assert!(prepared.iter().all(|row| !row.readable));
    assert_eq!(
        count(&seen, &format!("PUT /api/v1/shared-folders/{root}/key")),
        1,
        "{seen:?}"
    );
    assert_eq!(
        exact(&seen, &format!("GET /api/v1/shared-folders/{root}")),
        1,
        "{seen:?}"
    );
    assert_eq!(count(&seen, "GET /api/v1/shares/invites/sent"), 1, "{seen:?}");
}

#[test]
fn sf2265_one_invites_missing_rows_does_not_abort_the_proof_search() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000016";
    let item = "20000000-0000-4000-8000-000000000016";
    let folder = FileKey::from_bytes([0x8d; 32]);
    let child = beebeeb_core::kdf::derive_file_key(&master, item.as_bytes());
    let wrap = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&folder, child.as_bytes()).unwrap());
    // The newer invite's rows are gone (a race); the older one still proves.
    let invites = json!({"invites":[
        {"id":"inv-16-new","file_id":root,"is_folder_share":true,"status":"approved",
         "created_at":"2026-10-28T00:00:00Z","encrypted_owner_folder_key":legacy_copy(&master, root, &folder)},
        {"id":"inv-16-old","file_id":root,"is_folder_share":true,"status":"approved",
         "created_at":"2026-10-20T00:00:00Z","encrypted_owner_folder_key":legacy_copy(&master, root, &folder)}
    ]});
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let (status, body) = if req.path == "/api/v1/shares/invites/sent" {
            (200, invites.clone())
        } else if req.path == "/api/v1/shares/invites/inv-16-new/folder-keys" {
            (400, json!({"error":"invite_not_approved"}))
        } else if req.path == "/api/v1/shares/invites/inv-16-old/folder-keys" {
            (200, json!({"keys":[{"file_id":item,"encrypted_file_key":wrap}]}))
        } else {
            (404, json!({}))
        };
        Some((status, serde_json::to_vec(&body).unwrap()))
    });
    let db = StateDb::open(":memory:").unwrap();
    let api = fixture.api();
    let stop = AtomicBool::new(false);
    let scope = AccountScope::new(&db, &api, &stop);
    let custody = run(Batch::default().invite_owner_key(&scope, &master, root))
        .expect("a gone invite must not abort the batch")
        .expect("the older approved invite still proves the key");
    assert_eq!(custody.key.as_bytes(), folder.as_bytes());
    assert!(!custody.remember);
    let seen = seen.lock().unwrap().clone();
    assert!(
        seen.iter().any(|line| line.contains("inv-16-new/folder-keys")),
        "{seen:?}"
    );
    assert!(
        seen.iter().any(|line| line.contains("inv-16-old/folder-keys")),
        "{seen:?}"
    );
}

#[test]
fn sf2265_purpose_bound_invites_are_tried_before_newer_legacy_ones() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000017";
    let item = "20000000-0000-4000-8000-000000000017";
    let purpose_key = FileKey::from_bytes([0x51; 32]);
    let legacy_key = FileKey::from_bytes([0x8d; 32]);
    let child = beebeeb_core::kdf::derive_file_key(&master, item.as_bytes());
    let wrap = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&legacy_key, child.as_bytes()).unwrap());
    // The newer invite is legacy and would prove; the older one is
    // purpose-bound and must win (review P3-6).
    let invites = json!({"invites":[
        {"id":"inv-17-legacy","file_id":root,"is_folder_share":true,"status":"approved",
         "created_at":"2026-10-28T00:00:00Z","encrypted_owner_folder_key":legacy_copy(&master, root, &legacy_key)},
        {"id":"inv-17-purposed","file_id":root,"is_folder_share":true,"status":"approved",
         "created_at":"2026-10-20T00:00:00Z","encrypted_owner_folder_key":purpose_bound_copy(&master, root, &purpose_key)}
    ]});
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let bodies = Arc::new(Mutex::new(Vec::<Value>::new()));
    let (log, body_log, stored) = (seen.clone(), bodies.clone(), bodies.clone());
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let (status, body) = if req.path == "/api/v1/shares/invites/sent" {
            (200, invites.clone())
        } else if req.path.ends_with("/folder-keys") {
            (200, json!({"keys":[{"file_id":item,"encrypted_file_key":wrap}]}))
        } else if req.path == format!("/api/v1/shared-folders/{root}/key") && req.method == "PUT" {
            body_log.lock().unwrap().push(req.body.clone());
            (200, json!({"created":true,"key_version":1}))
        } else if req.path == format!("/api/v1/shared-folders/{root}") {
            match stored.lock().unwrap().first() {
                Some(body) => (
                    200,
                    json!({"folder_id":root,"role":"owner","key_version":1,
                           "owner_wrapped_folder_key":body["owner_wrapped_folder_key"]}),
                ),
                None => (404, json!({})),
            }
        } else {
            (404, json!({}))
        };
        Some((status, serde_json::to_vec(&body).unwrap()))
    });
    let db = StateDb::open(":memory:").unwrap();
    let api = fixture.api();
    let stop = AtomicBool::new(false);
    let scope = AccountScope::new(&db, &api, &stop);
    let custody = run(Batch::default().invite_owner_key(&scope, &master, root))
        .unwrap()
        .expect("the purpose-bound invite opens");
    assert_eq!(custody.key.as_bytes(), purpose_key.as_bytes());
    assert!(custody.remember, "a purpose-bound copy registers custody");
    assert_eq!(bodies.lock().unwrap().len(), 1);
    assert_eq!(count(&seen, "/folder-keys"), 0, "no legacy read at all: {seen:?}");
}

#[test]
fn sf2276_invite_rows_failures_that_are_not_gone_abort_the_proof_search() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000018";
    let item = "20000000-0000-4000-8000-000000000018";
    let folder = FileKey::from_bytes([0x8d; 32]);
    let child = beebeeb_core::kdf::derive_file_key(&master, item.as_bytes());
    let wrap = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&folder, child.as_bytes()).unwrap());
    let invites = json!({"invites":[
        {"id":"inv-18-new","file_id":root,"is_folder_share":true,"status":"approved",
         "created_at":"2026-10-28T00:00:00Z","encrypted_owner_folder_key":legacy_copy(&master, root, &folder)},
        {"id":"inv-18-old","file_id":root,"is_folder_share":true,"status":"approved",
         "created_at":"2026-10-20T00:00:00Z","encrypted_owner_folder_key":legacy_copy(&master, root, &folder)}
    ]});
    // Review P3-A: only the server's "gone / not approved" answers on the
    // proof rows (400 invite-not-approved, 404 invite missing, invites.rs) may
    // skip an approved invite. Auth failures, other client errors and server
    // errors are the batch's failure, never a reason to read on.
    for status in [401u16, 403, 500] {
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let (log, invites, wrap) = (seen.clone(), invites.clone(), wrap.clone());
        let fixture = Fixture::wire(move |req| {
            log.lock().unwrap().push(format!("{} {}", req.method, req.path));
            let (code, body) = if req.path == "/api/v1/shares/invites/sent" {
                (200, invites.clone())
            } else if req.path == "/api/v1/shares/invites/inv-18-new/folder-keys" {
                (status, json!({"error":"unexpected"}))
            } else if req.path == "/api/v1/shares/invites/inv-18-old/folder-keys" {
                (200, json!({"keys":[{"file_id":item,"encrypted_file_key":wrap}]}))
            } else {
                (404, json!({}))
            };
            Some((code, serde_json::to_vec(&body).unwrap()))
        });
        let db = StateDb::open(":memory:").unwrap();
        let api = fixture.api();
        let stop = AtomicBool::new(false);
        let scope = AccountScope::new(&db, &api, &stop);
        let error = run(Batch::default().invite_owner_key(&scope, &master, root))
            .err()
            .unwrap_or_else(|| panic!("a {status} from the proof rows must abort the batch"));
        assert!(error.to_string().contains(&status.to_string()), "{status}: {error}");
        let seen = seen.lock().unwrap().clone();
        assert!(
            !seen.iter().any(|line| line.contains("inv-18-old")),
            "a {status} is not 'try the next invite': {seen:?}"
        );
    }
}

#[test]
fn sf2276_a_404_from_a_gone_invite_still_tries_the_next_one() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000019";
    let item = "20000000-0000-4000-8000-000000000019";
    let folder = FileKey::from_bytes([0x8d; 32]);
    let child = beebeeb_core::kdf::derive_file_key(&master, item.as_bytes());
    let wrap = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&folder, child.as_bytes()).unwrap());
    let invites = json!({"invites":[
        {"id":"inv-19-new","file_id":root,"is_folder_share":true,"status":"approved",
         "created_at":"2026-10-28T00:00:00Z","encrypted_owner_folder_key":legacy_copy(&master, root, &folder)},
        {"id":"inv-19-old","file_id":root,"is_folder_share":true,"status":"approved",
         "created_at":"2026-10-20T00:00:00Z","encrypted_owner_folder_key":legacy_copy(&master, root, &folder)}
    ]});
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let (status, body) = if req.path == "/api/v1/shares/invites/sent" {
            (200, invites.clone())
        } else if req.path == "/api/v1/shares/invites/inv-19-new/folder-keys" {
            (404, json!({"error":"not_found"}))
        } else if req.path == "/api/v1/shares/invites/inv-19-old/folder-keys" {
            (200, json!({"keys":[{"file_id":item,"encrypted_file_key":wrap}]}))
        } else {
            (404, json!({}))
        };
        Some((status, serde_json::to_vec(&body).unwrap()))
    });
    let db = StateDb::open(":memory:").unwrap();
    let api = fixture.api();
    let stop = AtomicBool::new(false);
    let scope = AccountScope::new(&db, &api, &stop);
    let custody = run(Batch::default().invite_owner_key(&scope, &master, root))
        .expect("a 404 from one invite must not abort the batch")
        .expect("the older approved invite still proves the key");
    assert_eq!(custody.key.as_bytes(), folder.as_bytes());
    assert!(!custody.remember);
    let seen = seen.lock().unwrap().clone();
    assert!(
        seen.iter().any(|line| line.contains("inv-19-old/folder-keys")),
        "{seen:?}"
    );
}

#[test]
fn sf2276_a_no_custody_repair_stops_retrying_the_index() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000020";
    let item = "20000000-0000-4000-8000-000000000020";
    let folder = FileKey::from_bytes([0x8d; 32]);
    let child = beebeeb_core::kdf::derive_file_key(&master, item.as_bytes());
    let wrap = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&folder, child.as_bytes()).unwrap());
    let invites = json!({"invites":[{
        "id":"inv-20","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-28T00:00:00Z",
        "encrypted_owner_folder_key":legacy_copy(&master, root, &folder)
    }]});
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let (status, body) = if req.path == "/api/v1/shares/invites/sent" {
            (200, invites.clone())
        } else if req.path.ends_with("/folder-keys") {
            (200, json!({"keys":[{"file_id":item,"encrypted_file_key":wrap}]}))
        } else {
            (404, json!({}))
        };
        Some((status, serde_json::to_vec(&body).unwrap()))
    });
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(StateDb::open(temp.path().join("db")).unwrap());
    let bridge = crate::engine_bridge::EngineBridge::new(db.clone(), Arc::new(fixture.api()));
    db.mark_shared_index_dirty(root).unwrap();
    run(bridge.maintain_owned_keys(1_000)).unwrap();
    let after_first = seen.lock().unwrap().len();
    assert!(after_first > 0, "{seen:?}");
    // Review P3-B: a legacy-proven folder has no custody, and no retry can
    // change that before its owner upgrades. The maintenance pass clears the
    // dirty marker at the revision it read instead of asking again every 30 s.
    assert_eq!(
        db.shared_index_dirty_revision(root).unwrap(),
        None,
        "the index marker is cleared: {seen:?}"
    );
    run(bridge.maintain_owned_keys(2_000)).unwrap();
    assert_eq!(
        seen.lock().unwrap().len(),
        after_first,
        "a second pass makes no requests: {seen:?}"
    );
}

#[test]
fn sf2276_a_404_from_the_custody_put_does_not_abort_the_snapshot() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000021";
    let item = "20000000-0000-4000-8000-000000000021";
    let folder = FileKey::from_bytes([55; 32]);
    let invites = json!({"invites":[{
        "id":"inv-21","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-28T00:00:00Z",
        "encrypted_owner_folder_key":purpose_bound_copy(&master, root, &folder)
    }]});
    // Review P3-C: the server answers 404 from the custody PUT when the folder
    // is missing, not owned, or trashed (shared_folders.rs:293-304) and when
    // the stored row is gone or owned by someone else (:650). A snapshot can
    // hold a live row under a trashed folder (its query has no ancestor
    // check), so this is a typed no-custody, not a batch failure.
    let (fixture, seen) = prepare_world(
        root,
        invites,
        json!({"keys":[]}),
        json!({"keys":[],"next_cursor":null}),
        (404, json!({"error":"not_found"})),
        None,
    );
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    let rows = vec![
        foreign_child(item, root),
        foreign_child("20000000-0000-4000-8000-000000000221", root),
    ];
    let prepared = run(bridge.prepare_owned_rows(&rows))
        .unwrap_or_else(|error| panic!("a 404 from the custody PUT must not abort the snapshot: {error}; {seen:?}"));
    assert_eq!(prepared.len(), 2);
    assert!(prepared.iter().all(|row| !row.readable));
    assert!(prepared.iter().all(|row| row.record.blocked_fingerprint.is_some()));
    assert!(prepared.iter().all(|row| row.record.pending_owner_wrap.is_none()));
    assert_eq!(
        count(&seen, &format!("PUT /api/v1/shared-folders/{root}/key")),
        1,
        "one typed no-custody per folder, not per row: {seen:?}"
    );
    assert_eq!(
        exact(&seen, &format!("GET /api/v1/shared-folders/{root}")),
        1,
        "{seen:?}"
    );
    assert_eq!(
        count(&seen, "/folder-keys"),
        0,
        "a typed no-custody is never retried as a legacy read: {seen:?}"
    );
    assert_eq!(count(&seen, "/owner-key"), 0, "{seen:?}");
    assert!(!bridge.db().has_seen_folder_custody(root).unwrap());
}

#[test]
fn sf2276_a_501_that_does_not_name_the_feature_stays_an_error() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000022";
    let item = "20000000-0000-4000-8000-000000000022";
    let folder = FileKey::from_bytes([55; 32]);
    let invites = json!({"invites":[{
        "id":"inv-22","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-28T00:00:00Z",
        "encrypted_owner_folder_key":purpose_bound_copy(&master, root, &folder)
    }]});
    // Review P3-D: the 501 that means "no custody here" is the shared-folders
    // feature gate, and it names itself in its body. Any other 501 is a server
    // failure the snapshot must surface.
    let (fixture, seen) = prepare_world(
        root,
        invites,
        json!({"keys":[]}),
        json!({"keys":[],"next_cursor":null}),
        (
            501,
            json!({"error":"not_implemented_here","message":"some other refusal"}),
        ),
        None,
    );
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    let error = run(bridge.prepare_owned_rows(&[foreign_child(item, root)]))
        .err()
        .unwrap_or_else(|| panic!("only the feature gate's own 501 is 'no custody': {seen:?}"));
    assert!(error.to_string().contains("not_implemented_here"), "{error}");
}

/// Review 2279 item 1: index work that cannot run before custody is not
/// dropped. It is deferred durably, makes no request on later passes, and the
/// custody GET that finally opens revives it even though the rows never change.
#[test]
fn sf2279_index_repair_waits_for_custody_and_resumes_when_it_appears() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000030";
    let item = "20000000-0000-4000-8000-000000000030";
    let folder = FileKey::from_bytes([55; 32]);
    let child = FileKey::from_bytes([66; 32]);
    let child_wrap =
        STANDARD.encode(beebeeb_sync::key_resolution::wrap_child_key(&folder, root, item, 1, &child).unwrap());
    let item_name = beebeeb_core::encrypt::encrypt_name_for(
        &child,
        "holiday.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let row = json!({"id":item,"name_encrypted":item_name,"parent_id":root,"is_folder":false,
        "version_number":1,"size_bytes":4,"updated_at":1,
        "acted_via_share_id":"fixture-share","uploaded_by":null,
        "owner_wrapped_file_key":null});
    let available = Arc::new(Mutex::new(false));
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let (log, ready) = (seen.clone(), available.clone());
    let served_row = row.clone();
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let ready = *ready.lock().unwrap();
        let (status, body) = if req.path == "/api/v1/shares/invites/sent" {
            (200, json!({"invites":[]}))
        } else if req.path == format!("/api/v1/shared-folders/{root}") && !ready {
            (404, json!({}))
        } else if req.path == format!("/api/v1/shared-folders/{root}") {
            (
                200,
                json!({"folder_id":root,"role":"owner","key_version":1,
                "owner_wrapped_folder_key":STANDARD.encode(
                    beebeeb_sync::key_resolution::wrap_folder_custody(&master, root, &folder).unwrap())}),
            )
        } else if req.path.starts_with(&format!("/api/v1/shared-folders/{root}/keys")) {
            (
                200,
                json!({"keys":[{"file_id":item,"encrypted_file_key":child_wrap}],"next_cursor":null}),
            )
        } else if req.path.starts_with("/api/v1/files?") {
            (200, json!({"files":[served_row.clone()],"next_cursor":null}))
        } else if req.path == format!("/api/v1/shared-folders/{root}/search-index/shards") {
            (200, json!({"shards":[]}))
        } else if req.method == "PUT" && req.path.contains("/search-index/shards/") {
            (200, json!({"bucket":0,"page":0,"version":1}))
        } else {
            (404, json!({}))
        };
        Some((status, serde_json::to_vec(&body).unwrap()))
    });
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(StateDb::open(temp.path().join("db")).unwrap());
    let bridge = crate::engine_bridge::EngineBridge::new(db.clone(), Arc::new(fixture.api()));
    db.mark_shared_index_dirty(root).unwrap();
    run(bridge.maintain_owned_keys(1_000)).unwrap();
    assert!(
        db.dirty_shared_index_roots().unwrap().is_empty(),
        "a no-custody root is deferred, not retried: {seen:?}"
    );
    assert_eq!(
        count(&seen, "/search-index/"),
        0,
        "no index work without custody: {seen:?}"
    );
    let after_first = seen.lock().unwrap().len();
    run(bridge.maintain_owned_keys(2_000)).unwrap();
    assert_eq!(
        seen.lock().unwrap().len(),
        after_first,
        "a later pass makes no request at all: {seen:?}"
    );
    // The owner upgrades the folder; its rows do not change.
    *available.lock().unwrap() = true;
    run(bridge.shared_folder_custody_key(root)).unwrap();
    assert_eq!(
        db.dirty_shared_index_roots().unwrap(),
        vec![root.to_string()],
        "the custody GET that opens revives the deferred repair: {seen:?}"
    );
    run(bridge.maintain_owned_keys(3_000)).unwrap();
    assert!(
        count(&seen, "/search-index/shards/") > 0,
        "the next pass repairs the index: {seen:?}"
    );
    // Ingesting the row for the first time legitimately re-marks the root
    // during that same pass (the row's fingerprint changed), so the marker
    // clears on the pass after that one.
    run(bridge.maintain_owned_keys(4_000)).unwrap();
    assert!(db.dirty_shared_index_roots().unwrap().is_empty(), "{seen:?}");
}

/// Review 2279 item 2 / R1: an `upgrade_required` folder is deferred like "no
/// custody" instead of sending the same refused custody PUT every 30 seconds.
#[test]
fn sf2279_an_upgrade_required_folder_is_not_retried_every_pass() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000031";
    let folder = FileKey::from_bytes([0x9a; 32]);
    let invites = json!({"invites":[{
        "id":"inv-31","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-29T00:00:00Z",
        "encrypted_owner_folder_key":purpose_bound_copy(&master, root, &folder)
    }]});
    let (fixture, seen) = prepare_world(
        root,
        invites,
        json!({"keys":[]}),
        json!({"keys":[],"next_cursor":null}),
        (409, json!({"error":"upgrade_required"})),
        None,
    );
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(StateDb::open(temp.path().join("db")).unwrap());
    let bridge = crate::engine_bridge::EngineBridge::new(db.clone(), Arc::new(fixture.api()));
    db.mark_shared_index_dirty(root).unwrap();
    run(bridge.maintain_owned_keys(1_000)).unwrap();
    let put = format!("PUT /api/v1/shared-folders/{root}/key");
    assert_eq!(exact(&seen, &put), 1, "{seen:?}");
    run(bridge.maintain_owned_keys(2_000)).unwrap();
    assert_eq!(
        exact(&seen, &put),
        1,
        "an upgrade-required folder makes no custody PUT on the second pass: {seen:?}"
    );
}

/// Review 2279 item 3 / R2 / L7c: a custody 404 for a folder this device has
/// seen with custody refuses only that row. It is never read as legacy, the
/// other rows proceed, and the seen-custody fact is never forgotten.
#[test]
fn sf2279_a_seen_custody_404_blocks_only_its_row() {
    use beebeeb_core::kdf::MasterKey;
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000032";
    let item = "20000000-0000-4000-8000-000000000032";
    let owned = "40000000-0000-4000-8000-000000000032";
    let owned_key = beebeeb_core::kdf::derive_file_key(&master, owned.as_bytes());
    let owned_name = beebeeb_core::encrypt::encrypt_name_for(
        &owned_key,
        "owned.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let owned_row = json!({"id":owned,"name_encrypted":owned_name,"parent_id":root,
        "is_folder":false,"version_number":1,"size_bytes":4,"updated_at":1});
    let (fixture, seen) = prepare_world(
        root,
        json!({"invites":[]}),
        json!({"keys":[]}),
        json!({"keys":[],"next_cursor":null}),
        (404, json!({})),
        None,
    );
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(StateDb::open(temp.path().join("db")).unwrap());
    let bridge = crate::engine_bridge::EngineBridge::new(db.clone(), Arc::new(fixture.api()));
    // This device has seen custody for the folder before (L7c/L7d).
    db.remember_folder_custody_at_revision(root, db.account_revision())
        .unwrap();
    let prepared = run(bridge.prepare_owned_rows(&[foreign_child(item, root), owned_row]))
        .unwrap_or_else(|error| panic!("a seen-custody 404 must not abort the snapshot: {error}; {seen:?}"));
    assert_eq!(prepared.len(), 2);
    let blocked = prepared.iter().find(|row| row.record.file_id == item).unwrap();
    assert!(!blocked.readable, "the refused row is blocked");
    assert!(blocked.record.blocked_fingerprint.is_some());
    assert!(
        blocked.record.shared_root_id.is_none(),
        "no root is recorded for a refused row"
    );
    assert!(
        prepared
            .iter()
            .find(|row| row.record.file_id == owned)
            .unwrap()
            .readable,
        "the other rows proceed: {seen:?}"
    );
    assert_eq!(count(&seen, "/folder-keys"), 0, "never read as legacy: {seen:?}");
    assert_eq!(
        count(&seen, "/shared-folders"),
        1,
        "one custody ask per folder: {seen:?}"
    );
    assert!(
        db.has_seen_folder_custody(root).unwrap(),
        "the seen-custody fact is not forgotten"
    );
}

/// L7d F1 (review 2279): a folder this device has seen with custody is never
/// deferred. Its dirty marker stays, so maintenance asks again (one custody GET
/// per 30 s pass) instead of hiding the work forever behind a deferral.
#[test]
fn sf2279_a_seen_custody_root_keeps_its_dirty_marker() {
    let root = "10000000-0000-4000-8000-000000000033";
    let (fixture, seen) = prepare_world(
        root,
        json!({"invites":[]}),
        json!({"keys":[]}),
        json!({"keys":[],"next_cursor":null}),
        (404, json!({})),
        None,
    );
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(StateDb::open(temp.path().join("db")).unwrap());
    let bridge = crate::engine_bridge::EngineBridge::new(db.clone(), Arc::new(fixture.api()));
    db.remember_folder_custody_at_revision(root, db.account_revision())
        .unwrap();
    db.mark_shared_index_dirty(root).unwrap();
    let ask = format!("GET /api/v1/shared-folders/{root}");
    run(bridge.maintain_owned_keys(1_000)).unwrap();
    assert_eq!(
        db.dirty_shared_index_roots().unwrap(),
        vec![root.to_string()],
        "the dirty marker stays for a seen-custody root: {seen:?}"
    );
    assert!(
        !db.revive_deferred_shared_index(root).unwrap(),
        "no deferral is written: {seen:?}"
    );
    assert_eq!(exact(&seen, &ask), 1, "one custody ask: {seen:?}");
    assert_eq!(count(&seen, "/search-index/"), 0, "no index work: {seen:?}");
    run(bridge.maintain_owned_keys(2_000)).unwrap();
    assert_eq!(
        exact(&seen, &ask),
        2,
        "the next pass asks again, as on main before this PR: {seen:?}"
    );
    assert_eq!(db.dirty_shared_index_roots().unwrap(), vec![root.to_string()]);
}

/// L7d F3 (review 2279): the deferral is written BEFORE the custody lookup, so
/// a custody GET that opens while the repair runs revives the work instead of
/// being stranded behind a deferral written after that revive. The fixture's
/// handler writes custody from its own thread while it answers 404 — the exact
/// interleaving the old ordering lost. The next pass then repairs the index.
#[test]
fn sf2279_a_custody_write_during_the_pass_is_not_stranded() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000034";
    let item = "20000000-0000-4000-8000-000000000034";
    let folder = FileKey::from_bytes([55; 32]);
    let child = FileKey::from_bytes([66; 32]);
    let child_wrap =
        STANDARD.encode(beebeeb_sync::key_resolution::wrap_child_key(&folder, root, item, 1, &child).unwrap());
    let item_name = beebeeb_core::encrypt::encrypt_name_for(
        &child,
        "holiday.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let row = json!({"id":item,"name_encrypted":item_name,"parent_id":root,"is_folder":false,
        "version_number":1,"size_bytes":4,"updated_at":1,
        "acted_via_share_id":"fixture-share","uploaded_by":null,
        "owner_wrapped_file_key":null});
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(StateDb::open(temp.path().join("db")).unwrap());
    let revision = db.account_revision();
    let available = Arc::new(Mutex::new(false));
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let (log, ready, handle) = (seen.clone(), available.clone(), db.clone());
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let ready = *ready.lock().unwrap();
        let (status, body) = if req.path == "/api/v1/shares/invites/sent" {
            (200, json!({"invites":[]}))
        } else if req.path == format!("/api/v1/shared-folders/{root}") && !ready {
            // Custody opens while the repair is in flight: the production
            // `AccountScope::remember_custody` inserts the fact, then revives
            // the deferred index work.
            handle.remember_folder_custody_at_revision(root, revision).unwrap();
            handle.revive_deferred_shared_index(root).unwrap();
            (404, json!({}))
        } else if req.path == format!("/api/v1/shared-folders/{root}") {
            (
                200,
                json!({"folder_id":root,"role":"owner","key_version":1,
                "owner_wrapped_folder_key":STANDARD.encode(
                    beebeeb_sync::key_resolution::wrap_folder_custody(&master, root, &folder).unwrap())}),
            )
        } else if req.path.starts_with(&format!("/api/v1/shared-folders/{root}/keys")) {
            (
                200,
                json!({"keys":[{"file_id":item,"encrypted_file_key":child_wrap}],"next_cursor":null}),
            )
        } else if req.path.starts_with("/api/v1/files?") {
            (200, json!({"files":[row.clone()],"next_cursor":null}))
        } else if req.path == format!("/api/v1/shared-folders/{root}/search-index/shards") {
            (200, json!({"shards":[]}))
        } else if req.method == "PUT" && req.path.contains("/search-index/shards/") {
            (200, json!({"bucket":0,"page":0,"version":1}))
        } else {
            (404, json!({}))
        };
        Some((status, serde_json::to_vec(&body).unwrap()))
    });
    let bridge = crate::engine_bridge::EngineBridge::new(db.clone(), Arc::new(fixture.api()));
    db.mark_shared_index_dirty(root).unwrap();
    let report = run(bridge.repair_shared_folder_index(root)).unwrap();
    assert_eq!(
        report.deferred, 0,
        "a custody write during the pass revives instead of deferring: {seen:?}"
    );
    assert_eq!(
        db.dirty_shared_index_roots().unwrap(),
        vec![root.to_string()],
        "the revive re-dirties the root: {seen:?}"
    );
    assert!(
        !db.revive_deferred_shared_index(root).unwrap(),
        "no deferral is left behind: {seen:?}"
    );
    assert_eq!(
        count(&seen, "/search-index/"),
        0,
        "nothing ran before custody was open: {seen:?}"
    );
    *available.lock().unwrap() = true;
    run(bridge.maintain_owned_keys(2_000)).unwrap();
    assert!(
        count(&seen, "/search-index/shards/") > 0,
        "the next pass repairs the index: {seen:?}"
    );
}

/// The production walk revives a deferred root: `prepare_owned_rows` opens the
/// root's custody record for a row, and `remember_custody` re-dirties the
/// index. The resume test above drives `shared_folder_custody_key` directly
/// (the reviewer's note); this one goes through the snapshot walk the index
/// repair and the activity feed also use.
#[test]
fn sf2279_prepare_owned_rows_revives_a_deferred_root() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000035";
    let item = "20000000-0000-4000-8000-000000000035";
    let folder = FileKey::from_bytes([55; 32]);
    let child = FileKey::from_bytes([66; 32]);
    let child_wrap =
        STANDARD.encode(beebeeb_sync::key_resolution::wrap_child_key(&folder, root, item, 1, &child).unwrap());
    let item_name = beebeeb_core::encrypt::encrypt_name_for(
        &child,
        "holiday.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let row = json!({"id":item,"name_encrypted":item_name,"parent_id":root,"is_folder":false,
        "version_number":1,"size_bytes":4,"updated_at":1,
        "acted_via_share_id":"fixture-share","uploaded_by":null,
        "owner_wrapped_file_key":null});
    let available = Arc::new(Mutex::new(false));
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let (log, ready) = (seen.clone(), available.clone());
    let served_row = row.clone();
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let ready = *ready.lock().unwrap();
        let (status, body) = if req.path == "/api/v1/shares/invites/sent" {
            (200, json!({"invites":[]}))
        } else if req.path == format!("/api/v1/shared-folders/{root}") && !ready {
            (404, json!({}))
        } else if req.path == format!("/api/v1/shared-folders/{root}") {
            (
                200,
                json!({"folder_id":root,"role":"owner","key_version":1,
                "owner_wrapped_folder_key":STANDARD.encode(
                    beebeeb_sync::key_resolution::wrap_folder_custody(&master, root, &folder).unwrap())}),
            )
        } else if req.path.starts_with(&format!("/api/v1/shared-folders/{root}/keys")) {
            (
                200,
                json!({"keys":[{"file_id":item,"encrypted_file_key":child_wrap}],"next_cursor":null}),
            )
        } else if req.path.starts_with("/api/v1/files?") {
            (200, json!({"files":[served_row.clone()],"next_cursor":null}))
        } else if req.path == format!("/api/v1/shared-folders/{root}/search-index/shards") {
            (200, json!({"shards":[]}))
        } else if req.method == "PUT" && req.path.contains("/search-index/shards/") {
            (200, json!({"bucket":0,"page":0,"version":1}))
        } else {
            (404, json!({}))
        };
        Some((status, serde_json::to_vec(&body).unwrap()))
    });
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(StateDb::open(temp.path().join("db")).unwrap());
    let bridge = crate::engine_bridge::EngineBridge::new(db.clone(), Arc::new(fixture.api()));
    db.mark_shared_index_dirty(root).unwrap();
    let report = run(bridge.repair_shared_folder_index(root)).unwrap();
    assert_eq!(report.deferred, 1, "a never-seen root is deferred: {seen:?}");
    assert!(
        db.dirty_shared_index_roots().unwrap().is_empty(),
        "the deferral replaces the dirty marker: {seen:?}"
    );
    // The owner upgrades the folder; its rows do not change.
    *available.lock().unwrap() = true;
    let prepared = run(bridge.prepare_owned_rows(std::slice::from_ref(&row))).unwrap();
    assert_eq!(prepared.len(), 1);
    assert!(prepared[0].readable, "the walk opens the row: {seen:?}");
    assert_eq!(
        db.dirty_shared_index_roots().unwrap(),
        vec![root.to_string()],
        "the walk's remember_custody revives the deferred root: {seen:?}"
    );
    run(bridge.maintain_owned_keys(2_000)).unwrap();
    assert!(
        count(&seen, "/search-index/shards/") > 0,
        "the next pass repairs the index: {seen:?}"
    );
}
