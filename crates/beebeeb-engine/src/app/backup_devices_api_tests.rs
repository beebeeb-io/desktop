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
use crate::identity::TEST;
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
    /// A folder's listing, complete until `cut_short` says otherwise.
    fn listing(&mut self, folder: &str) -> &mut (Vec<BackupChild>, bool) {
        self.children.entry(folder.into()).or_insert_with(|| (Vec::new(), true))
    }
    fn folder(&mut self, parent: &str, id: &str, name: Option<&str>) {
        self.listing(parent).0.push(child(id, name, true));
        self.listing(id);
    }
    fn file(&mut self, parent: &str, id: &str, name: Option<&str>, plain: Option<&str>) {
        self.listing(parent).0.push(child(id, name, false));
        self.files.insert(id.into(), plain.map(|text| text.as_bytes().to_vec()));
    }
    fn manifest(&mut self, folder: &str, id: &str, text: &str) {
        self.file(folder, id, Some(MANIFEST_NAME), Some(text));
    }
    fn cut_short(&mut self, folder: &str) {
        self.listing(folder).1 = false;
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
    runtime.block_on(read_devices(tree, ROOT, this_pc, Some(&me()), None))
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
fn the_old_mirrors_folder_of_this_pc_is_left_out_while_no_layout_names_this_pcs_folder() {
    let legacy = "00000000-0000-4000-8000-0000000000b5";
    let mut tree = Tree::default();
    // This PC's exact name and no manifest: core adopts it as this PC's (`find_device_folder`).
    tree.folder(ROOT, legacy, Some("Invented PC"));
    tree.folder(legacy, "legacy-pictures", Some("Pictures"));
    tree.folder(ROOT, MAC, Some("Invented Studio"));
    tree.manifest(MAC, "m-mac", &mac_manifest());
    let (devices, _) = read(&tree, None).unwrap();
    let ids: Vec<&str> = devices.iter().map(|d| d.folder_id.as_str()).collect();
    assert_eq!(ids, [MAC], "never offered for delete as another device");
    // With a layout, its folder is this PC's; a manifest-less folder of the same name is listed as it is.
    let (devices, _) = read(&tree, Some(THIS_PC)).unwrap();
    assert_eq!(devices.len(), 2);
    assert_eq!(devices[0].read, DeviceRead::NoStatusFile);
}

/// Review of desktop#425: while no layout names this PC's folder, a folder of this PC's device-folder name (bare or
/// suffixed, any case) whose manifest does not name another device may be this PC's: listed as it reads, marked, and
/// never offered for delete. A name that does not open is marked too. With a layout, no row is.
#[test]
fn a_folder_that_may_be_this_pcs_is_marked_while_no_layout_names_this_pcs_folder() {
    let me = me();
    let suffixed = beebeeb_core::backup::device_folder_name(
        &me.name,
        &me.device_id,
        &[beebeeb_core::backup::TakenName {
            name: me.name.clone(),
            device_id: None,
        }],
    );
    assert_ne!(suffixed, me.name, "a collision suffix: {suffixed}");
    let mut tree = Tree::default();
    // This PC's name, a manifest that does not open: may be this PC's.
    tree.folder(ROOT, "p-closed", Some("invented pc"));
    tree.file("p-closed", "m-closed", Some(MANIFEST_NAME), None);
    // This PC's suffixed name, a listing that fails.
    tree.folder(ROOT, "p-suffixed", Some(&suffixed));
    tree.unreachable.push("p-suffixed".into());
    // This PC's name, a manifest that names another device: that device's.
    tree.folder(ROOT, "p-other", Some("Invented PC"));
    tree.manifest("p-other", "m-other", &mac_manifest());
    // Another name, a manifest that does not open: not this PC's by name.
    tree.folder(ROOT, "s-closed", Some("Invented Studio"));
    tree.file("s-closed", "m-s-closed", Some(MANIFEST_NAME), None);
    // A name that does not open, nothing readable: may be.
    tree.folder(ROOT, "nameless", None);
    tree.unreachable.push("nameless".into());

    let (devices, _) = read(&tree, None).unwrap();
    let marks: Vec<(&str, &DeviceRead, bool)> = devices
        .iter()
        .map(|d| (d.folder_id.as_str(), &d.read, d.may_be_this_pc))
        .collect();
    assert_eq!(marks.len(), 5, "every folder is listed: {marks:?}");
    assert_eq!(marks[0], ("p-closed", &DeviceRead::Unreadable, true));
    assert_eq!(marks[1], ("p-suffixed", &DeviceRead::NotReached, true));
    assert_eq!((marks[2].0, marks[2].2), ("p-other", false));
    assert_eq!(marks[3], ("s-closed", &DeviceRead::Unreadable, false));
    assert_eq!(marks[4], ("nameless", &DeviceRead::NotReached, true));
    // A layout names this PC's folder: no other row is this PC's.
    let (devices, _) = read(&tree, Some(THIS_PC)).unwrap();
    assert!(devices.iter().all(|d| !d.may_be_this_pc));
    // No machine id (macOS until its id lands): no pass ran here, so a manifest naming any device is another's,
    // and this PC's computer name alone decides, with any collision suffix.
    let pc = Some("Invented PC");
    assert!(!may_be_this_pcs(Some("Invented PC"), Owner::Another, None, pc));
    assert!(may_be_this_pcs(Some("invented pc"), Owner::Unknown, None, pc));
    assert!(may_be_this_pcs(Some("Invented PC (1a2b)"), Owner::Unknown, None, pc));
    assert!(!may_be_this_pcs(Some("Invented PC 2"), Owner::Unknown, None, pc));
    assert!(!may_be_this_pcs(Some("Invented Studio"), Owner::Unknown, None, pc));
    assert!(
        may_be_this_pcs(None, Owner::Unknown, None, pc),
        "a name that does not open"
    );
    // Neither a machine id nor a computer name: every folder that names no device may be.
    assert!(may_be_this_pcs(Some("Invented Studio"), Owner::Unknown, None, None));
    assert!(!may_be_this_pcs(Some("Invented Studio"), Owner::Another, None, None));
    assert!(!may_be_this_pcs(
        Some("Invented Studio"),
        Owner::Unknown,
        Some(&me),
        None
    ));
    assert!(may_be_this_pcs(Some("Invented Studio"), Owner::ThisPc, Some(&me), None));
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
/// A folder of this PC's device-folder name, under no layout.
const DEV_NAMED: &str = "00000000-0000-4000-8000-0000000000c3";
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
    /// Set: the fake moves this engine's session (the account signs in again) while it answers a delete it carried
    /// out, as a lock or a sign-in during the call would.
    moves_on_delete: Arc<OnceLock<std::sync::Weak<TestEngine>>>,
    _dir: tempfile::TempDir,
}

fn folder_json(id: &str) -> serde_json::Value {
    // A sealed name (an envelope, not plain text: core passes plain text through as a legacy name) that opens under
    // no key this PC holds: the row is kept, nameless.
    let sealed = r#"{"nonce":"AAAAAAAAAAAAAAAA","ciphertext":"AAAAAAAAAAAAAAAAAAAAAAAA"}"#;
    serde_json::json!({ "id": id, "name_encrypted": sealed, "is_folder": true, "is_trashed": false,
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
    let moves_on_delete = Arc::new(OnceLock::<std::sync::Weak<TestEngine>>::new());
    let moves = moves_on_delete.clone();
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
                    if let Some(engine) = moves.get().and_then(std::sync::Weak::upgrade) {
                        sign_in(&engine, &fake_account);
                    }
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
    let engine = Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the handle opens"));
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
        moves_on_delete,
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

/// The session moves while a delete runs that reached Beebeeb and moved the backup to Trash. Its
/// answer is not handed over, and it is never "signed out, nothing changed" either: it is unconfirmed, the answer
/// that may have moved something. "Stop and move the backup to Trash" (DW-1410) then leaves the folder out of the set
/// (nothing is put back) and says the move may have happened. A session gone before the call still answers signed
/// out, nothing sent (`nothing_is_read_or_deleted_for_a_session_that_is_gone`).
#[test]
fn a_session_that_moves_during_the_delete_answers_that_the_backup_may_have_moved() {
    let rig = rig(Answers::default(), true);
    rig.moves_on_delete.set(Arc::downgrade(&rig.engine)).unwrap();
    let answer = rig
        .engine
        .delete_device_backup_for(rig.session, DEV_MAC, PASSWORD.as_bytes());
    assert_eq!(*rig.deleted.lock().unwrap(), [DEV_MAC], "the delete reached Beebeeb");
    assert_ne!(rig.engine.session_turn(), rig.session, "the session moved during it");
    assert_eq!(answer, Err(DeleteBackupError::Failed("server")));

    let mut put_back = false;
    let refused = crate::app::bindings::backup_folders::remove_steps(
        Some(PASSWORD.as_bytes()),
        Some(DEV_MAC),
        || Ok(()),
        |_, _| answer.clone(),
        |()| put_back = true,
    )
    .unwrap_err();
    assert!(refused.stopped_backing_up(), "{refused:?}");
    assert!(!put_back, "the folder is not put back");
    assert!(!refused.words().contains("Nothing was changed"), "{}", refused.words());
}

/// Every answer a delete can give once its session moved. Refused before anything could move (the password, the rule,
/// a backup not found) or never run: signed out, and "nothing changed" is true. It may have moved the backup to Trash
/// (done, no answer, the server's error): unconfirmed.
#[test]
fn a_moved_sessions_answer_is_unconfirmed_only_when_the_delete_may_have_moved_something() {
    use DeleteBackupError::{Failed, NotABackup, NotFound, Offline, RateLimited, Refused, SignedOut, WrongPassword};
    let cases = [
        (None, SignedOut),
        (Some(Err(WrongPassword)), SignedOut),
        (Some(Err(RateLimited(30))), SignedOut),
        (Some(Err(NotFound)), SignedOut),
        (Some(Err(NotABackup)), SignedOut),
        (Some(Err(Refused("invented words".into()))), SignedOut),
        (Some(Err(Failed("state"))), SignedOut),
        (Some(Err(SignedOut)), SignedOut),
        (Some(Ok(())), Failed("server")),
        (Some(Err(Offline)), Failed("server")),
        (Some(Err(Failed("server"))), Failed("server")),
    ];
    for (answered, expected) in cases {
        assert_eq!(after_session_moved(answered.as_ref()), Err(expected), "{answered:?}");
    }
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

// ---- review of desktop#425: a locked screen, and this PC's own folder ----------------------------------------------

/// Red at 073aeafb by the refusal's KIND (and the census in `lock_api_tests`): the base copied the token with
/// `while_session_is`, which does not ask the screen lock, and only `local_state()` refused after it (the delete as
/// `Failed("state")`). Now both copy through `person_session`, which refuses the locked screen before anything leaves
/// the session's lock. Nothing is sent in either.
#[test]
fn a_locked_screen_reads_and_deletes_nothing() {
    let rig = rig(Answers::default(), true);
    // The lock's start rule on a stored session (as `explorer_api_tests`): the screen is locked, the turn is the one
    // read while it is, as a press drawn over the lock screen would read it.
    *rig.engine.lock_flow.lock().unwrap() = beebeeb_api::flows::LockFlow::at_start(beebeeb_api::flows::StartFacts {
        session_stored: true,
        ask_password_at_start: Some(false),
        vault_closed: Some(false),
        app_lock: Some(true),
    });
    assert_eq!(rig.engine.lock_state(), super::super::lock_api::LockState::ScreenLocked);
    let session = rig.engine.session_turn();
    assert_eq!(session, rig.session);
    assert_eq!(
        rig.engine.backup_devices_for(session).unwrap_err(),
        DevicesRefused::Local(LocalStateRefused::Locked)
    );
    assert_eq!(
        rig.engine
            .delete_device_backup_for(session, DEV_MAC, PASSWORD.as_bytes())
            .unwrap_err(),
        DeleteBackupError::SignedOut
    );
    assert_eq!(routes(&rig), Vec::<String>::new(), "nothing was sent");
    assert!(rig.deleted.lock().unwrap().is_empty());
}

/// Red at 073aeafb: the base sent the DELETE of this PC's own device folder behind the password (the list was the
/// only guard). The engine refuses it now, before the password is proven.
#[test]
fn this_pcs_own_folder_is_never_deleted_when_the_layout_names_it() {
    let rig = rig(Answers::default(), true);
    assert!(matches!(
        rig.engine
            .delete_device_backup_for(rig.session, DEV_OWN, PASSWORD.as_bytes()),
        Err(DeleteBackupError::Refused(_))
    ));
    assert_eq!(routes(&rig), Vec::<String>::new(), "nothing was sent");
    assert!(rig.deleted.lock().unwrap().is_empty());
}

/// Red at 073aeafb: with no layout yet (a reset PC's first sync), a folder of this PC's device-folder name whose
/// manifest does not open was deleted behind the password. The engine reads its manifest first and refuses it,
/// before the password is proven; a folder of another name whose manifest does not open either is still deleted.
#[test]
fn a_folder_that_may_be_this_pcs_is_not_deleted_while_no_layout_names_it() {
    // This PC as the engine sees it on this host: its device identity, else (no machine id: macOS today) its
    // computer name alone.
    let me = crate::backup::device::device_self(&TEST);
    let named = match &me {
        Some(me) => beebeeb_core::backup::device_folder_name(&me.name, &me.device_id, &[]),
        None => {
            let pc = this_pc_name(&TEST, None).expect("the host gives a computer name");
            beebeeb_core::backup::device_folder_name(&pc, "", &[])
        }
    };
    // Its only file has a name that does not open here: it may be the manifest.
    let mut unopened = folder_json("00000000-0000-4000-8000-0000000000d1");
    unopened["is_folder"] = serde_json::json!(false);
    let mut listings = HashMap::new();
    listings.insert(DEV_NAMED.to_string(), vec![unopened.clone()]);
    listings.insert(DEV_MAC.to_string(), vec![unopened]);
    let rig = rig(
        Answers {
            listings,
            ..Answers::default()
        },
        false,
    );
    let db = StateDb::open(&rig._dir.path().join("state.db")).unwrap();
    row(&db, DEV_NAMED, &format!("Backups/{named}"), Some(GLUE_ROOT));
    drop(db);
    assert!(matches!(
        rig.engine
            .delete_device_backup_for(rig.session, DEV_NAMED, PASSWORD.as_bytes()),
        Err(DeleteBackupError::Refused(_))
    ));
    assert!(
        !routes(&rig)
            .iter()
            .any(|r| r.starts_with("DELETE ") || r.contains("confirm-opaque")),
        "no proof and no delete: {:?}",
        routes(&rig)
    );
    assert!(rig.deleted.lock().unwrap().is_empty());
    // Another name: the same unreadable manifest does not make it this PC's.
    assert_eq!(
        rig.engine
            .delete_device_backup_for(rig.session, DEV_MAC, PASSWORD.as_bytes()),
        Ok(())
    );
    assert_eq!(*rig.deleted.lock().unwrap(), [DEV_MAC]);
}

/// Everything `body` logs through `tracing`, at every level, on this thread (as `share_invite_api_tests`).
fn capture_logs<T>(body: impl FnOnce() -> T) -> (T, String) {
    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let buffer: Arc<Mutex<Vec<u8>>> = Arc::default();
    let writer = Capture(buffer.clone());
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    let out = body();
    drop(guard);
    let text = String::from_utf8_lossy(&buffer.lock().unwrap()).into_owned();
    (out, text)
}

#[test]
fn no_password_name_or_manifest_text_reaches_the_logs() {
    let mut listings = HashMap::new();
    listings.insert(GLUE_ROOT.to_string(), vec![folder_json(DEV_OWN), folder_json(DEV_MAC)]);
    let rig = rig(
        Answers {
            listings,
            ..Answers::default()
        },
        true,
    );
    let ((), logs) = capture_logs(|| {
        // The capture itself is proven to read this thread's lines.
        tracing::info!("capture-sentinel");
        let _ = rig.engine.backup_devices_for(rig.session);
        let _ = rig
            .engine
            .delete_device_backup_for(rig.session, DEV_MAC, b"a wrong invented password");
        let _ = rig
            .engine
            .delete_device_backup_for(rig.session, DEV_MAC, PASSWORD.as_bytes());
    });
    assert!(logs.contains("capture-sentinel"), "the capture reads this thread");
    let token = account().session_token.clone();
    for secret in [
        PASSWORD,
        "a wrong invented password",
        "Invented PC",
        "Invented Studio",
        token.as_str(),
    ] {
        assert!(!logs.contains(secret), "a log line carries a secret or a name");
    }
    assert_eq!(
        *rig.deleted.lock().unwrap(),
        [DEV_MAC],
        "the calls ran: the capture saw real work"
    );
}
