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
    body: Value,
}
struct Fixture {
    base: String,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Fixture {
    fn new(mut answer: impl FnMut(Request) -> Option<Value> + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let thread = thread::spawn(move || {
            while !signal.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let request = read_request(&mut stream);
                        if let Some(body) = answer(request) {
                            let bytes = serde_json::to_vec(&body).unwrap();
                            let header = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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
        self.thread.take().unwrap().join().unwrap();
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
    Request { path, body }
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
    let envelope = beebeeb_sync::key_resolution::wrap_key_to_owner(&master, "item", &key).unwrap();
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
