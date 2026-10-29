use super::*;
use crate::engine_bridge::{EngineBridge, sync_tick_outcome};
use beebeeb_core::{
    chunk_stream::ExpectedChunkFormat,
    kdf::{FileKey, MasterKey},
    search_index::{EncryptedShard, SearchIndex, ShardKeySource},
};
use beebeeb_sync::key_resolution::{wrap_child_key, wrap_folder_custody, wrap_owner_file_key};
use std::collections::BTreeMap;
const ROOT: &str = "10000000-0000-4000-8000-000000000001";
const FOLDER: &str = "20000000-0000-4000-8000-000000000002";
const GUEST: &str = "30000000-0000-4000-8000-000000000003";
const SECOND: &str = "40000000-0000-4000-8000-000000000004";
fn master() -> MasterKey {
    MasterKey::from_bytes([7; 32])
}
fn folder_key() -> FileKey {
    FileKey::from_bytes([55; 32])
}
fn guest_key() -> FileKey {
    FileKey::from_bytes([66; 32])
}
fn name(key: &FileKey, text: &str) -> String {
    beebeeb_core::encrypt::encrypt_name_for(key, text, None, ExpectedChunkFormat::Unrecorded).unwrap()
}
fn owned_node(id: &str, text: &str, parent: Option<&str>, folder: bool) -> Value {
    let key = beebeeb_core::kdf::derive_file_key(&master(), id.as_bytes());
    json!({"id":id,"name_encrypted":name(&key,text),"parent_id":parent,"is_folder":folder,"version_number":1,"size_bytes":4,"updated_at":1})
}
fn foreign_node(id: &str, text: &str, parent: &str, folder: bool, key: &FileKey) -> Value {
    json!({"id":id,"name_encrypted":name(key,text),"parent_id":parent,"is_folder":folder,"version_number":1,"size_bytes":4,"updated_at":1,"acted_via_share_id":"fixture-share","uploaded_by":null,"owner_wrapped_file_key":null})
}
struct World {
    rows: BTreeMap<String, Value>,
    inventory: Vec<Value>,
    requests: Vec<Request>,
    snapshot: Vec<Value>,
    ops: Vec<Value>,
    seq: i64,
    adoption_successes: usize,
    offline: bool,
    timeout_commit: bool,
    custody_revoked: bool,
    conflicts: usize,
    writes: usize,
    shards: BTreeMap<(u32, u32), (i64, Vec<u8>)>,
    activity: Vec<Value>,
    /// The shared-folders feature gate is off: the custody GET answers 404 and
    /// the custody PUT 501 `shared_folders_v1` (server `feature_gate.rs`).
    custody_flag_off: bool,
    invite_owner_copy: Option<String>,
}
impl World {
    fn new(nested: bool) -> Self {
        let root = owned_node(ROOT, "shared", None, true);
        let guest = foreign_node(
            GUEST,
            "holiday.txt",
            if nested { FOLDER } else { ROOT },
            false,
            &guest_key(),
        );
        let mut rows = BTreeMap::from([(ROOT.into(), root), (GUEST.into(), guest)]);
        let mut inventory = vec![
            json!({"file_id":GUEST,"encrypted_file_key":STANDARD.encode(wrap_child_key(&folder_key(),ROOT,GUEST,1,&guest_key()).unwrap())}),
        ];
        if nested {
            rows.insert(
                FOLDER.into(),
                foreign_node(FOLDER, "trips", ROOT, true, &FileKey::from_bytes([77; 32])),
            );
            inventory.push(json!({"file_id":FOLDER,"encrypted_file_key":STANDARD.encode(wrap_child_key(&folder_key(),ROOT,FOLDER,2,&FileKey::from_bytes([77;32])).unwrap())}));
        }
        let snapshot = rows.values().cloned().collect();
        Self {
            rows,
            inventory,
            requests: Vec::new(),
            snapshot,
            ops: Vec::new(),
            seq: 1,
            adoption_successes: 0,
            offline: false,
            timeout_commit: false,
            custody_revoked: false,
            conflicts: 0,
            writes: 0,
            shards: BTreeMap::new(),
            activity: Vec::new(),
            custody_flag_off: false,
            invite_owner_copy: None,
        }
    }
    fn fixture(world: Arc<Mutex<Self>>) -> Fixture {
        Fixture::wire(move |req| world.lock().unwrap().reply(req))
    }
    fn reply(&mut self, req: Request) -> Option<(u16, Vec<u8>)> {
        let path = req.path.clone();
        let method = req.method.clone();
        self.requests.push(req.clone());
        let json_reply = |status, body: Value| Some((status, serde_json::to_vec(&body).unwrap()));
        if self.offline {
            return json_reply(503, json!({"code":"fixture_offline"}));
        }
        if path == "/api/v1/sync/snapshot" {
            return json_reply(200, json!({"seq_id":self.seq,"nodes":self.snapshot}));
        }
        if path.starts_with("/api/v1/sync/ops") {
            return json_reply(200, json!({"ops":self.ops,"since":self.seq,"head_seq":self.seq}));
        }
        if path == "/api/v1/shares/invites/sent" {
            let invites = self
                .invite_owner_copy
                .as_ref()
                .map(|copy| {
                    vec![json!({"id":"inv-flag-off","file_id":ROOT,"is_folder_share":true,
                                "status":"approved","created_at":"2026-10-28T00:00:00Z",
                                "encrypted_owner_folder_key":copy})]
                })
                .unwrap_or_default();
            return json_reply(200, json!({"invites":invites}));
        }
        if path == format!("/api/v1/shared-folders/{ROOT}") && self.custody_flag_off {
            return json_reply(404, json!({}));
        }
        if path == format!("/api/v1/shared-folders/{ROOT}/key") && method == "PUT" && self.custody_flag_off {
            return json_reply(
                501,
                json!({"error":"shared_folders_v1","message":"This server does not support shared folders yet."}),
            );
        }
        if path == format!("/api/v1/shared-folders/{ROOT}/activity") {
            return json_reply(200, json!({"activity":self.activity}));
        }
        if path == format!("/api/v1/shared-folders/{ROOT}") && !self.custody_revoked {
            return json_reply(
                200,
                json!({"folder_id":ROOT,"role":"owner","key_version":1,"owner_wrapped_folder_key":STANDARD.encode(wrap_folder_custody(&master(),ROOT,&folder_key()).unwrap())}),
            );
        }
        if path.starts_with(&format!("/api/v1/shared-folders/{ROOT}/keys")) && !self.custody_revoked {
            return json_reply(200, json!({"keys":self.inventory,"next_cursor":null}));
        }
        if path.starts_with("/api/v1/files?") {
            let parent = path.split("parent_id=").nth(1).unwrap_or("").split('&').next().unwrap();
            return json_reply(
                200,
                json!({"files":self.rows.values().filter(|r|r["parent_id"].as_str()==Some(parent)).cloned().collect::<Vec<_>>(),"next_cursor":null}),
            );
        }
        let index_prefix = format!("/api/v1/shared-folders/{ROOT}/search-index/shards");
        if path == index_prefix {
            return json_reply(
                200,
                json!({"shards":self.shards.iter().map(|((bucket,page),(version,_))|json!({"bucket":bucket,"page":page,"version":version})).collect::<Vec<_>>()}),
            );
        }
        if let Some(suffix) = path.strip_prefix(&(index_prefix + "/")) {
            let coords = suffix
                .split('?')
                .next()
                .unwrap()
                .split('/')
                .map(|n| n.parse::<u32>().unwrap())
                .collect::<Vec<_>>();
            let coord = (coords[0], coords[1]);
            if method == "GET" {
                return self
                    .shards
                    .get(&coord)
                    .map(|(_, bytes)| (200, bytes.clone()))
                    .or_else(|| json_reply(404, json!({})));
            }
            self.writes += 1;
            if self.conflicts > 0 {
                self.conflicts -= 1;
                return json_reply(409, json!({"code":"index_version_conflict"}));
            }
            let version = self.shards.get(&coord).map_or(1, |(v, _)| v + 1);
            if method == "DELETE" {
                self.shards.remove(&coord);
            } else {
                self.shards.insert(coord, (version, req.bytes));
            }
            return json_reply(200, json!({"bucket":coord.0,"page":coord.1,"version":version}));
        }
        if let Some(suffix) = path.strip_prefix("/api/v1/files/") {
            if let Some(id) = suffix.strip_suffix("/owner-key") {
                let row = self.rows.get_mut(id).unwrap();
                let adopted = row["owner_wrapped_file_key"].is_null();
                if adopted {
                    row["owner_wrapped_file_key"] = req.body["owner_wrapped_file_key"].clone();
                    self.adoption_successes += 1;
                }
                if self.timeout_commit {
                    self.timeout_commit = false;
                    self.offline = true;
                    return None;
                }
                return json_reply(200, json!({"adopted":adopted}));
            }
            return json_reply(
                if self.rows.contains_key(suffix) { 200 } else { 404 },
                self.rows.get(suffix).cloned().unwrap_or(json!({})),
            );
        }
        json_reply(404, json!({"code":"fixture_absent"}))
    }
}
fn bridge(db: &std::path::Path, fixture: &Fixture) -> EngineBridge {
    EngineBridge::new(Arc::new(StateDb::open(db).unwrap()), Arc::new(fixture.api()))
}
fn tick(bridge: &EngineBridge, root: &std::path::Path) {
    run(sync_tick_outcome(bridge, root)).unwrap();
}

#[test]
fn sf2237d_t4_snapshot_without_key_fields() {
    let world = Arc::new(Mutex::new(World::new(false)));
    {
        let mut w = world.lock().unwrap();
        for row in &mut w.snapshot {
            let obj = row.as_object_mut().unwrap();
            obj.remove("acted_via_share_id");
            obj.remove("uploaded_by");
            obj.remove("owner_wrapped_file_key");
        }
    }
    let fixture = World::fixture(world.clone());
    let temp = tempfile::tempdir().unwrap();
    let b = bridge(&temp.path().join("db"), &fixture);
    tick(&b, temp.path());
    assert_eq!(b.db().get_file(GUEST).unwrap().unwrap().path, "shared/holiday.txt");
    assert_eq!(world.lock().unwrap().adoption_successes, 1);
    assert_eq!(b.db().get_sync_cursor().unwrap(), Some(1));
}
#[test]
fn sf2237d_t4_sparse_create_and_update() {
    let world = Arc::new(Mutex::new(World::new(false)));
    world.lock().unwrap().snapshot.retain(|r| r["id"] == ROOT);
    let fixture = World::fixture(world.clone());
    let temp = tempfile::tempdir().unwrap();
    let b = bridge(&temp.path().join("db"), &fixture);
    tick(&b, temp.path());
    {
        let mut w = world.lock().unwrap();
        w.seq = 2;
        w.ops =
            vec![json!({"seq_id":2,"op_type":"file_create","payload":{"id":GUEST,"parent_id":ROOT,"size_bytes":4}})];
    }
    tick(&b, temp.path());
    assert_eq!(b.db().get_file(GUEST).unwrap().unwrap().path, "shared/holiday.txt");
    b.db().clear_shared_index_dirty(ROOT).unwrap();
    {
        let mut w = world.lock().unwrap();
        w.seq = 3;
        w.rows.get_mut(GUEST).unwrap()["version_number"] = json!(2);
        w.ops =
            vec![json!({"seq_id":3,"op_type":"file_update","payload":{"id":GUEST,"version_number":2,"size_bytes":4}})];
    }
    tick(&b, temp.path());
    assert_eq!(
        b.db().get_file_contract_state(GUEST).unwrap().unwrap().current_version,
        2
    );
    assert_eq!(b.db().dirty_shared_index_roots().unwrap(), vec![ROOT.to_string()]);
}
#[test]
fn sf2237d_t4_nested_guest_folder_names() {
    let world = Arc::new(Mutex::new(World::new(true)));
    let fixture = World::fixture(world.clone());
    let temp = tempfile::tempdir().unwrap();
    let b = bridge(&temp.path().join("db"), &fixture);
    tick(&b, temp.path());
    assert_eq!(
        b.db().get_file(GUEST).unwrap().unwrap().path,
        "shared/trips/holiday.txt"
    );
    assert_eq!(
        b.db()
            .get_owned_key_record(GUEST)
            .unwrap()
            .unwrap()
            .shared_root_id
            .as_deref(),
        Some(ROOT)
    );
    assert_eq!(world.lock().unwrap().adoption_successes, 2);
}
#[test]
fn sf2237d_t4_adopt_once_and_restart() {
    let world = Arc::new(Mutex::new(World::new(false)));
    world.lock().unwrap().timeout_commit = true;
    let fixture = World::fixture(world.clone());
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db");
    let b = bridge(&path, &fixture);
    tick(&b, temp.path());
    let pending_after_timeout = usize::from(
        b.db()
            .get_owned_key_record(GUEST)
            .unwrap()
            .unwrap()
            .pending_owner_wrap
            .is_some(),
    );
    assert_eq!(pending_after_timeout, 1);
    drop(b);
    world.lock().unwrap().offline = false;
    let b = bridge(&path, &fixture);
    run(b.retry_owner_adoptions()).unwrap();
    assert!(
        b.db()
            .get_owned_key_record(GUEST)
            .unwrap()
            .unwrap()
            .pending_owner_wrap
            .is_none()
    );
    assert_eq!(world.lock().unwrap().adoption_successes, 1);
    assert_eq!(
        run(b.resolved_owned_key(GUEST)).unwrap().as_bytes(),
        guest_key().as_bytes()
    );
}
#[test]
fn sf2237d_t4_adoption_race_revoke_move_out() {
    let world = Arc::new(Mutex::new(World::new(false)));
    let fixture = World::fixture(world.clone());
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db");
    let b = bridge(&path, &fixture);
    let row = world.lock().unwrap().rows[GUEST].clone();
    let prepared = run(b.prepare_owned_rows(&[row])).unwrap();
    assert!(prepared[0].readable);
    world.lock().unwrap().rows.get_mut(GUEST).unwrap()["owner_wrapped_file_key"] =
        json!(STANDARD.encode(wrap_owner_file_key(&master(), GUEST, &guest_key()).unwrap()));
    run(b.retry_owner_adoptions()).unwrap();
    drop(b);
    {
        let mut w = world.lock().unwrap();
        w.custody_revoked = true;
        w.rows.get_mut(GUEST).unwrap()["parent_id"] = Value::Null;
        w.requests.clear();
    }
    let b = bridge(&path, &fixture);
    assert_eq!(
        run(b.resolved_owned_key(GUEST)).unwrap().as_bytes(),
        guest_key().as_bytes()
    );
    let link_requests_after_move = world
        .lock()
        .unwrap()
        .requests
        .iter()
        .filter(|r| r.path.contains("shared-folders"))
        .count();
    assert_eq!(link_requests_after_move, 0);
}
#[test]
fn sf2237d_t4_bad_row_does_not_prune_or_upload() {
    let world = Arc::new(Mutex::new(World::new(false)));
    {
        let mut w = world.lock().unwrap();
        w.inventory[0]["encrypted_file_key"] = json!(STANDARD.encode(vec![9u8; 60]));
        let second = owned_node(SECOND, "valid.txt", None, false);
        w.rows.insert(SECOND.into(), second.clone());
        w.snapshot.push(second);
    }
    let fixture = World::fixture(world.clone());
    let temp = tempfile::tempdir().unwrap();
    let b = bridge(&temp.path().join("db"), &fixture);
    tick(&b, temp.path());
    let bad = b.db().get_file(GUEST).unwrap().unwrap();
    assert!(!bad.path.contains('{'));
    assert!(!bad.path.contains("holiday"));
    assert!(
        b.db()
            .get_owned_key_record(GUEST)
            .unwrap()
            .unwrap()
            .blocked_fingerprint
            .is_some()
    );
    assert!(b.db().get_file(SECOND).unwrap().is_some());
    assert_eq!(
        world
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|r| r.method == "DELETE" || r.path.contains("uploads/"))
            .count(),
        0
    );
    assert_eq!(b.db().get_sync_cursor().unwrap(), Some(1));
}
#[test]
fn sf2237d_t4_own_and_shared_index_repair() {
    let world = Arc::new(Mutex::new(World::new(false)));
    let fixture = World::fixture(world.clone());
    let temp = tempfile::tempdir().unwrap();
    let b = bridge(&temp.path().join("db"), &fixture);
    tick(&b, temp.path());
    let index = crate::desktop_search::build_local_index(b.db()).unwrap().index;
    let guest_id = GUEST.to_string();
    assert!(index.query("holiday").contains(&guest_id));
    b.db().mark_shared_index_dirty(ROOT).unwrap();
    world.lock().unwrap().conflicts = 3;
    let report = run(b.repair_shared_folder_index(ROOT)).unwrap();
    assert!(report.deferred > 0);
    assert!(world.lock().unwrap().writes <= 3);
    assert!(b.db().dirty_shared_index_roots().unwrap().contains(&ROOT.to_string()));
    {
        let mut w = world.lock().unwrap();
        let concurrent = SearchIndex::build(&[(SECOND.into(), "concurrent.txt".into())], 64);
        for shard in concurrent
            .encrypt_shards_with(ShardKeySource::Folder(&folder_key()))
            .unwrap()
        {
            w.shards.insert((shard.bucket, shard.page), (1, shard.blob));
        }
        w.writes = 0;
    }
    let report = run(b.repair_shared_folder_index(ROOT)).unwrap();
    assert!(report.written > 0);
    assert!(!report.partial);
    assert!(b.db().dirty_shared_index_roots().unwrap().is_empty());
    let shards: Vec<_> = world
        .lock()
        .unwrap()
        .shards
        .iter()
        .map(|((bucket, page), (_, blob))| EncryptedShard {
            bucket: *bucket,
            page: *page,
            blob: blob.clone(),
        })
        .collect();
    let shared = SearchIndex::from_encrypted_shards_with(ShardKeySource::Folder(&folder_key()), &shards, 64).unwrap();
    assert!(shared.query("holiday").contains(GUEST));
    assert!(shared.query("concurrent").contains(SECOND));
}
#[test]
fn sf2237d_t4_activity_stays_local() {
    let world = Arc::new(Mutex::new(World::new(false)));
    world.lock().unwrap().activity = vec![
        json!({"id":"event","kind":"add","item_count":1,"share_label_encrypted":name(&folder_key(),"Family"),"actor_name_encrypted":name(&folder_key(),"Lewis")}),
    ];
    let fixture = World::fixture(world.clone());
    let temp = tempfile::tempdir().unwrap();
    let b = bridge(&temp.path().join("db"), &fixture);
    let activity = run(b.shared_folder_activity(ROOT)).unwrap();
    assert_eq!(activity[0]["label"], "Family");
    assert_eq!(activity[0]["actor"], "Lewis");
    b.db()
        .put_owned_key_record(&OwnedKeyRecord {
            file_id: GUEST.into(),
            shared_root_id: Some(ROOT.into()),
            ..Default::default()
        })
        .unwrap();
    let mut feed = crate::account_dto::ActivityFeed {
        events: vec![],
        total: 0,
        page: Some(1),
    };
    run(b.append_shared_activity(&mut feed)).unwrap();
    assert_eq!(feed.events.len(), 1);
    assert_eq!(feed.events[0].subject.as_deref(), Some("Family"));
    assert!(feed.events[0].details.as_ref().unwrap().contains("Lewis"));
    assert_eq!(
        world
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|r| r.method != "GET")
            .count(),
        0
    );
    for r in &world.lock().unwrap().requests {
        let body = String::from_utf8_lossy(&r.bytes);
        assert!(!body.contains("Family") && !body.contains("Lewis") && !body.contains("holiday"));
    }
}

#[test]
fn sf2265_flag_off_501_from_the_custody_put_does_not_fail_the_snapshot() {
    let world = Arc::new(Mutex::new(World::new(false)));
    {
        let mut w = world.lock().unwrap();
        w.custody_flag_off = true;
        let blob = beebeeb_core::kdf::wrap_invite_owner_copy(&master(), ROOT.as_bytes(), &folder_key()).unwrap();
        let mut copy = blob.nonce;
        copy.extend_from_slice(&blob.ciphertext);
        w.invite_owner_copy = Some(STANDARD.encode(&copy));
    }
    let fixture = World::fixture(world.clone());
    let temp = tempfile::tempdir().unwrap();
    let b = bridge(&temp.path().join("db"), &fixture);
    // P2-2: web main writes purpose-bound owner copies with the flag off. The
    // 501 must not abort the bootstrap snapshot.
    tick(&b, temp.path());
    assert_eq!(b.db().get_sync_cursor().unwrap(), Some(1));
    let guest = b.db().get_file(GUEST).unwrap().unwrap();
    assert!(!guest.path.contains("holiday"), "the row stays blocked: {guest:?}");
    let w = world.lock().unwrap();
    assert_eq!(
        w.requests.iter().filter(|r| r.path.ends_with("/owner-key")).count(),
        0,
        "no L8 envelope is pinned"
    );
    assert_eq!(
        w.requests
            .iter()
            .filter(|r| r.path.ends_with(&format!("/shared-folders/{ROOT}/key")) && r.method == "PUT")
            .count(),
        1,
        "one custody PUT, never a legacy retry"
    );
    assert_eq!(
        w.requests.iter().filter(|r| r.path.ends_with("/folder-keys")).count(),
        0
    );
}
