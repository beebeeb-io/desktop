//! Other devices' backups at the engine: the read over a tree of plain manifests (what is listed, what is left out,
//! the honest rows), and the glue against the fake server on loopback (the session rule, the root, the delete with
//! a real OPAQUE proof). The read through real sealed names and encrypted chunks is in
//! `engine_bridge/tests/backup_layout.rs` (`other_devices_are_read_through_their_sealed_manifests`). Every name is
//! invented.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use beebeeb_core::backup::{DeviceSelf, MANIFEST_NAME};
use beebeeb_types::backup::{Category, Platform, SystemRole};

use super::*;
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::app::EngineConfig;
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{BackupLayout, FileEntry, FileStatus, ItemKind, StateDb};
use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE};
use crate::test_api::{ConfirmTokens, FakeApi, Reply, Request};

// ---- the read, over a tree of plain manifests ----------------------------------------------------------------------

/// A folder tree in memory: each folder's children, each file's plaintext (`None`: it does not open), and the
/// folders whose listing or download fails.
#[derive(Default)]
struct Tree {
    children: HashMap<String, (Vec<BackupChild>, bool)>,
    files: HashMap<String, Option<Vec<u8>>>,
    unreachable: Vec<String>,
}

impl Tree {
    fn folder(&mut self, parent: &str, id: &str, name: Option<&str>) {
        self.children
            .entry(parent.into())
            .or_default()
            .0
            .push(child(id, name, true));
        self.children.entry(id.into()).or_insert((Vec::new(), true));
    }
    fn file(&mut self, parent: &str, id: &str, name: Option<&str>, plain: Option<&str>) {
        self.children
            .entry(parent.into())
            .or_default()
            .0
            .push(child(id, name, false));
        self.files.insert(id.into(), plain.map(|text| text.as_bytes().to_vec()));
    }
    fn manifest(&mut self, folder: &str, id: &str, text: &str) {
        self.file(folder, id, Some(MANIFEST_NAME), Some(text));
    }
    fn cut_short(&mut self, folder: &str) {
        self.children.entry(folder.into()).or_default().1 = false;
    }
}

fn child(id: &str, name: Option<&str>, is_folder: bool) -> BackupChild {
    BackupChild {
        id: id.into(),
        name: name.map(str::to_string),
        is_folder,
        size: 10,
        uploaded_at_ms: 0,
        chunk_count: 1,
        record: None,
    }
}

fn unreachable() -> EngineError {
    EngineError::Api(anyhow::anyhow!("no answer"))
}

impl DeviceTree for Tree {
    async fn children(&self, parent: &str) -> Result<(Vec<BackupChild>, bool), EngineError> {
        if self.unreachable.iter().any(|id| id == parent) {
            return Err(unreachable());
        }
        Ok(self.children.get(parent).cloned().unwrap_or((Vec::new(), true)))
    }
    async fn small_file(&self, file: &BackupChild) -> Result<Option<Vec<u8>>, EngineError> {
        if self.unreachable.iter().any(|id| id == &file.id) {
            return Err(unreachable());
        }
        Ok(self.files.get(&file.id).cloned().flatten())
    }
}

const ROOT: &str = "00000000-0000-4000-8000-0000000000b0";
const THIS_PC: &str = "00000000-0000-4000-8000-0000000000b1";
const MAC: &str = "00000000-0000-4000-8000-0000000000b2";
const PHONE: &str = "00000000-0000-4000-8000-0000000000b3";
const ME_AGAIN: &str = "00000000-0000-4000-8000-0000000000b4";

fn me() -> DeviceSelf {
    DeviceSelf {
        device_id: beebeeb_core::backup::device_id_from_machine_id("4c4c4544-0000-0000-0000-00000000aaaa"),
        name: "Invented PC".into(),
        platform: Platform::Windows,
    }
}

fn mac_manifest() -> String {
    let mac = DeviceSelf {
        device_id: beebeeb_core::backup::device_id_from_machine_id("4c4c4544-0000-0000-0000-00000000bbbb"),
        name: "Invented Studio".into(),
        platform: Platform::Macos,
    };
    beebeeb_core::backup::serialize_manifest(&crate::backup::layout::device_manifest(
        &mac,
        &[Category::Pictures, Category::Documents, Category::Music],
    ))
}

/// As the iOS app writes it (`BackupService.ts`): no platform, a model, `last_sync` never filled in.
fn phone_manifest(model: &str, last_sync: Option<&str>) -> String {
    serde_json::json!({
        "version": 1, "device_id": "ios-invented-01", "device_name": "Invented phone", "device_model": model,
        "os_version": "26.5", "app_version": "1.0.0", "created_at": "2026-10-01T10:00:00Z",
        "backups": {
            "camera_roll": { "enabled": true, "last_sync": last_sync, "items_synced": 0 },
            "contacts": { "enabled": false, "last_sync": null, "items_synced": 0 },
        },
    })
    .to_string()
}

fn read(tree: &Tree, this_pc: Option<&str>) -> Result<(Vec<BackupDevice>, bool), AccountCallError> {
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    runtime.block_on(read_devices(tree, ROOT, this_pc, Some(&me())))
}

#[test]
fn two_devices_are_listed_and_this_pc_is_left_out_by_its_folder_and_by_its_manifest() {
    let mut tree = Tree::default();
    tree.folder(ROOT, THIS_PC, Some("Invented PC"));
    tree.folder(ROOT, MAC, Some("Invented Studio"));
    tree.manifest(MAC, "m-mac", &mac_manifest());
    // Two backed-up folders are in the Mac's folder now; its manifest named three at setup.
    tree.folder(MAC, "mac-pictures", Some("Pictures"));
    tree.folder(MAC, "mac-documents", Some("Documents"));
    tree.folder(ROOT, PHONE, Some("Invented phone"));
    tree.manifest(PHONE, "m-phone", &phone_manifest("iPhone 17 Pro", None));
    tree.folder(PHONE, "phone-photos", Some("Photos"));
    // This PC again, under another folder (a layout not cached yet): its manifest names this PC's id.
    tree.folder(ROOT, ME_AGAIN, Some("Invented PC (2)"));
    let mine = beebeeb_core::backup::serialize_manifest(&crate::backup::layout::device_manifest(&me(), &[]));
    tree.manifest(ME_AGAIN, "m-me", &mine);
    // A file at the root is not a device.
    tree.file(ROOT, "loose", Some("notes.txt"), Some("x"));

    let (devices, complete) = read(&tree, Some(THIS_PC)).unwrap();
    assert!(complete);
    let ids: Vec<&str> = devices.iter().map(|d| d.folder_id.as_str()).collect();
    assert_eq!(ids, [MAC, PHONE]);
    let DeviceRead::Read(mac) = &devices[0].read else {
        panic!("the Mac's manifest is read: {:?}", devices[0].read);
    };
    assert_eq!(mac.name.as_deref(), Some("Invented Studio"));
    assert_eq!(mac.class, DeviceClass::Computer);
    assert_eq!(
        mac.folders, 2,
        "the folders listed now, not the manifest's setup-time list"
    );
    assert_eq!(mac.last_backup_ms, None, "a desktop's manifest names no pass time");
    assert_eq!(mac.model, None);
    let DeviceRead::Read(phone) = &devices[1].read else {
        panic!("the phone's manifest is read");
    };
    assert_eq!(phone.class, DeviceClass::Phone);
    assert_eq!(phone.model.as_deref(), Some("iPhone 17 Pro"));
    assert_eq!(phone.folders, 1);
    assert_eq!(phone.last_backup_ms, None, "iOS never fills in last_sync");
    assert_eq!(devices[1].folder_name.as_deref(), Some("Invented phone"));
    // Names never reach a {:?}.
    let printed = format!("{devices:?}");
    assert!(!printed.contains("Invented") && !printed.contains("iPhone") && !printed.contains(MAC));
}

#[test]
fn a_device_this_pc_cannot_read_is_a_row_that_says_so() {
    let mut tree = Tree::default();
    // The manifest does not open on this PC (no key, no record, a frame that does not open).
    tree.folder(ROOT, "d-closed", Some("A"));
    tree.file("d-closed", "m-closed", Some(MANIFEST_NAME), None);
    // It opens and is not JSON.
    tree.folder(ROOT, "d-garbled", Some("B"));
    tree.manifest("d-garbled", "m-garbled", "not json at all");
    // A file whose name does not open: it could be the manifest.
    tree.folder(ROOT, "d-unnamed", None);
    tree.file("d-unnamed", "m-unnamed", None, Some("{}"));
    // No manifest at all: an old mirror's folder.
    tree.folder(ROOT, "d-none", Some("D"));
    tree.folder("d-none", "d-none-pictures", Some("Pictures"));
    // No manifest in a listing that was cut short: it may be in the part not listed.
    tree.folder(ROOT, "d-short", Some("E"));
    tree.cut_short("d-short");
    // The folder's listing fails; another's manifest download fails.
    tree.folder(ROOT, "d-gone", Some("F"));
    tree.unreachable.push("d-gone".into());
    tree.folder(ROOT, "d-dropped", Some("G"));
    tree.manifest("d-dropped", "m-dropped", &mac_manifest());
    tree.unreachable.push("m-dropped".into());

    let (devices, _) = read(&tree, None).unwrap();
    let reads: Vec<(&str, &DeviceRead)> = devices.iter().map(|d| (d.folder_id.as_str(), &d.read)).collect();
    assert_eq!(
        reads,
        [
            ("d-closed", &DeviceRead::Unreadable),
            ("d-garbled", &DeviceRead::Unreadable),
            ("d-unnamed", &DeviceRead::Unreadable),
            ("d-none", &DeviceRead::NoStatusFile),
            ("d-short", &DeviceRead::NotReached),
            ("d-gone", &DeviceRead::NotReached),
            ("d-dropped", &DeviceRead::NotReached),
        ]
    );
    assert_eq!(devices[2].folder_name, None, "a name that does not open stays unknown");
}

#[test]
fn a_root_that_cannot_be_listed_is_a_refusal_and_a_cut_short_one_says_so() {
    let mut tree = Tree::default();
    tree.unreachable.push(ROOT.into());
    assert_eq!(read(&tree, None).unwrap_err(), AccountCallError::ServerError);

    let mut tree = Tree::default();
    tree.folder(ROOT, MAC, Some("Invented Studio"));
    tree.manifest(MAC, "m-mac", &mac_manifest());
    tree.cut_short(ROOT);
    let (devices, complete) = read(&tree, None).unwrap();
    assert!(!complete, "a listing cut short is said, not passed off as every device");
    assert_eq!(devices.len(), 1);
}

#[test]
fn a_summary_names_only_what_the_manifest_says() {
    let parse = |text: &str| beebeeb_core::backup::parse_manifest(text).unwrap();
    let ipad = summary_of(
        &parse(&phone_manifest("iPad Air 11-inch", Some("2026-11-14T08:30:00Z"))),
        1,
    );
    assert_eq!(ipad.class, DeviceClass::Tablet);
    assert_eq!(
        ipad.last_backup_ms,
        Some(
            chrono::DateTime::parse_from_rfc3339("2026-11-14T08:30:00Z")
                .unwrap()
                .timestamp_millis()
        ),
        "a sync time the file does name is shown"
    );
    let bare = summary_of(&parse(r#"{"device_name": "  "}"#), 0);
    assert_eq!((bare.name, bare.class, bare.model), (None, DeviceClass::Unknown, None));
    let linux = summary_of(
        &parse(r#"{"device_name":"Box","platform":"linux","device_model":"x"}"#),
        3,
    );
    assert_eq!(
        (linux.class, linux.model, linux.folders),
        (DeviceClass::Computer, None, 3)
    );
    let passed = summary_of(
        &parse(
            r#"{"platform":"windows","folders":[{"category":"pictures","last_pass_at_ms":5},{"category":"music","last_pass_at_ms":9}]}"#,
        ),
        2,
    );
    assert_eq!(passed.last_backup_ms, Some(9));
    let long = "M".repeat(200);
    let model = summary_of(&parse(&phone_manifest(&long, None)), 0).model.unwrap();
    assert_eq!(model.chars().count(), MODEL_MAX);
}

// ---- the glue, against the fake server -----------------------------------------------------------------------------

const PASSWORD: &str = "an invented password";
const DEV_MAC: &str = "00000000-0000-4000-8000-0000000000c2";
const DEV_OWN: &str = "00000000-0000-4000-8000-0000000000c1";
const NOT_SYNCED: &str = "00000000-0000-4000-8000-0000000000c9";
const GLUE_ROOT: &str = "00000000-0000-4000-8000-0000000000c0";

type TestEngine = Engine<MemoryCredentials>;

fn account() -> Arc<FakeAccount> {
    static ACCOUNT: OnceLock<Arc<FakeAccount>> = OnceLock::new();
    ACCOUNT
        .get_or_init(|| Arc::new(FakeAccount::register("backup-devices@example.eu", PASSWORD, TEST_PHRASE).0))
        .clone()
}

/// What the fake answers: the flagged root (`None`: 404), each folder's listing, and the status of a listing that
/// fails.
#[derive(Default)]
struct Answers {
    root: Option<String>,
    listings: HashMap<String, Vec<serde_json::Value>>,
    failing: Option<u16>,
}

struct Rig {
    engine: Arc<TestEngine>,
    fake: FakeApi,
    session: SessionGeneration,
    deleted: Arc<Mutex<Vec<String>>>,
    _dir: tempfile::TempDir,
}

fn folder_json(id: &str) -> serde_json::Value {
    // A name sealed for no key this PC holds: it does not open (the row is kept, nameless).
    serde_json::json!({ "id": id, "name_encrypted": "AAAA", "is_folder": true, "is_trashed": false,
        "size_bytes": 0, "chunk_count": 0, "uploaded_at": "2026-11-01T10:00:00Z" })
}

fn row(db: &StateDb, file_id: &str, path: &str, parent_id: Option<&str>) {
    let entry = FileEntry {
        file_id: file_id.into(),
        path: path.into(),
        status: FileStatus::CloudOnly,
        size_bytes: 0,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: parent_id.map(str::to_string),
        item_kind: ItemKind::Folder,
    };
    db.upsert_file_with_contract(&entry, |contract| {
        contract.parent_id = entry.parent_id.clone();
        contract.item_kind = entry.item_kind.clone();
    })
    .unwrap();
}

/// A signed-in engine whose local state holds Backups/{Invented PC (this PC, the layout's), Mac}.
fn rig(answers: Answers, cached_layout: bool) -> Rig {
    let account = account();
    let tokens = Arc::new(ConfirmTokens::default());
    let deleted = Arc::new(Mutex::new(Vec::new()));
    let (fake_account, deleted_in, token) = (account.clone(), deleted.clone(), account.session_token.clone());
    let fake = FakeApi::start(Arc::new(move |req: &Request| {
        if req.bearer() != Some(token.as_str()) {
            return Reply::error(401, "unauthorized");
        }
        match req.route().as_str() {
            "POST /api/v1/auth/confirm-opaque-start" => fake_account.opaque.login_start(&req.json()),
            "POST /api/v1/auth/confirm-opaque-finish" => {
                if fake_account.opaque.login_finish(&req.json()) {
                    Reply::ok(serde_json::json!({ "confirmation_token": tokens.mint(), "expires_at": "soon" }))
                } else {
                    Reply::error(401, "invalid_credentials")
                }
            }
            "GET /api/v1/backups/root" => match &answers.root {
                Some(id) => Reply::ok(serde_json::json!({ "id": id })),
                None => Reply::error(404, "not_found"),
            },
            "GET /api/v1/files" => {
                if let Some(status) = answers.failing {
                    return Reply::error(status, "failed");
                }
                let parent = req.query("parent_id").unwrap_or_default();
                let files = answers.listings.get(parent).cloned().unwrap_or_default();
                Reply::ok(serde_json::json!({ "files": files, "next_cursor": null }))
            }
            route => match route.strip_prefix("DELETE /api/v1/files/") {
                Some(id) if tokens.spend(req) => {
                    deleted_in.lock().unwrap().push(id.to_string());
                    Reply::empty(204)
                }
                Some(_) => Reply::error(403, "step_up_required"),
                None => Reply::error(404, "not_found"),
            },
        }
    }));
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "backup-devices-test-slot".into(),
        state_dir,
        api_base: fake.base.clone(),
    };
    let engine = Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"));
    let dir = tempfile::tempdir().unwrap();
    let state_path = dir.path().join("state.db");
    let db = StateDb::open(&state_path).unwrap();
    db.set_owner(&Identity::new(Some(&account.user_id), Some(&account.email)))
        .unwrap();
    row(&db, GLUE_ROOT, "Backups", None);
    row(&db, DEV_OWN, "Backups/Invented PC", Some(GLUE_ROOT));
    row(&db, DEV_MAC, "Backups/Invented Studio", Some(GLUE_ROOT));
    assert!(db.set_system_role(GLUE_ROOT, Some(SystemRole::BackupsRoot)).unwrap());
    if cached_layout {
        let layout = BackupLayout {
            root_id: GLUE_ROOT.into(),
            device_folder_id: DEV_OWN.into(),
            device_folder_name: "Invented PC".into(),
        };
        assert!(db.set_backup_layout(&layout).unwrap());
    }
    drop(db);
    engine.use_local_state_for_test(state_path);
    let session = sign_in(&engine, &account);
    Rig {
        engine,
        fake,
        session,
        deleted,
        _dir: dir,
    }
}

fn sign_in(engine: &TestEngine, account: &FakeAccount) -> SessionGeneration {
    {
        let _writes = engine.writes.write().unwrap();
        engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        *engine.acct.session.lock().unwrap() = Some(Session {
            token: account.session_token.clone(),
            master_key: account.master_key,
            email: Some(account.email.clone()),
        });
        engine.remember_session_user_id(Some(account.user_id.clone()));
        engine.set_auth_present(true);
    }
    engine.session_turn()
}

fn routes(rig: &Rig) -> Vec<String> {
    rig.fake.requests().iter().map(Request::route).collect()
}

#[test]
fn nothing_is_read_or_deleted_for_a_session_that_is_gone() {
    let rig = rig(Answers::default(), true);
    rig.engine.forget_session(SessionTransition::SignOut);
    assert_eq!(
        rig.engine.backup_devices_for(rig.session).unwrap_err(),
        DevicesRefused::NotSignedIn
    );
    assert_eq!(
        rig.engine
            .delete_device_backup_for(rig.session, DEV_MAC, PASSWORD.as_bytes())
            .unwrap_err(),
        DeleteBackupError::SignedOut
    );
    // The same account signs in again: a press of the earlier session is still not this one's.
    let now = sign_in(&rig.engine, &account());
    assert_ne!(now, rig.session);
    assert_eq!(
        rig.engine
            .delete_device_backup_for(rig.session, DEV_MAC, PASSWORD.as_bytes())
            .unwrap_err(),
        DeleteBackupError::SignedOut
    );
    assert_eq!(routes(&rig), Vec::<String>::new(), "nothing was sent");
    assert!(rig.deleted.lock().unwrap().is_empty());
}

#[test]
fn no_root_means_no_other_devices_and_nothing_is_claimed() {
    let rig = rig(Answers::default(), false);
    let read = rig.engine.backup_devices_for(rig.session).unwrap();
    assert_eq!(read.session, rig.session);
    assert!(read.devices.is_empty() && read.complete);
    assert_eq!(
        routes(&rig),
        ["GET /api/v1/backups/root"],
        "read, never claimed or created"
    );
}

#[test]
fn the_cached_root_is_listed_and_this_pcs_folder_is_left_out() {
    let mut listings = HashMap::new();
    listings.insert(GLUE_ROOT.to_string(), vec![folder_json(DEV_OWN), folder_json(DEV_MAC)]);
    let rig = rig(
        Answers {
            listings,
            ..Answers::default()
        },
        true,
    );
    let read = rig.engine.backup_devices_for(rig.session).unwrap();
    assert_eq!(read.devices.len(), 1);
    assert_eq!(read.devices[0].folder_id, DEV_MAC);
    assert_eq!(
        read.devices[0].folder_name, None,
        "a name that does not open stays unknown"
    );
    assert_eq!(read.devices[0].read, DeviceRead::NoStatusFile);
    let routes = routes(&rig);
    assert!(
        !routes.iter().any(|r| r.contains("/backups/root")),
        "the cached root is used"
    );
    assert_eq!(routes.len(), 2, "the root's listing and the Mac's: {routes:?}");
}

#[test]
fn a_failed_root_listing_is_a_refusal_not_an_empty_list() {
    let rig = rig(
        Answers {
            root: Some(GLUE_ROOT.into()),
            failing: Some(500),
            ..Answers::default()
        },
        false,
    );
    assert!(matches!(
        rig.engine.backup_devices_for(rig.session),
        Err(DevicesRefused::Call(AccountCallError::ServerError))
    ));
}

#[test]
fn a_wrong_password_deletes_nothing_and_the_right_one_deletes_that_one_backup() {
    let rig = rig(Answers::default(), true);
    assert_eq!(
        rig.engine
            .delete_device_backup_for(rig.session, DEV_MAC, b"not the password")
            .unwrap_err(),
        DeleteBackupError::WrongPassword
    );
    assert!(rig.deleted.lock().unwrap().is_empty());
    assert!(!routes(&rig).iter().any(|r| r.starts_with("DELETE ")));

    assert_eq!(
        rig.engine
            .delete_device_backup_for(rig.session, DEV_MAC, PASSWORD.as_bytes()),
        Ok(())
    );
    assert_eq!(*rig.deleted.lock().unwrap(), [DEV_MAC]);
}

#[test]
fn a_device_this_pc_has_not_synced_yet_and_the_root_are_refused_with_nothing_sent() {
    let rig = rig(Answers::default(), true);
    assert_eq!(
        rig.engine
            .delete_device_backup_for(rig.session, NOT_SYNCED, PASSWORD.as_bytes())
            .unwrap_err(),
        DeleteBackupError::NotFound
    );
    assert!(matches!(
        rig.engine
            .delete_device_backup_for(rig.session, GLUE_ROOT, PASSWORD.as_bytes()),
        Err(DeleteBackupError::Refused(_))
    ));
    assert_eq!(routes(&rig), Vec::<String>::new(), "nothing was sent");
}
