//! The delete-backup call against the local fake API: the password is proven with real OPAQUE, the server's
//! single-use token is spent on the delete of exactly one folder, and nothing but a whole backup is ever sent.

use super::*;
use crate::backup_protect::OwnBackupFolder;
use crate::state_db::{FileEntry, FileStatus, ItemKind};
use crate::test_api::{ConfirmTokens, FakeApi, OpaqueAccount, Reply, Request};
use beebeeb_core::backup::{MANIFEST_NAME, MSG_DELETE};
use beebeeb_types::backup::SystemRole;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

const PASSWORD: &str = "correct horse battery staple";
const SESSION: &str = "session-of-the-test-account";

const ROOT: &str = "00000000-0000-4000-8000-000000000001";
const DEV_OWN: &str = "00000000-0000-4000-8000-000000000002";
const OWN_PICTURES: &str = "00000000-0000-4000-8000-000000000003";
const OWN_MUSIC: &str = "00000000-0000-4000-8000-000000000004";
const DEV_MAC: &str = "00000000-0000-4000-8000-000000000005";
const MAC_DESKTOP: &str = "00000000-0000-4000-8000-000000000006";
const MAC_FILE: &str = "00000000-0000-4000-8000-000000000007";
const MANIFEST: &str = "00000000-0000-4000-8000-000000000008";
const OUTSIDE: &str = "00000000-0000-4000-8000-000000000009";
const UNKNOWN: &str = "00000000-0000-4000-8000-00000000000a";

/// One registered account for every test here (a registration runs Argon2id once).
fn account() -> Arc<OpaqueAccount> {
    static ACCOUNT: OnceLock<Arc<OpaqueAccount>> = OnceLock::new();
    ACCOUNT
        .get_or_init(|| Arc::new(OpaqueAccount::register("delete-backup@example.test", PASSWORD)))
        .clone()
}

/// The fake server: the two step-up legs and the delete, which it refuses without a token it minted (403
/// `step_up_required`, as the real server answers for a Backups folder). `deleted` is what it deleted.
fn server() -> (FakeApi, Arc<Mutex<Vec<String>>>) {
    let account = account();
    let tokens = Arc::new(ConfirmTokens::default());
    let deleted = Arc::new(Mutex::new(Vec::new()));
    let deleted_in = deleted.clone();
    let api = FakeApi::start(Arc::new(move |request: &Request| {
        if request.bearer() != Some(SESSION) {
            return Reply::error(401, "unauthorized");
        }
        let route = request.route();
        match route.as_str() {
            "POST /api/v1/auth/confirm-opaque-start" => account.login_start(&request.json()),
            "POST /api/v1/auth/confirm-opaque-finish" => {
                if account.login_finish(&request.json()) {
                    Reply::ok(serde_json::json!({ "confirmation_token": tokens.mint(), "expires_at": "soon" }))
                } else {
                    Reply::error(401, "invalid_credentials")
                }
            }
            _ => match route.strip_prefix("DELETE /api/v1/files/") {
                Some(id) if tokens.spend(request) => {
                    deleted_in.lock().unwrap().push(id.to_string());
                    Reply::empty(204)
                }
                Some(_) => Reply::error(403, "step_up_required"),
                None => Reply::error(404, "not_found"),
            },
        }
    }));
    (api, deleted)
}

fn row(db: &StateDb, file_id: &str, path: &str, parent_id: Option<&str>, kind: ItemKind) {
    let entry = FileEntry {
        file_id: file_id.into(),
        path: path.into(),
        status: FileStatus::CloudOnly,
        size_bytes: 3,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: parent_id.map(str::to_string),
        item_kind: kind,
    };
    db.upsert_file_with_contract(&entry, |contract| {
        contract.parent_id = entry.parent_id.clone();
        contract.item_kind = entry.item_kind.clone();
    })
    .unwrap();
}

/// Backups/{GLegion (this device, backs up Pictures now; Music is a past backup), Mac/Desktop/a.txt}, and a file
/// outside.
fn fixture() -> (tempfile::TempDir, StateDb, OwnBackups) {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    row(&db, ROOT, "Backups", None, ItemKind::Folder);
    row(&db, DEV_OWN, "Backups/GLegion", Some(ROOT), ItemKind::Folder);
    let manifest = format!("Backups/GLegion/{MANIFEST_NAME}");
    row(&db, MANIFEST, &manifest, Some(DEV_OWN), ItemKind::File);
    row(
        &db,
        OWN_PICTURES,
        "Backups/GLegion/Pictures",
        Some(DEV_OWN),
        ItemKind::Folder,
    );
    row(&db, OWN_MUSIC, "Backups/GLegion/Music", Some(DEV_OWN), ItemKind::Folder);
    row(&db, DEV_MAC, "Backups/Mac", Some(ROOT), ItemKind::Folder);
    row(&db, MAC_DESKTOP, "Backups/Mac/Desktop", Some(DEV_MAC), ItemKind::Folder);
    row(
        &db,
        MAC_FILE,
        "Backups/Mac/Desktop/a.txt",
        Some(MAC_DESKTOP),
        ItemKind::File,
    );
    row(&db, OUTSIDE, "notes.txt", None, ItemKind::File);
    assert!(db.set_system_role(ROOT, Some(SystemRole::BackupsRoot)).unwrap());
    let own = OwnBackups {
        device_folder: Some("GLegion".into()),
        folders: vec![OwnBackupFolder {
            display_name: "Pictures".into(),
            source_path: PathBuf::from("/home/a/Pictures"),
        }],
    };
    (dir, db, own)
}

fn run(api: &FakeApi, db: &StateDb, own: &OwnBackups, id: &str, password: &str) -> Result<(), DeleteBackupError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = auth_transport::client().unwrap();
    runtime.block_on(delete_backup(
        &api.base,
        &client,
        SESSION,
        db,
        own,
        id,
        password.as_bytes(),
    ))
}

fn deletes(api: &FakeApi) -> Vec<String> {
    api.routes().into_iter().filter(|r| r.starts_with("DELETE ")).collect()
}

#[test]
fn a_wrong_password_is_refused_and_no_delete_is_sent() {
    let (api, deleted) = server();
    let (_dir, db, own) = fixture();
    assert_eq!(
        run(&api, &db, &own, DEV_MAC, "not the password"),
        Err(DeleteBackupError::WrongPassword)
    );
    assert_eq!(run(&api, &db, &own, DEV_MAC, ""), Err(DeleteBackupError::WrongPassword));
    // The one request is the start leg of the first try: OPAQUE fails on this side before the finish leg.
    assert_eq!(api.routes(), ["POST /api/v1/auth/confirm-opaque-start"]);
    assert!(deleted.lock().unwrap().is_empty());
}

#[test]
fn the_right_password_deletes_only_that_devices_backup() {
    let (api, deleted) = server();
    let (_dir, db, own) = fixture();
    assert_eq!(run(&api, &db, &own, DEV_MAC, PASSWORD), Ok(()));
    assert_eq!(*deleted.lock().unwrap(), [DEV_MAC]);
    assert_eq!(
        api.routes(),
        [
            "POST /api/v1/auth/confirm-opaque-start".to_string(),
            "POST /api/v1/auth/confirm-opaque-finish".to_string(),
            format!("DELETE /api/v1/files/{DEV_MAC}"),
        ]
    );
    // Each delete proves the password again: the token is single-use. One backed-up folder of this device that is
    // not backed up now is a backup too.
    assert_eq!(run(&api, &db, &own, OWN_MUSIC, PASSWORD), Ok(()));
    assert_eq!(*deleted.lock().unwrap(), [DEV_MAC, OWN_MUSIC]);
    assert_eq!(deletes(&api).len(), 2);
}

#[test]
fn the_root_and_everything_that_is_not_a_whole_backup_never_reach_the_server() {
    let (api, deleted) = server();
    let (_dir, db, own) = fixture();
    assert_eq!(
        run(&api, &db, &own, ROOT, PASSWORD),
        Err(DeleteBackupError::Refused(MSG_DELETE.to_string()))
    );
    // This device's own backup of a folder it backs up now: change the original instead.
    assert!(matches!(
        run(&api, &db, &own, OWN_PICTURES, PASSWORD),
        Err(DeleteBackupError::Refused(words)) if words.contains("/home/a/Pictures")
    ));
    for (id, want) in [
        (OUTSIDE, DeleteBackupError::NotABackup),
        (MAC_FILE, DeleteBackupError::NotABackup),
        (
            MANIFEST,
            DeleteBackupError::Refused(beebeeb_core::backup::MSG_READ_ONLY.to_string()),
        ),
        (UNKNOWN, DeleteBackupError::NotFound),
        ("../auth/logout", DeleteBackupError::NotFound),
        ("00000000-0000-4000-8000-00000000000A", DeleteBackupError::NotFound),
    ] {
        assert_eq!(run(&api, &db, &own, id, PASSWORD), Err(want), "{id}");
    }
    assert_eq!(api.routes(), Vec::<String>::new());
    assert!(deleted.lock().unwrap().is_empty());
}

#[test]
fn a_server_that_refuses_the_delete_is_an_error_not_a_success() {
    let (_dir, db, own) = fixture();
    let account = account();
    let api = FakeApi::start(Arc::new(move |request: &Request| match request.route().as_str() {
        "POST /api/v1/auth/confirm-opaque-start" => account.login_start(&request.json()),
        "POST /api/v1/auth/confirm-opaque-finish" => {
            Reply::ok(serde_json::json!({ "confirmation_token": "t", "expires_at": "soon" }))
        }
        _ => Reply::error(403, "step_up_required"),
    }));
    assert_eq!(
        run(&api, &db, &own, DEV_MAC, PASSWORD),
        Err(DeleteBackupError::Failed("server"))
    );
}
